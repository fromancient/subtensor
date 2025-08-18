# Subtensor Native Runtime Simulation

This document describes how to build and run the native runtime-based "no-tx" simulator for Subtensor.

## Overview

The simulator creates blocks with only inherent extrinsics (no user transactions) and outputs per-block metrics in JSON Lines (JSONL) format. This is useful for analyzing emission patterns and network behavior without the overhead of user transactions.

## Base Commit

- **Hash**: `312c0be95`
- **Date**: March 2024
- **Tag**: `v3.2.4`

## Building

### Prerequisites

1. Ensure Rust toolchain is installed (use `rust-toolchain.toml` if present):
   ```bash
   rustup show
   ```

2. Build with the simulation feature flag:
   ```bash
   cargo build --release --features sim-json
   ```

## Running the Simulator

### Basic Usage

```bash
target/release/subtensor-sim \
  --start-block 1234567 \
  --horizon-blocks 600 \
  --json-out ./no_tx_600.jsonl \
  --progress-every 100
```

### Command Line Options

- `--start-block <BLOCK>`: Start block number or hash (hex, e.g., "0x1234...")
- `--horizon-blocks <N>`: Number of blocks to simulate
- `--horizon-days <D>`: Number of days to simulate (7200 blocks/day)
- `--json-out <PATH>`: Output JSON file path (required)
- `--chain <CHAIN>`: Chain specification (default: "local")
- `--feature-inherent-only`: Ensure no user transactions (default: true)
- `--progress-every <N>`: Log progress every N blocks (default: 100)
- `--flush-every <N>`: Flush output every N blocks (default: 100)

### Examples

#### Simulate 2 hours worth of blocks (600 blocks)
```bash
target/release/subtensor-sim \
  --start-block 1234567 \
  --horizon-blocks 600 \
  --json-out ./no_tx_600.jsonl \
  --progress-every 100
```

#### Simulate 1 day worth of blocks
```bash
target/release/subtensor-sim \
  --start-block 1234567 \
  --horizon-days 1 \
  --json-out ./no_tx_1day.jsonl \
  --progress-every 1000
```

#### Start from current best block
```bash
target/release/subtensor-sim \
  --horizon-blocks 100 \
  --json-out ./no_tx_100.jsonl \
  --progress-every 10
```

#### Use different chain specification
```bash
target/release/subtensor-sim \
  --chain dev \
  --horizon-blocks 100 \
  --json-out ./no_tx_dev_100.jsonl
```

## Output Format

The simulator outputs JSON Lines (JSONL) format, with one JSON object per line:

```json
{
  "block_number": 1234567,
  "state_root": "0x1234567890abcdef...",
  "timestamp_ms": 1723800000000,
  "subnets": [
    {
      "netuid": 1,
      "stake_total": "123456789000000000",
      "emission_per_block": "900000000000000",
      "participants": 42
    }
  ]
}
```

### Field Descriptions

- `block_number`: The block number (u64)
- `state_root`: The state root hash as hex string
- `timestamp_ms`: Block timestamp in milliseconds
- `subnets`: Array of subnet metrics
  - `netuid`: Subnet identifier (u16)
  - `stake_total`: Total stake on subnet as string (u128)
  - `emission_per_block`: Emission per block as string (u128)
  - `participants`: Number of participants (u32)

## Performance

- Target: ≥ 5k blocks/min on modern dev laptop
- Uses buffered I/O for efficient file writing
- Periodic flushing to ensure data persistence
- Graceful shutdown on Ctrl+C

## Validation

### Manual Validation

1. Run simulator for ~100 blocks:
   ```bash
   target/release/subtensor-sim \
     --horizon-blocks 100 \
     --json-out ./test_validation.jsonl \
     --progress-every 10
   ```

2. Verify output:
   - File grows by ~100 lines
   - `block_number` increases monotonically
   - `state_root` changes (unless deterministic identical state)
   - No user extrinsics included

### Automated Tests

```bash
# Build tests
cargo test -p subtensor-sim

# Run linter
cargo clippy -p subtensor-sim -D warnings

# Format check
cargo fmt --all
```

## Troubleshooting

### Common Issues

1. **Feature flag not enabled**: Ensure `--features sim-json` is used during build
2. **Invalid block hash**: Use proper hex format with "0x" prefix
3. **File permission errors**: Ensure write permissions for output directory
4. **Memory issues**: Reduce `--flush-every` for large simulations

### Debug Mode

Enable debug logging:
```bash
RUST_LOG=debug target/release/subtensor-sim \
  --horizon-blocks 10 \
  --json-out ./debug_test.jsonl
```

## Architecture

### Components

1. **YieldsApi**: Runtime API for collecting block metrics
2. **Simulator Binary**: CLI tool for running simulations
3. **Block Production**: Native runtime execution without WASM
4. **JSONL Output**: Efficient append-only file format

### Feature Gates

All simulation code is behind the `sim-json` feature flag:
- Runtime API implementation
- Simulation binary
- No impact on production builds

## Security

- No network writes beyond reading local node state
- Read-only access to blockchain data
- All simulation code paths under feature flag
- No production impact unless explicitly enabled
