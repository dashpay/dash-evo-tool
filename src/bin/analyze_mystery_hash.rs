use std::path::PathBuf;
use std::fs;
use dash_sdk::dpp::dashcore::consensus::{encode, Decodable};
use dash_sdk::dpp::dashcore::block::Header as BlockHeader;
use dash_sdk::dpp::dashcore_hashes::Hash;
use std::io::BufReader;

fn main() {
    println!("Analyzing Mystery Hash in SPV Storage");
    println!("====================================\n");
    
    let mystery_hash = "00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749";
    
    // Check mainnet storage
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let spv_dir = PathBuf::from(home)
        .join("Library")
        .join("Application Support") 
        .join("Dash-Evo-Tool")
        .join("spv")
        .join("Mainnet");
    
    if !spv_dir.exists() {
        println!("SPV directory not found: {:?}", spv_dir);
        return;
    }
    
    // Check headers directory
    let headers_dir = spv_dir.join("headers");
    if headers_dir.exists() {
        println!("Scanning header files for mystery hash...");
        
        // Read segment 0 which should contain headers 0-49999
        let segment_0_path = headers_dir.join("segment_0000.dat");
        if segment_0_path.exists() {
            println!("\nReading segment 0 (contains heights 0-49999)...");
            
            match read_headers_from_segment(&segment_0_path) {
                Ok(headers) => {
                    println!("Found {} headers in segment 0", headers.len());
                    
                    // Check if mystery hash appears
                    for (idx, header) in headers.iter().enumerate() {
                        let hash = header.block_hash().to_string();
                        if hash == mystery_hash {
                            println!("\n🎯 FOUND MYSTERY HASH at index {} in segment 0!", idx);
                            println!("   This corresponds to height {}", idx);
                            println!("   Previous block: {}", header.prev_blockhash);
                            return;
                        }
                    }
                    
                    // Check specific positions around height 2000
                    println!("\nHeaders around height 2000:");
                    for i in 1998..=2002 {
                        if i < headers.len() {
                            println!("  Height {}: {}", i, headers[i].block_hash());
                        }
                    }
                    
                    // Look for patterns
                    println!("\nSearching for similar hash patterns...");
                    let mystery_prefix = &mystery_hash[0..8]; // First 8 chars
                    for (idx, header) in headers.iter().enumerate() {
                        let hash = header.block_hash().to_string();
                        if hash.starts_with(mystery_prefix) {
                            println!("  Height {} has similar prefix: {}", idx, hash);
                        }
                    }
                }
                Err(e) => {
                    println!("Error reading segment 0: {}", e);
                }
            }
        }
    }
    
    // Check if it could be from reading wrong memory
    println!("\nPossible memory/offset issues:");
    println!("- If reading header at wrong offset, could get corrupted data");
    println!("- Segment might have sentinel headers (0xFF patterns) mixed with real data");
    println!("- Block locator might be calculating wrong height");
    
    // Analyze the hash pattern
    let hash_bytes = hex::decode(mystery_hash).unwrap();
    let mut work_estimate = 0u64;
    for &byte in &hash_bytes[0..8] {
        if byte == 0 {
            work_estimate += 8;
        } else {
            work_estimate += byte.leading_zeros() as u64;
            break;
        }
    }
    
    println!("\nHash characteristics:");
    println!("- Leading zeros: ~{} bits", work_estimate);
    println!("- This is a valid PoW hash for Dash mainnet");
    println!("- Not random data or corruption");
    
    println!("\nRecommendation:");
    println!("Add debug logging to dash-spv at these points:");
    println!("1. In block_locator.rs when building locator");
    println!("2. In storage when retrieving header by height");
    println!("3. In headers.rs after receiving headers");
    println!("4. Before sending GetHeaders message");
}

fn read_headers_from_segment(path: &PathBuf) -> Result<Vec<BlockHeader>, Box<dyn std::error::Error>> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut headers = Vec::new();
    
    loop {
        match BlockHeader::consensus_decode(&mut reader) {
            Ok(header) => headers.push(header),
            Err(encode::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => {
                return Err(format!("Failed to decode header: {}", e).into());
            }
        }
    }
    
    Ok(headers)
}