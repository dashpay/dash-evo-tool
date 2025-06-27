#!/bin/bash

echo "Starting Dash Evo Tool with SPV debug logging..."
echo "=============================================="
echo ""
echo "Watch for these key events:"
echo "1. When you click 'Start SPV' in the UI"
echo "2. DAPI address extraction and P2P conversion"
echo "3. Peer group selection (e.g., 'Trying peer group 0 with peers: ...')"
echo "4. Sync progress and stuck detection"
echo "5. Automatic peer group switching"
echo ""
echo "Press Ctrl+C to stop"
echo ""

# Set logging for SPV manager and dash-spv
export RUST_LOG="dash_evo_tool::components::spv_manager=debug,dash_spv=info,dash_evo_tool=info"

# Run the app
cargo run