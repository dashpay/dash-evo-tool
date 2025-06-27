# SPV Sync Issue Analysis - Dash Mainnet

## Root Cause

The SPV sync is failing because:

1. **Genesis Block Issue**: The SPV client is trying to sync from the genesis block (height 0)
2. **Mainnet Node Limitations**: Most Dash mainnet nodes only keep recent blocks (approximately from height 2.29M onwards) and have pruned older blocks
3. **Protocol Mismatch**: When the client requests headers from height 0, mainnet nodes can't provide them because they don't have that data

## What's Happening in Detail

Looking at the sync_state.json, we can see:
- `chain_tip.height`: 0 (genesis)
- `sync_progress.header_height`: 0
- `checkpoints`: [] (empty)

This means the SPV client is starting from the very beginning of the blockchain, but mainnet nodes can't serve blocks that old.

## Why Height 2000?

There's nothing special about height 2000 in Dash mainnet. The issue is that the sync process requests headers in batches of 2000, so:
- First request: headers 0-2000 
- The peer can't provide these (too old)
- Sync stalls waiting for a response that will never come

## The Solution

The code attempts to set a checkpoint at height 2,290,000 but it's not being applied correctly because:

1. The `initialize_client()` method only creates a checkpoint if sync_state.json doesn't exist
2. The `set_recent_checkpoint()` method tries to modify the state after the client has already loaded it
3. The checkpoint needs to be in place BEFORE the SPV client starts

## Fix Implementation

I've created `reset_spv_with_checkpoint.sh` which:
1. Backs up the existing sync state
2. Creates a new sync_state.json with checkpoint at height 2,290,000
3. Clears the headers directory for a clean start

## To Apply the Fix

1. Stop the Dash Evo Tool if it's running
2. Run: `./reset_spv_with_checkpoint.sh`
3. Start Dash Evo Tool
4. Click "Initialize SPV"
5. Click "Start SPV Sync"

The sync should now start from height 2,290,000 and successfully sync with mainnet nodes.

## Alternative Solutions

1. **Use Testnet**: Testnet nodes typically maintain full blockchain history
2. **Find Archive Nodes**: Some nodes maintain full history but they're rare
3. **Implement Dynamic Checkpoint Discovery**: Query the peer for its earliest block and start from there

## Code Improvements Needed

The SPV manager should:
1. Detect when sync is stuck at genesis on mainnet
2. Automatically apply a recent checkpoint before starting
3. Handle the case where peers don't have old blocks
4. Provide better error messages explaining the issue