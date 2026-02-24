#!/usr/bin/env bash
# Configure Dash Evo Tool for a local dashmate network.
# Reads ports and credentials from dashmate config, then updates the .env file.
#
# Usage: ./configure-local.sh [config_name]
#   config_name: dashmate config name (default: local_seed)
#
# Environment variables:
#   DASHMATE_CMD: dashmate command to use (default: dashmate)
#                 e.g. DASHMATE_CMD="yarn dashmate" ./configure-local.sh

set -euo pipefail

CONFIG="${1:-local_seed}"
DASHMATE="${DASHMATE_CMD:-dashmate}"

# Detect .env path
case "$(uname)" in
  Darwin) ENV_DIR="$HOME/Library/Application Support/Dash-Evo-Tool" ;;
  *)      ENV_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/dash-evo-tool" ;;
esac
ENV_FILE="$ENV_DIR/.env"

echo "Reading dashmate config (--config=$CONFIG)..."

RPC_PASSWORD=$($DASHMATE config get core.rpc.users.dashmate.password --config="$CONFIG" 2>/dev/null) \
  || { echo "Error: Could not read RPC password. Is dashmate installed and '$CONFIG' configured?"; exit 1; }
RPC_PORT=$($DASHMATE config get core.rpc.port --config="$CONFIG")
ZMQ_PORT=$($DASHMATE config get core.zmq.port --config="$CONFIG")
DAPI_PORT=$($DASHMATE config get platform.gateway.listeners.dapiAndDrive.port --config="$CONFIG")

HOST="127.0.0.1"

echo ""
echo "  RPC Password : (found)"
echo "  RPC Port     : $RPC_PORT"
echo "  DAPI Port    : $DAPI_PORT"
echo "  ZMQ Port     : $ZMQ_PORT"
echo "  Host         : $HOST"
echo ""

LOCAL_BLOCK="LOCAL_dapi_addresses=http://${HOST}:${DAPI_PORT}
LOCAL_core_host=${HOST}
LOCAL_core_rpc_port=${RPC_PORT}
LOCAL_core_rpc_user=dashmate
LOCAL_core_rpc_password=${RPC_PASSWORD}
LOCAL_insight_api_url=http://localhost:3001/insight-api
LOCAL_core_zmq_endpoint=tcp://${HOST}:${ZMQ_PORT}
LOCAL_show_in_ui=true"

# Ensure directory exists
mkdir -p "$ENV_DIR"

if [ ! -f "$ENV_FILE" ]; then
  echo "$LOCAL_BLOCK" > "$ENV_FILE"
  echo "Created $ENV_FILE with LOCAL config."
  exit 0
fi

# Remove existing LOCAL_ lines
CLEANED=$(grep -v '^LOCAL_' "$ENV_FILE" || true)

# Remove trailing blank lines, then append LOCAL block
CLEANED=$(echo "$CLEANED" | sed -e :a -e '/^\n*$/{$d;N;ba}')

printf '%s\n\n%s\n' "$CLEANED" "$LOCAL_BLOCK" > "$ENV_FILE"

echo "Updated $ENV_FILE with LOCAL config."
