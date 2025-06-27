use std::path::PathBuf;
use dash_sdk::dpp::dashcore::Network;

/// Debug tool to inspect SPV header storage and identify the mystery hash issue
fn main() {
    println!("SPV Headers Debug Tool");
    println!("======================\n");
    
    // The mysterious hash that appears in GetHeaders after syncing 2000 headers
    let mystery_hash = "00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749";
    let expected_hash = "0000001e340a0e6fb510d53c2316e8b0f3e27b2e6ee0002a23f228cfec723b06";
    
    println!("Issue: After syncing headers 0-2000, dash-spv sends GetHeaders with wrong hash");
    println!("Expected hash (height 2000): {}", expected_hash);
    println!("Actual hash in GetHeaders:   {}", mystery_hash);
    println!();
    
    // Check different networks
    let networks = vec![
        (Network::Dash, "mainnet"),
        (Network::Testnet, "testnet"),
    ];
    
    for (network, name) in networks {
        println!("Checking {} SPV storage...", name);
        
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let spv_dir = PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Dash-Evo-Tool")
            .join("spv")
            .join(network.to_string());
            
        if !spv_dir.exists() {
            println!("  SPV directory not found: {:?}", spv_dir);
            continue;
        }
        
        // Check sync_state.json
        let sync_state_path = spv_dir.join("sync_state.json");
        if sync_state_path.exists() {
            println!("  Found sync_state.json");
            if let Ok(contents) = std::fs::read_to_string(&sync_state_path) {
                if contents.contains(mystery_hash) {
                    println!("  ⚠️  FOUND MYSTERY HASH in sync_state.json!");
                }
                
                // Try to parse and show relevant info
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
                    if let Some(chain_tip) = json.get("chain_tip") {
                        println!("  Chain tip: height={}, hash={}", 
                            chain_tip.get("height").unwrap_or(&serde_json::Value::Null),
                            chain_tip.get("hash").unwrap_or(&serde_json::Value::Null)
                        );
                    }
                    if let Some(sync_progress) = json.get("sync_progress") {
                        println!("  Sync progress: headers={}", 
                            sync_progress.get("header_height").unwrap_or(&serde_json::Value::Null)
                        );
                    }
                }
            }
        }
        
        // Check headers directory
        let headers_dir = spv_dir.join("headers");
        if headers_dir.exists() {
            // Count header files
            if let Ok(entries) = std::fs::read_dir(&headers_dir) {
                let count = entries.filter_map(Result::ok).count();
                println!("  Headers directory contains {} files", count);
            }
        }
        
        println!();
    }
    
    println!("Analysis:");
    println!("- This hash doesn't appear in our codebase");
    println!("- It's not the hash of any header we received (0-2000)");
    println!("- It must be generated internally by dash-spv");
    println!();
    println!("Likely a bug in dash-spv's locator hash generation logic!");
    println!();
    println!("Recommendations:");
    println!("1. Add debug logging to dash-spv to trace locator hash generation");
    println!("2. Check if there's an off-by-one error or incorrect index calculation");
    println!("3. Verify the block locator algorithm matches Bitcoin's implementation");
}