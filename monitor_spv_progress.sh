#!/bin/bash

echo "Monitoring SPV sync progress..."
echo "================================"
echo ""
echo "Watching for:"
echo "- Progress monitor ticks and errors"
echo "- Peer group switching"
echo "- Header sync progress"
echo ""

# Clear log before starting
> "/Users/pauldelucia/Library/Application Support/Dash-Evo-Tool/det.log"

# Start the app in background
echo "Starting dash-evo-tool..."
cargo run --release &
APP_PID=$!

# Give it time to start
sleep 3

# Monitor the log
tail -f "/Users/pauldelucia/Library/Application Support/Dash-Evo-Tool/det.log" | \
    grep --line-buffered -E "(PROGRESS MONITOR|peer group|switching|Failed to get sync progress|Still at height|check #)" | \
    while IFS= read -r line; do
        # Add timestamp for clarity
        echo "[$(date +%H:%M:%S)] $line"
        
        # Check if we've switched peer groups
        if echo "$line" | grep -q "switching to next peer group"; then
            echo ">>> PEER GROUP SWITCH DETECTED <<<"
        fi
    done

# Cleanup on exit
trap "kill $APP_PID 2>/dev/null" EXIT