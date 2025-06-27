#!/bin/bash

# Script to reset SPV sync state with a recent checkpoint for Dash mainnet
# This is necessary because most mainnet nodes only serve recent blocks (~2.29M+)

echo "Resetting Dash mainnet SPV with checkpoint at height 2,290,000..."

# Define the SPV data directory
SPV_DIR="$HOME/Library/Application Support/Dash-Evo-Tool/spv/dash"

# Backup existing state if it exists
if [ -f "$SPV_DIR/sync_state.json" ]; then
    echo "Backing up existing sync_state.json..."
    cp "$SPV_DIR/sync_state.json" "$SPV_DIR/sync_state.json.backup.$(date +%Y%m%d_%H%M%S)"
fi

# Create the directory if it doesn't exist
mkdir -p "$SPV_DIR"
mkdir -p "$SPV_DIR/headers"
mkdir -p "$SPV_DIR/filters" 
mkdir -p "$SPV_DIR/state"

# Create sync_state.json with checkpoint at height 2,290,000
cat > "$SPV_DIR/sync_state.json" << 'EOF'
{
  "version": 1,
  "network": "dash",
  "chain_tip": {
    "height": 2290000,
    "hash": "00000000000000158a0aa3adfd733a2e58bd1d78c88a5ecfe2a51d37fc90d844",
    "prev_hash": "0000000000000000000000000000000000000000000000000000000000000000",
    "time": 1734883200
  },
  "sync_progress": {
    "header_height": 2290000,
    "filter_header_height": 0,
    "masternode_height": 0,
    "peer_count": 0,
    "headers_synced": false,
    "filter_headers_synced": false,
    "masternodes_synced": false,
    "filter_sync_available": false,
    "filters_downloaded": 0,
    "last_synced_filter_height": null,
    "sync_start": {
      "secs_since_epoch": 0,
      "nanos_since_epoch": 0
    },
    "last_update": {
      "secs_since_epoch": 0,
      "nanos_since_epoch": 0
    }
  },
  "checkpoints": [{
    "height": 2290000,
    "hash": "00000000000000158a0aa3adfd733a2e58bd1d78c88a5ecfe2a51d37fc90d844",
    "time": 1734883200
  }],
  "masternode_sync": {
    "last_synced_height": null,
    "is_synced": false,
    "masternode_count": 0,
    "last_diff_height": null
  },
  "filter_sync": {
    "filter_header_height": 0,
    "filter_height": 0,
    "filters_downloaded": 0,
    "matched_heights": [],
    "filter_sync_available": false
  },
  "saved_at": {
    "secs_since_epoch": 0,
    "nanos_since_epoch": 0
  },
  "chain_work": "ChainWork { work: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 16] }"
}
EOF

# Clear the headers directory to ensure clean sync
echo "Clearing headers directory..."
rm -rf "$SPV_DIR/headers"/*

echo "Done! SPV sync state has been reset with checkpoint at height 2,290,000"
echo ""
echo "Next steps:"
echo "1. Start the Dash Evo Tool application"
echo "2. Click 'Initialize SPV' button"
echo "3. Click 'Start SPV Sync' button"
echo ""
echo "The sync should now start from height 2,290,000 instead of genesis,"
echo "which will work with mainnet nodes that only serve recent blocks."