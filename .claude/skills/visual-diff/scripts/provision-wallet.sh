#!/usr/bin/env bash
# Imports the test wallet E2E_WALLET_MNEMONIC into a GUI data directory on
# testnet, headlessly, before the GUI is launched on it.
# usage: provision-wallet.sh DET_CLI DATA_DIR [--env-file FILE] [--alias NAME] [--gui-version VERSION]
# FILE defaults to .env at the repository root; NAME defaults to e2e.
# The phrase goes from the env file straight to det-cli's stdin: never echoed,
# never an argument, never written to a file.
#
# Release zips ship only the GUI binary. DET_CLI must be built from the same
# commit or tag as the GUI under test: both open the same database, and a
# different build may write a schema the GUI cannot read. With --gui-version,
# a det-cli of another version is refused.
# TODO: allow `Bash(.claude/skills/visual-diff/scripts/provision-wallet.sh *)` in
# .claude/settings.json beside type-secret.sh; adding it needs the user's approval.
set -euo pipefail
usage() {
  echo "usage: provision-wallet.sh DET_CLI DATA_DIR [--env-file FILE] [--alias NAME] [--gui-version VERSION]" >&2
  exit 2
}
[ $# -ge 2 ] || usage
DET_CLI="$1"
DATA_DIR="$2"
shift 2
ENV_FILE=""
ALIAS="e2e"
GUI_VERSION=""
while [ $# -gt 0 ]; do
  case "$1" in
    --env-file) ENV_FILE="${2:?--env-file needs a path}"; shift 2 ;;
    --alias) ALIAS="${2:?--alias needs a name}"; shift 2 ;;
    --gui-version) GUI_VERSION="${2:?--gui-version needs a version}"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$ENV_FILE" ] || ENV_FILE="$(git rev-parse --show-toplevel)/.env"
[ -x "$DET_CLI" ] || { echo "provision-wallet: $DET_CLI is not an executable det-cli" >&2; exit 2; }
[ -d "$DATA_DIR" ] || { echo "provision-wallet: $DATA_DIR is not a directory" >&2; exit 2; }
[ -f "$ENV_FILE" ] || { echo "provision-wallet: $ENV_FILE does not exist" >&2; exit 2; }

# Standalone mode on DATA_DIR only: never the real config dirs, never a server.
det() { env -u MCP_API_KEY DASH_EVO_DATA_DIR="$DATA_DIR" RUST_LOG=off "$DET_CLI" --standalone "$@"; }

cli_version="$(env -u MCP_API_KEY DASH_EVO_DATA_DIR="$DATA_DIR" "$DET_CLI" --version | awk '{ print $NF }')"
if [ -n "$GUI_VERSION" ]; then
  if [ "${GUI_VERSION#v}" != "$cli_version" ]; then
    echo "provision-wallet: det-cli is $cli_version but the GUI is ${GUI_VERSION#v}; build det-cli from the GUI's commit" >&2
    exit 1
  fi
else
  echo "provision-wallet: det-cli $cli_version; make sure it is built from the same commit as the GUI under test" >&2
fi

if ! grep -q '^E2E_WALLET_MNEMONIC=..*' "$ENV_FILE"; then
  echo "provision-wallet: E2E_WALLET_MNEMONIC is not set" >&2
  exit 1
fi

det network-switch network=testnet >/dev/null
# Strip the name, optional surrounding quotes and the trailing newline.
grep -m1 '^E2E_WALLET_MNEMONIC=' "$ENV_FILE" | cut -d= -f2- |
  sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\(.*\)'\$/\1/" | tr -d '\r\n' |
  det core-wallet-import --mnemonic-stdin network=testnet "alias=$ALIAS"
