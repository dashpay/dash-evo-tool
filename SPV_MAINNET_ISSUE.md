# SPV Mainnet Sync Issue

## Problem Summary

The SPV sync fails on Mainnet because of a fundamental mismatch between what the dash-spv library requests and what mainnet nodes provide:

1. **dash-spv behavior**: Requests headers starting from genesis block (height 0)
2. **Mainnet nodes behavior**: Only serve recent blocks (approximately from height 2,290,000)
3. **Result**: Nodes ignore the genesis header requests, causing timeouts

## Technical Details

### What's Happening

When you start SPV sync, the logs show:
```
Sent message to 104.200.24.196:9999: GetHeaders(GetHeadersMessage { 
  version: 70237, 
  locator_hashes: [0x00000ffd590b1485b3caadc19b22e6379c733355108f107a430458cdf3407ab6], 
  stop_hash: 0x0000000000000000000000000000000000000000000000000000000000000000 
})
```

That hash `0x00000ffd590b...` is the Dash genesis block. Mainnet nodes don't have blocks that old in their serving window.

### Why This Happens

1. Most Dash mainnet nodes prune old blocks to save disk space
2. They typically only keep recent blocks (last ~50,000 blocks)
3. The dash-spv library doesn't have a mechanism to start from a recent checkpoint
4. The library always tries to sync from genesis, which mainnet nodes can't provide

## Workarounds

### Option 1: Use Testnet (Recommended)

Testnet nodes typically serve the full blockchain history, so SPV sync works properly:

1. Switch to Testnet network in the app
2. Initialize and start SPV sync
3. Headers will sync successfully

### Option 2: Wait for Library Update

The dash-spv library needs to be updated to support:
- Starting sync from recent checkpoints
- Requesting headers that mainnet nodes actually have
- Similar to how Bitcoin SPV wallets use checkpoints

## Code Changes Made

1. Added better error messages when mainnet sync fails
2. Added more testnet peers for reliability
3. Created diagnostic logging to identify the issue
4. Added placeholder for future checkpoint implementation

## Testing

Run the test script to see the issue:
```bash
./test_spv_mainnet_issue.sh
```

This will monitor the logs and highlight:
- Yellow: Timeout messages
- Red: Error explanations
- Green: Successful header sync (only on testnet)

## Future Solution

The proper fix requires changes to the dash-spv library to:

1. Accept a starting checkpoint (block height + hash)
2. Build the locator from that checkpoint instead of genesis
3. Similar to how the colleague's demo might handle recent blocks

Until then, use Testnet for SPV functionality.