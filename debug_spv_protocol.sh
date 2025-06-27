#!/bin/bash

echo "SPV Protocol Debugging Script"
echo "============================"
echo ""
echo "This script will monitor SPV protocol messages to diagnose the GetHeaders vs GetHeaders2 issue"
echo ""
echo "Things to watch for:"
echo "1. 'Send getheaders2' vs 'Send getheaders' messages"
echo "2. 'Handle headers' responses (should see 'with 2000 headers')"
echo "3. Protocol version negotiation messages"
echo "4. Peer connection handshakes"
echo ""
echo "Press Ctrl+C to stop"
echo ""

# Run with very detailed SPV logging to see protocol messages
export RUST_LOG="dash_spv=trace,dash_evo_tool::components::spv_manager=trace"

# Run and filter for protocol-related messages
cargo run 2>&1 | grep --line-buffered -E "(Send getheaders|Handle headers|protocol|version|handshake|GetHeaders|peer connected|message type|recv|send)" | \
    while IFS= read -r line; do
        # Color code different message types
        if echo "$line" | grep -q "Send getheaders"; then
            echo -e "\033[31m[SEND] $line\033[0m"  # Red for sends
        elif echo "$line" | grep -q "Handle headers"; then
            echo -e "\033[32m[RECV] $line\033[0m"  # Green for receives
        elif echo "$line" | grep -q "protocol\|version"; then
            echo -e "\033[33m[PROTO] $line\033[0m"  # Yellow for protocol
        elif echo "$line" | grep -q "peer connected"; then
            echo -e "\033[36m[CONN] $line\033[0m"  # Cyan for connections
        else
            echo "[LOG] $line"
        fi
    done