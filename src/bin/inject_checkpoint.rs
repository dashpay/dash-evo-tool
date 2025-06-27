use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;

#[derive(Debug, Serialize, Deserialize)]
struct SyncState {
    version: u32,
    network: String,
    chain_tip: ChainTip,
    sync_progress: SyncProgress,
    checkpoints: Vec<Checkpoint>,
    #[serde(flatten)]
    other_fields: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChainTip {
    height: u32,
    hash: String,
    prev_hash: String,
    time: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct SyncProgress {
    header_height: u32,
    filter_header_height: u32,
    #[serde(flatten)]
    other_fields: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Checkpoint {
    height: u32,
    hash: String,
    prev_hash: Option<String>,
    time: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get the path to the SPV state
    let home = directories::UserDirs::new()
        .ok_or("Could not find user directories")?
        .home_dir()
        .to_path_buf();
    let spv_state_path = home
        .join("Library")
        .join("Application Support")
        .join("Dash-Evo-Tool")
        .join("spv")
        .join("dash")
        .join("sync_state.json");

    println!("Reading sync state from: {:?}", spv_state_path);

    // Read the current sync state
    let contents = fs::read_to_string(&spv_state_path)?;
    let mut sync_state: serde_json::Value = serde_json::from_str(&contents)?;

    // Define our checkpoint
    let checkpoint_height = 2290000;
    let checkpoint_hash = "00000000000000158a0aa3adfd733a2e58bd1d78c88a5ecfe2a51d37fc90d844";
    let checkpoint_time = 1734883200; // Approximate timestamp for Dec 2024

    // Create the checkpoint object
    let checkpoint = serde_json::json!({
        "height": checkpoint_height,
        "hash": checkpoint_hash,
        "prev_hash": null,
        "time": checkpoint_time
    });

    // Add checkpoint to the checkpoints array
    if let Some(checkpoints) = sync_state.get_mut("checkpoints") {
        if let Some(checkpoints_array) = checkpoints.as_array_mut() {
            // Clear existing checkpoints and add our new one
            checkpoints_array.clear();
            checkpoints_array.push(checkpoint.clone());
            println!("Added checkpoint at height {} with hash {}", checkpoint_height, checkpoint_hash);
        }
    }

    // Update the chain_tip to our checkpoint
    if let Some(chain_tip) = sync_state.get_mut("chain_tip") {
        chain_tip["height"] = serde_json::json!(checkpoint_height);
        chain_tip["hash"] = serde_json::json!(checkpoint_hash);
        chain_tip["time"] = serde_json::json!(checkpoint_time);
        println!("Updated chain_tip to checkpoint");
    }

    // Update sync progress
    if let Some(sync_progress) = sync_state.get_mut("sync_progress") {
        sync_progress["header_height"] = serde_json::json!(checkpoint_height);
        sync_progress["headers_synced"] = serde_json::json!(false); // Still need to sync from here
        println!("Updated sync progress to start from checkpoint");
    }

    // Create a backup of the original file
    let backup_path = spv_state_path.with_extension("json.backup");
    fs::copy(&spv_state_path, &backup_path)?;
    println!("Created backup at: {:?}", backup_path);

    // Write the modified state back
    let pretty_json = serde_json::to_string_pretty(&sync_state)?;
    fs::write(&spv_state_path, pretty_json)?;
    println!("Successfully updated sync_state.json with checkpoint!");

    println!("\nNote: You should also clear the headers directory to ensure a clean sync from the checkpoint.");
    
    Ok(())
}