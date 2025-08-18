use clap::Parser;
use futures::executor::block_on;
use log::{error, info, warn};
use serde_json::json;
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::PathBuf,
    process,
    sync::Arc,
    time::Duration,
};
use tokio::signal;

use sc_cli::{ChainSpec, CliConfiguration, DatabaseParams, ImportParams, KeystoreParams, NetworkParams, NodeKeyParams, PruningParams, Result as CliResult, SharedParams, SubstrateCli};
use sc_service::{config::Configuration, ChainSpec as ChainSpecTrait};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_consensus::BlockImport;
use sp_core::{crypto::Pair, sr25519};
use sp_inherents::InherentDataProviders;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT, Zero};

use node_subtensor_runtime::{Block, Hash, Header};
use subtensor_custom_rpc_runtime_api::YieldsApi;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Start block number or hash (hex)
    #[arg(long, value_name = "BLOCK")]
    start_block: Option<String>,

    /// Number of blocks to simulate
    #[arg(long, value_name = "N")]
    horizon_blocks: Option<u64>,

    /// Number of days to simulate (7200 blocks/day)
    #[arg(long, value_name = "D")]
    horizon_days: Option<u64>,

    /// Output JSON file path
    #[arg(long, value_name = "PATH")]
    json_out: PathBuf,

    /// Chain specification
    #[arg(long, value_name = "CHAIN", default_value = "local")]
    chain: String,

    /// Ensure no user transactions are included
    #[arg(long, default_value = "true")]
    feature_inherent_only: bool,

    /// Log progress every N blocks
    #[arg(long, value_name = "N", default_value = "100")]
    progress_every: u64,

    /// Flush output every N blocks
    #[arg(long, value_name = "N", default_value = "100")]
    flush_every: u64,

    #[clap(flatten)]
    shared_params: SharedParams,

    #[clap(flatten)]
    import_params: ImportParams,

    #[clap(flatten)]
    network_params: NetworkParams,

    #[clap(flatten)]
    keystore_params: KeystoreParams,

    #[clap(flatten)]
    node_key_params: NodeKeyParams,

    #[clap(flatten)]
    database_params: DatabaseParams,

    #[clap(flatten)]
    pruning_params: PruningParams,
}

impl Cli {
    fn load_spec(&self) -> Result<Box<dyn ChainSpecTrait>, String> {
        Ok(match self.chain.as_str() {
            "dev" => Box::new(chain_spec::devnet::devnet_config()?),
            "local" => Box::new(chain_spec::localnet::localnet_config(false)?),
            "finney" => Box::new(chain_spec::finney::finney_mainnet_config()?),
            "test_finney" => Box::new(chain_spec::testnet::finney_testnet_config()?),
            path => Box::new(chain_spec::ChainSpec::from_json_file(
                std::path::PathBuf::from(path),
            )?),
        })
    }
}

impl CliConfiguration for Cli {
    fn shared_params(&self) -> &SharedParams {
        &self.shared_params
    }

    fn import_params(&self) -> Option<&ImportParams> {
        Some(&self.import_params)
    }

    fn network_params(&self) -> Option<&NetworkParams> {
        Some(&self.network_params)
    }

    fn keystore_params(&self) -> Option<&KeystoreParams> {
        Some(&self.keystore_params)
    }

    fn node_key_params(&self) -> Option<&NodeKeyParams> {
        Some(&self.node_key_params)
    }

    fn database_params(&self) -> Option<&DatabaseParams> {
        Some(&self.database_params)
    }

    fn pruning_params(&self) -> Option<&PruningParams> {
        Some(&self.pruning_params)
    }
}

struct Simulator {
    cli: Cli,
    config: Configuration,
    client: Arc<sc_client::Client<sc_client::LocalCallExecutor<node_subtensor_runtime::Block, sc_client::LocalBackend<node_subtensor_runtime::Block>>, node_subtensor_runtime::Block, sc_client::LocalCallExecutor<node_subtensor_runtime::Block, sc_client::LocalBackend<node_subtensor_runtime::Block>>>>,
    output_file: BufWriter<std::fs::File>,
    block_count: u64,
}

impl Simulator {
    fn new(cli: Cli) -> Result<Self, Box<dyn std::error::Error>> {
        // Initialize logging
        env_logger::init();

        // Load chain spec
        let chain_spec = cli.load_spec()?;

        // Create service configuration
        let config = sc_cli::create_config_with_db_path(
            &cli,
            &chain_spec,
            &cli.database_params,
        )?;

        // Create client
        let client = Arc::new(sc_client::Client::new(
            config.clone(),
            sc_client::LocalCallExecutor::new(
                config.clone(),
                sc_client::LocalBackend::new(config.clone())?,
            ),
        )?);

        // Open output file
        let output_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&cli.json_out)?;
        let output_file = BufWriter::new(output_file);

