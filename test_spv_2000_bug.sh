#!/bin/bash

echo "Testing dash-spv 2000 header bug"
echo "================================"
echo ""
echo "This test will:"
echo "1. Clear SPV data to start fresh"
echo "2. Run dash-evo-tool with trace logging"
echo "3. Monitor for the bug after 2000 headers"
echo ""

# Confirm before clearing data
read -p "This will clear your SPV data. Continue? (y/n) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    exit 1
fi

# Clear SPV data
SPV_DIR="$HOME/Library/Application Support/Dash-Evo-Tool/spv/dash"
if [ -d "$SPV_DIR" ]; then
    echo "Clearing SPV data..."
    rm -rf "$SPV_DIR"
    echo "SPV data cleared"
else
    echo "No existing SPV data found"
fi

# Create a log file for this test
LOG_FILE="spv_2000_bug_test_$(date +%Y%m%d_%H%M%S).log"
echo "Logging to: $LOG_FILE"
echo ""

# Run with maximum logging for dash-spv
echo "Starting dash-evo-tool with trace logging..."
echo "Watch for:"
echo "- 'Send getheaders' messages"
echo "- Headers being processed (should see 'Handle headers message with 2000 headers')"
echo "- The mystery hash: 00000014ae902cd16b2109ee531d006780ad3303af01e7b938e182c30c99f749"
echo ""

RUST_LOG=dash_spv=trace,dash_evo_tool::components::spv_manager=debug cargo run 2>&1 | tee "$LOG_FILE" | \
    grep --line-buffered -E "(getheaders|GetHeaders|headers message|locator|height.*2000|00000014ae902cd)" | \
    while IFS= read -r line; do
        # Highlight important lines
        if echo "$line" | grep -q "00000014ae902cd"; then
            echo -e "\033[31m[BUG DETECTED] $line\033[0m"  # Red
            echo ""
            echo "❌ FOUND THE BUG! Mystery hash detected in GetHeaders message!"
            echo "Check the log file for full context: $LOG_FILE"
        elif echo "$line" | grep -q "headers message with 2000"; then
            echo -e "\033[33m[CRITICAL] $line\033[0m"  # Yellow
            echo "⚠️  Just received 2000 headers - bug should occur in next GetHeaders!"
        elif echo "$line" | grep -q -i "getheaders"; then
            echo -e "\033[36m[GETHEADERS] $line\033[0m"  # Cyan
        else
            echo "$line"
        fi
    done

echo ""
echo "Test complete. Check $LOG_FILE for full logs."