# dash-spv Bug Analysis: Incorrect Hash in GetHeaders After 2000 Headers

## Issue Summary

After syncing exactly 2000 headers (heights 0-2000), dash-spv sends a GetHeaders message with an incorrect block locator hash:
- **Incorrect hash sent**: `00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749`
- **Expected hash (height 2000)**: `0000001e340a0e6fb510d53c2316e8b0f3e27b2e6ee0002a23f228cfec723b06`

This causes peers to not recognize the hash and fail to send the next batch of headers, stalling the sync process.

## Root Cause Analysis

The bug is likely in the block locator generation algorithm in dash-spv. Block locators are used in the Bitcoin/Dash P2P protocol to efficiently communicate what headers a node has. The algorithm typically works as follows:

1. Start with the tip (highest block)
2. Add recent blocks at small intervals (every block for the first 10)
3. Exponentially increase the interval as you go back in the chain
4. Always include the genesis block as the last entry

The bug appears to be generating an invalid hash that doesn't correspond to any actual block at the expected height.

## Where to Look in dash-spv

Based on the codebase structure, the bug is likely in one of these areas:

1. **Block Locator Generation**:
   - Look for functions like `get_block_locator()`, `build_block_locator()`, or similar
   - Check the logic that selects which block hashes to include
   - Pay special attention to index calculations around the 2000 mark

2. **GetHeaders Message Construction**:
   - Find where `GetHeaders` messages are created
   - Check how the block locator is passed to the message
   - Look for any special handling when exactly 2000 headers have been synced

3. **Header Storage/Retrieval**:
   - Check if there's an off-by-one error when retrieving headers from storage
   - Verify that the header at index 1999 (height 2000) is correctly stored and retrieved

## Potential Fixes

1. **Index Calculation Error**: 
   ```rust
   // Incorrect (example)
   let index = height / 2000; // This might cause issues at exactly 2000
   
   // Correct
   let index = (height - 1) / 2000; // Proper zero-based indexing
   ```

2. **Block Locator Algorithm Issue**:
   ```rust
   // The algorithm might have a special case at 2000 that's buggy
   // Check for any hardcoded values or special handling around 2000
   ```

3. **Hash Retrieval Error**:
   ```rust
   // Ensure the correct header is being retrieved
   // Check if height vs index confusion exists
   ```

## Testing the Fix

1. Clear SPV storage
2. Start syncing from genesis
3. Monitor when exactly 2000 headers are synced
4. Check the GetHeaders message sent after that point
5. Verify it contains the correct hash for height 2000

## Temporary Workaround

The current workaround in dash-evo-tool is to use a checkpoint at height 2,290,000 for mainnet to skip the problematic genesis sync entirely. This works but doesn't solve the underlying issue.

## References

- The debug tool at `src/bin/debug_spv_headers.rs` was created to help diagnose this issue
- The issue manifests after syncing headers 0-2000 (2001 headers total)
- The mysterious hash doesn't appear in any of the actual headers received