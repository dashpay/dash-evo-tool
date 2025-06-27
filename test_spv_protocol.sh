#!/bin/bash

echo "SPV Protocol Version Test"
echo "========================"
echo ""
echo "This script tests SPV with different configurations to identify protocol issues"
echo ""

# Function to test with specific logging
test_spv_config() {
    local test_name=$1
    local rust_log=$2
    
    echo ""
    echo "Test: $test_name"
    echo "RUST_LOG: $rust_log"
    echo "-------------------"
    echo ""
    
    # Run for 30 seconds and capture key protocol messages
    timeout 30s bash -c "RUST_LOG='$rust_log' cargo run 2>&1" | \
        grep -E "(Send getheaders|Handle headers|GetHeaders|protocol version|negotiated|handshake)" | \
        head -20
    
    echo ""
    echo "Test completed. If you see 'Send getheaders2', this indicates newer protocol."
    echo "If you see 'Send getheaders' (without '2'), this indicates older protocol."
}

# Test 1: Maximum verbosity for dash-spv
echo "=== TEST 1: Full SPV Trace Logging ==="
test_spv_config "Full Trace" "dash_spv=trace"

# Test 2: Focus on network messages
echo ""
echo "=== TEST 2: Network Message Logging ==="
test_spv_config "Network Focus" "dash_spv::network=trace,dash_spv::protocol=trace"

# Test 3: Check peer connections
echo ""
echo "=== TEST 3: Peer Connection Logging ==="
test_spv_config "Peer Connections" "dash_spv::peer=trace,dash_spv::connection=trace"

echo ""
echo "====================================="
echo "Analysis Complete"
echo ""
echo "If all tests show 'getheaders2' instead of 'getheaders', this confirms"
echo "a protocol version mismatch with the peers."
echo ""
echo "Next steps:"
echo "1. Check if dash-spv library can be configured for older protocol"
echo "2. Consider using different peers that support newer protocol"
echo "3. Patch dash-spv to use GetHeaders instead of GetHeaders2"