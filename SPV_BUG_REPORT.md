# SPV Header Sync Bug Report

## Issue Summary

After successfully syncing headers 0-2000, the dash-spv client sends a GetHeaders request with an incorrect locator hash that never appeared in the previous sync process.

## Details

### Expected Behavior
- After storing headers 0-2000, the next GetHeaders request should use the hash of block 2000 (or a recent block) as the locator
- Expected hash at height 2000: `0x0000001e340a0e6fb510d53c2316e8b0f3e27b2e6ee0002a23f228cfec723b06`

### Actual Behavior
- GetHeaders request uses hash: `0x00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749`
- This hash never appeared in any previous log messages
- This hash doesn't correspond to any block height we've synced

### Log Evidence

```
[2025-01-26T17:39:39Z DEBUG dash_spv::tasks::blockdata] Stored headers 1600..=2000
[2025-01-26T17:39:39Z DEBUG dash_spv::blockdata::chainstore] Last stored header: height=2000, hash=0x0000001e340a0e6fb510d53c2316e8b0f3e27b2e6ee0002a23f228cfec723b06
[2025-01-26T17:39:39Z DEBUG dash_spv::spv] Send GetHeaders { locator_hashes: [0x00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749], hash_stop: 0x0000000000000000000000000000000000000000000000000000000000000000 }
```

## Analysis

1. **The hash is not from our sync process**: We've verified that this hash never appears in any headers we received during the 0-2000 sync.

2. **The hash format is valid**: It has the correct number of leading zeros for a Dash block hash, suggesting it's not random garbage data.

3. **Timing**: This happens immediately after storing headers 1600-2000, suggesting the issue is in how dash-spv generates the locator hash after a batch of headers is processed.

## Possible Causes

1. **Bug in locator hash generation**: The dash-spv library might be incorrectly calculating which block hash to use as a locator.

2. **Off-by-one error**: The code might be trying to access a block at an incorrect height/index.

3. **State corruption**: The internal state tracking which headers have been synced might be corrupted.

4. **Wrong chain/fork**: The hash might be from a different chain or a reorg that wasn't properly handled.

## Impact

This bug prevents the SPV client from continuing to sync headers beyond height 2000, as peers likely don't recognize the invalid locator hash and don't respond with more headers.

## Workaround Suggestions

1. **Clear SPV state**: Force a complete resync from genesis (though this just delays hitting the same bug).

2. **Implement retry logic**: Detect when sync stalls and try with different locator hashes.

3. **Patch dash-spv**: Add logging to trace where this hash is generated and fix the root cause.

## Reproduction Steps

1. Start dash-spv client on testnet
2. Begin header sync from genesis
3. Wait for headers 0-2000 to sync
4. Observe the GetHeaders request with incorrect locator hash
5. Notice that no more headers are received after this point

## Environment

- dash-spv: Local path dependency (../Dash/rust-dashcore/dash-spv)
- Network: Testnet
- Platform: macOS (Darwin 24.5.0)

## Recommendation

This appears to be a bug in the dash-spv library that should be reported upstream. The locator hash generation logic needs to be reviewed and fixed to ensure it uses the correct block hash from the actually synced headers.