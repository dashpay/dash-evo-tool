#!/bin/bash

echo "Starting dash-evo-tool and monitoring SPV logs..."
echo "================================================="
echo ""
echo "This will show:"
echo "1. DAPI address parsing and P2P conversion"
echo "2. Peer group selection and cycling"
echo "3. SPV sync progress"
echo ""

# Run with specific SPV logging
RUST_LOG=dash_evo_tool::components::spv_manager=trace,dash_spv=debug cargo run 2>&1 | \
    grep --line-buffered -v "warning:" | \
    grep --line-buffered -E "(SPV|peer|DAPI|P2P|Trying|Added|Using|stuck|switch|group|address|sync|header)" | \
    while IFS= read -r line; do
        # Color code different message types
        if echo "$line" | grep -q "Added P2P address"; then
            echo -e "\033[32m[P2P] $line\033[0m"  # Green
        elif echo "$line" | grep -q "Trying peer group"; then
            echo -e "\033[36m[GROUP] $line\033[0m"  # Cyan
        elif echo "$line" | grep -q "stuck"; then
            echo -e "\033[33m[STUCK] $line\033[0m"  # Yellow
        elif echo "$line" | grep -q "switch"; then
            echo -e "\033[35m[SWITCH] $line\033[0m"  # Magenta
        elif echo "$line" | grep -q "header"; then
            echo -e "\033[34m[HEADER] $line\033[0m"  # Blue
        else
            echo "[LOG] $line"
        fi
    done