        Ok(Self {
            cli,
            config,
            client,
            output_file,
            block_count: 0,
        })
    }

    fn get_start_block(&self) -> Result<Hash, Box<dyn std::error::Error>> {
        match &self.cli.start_block {
            Some(block) => {
                if block.starts_with("0x") {
                    // Parse as hex hash
                    let hash_bytes = hex::decode(&block[2..])?;
                    if hash_bytes.len() != 32 {
                        return Err("Invalid hash length".into());
                    }
                    let mut hash = [0u8; 32];
                    hash.copy_from_slice(&hash_bytes);
                    Ok(Hash::from(hash))
                } else {
                    // Parse as block number
                    let block_num: u64 = block.parse()?;
                    let hash = self.client.hash(block_num.into())?
                        .ok_or("Block not found")?;
                    Ok(hash)
                }
            }
            None => {
                // Use current best block
                let best_hash = self.client.info().best_hash;
                Ok(best_hash)
            }
        }
    }

    fn get_horizon_blocks(&self) -> Result<u64, Box<dyn std::error::Error>> {
        match (self.cli.horizon_blocks, self.cli.horizon_days) {
            (Some(blocks), None) => Ok(blocks),
            (None, Some(days)) => Ok(days * 7200), // 7200 blocks per day
            (Some(_), Some(_)) => Err("Cannot specify both --horizon-blocks and --horizon-days".into()),
            (None, None) => Err("Must specify either --horizon-blocks or --horizon-days".into()),
        }
    }

    fn write_metrics(&mut self, metrics: subtensor_custom_rpc_runtime_api::BlockMetrics) -> Result<(), Box<dyn std::error::Error>> {
        let json_line = json!({
            "block_number": metrics.block_number,
            "state_root": format!("0x{}", hex::encode(metrics.state_root)),
            "timestamp_ms": metrics.timestamp_ms,
            "subnets": metrics.subnets.iter().map(|s| json!({
                "netuid": s.netuid,
                "stake_total": s.stake_total.to_string(),
                "emission_per_block": s.emission_per_block.to_string(),
                "participants": s.participants,
            })).collect::<Vec<_>>(),
        });

        writeln!(self.output_file, "{}", serde_json::to_string(&json_line)?)?;
        self.block_count += 1;

        // Flush periodically
        if self.block_count % self.cli.flush_every == 0 {
            self.output_file.flush()?;
        }

        Ok(())
    }

    fn step_block(&mut self, parent_hash: Hash) -> Result<Hash, Box<dyn std::error::Error>> {
        // Get parent header
        let parent_header = self.client.header(parent_hash)?
            .ok_or("Parent header not found")?;

        // Create inherent data
        let inherent_data_providers = InherentDataProviders::new();
        let inherent_data = inherent_data_providers.create_inherent_data()?;

        // Create block builder
        let mut block_builder = self.client.new_block_at(parent_hash, Default::default(), false)?;

        // Add inherent extrinsics only (no user transactions)
        let inherent_extrinsics = inherent_data.create_extrinsics();
        for extrinsic in inherent_extrinsics {
            block_builder.push(extrinsic)?;
        }

        // Build the block
        let (block, _) = block_builder.build()?;

        // Import the block
        let import_result = block_on(self.client.import_block(
            Default::default(),
            block.clone(),
        ))?;

        if let Err(e) = import_result {
            return Err(format!("Failed to import block: {:?}", e).into());
        }

        // Get metrics from the new block
        let metrics = self.client.runtime_api()
            .block_metrics(block.header.hash())?;

        // Write metrics to file
        self.write_metrics(metrics)?;

        Ok(block.header.hash())
    }

    fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let start_hash = self.get_start_block()?;
        let horizon_blocks = self.get_horizon_blocks()?;

        info!("Starting simulation from block {:?}", start_hash);
        info!("Simulating {} blocks", horizon_blocks);
        info!("Output file: {:?}", self.cli.json_out);

        let mut current_hash = start_hash;

        // Set up signal handling for graceful shutdown
        let mut shutdown_signal = signal::ctrl_c();

        for i in 0..horizon_blocks {
            // Check for shutdown signal
            if shutdown_signal.try_recv().is_ok() {
                info!("Received shutdown signal, stopping simulation");
                break;
            }

            // Step to next block
            current_hash = self.step_block(current_hash)?;

            // Log progress
            if (i + 1) % self.cli.progress_every == 0 {
                info!("Processed {} blocks", i + 1);
            }
        }

        // Final flush
        self.output_file.flush()?;
        info!("Simulation completed. Processed {} blocks", self.block_count);

        Ok(())
    }
}

fn main() -> CliResult<()> {
    let cli = Cli::parse();

    // Validate arguments
    if cli.horizon_blocks.is_some() && cli.horizon_days.is_some() {
        error!("Cannot specify both --horizon-blocks and --horizon-days");
        process::exit(1);
    }

    if cli.horizon_blocks.is_none() && cli.horizon_days.is_none() {
        error!("Must specify either --horizon-blocks or --horizon-days");
        process::exit(1);
    }

    // Create and run simulator
    match Simulator::new(cli) {
        Ok(mut simulator) => {
            if let Err(e) = simulator.run() {
                error!("Simulation failed: {}", e);
                process::exit(1);
            }
        }
        Err(e) => {
            error!("Failed to create simulator: {}", e);
            process::exit(1);
        }
    }

    Ok(())
}
