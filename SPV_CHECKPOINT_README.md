# SPV Checkpoint Implementation for Dash Mainnet

## Overview

This implementation adds checkpoint support to the SPV manager to work around the issue where mainnet Dash nodes typically only serve blocks from around height 2,290,000 onwards, not from genesis (height 0).

## Problem

When dash-spv tries to sync on mainnet, it requests headers starting from genesis (block 0). However, most mainnet nodes have pruned old blocks and only serve recent history (typically from around height 2,290,000). This causes the SPV sync to fail with no headers being downloaded.

## Solution

The implementation adds checkpoint functionality that:

1. **Automatically creates a checkpoint** for mainnet at height 2,290,000 when initializing SPV for the first time
2. **Modifies sync_state.json** to start syncing from the checkpoint instead of genesis
3. **Provides a reset script** to manually apply the checkpoint if needed

## Implementation Details

### Files Modified

1. **src/components/spv_manager.rs**
   - Added `set_recent_checkpoint()` method to inject checkpoint into sync state
   - Modified `initialize_client()` to create initial sync_state.json with checkpoint for mainnet
   - Updated logging to indicate checkpoint usage
   - Added checkpoint detection in progress monitoring

2. **src/bin/inject_checkpoint.rs**
   - Utility to manually inject checkpoint into existing sync_state.json

3. **reset_spv_with_checkpoint.sh**
   - Shell script to reset SPV data and apply checkpoint for mainnet

### Checkpoint Details

- **Height**: 2,290,000
- **Hash**: `00000000000000158a0aa3adfd733a2e58bd1d78c88a5ecfe2a51d37fc90d844`
- **Time**: 1734883200 (approximate timestamp for December 2024)

## Usage

### Automatic (Recommended)

Simply start SPV on mainnet - the checkpoint will be automatically applied on first run:

1. Start Dash Evo Tool
2. Switch to Mainnet
3. Click "Start SPV"
4. Click "Initialize SPV"
5. Click "Start SPV Sync"

### Manual Reset

If you need to reset SPV data and apply the checkpoint:

```bash
./reset_spv_with_checkpoint.sh
```

### Manual Checkpoint Injection

To manually inject a checkpoint into an existing sync state:

```bash
cargo run --bin inject_checkpoint
```

## How It Works

1. When SPV storage is first created for mainnet, the system creates a `sync_state.json` file with:
   - `chain_tip` set to the checkpoint height/hash
   - `sync_progress.header_height` set to checkpoint height
   - `checkpoints` array containing the checkpoint data

2. When dash-spv starts, it reads this state and begins requesting headers from the checkpoint height instead of genesis

3. Since mainnet nodes can serve blocks from height 2,290,000, the sync proceeds successfully

## Limitations

- This is a workaround since dash-spv doesn't expose a direct API for setting checkpoints
- The checkpoint must be applied before SPV client starts for the first time
- If sync fails, you may need to reset SPV data and let the checkpoint be reapplied

## Future Improvements

Ideally, dash-spv should:
1. Support checkpoint configuration in its API
2. Automatically detect when peers can't serve genesis and request from a recent height
3. Include built-in checkpoints for mainnet

Until then, this workaround enables SPV functionality on mainnet.