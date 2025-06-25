#!/bin/bash

# Test SPV connectivity on testnet
echo "Testing SPV on testnet..."

# First, modify the config to use testnet
cat > test_testnet_config.txt << 'EOF'
To test on testnet, modify src/components/spv_manager.rs:
1. Change Network::Dash to Network::Testnet
2. Update peer list to testnet peers
3. Clear SPV data: rm -rf ~/Library/Application\ Support/Dash-Evo-Tool/spv/
EOF

echo "Current network setting:"
grep -n "Network::" src/components/spv_manager.rs | grep -E "(Dash|Testnet|Regtest)" | head -5

echo -e "\n\nTestnet peers that should be used:"
echo "174.138.35.118:19999"
echo "149.28.22.65:19999"
echo "37.120.186.85:19999"

echo -e "\n\nChecking if any common ports are accessible:"
echo "Testing port 8333 (Bitcoin):"
nc -zv 37.120.186.85 8333 -w 2 2>&1 || echo "Failed"

echo "Testing port 80 (HTTP):"
nc -zv 37.120.186.85 80 -w 2 2>&1 || echo "Failed"

echo "Testing port 443 (HTTPS):"
nc -zv 37.120.186.85 443 -w 2 2>&1 || echo "Failed"