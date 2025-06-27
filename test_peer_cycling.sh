#!/bin/bash

# Enhanced test script for SPV peer cycling functionality

echo "=============================================="
echo "Enhanced SPV Peer Cycling Test"
echo "=============================================="
echo ""
echo "This script tests the improved peer cycling that detects when:"
echo "1. A peer stops responding at height 0 (30 second timeout)"
echo "2. A peer stops providing headers at any height (40 second timeout)"
echo "3. Automatic switching to next peer group"
echo "4. Continuation of sync from last known height"
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Extract DAPI addresses from .env if available
if [ -f .env ]; then
    echo -e "${CYAN}DAPI addresses from .env:${NC}"
    grep "MAINNET_DAPI_ADDRESSES" .env | cut -d'=' -f2 | tr ',' '\n' | while read addr; do
        # Extract host from https://host:port format
        host=$(echo "$addr" | sed 's|https://||' | cut -d':' -f1)
        echo -e "  DAPI: $addr -> P2P: ${BLUE}$host:9999${NC}"
    done
    echo ""
fi

echo "Starting dash-evo-tool with enhanced SPV monitoring..."
echo ""
echo -e "${YELLOW}Key events to watch for:${NC}"
echo -e "  ${GREEN}✓${NC} 'Sync progressing - headers: X'"
echo -e "  ${YELLOW}⚠${NC}  'No header sync progress at height X'"
echo -e "  ${RED}✗${NC}  'Header sync stuck at height X for Y seconds'"
echo -e "  ${BLUE}↻${NC}  'Switching to next peer group'"
echo -e "  ${GREEN}↗${NC}  'Continuing sync from height X with new peers'"
echo ""
echo "Press Ctrl+C to stop"
echo "=============================================="
echo ""

# Run with detailed SPV logging and format output
RUST_LOG=dash_evo_tool::components::spv_manager=debug,dash_spv=info cargo run 2>&1 | \
    while IFS= read -r line; do
        timestamp="[$(date +%H:%M:%S)]"
        
        # Color code different message types
        if echo "$line" | grep -q "Sync progressing"; then
            echo -e "${timestamp} ${GREEN}✓ Progress:${NC} $line"
        elif echo "$line" | grep -q "No header sync progress"; then
            echo -e "${timestamp} ${YELLOW}⚠ Warning:${NC} $line"
        elif echo "$line" | grep -q "Header sync stuck"; then
            echo -e "${timestamp} ${RED}✗ Stuck:${NC} $line"
        elif echo "$line" | grep -q "Switching to next peer group"; then
            echo -e "${timestamp} ${BLUE}↻ PEER SWITCH:${NC} $line"
            echo -e "${timestamp} ${BLUE}============ PEER GROUP CYCLING ============${NC}"
        elif echo "$line" | grep -q "Continuing sync from height"; then
            echo -e "${timestamp} ${GREEN}↗ Resuming:${NC} $line"
        elif echo "$line" | grep -q "Current sync height before switching"; then
            echo -e "${timestamp} ${CYAN}📍 Status:${NC} $line"
        elif echo "$line" | grep -q "Trying peer group"; then
            echo -e "${timestamp} ${BLUE}🔄 Peers:${NC} $line"
        elif echo "$line" | grep -q "PROGRESS MONITOR"; then
            # Show progress monitor messages in default color
            echo -e "${timestamp} $line"
        elif echo "$line" | grep -q "Failed to switch peer group"; then
            echo -e "${timestamp} ${RED}ERROR:${NC} $line"
        elif echo "$line" | grep -q "Exhausted peer groups"; then
            echo -e "${timestamp} ${RED}CRITICAL:${NC} $line"
        elif echo "$line" | grep -E "(peer|group|P2P|DAPI|Added|Using)"; then
            # Show other peer-related messages
            echo -e "${timestamp} ${CYAN}Info:${NC} $line"
        fi
    done