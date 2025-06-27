#!/bin/bash

echo "Testing SPV header sync fix..."
echo "=============================="
echo ""
echo "This test will verify that SPV can now properly sync headers after the fix."
echo ""

# Show current SPV directory structure
echo "Current SPV directory structure:"
ls -la "$HOME/Library/Application Support/Dash-Evo-Tool/spv/dash/"
echo ""

# Run with SPV logging to monitor the fix
echo "Starting dash-evo-tool with SPV monitoring..."
echo "Look for:"
echo "1. 'Creating SPV subdirectory' messages"
echo "2. Headers syncing progress (should be > 0)"
echo "3. No more 'No such file or directory' errors"
echo ""

RUST_LOG=dash_evo_tool::components::spv_manager=debug,dash_spv=info cargo run 2>&1 | \
    grep --line-buffered -E "(Creating SPV|Headers:|ChainLock|No such file|Storage error|SYNC STATUS)" | \
    while IFS= read -r line; do
        # Color code different message types
        if echo "$line" | grep -q "Creating SPV"; then
            echo -e "\033[32m[CREATE] $line\033[0m"  # Green
        elif echo "$line" | grep -q "Headers:"; then
            echo -e "\033[36m[SYNC] $line\033[0m"  # Cyan
        elif echo "$line" | grep -q "No such file\|Storage error"; then
            echo -e "\033[31m[ERROR] $line\033[0m"  # Red
        elif echo "$line" | grep -q "ChainLock"; then
            echo -e "\033[35m[CHAINLOCK] $line\033[0m"  # Magenta
        else
            echo "[LOG] $line"
        fi
    done