use std::collections::HashMap;
use dashcore::{BlockHash, block::Header as BlockHeader};
use dashcore_hashes::Hash;

/// Test to reproduce the block locator bug where after syncing 2000 headers,
/// the wrong hash is sent in GetHeaders message
fn main() {
    println!("Testing Block Locator Bug");
    println!("========================\n");
    
    // The mysterious hash that appears in GetHeaders after syncing 2000 headers
    let mystery_hash = "00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749";
    let expected_hash = "0000001e340a0e6fb510d53c2316e8b0f3e27b2e6ee0002a23f228cfec723b06";
    
    println!("Expected hash at height 2000: {}", expected_hash);
    println!("Actual hash in GetHeaders:    {}", mystery_hash);
    println!();
    
    // Simulate block locator algorithm from height 2000
    let tip_height = 2000;
    let mut heights = Vec::new();
    let mut current_height = tip_height;
    let mut step = 1u32;
    
    // Build the locator (same algorithm as in block_locator.rs)
    loop {
        heights.push(current_height);
        
        if current_height == 0 {
            break;
        }
        
        // Calculate next height
        if current_height > step {
            current_height = current_height.saturating_sub(step);
        } else {
            current_height = 0;
        }
        
        // After 10 blocks, start exponential backoff
        if heights.len() > 10 {
            step *= 2;
        }
        
        // Limit locator size
        if heights.len() >= 100 {
            if current_height > 0 {
                heights.push(0); // Add genesis
            }
            break;
        }
    }
    
    println!("Block locator heights from tip 2000:");
    for (i, h) in heights.iter().enumerate().take(20) {
        println!("  [{}] height {}", i, h);
    }
    println!("  ... ({} total heights)", heights.len());
    println!();
    
    // Check segment boundaries
    const HEADERS_PER_SEGMENT: u32 = 50_000;
    println!("Segment analysis:");
    println!("  Height 2000 is in segment {}, offset {}", 
        2000 / HEADERS_PER_SEGMENT,
        2000 % HEADERS_PER_SEGMENT
    );
    
    // Check if the mystery hash could be from reading wrong offset
    // The hash pattern suggests it might be reading from wrong memory location
    println!("\nPossible causes:");
    println!("1. Off-by-one error when retrieving header at tip");
    println!("2. Reading from uninitialized or corrupted memory");
    println!("3. Segment boundary issue (but 2000 is well within segment 0)");
    println!("4. Race condition between storage write and read");
    println!("5. Block locator using wrong height or storage returning wrong header");
    
    // Try to decode the mystery hash pattern
    let mystery_bytes = hex::decode(mystery_hash).unwrap();
    let mut reversed = mystery_bytes.clone();
    reversed.reverse();
    
    println!("\nMystery hash analysis:");
    println!("  First 4 bytes: {:02x} {:02x} {:02x} {:02x}", 
        reversed[0], reversed[1], reversed[2], reversed[3]);
    println!("  Pattern: Starts with 00000014 (very low hash, valid PoW)");
    println!("  This looks like a real block hash, not random data");
    
    // Check if this could be from wrong endianness or byte order
    println!("\nThe mystery hash is likely:");
    println!("- A real block hash from the chain");
    println!("- But NOT the hash at height 2000");
    println!("- Possibly from reading wrong height or memory location");
    println!("- Need to add debug logging to storage retrieval");
}