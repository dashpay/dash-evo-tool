#!/bin/sh
# Verify a masternode identity is live on the platform network by querying DAPI
# directly (bypasses DET entirely). Proves migration preserved a real, funded,
# on-chain identity independent of the client under test.
#
# Usage: query_identity_dapi.sh <proTxHash_hex> [platform_repo_path]
# Requires: grpcurl, and a checkout of dashpay/platform for the .proto files.
set -e
PROTX="$1"
PLATFORM="${2:-$HOME/Projects/dashpay/platform}"
PROTO="$PLATFORM/packages/dapi-grpc/protos/platform/v0/platform.proto"
GATEWAY="${GATEWAY:-127.0.0.1:2443}"   # dashmate local DAPI gateway (self-signed TLS)

if [ -z "$PROTX" ]; then echo "usage: $0 <proTxHash_hex> [platform_repo_path]"; exit 2; fi
ID_B64=$(python3 -c "import base64,sys; print(base64.b64encode(bytes.fromhex(sys.argv[1])).decode())" "$PROTX")

echo ">> getIdentityBalance for $PROTX"
grpcurl -insecure -max-time 15 -import-path "$PLATFORM/packages/dapi-grpc/protos" -proto "$PROTO" \
  -d "{\"v0\":{\"id\":\"$ID_B64\",\"prove\":false}}" \
  "$GATEWAY" org.dash.platform.dapi.v0.Platform/getIdentityBalance
