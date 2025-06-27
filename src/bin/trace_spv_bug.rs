use std::path::PathBuf;
use dash_sdk::dpp::dashcore::Network;
use serde_json::Value;

/// This tool helps trace the dash-spv bug where an incorrect hash is sent after syncing 2000 headers
fn main() {
    println!("dash-spv Bug Tracer");
    println!("===================\n");
    
    // The problematic scenario
    println!("Bug Details:");
    println!("- After syncing headers 0-2000 (2001 total headers)");
    println!("- dash-spv sends GetHeaders with wrong block locator hash");
    println!("- Wrong hash: 00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749");
    println!("- Expected hash at height 2000: 0000001e340a0e6fb510d53c2316e8b0f3e27b2e6ee0002a23f228cfec723b06");
    println!();
    
    // Check SPV storage
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let spv_dir = PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Dash-Evo-Tool")
        .join("spv")
        .join("dash");
    
    if !spv_dir.exists() {
        println!("SPV directory not found. Please run dash-evo-tool and sync some headers first.");
        return;
    }
    
    // Check sync_state.json
    println!("Checking sync_state.json...");
    let sync_state_path = spv_dir.join("sync_state.json");
    if let Ok(contents) = std::fs::read_to_string(&sync_state_path) {
        if let Ok(json) = serde_json::from_str::<Value>(&contents) {
            if let Some(progress) = json.get("sync_progress") {
                if let Some(height) = progress.get("header_height") {
                    println!("Current header height: {}", height);
                    
                    if height.as_u64() == Some(2000) {
                        println!("⚠️  AT CRITICAL HEIGHT 2000 - Bug likely to occur on next GetHeaders!");
                    }
                }
            }
            
            // Check for the mystery hash
            let contents_str = contents.to_lowercase();
            if contents_str.contains("00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749") {
                println!("❌ FOUND MYSTERY HASH IN SYNC STATE!");
            }
        }
    }
    
    // Check headers directory
    println!("\nChecking headers storage...");
    let headers_dir = spv_dir.join("headers");
    if headers_dir.exists() {
        // Count header files and check their naming
        if let Ok(entries) = std::fs::read_dir(&headers_dir) {
            let mut header_files: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with(".headers") {
                        Some(name)
                    } else {
                        None
                    }
                })
                .collect();
            
            header_files.sort();
            
            println!("Header files found: {}", header_files.len());
            for file in &header_files {
                println!("  - {}", file);
            }
            
            // The files are likely named by height ranges
            // Check if there's something special about the 2000 boundary
            for file in &header_files {
                if file.contains("2000") || file.contains("1999") || file.contains("2001") {
                    println!("  ⚠️  File {} might contain the boundary where bug occurs", file);
                }
            }
        }
    }
    
    println!("\nDebugging Steps:");
    println!("1. Clear SPV data: rm -rf ~/Library/Application\\ Support/Dash-Evo-Tool/spv/dash/");
    println!("2. Run dash-evo-tool with debug logging:");
    println!("   RUST_LOG=dash_spv=trace cargo run");
    println!("3. Start SPV sync and watch for:");
    println!("   - 'Send getheaders' messages");
    println!("   - Block locator construction");
    println!("   - What happens after receiving the 2000th header");
    println!("4. Look for the mystery hash in the logs");
    println!();
    println!("The bug is likely in one of these dash-spv components:");
    println!("- Block locator generation algorithm");
    println!("- Header storage/retrieval at boundaries");
    println!("- Index calculation (off-by-one error?)");
    println!("- Special handling at round numbers like 2000");
}