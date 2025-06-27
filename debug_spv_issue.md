# SPV Mystery Hash Debug Analysis

## Problem Summary
After syncing headers 0-1999 (2000 headers), dash-spv sends a GetHeaders message with hash:
`00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749`

But this hash:
- Does NOT exist in the stored headers
- Is NOT the expected hash at height 2000
- Appears to be generated incorrectly

## Evidence
1. Storage contains exactly 2000 headers (heights 0-1999)
2. The tip is at height 1999 with hash: `4857b851c0505e51e1b88005d7fd4778515b16d2ca2cf756fa9c8c59c19848d6`
3. The mystery hash is not found anywhere in the stored headers

## Likely Causes

### 1. Off-by-one error in request_headers
In `headers.rs` line 151:
```rust
let last_header = headers.last().unwrap();
self.request_headers(network, Some(last_header.block_hash()), storage).await?;
```
This correctly uses the last header's hash, so not the issue here.

### 2. Memory corruption or uninitialized data
The mystery hash starts with valid PoW (00000014...), suggesting it's a real block hash from somewhere else, not random data.

### 3. Block locator algorithm issue
The block locator might be reading from wrong memory or calculating wrong indices.

### 4. Storage retrieval bug
When building the block locator, `get_header()` might be returning wrong data or reading from wrong offset.

## Recommendations for Debugging

Add logging at these critical points in dash-spv:

1. **In `headers.rs` after receiving headers:**
   ```rust
   tracing::info!("Last header received: height={}, hash={}", 
       next_height - 1, headers.last().unwrap().block_hash());
   ```

2. **In `block_locator.rs` when building locator:**
   ```rust
   tracing::info!("Building locator from tip height {}", tip_height);
   if let Some(header) = storage.get_header(current_height).await? {
       tracing::info!("Added height {} with hash {} to locator", 
           current_height, header.block_hash());
   }
   ```

3. **In `storage/disk.rs` get_header method:**
   ```rust
   tracing::info!("get_header called for height {}", height);
   if let Some(header) = /* retrieve header */ {
       tracing::info!("Returning header with hash {}", header.block_hash());
   }
   ```

4. **Before sending GetHeaders:**
   ```rust
   tracing::info!("Sending GetHeaders with {} locator hashes", locator_hashes.len());
   for (i, hash) in locator_hashes.iter().enumerate().take(5) {
       tracing::info!("  Locator[{}]: {}", i, hash);
   }
   ```

## Theory
The mystery hash might be:
- A hash from a different network (testnet data mixed with mainnet?)
- A hash from a previous sync attempt that wasn't cleaned up
- Result of reading beyond the valid data in a segment
- A sentinel or placeholder value that's being misinterpreted

## Next Steps
1. Add the debug logging above to dash-spv
2. Run a fresh sync and capture the logs
3. The logs should reveal exactly where the mystery hash is coming from