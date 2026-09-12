#!/usr/bin/env bash
#
# Capture a migration fixture headlessly, by driving det-cli against a throwaway
# DASH_EVO_DATA_DIR until it holds a populated, discovered wallet profile.
#
#   scripts/migration-fixtures/capture-headless.sh \
#       --data-dir /data/tmp/fixture-capture \
#       --network testnet \
#       --mnemonic-env FIXTURE_WALLET_MNEMONIC
#
# The sequence is the one docs/CLI.md documents for standalone (stdio,
# lazy-init) mode, and each step exists because it writes something a later
# version has to migrate:
#
#   network-info            boots the binary and lazy-inits AppContext, which
#                           creates .env, the SQLite DBs and the secret vault
#   network-switch          materialises the per-network context and starts SPV
#   core-wallet-import      writes the seed through the secret seam and the
#                           wallet rows behind it
#   core-balances-get       SPV gate — retried until the chain is synced enough
#                           for the platform query below to be answerable
#   core-address-create     forces address derivation state to be persisted
#   platform-addresses-list triggers identity/DPNS discovery, which is what
#                           populates the rows migrations actually reshape
#
# Only after discovery returns a non-empty result is the directory worth
# packing; a fixture captured before discovery finished looks like a healthy
# empty profile and silently tests nothing. Pass --allow-empty-discovery to
# override, e.g. when deliberately capturing a wallet with no platform balance.
#
# This captures the CURRENT build's profile. Capturing a historical release
# means running its own binary — and v0.9.3 predates det-cli entirely, so that
# baseline needs the GUI route under Xvfb, not this script.
#
# SECURITY NOTE: det-cli takes tool parameters on argv only, so the recovery
# phrase is briefly visible in /proc/<pid>/cmdline to other processes on the
# same host for the duration of the import. The fixture wallet is a deliberately
# public, testnet-only wallet, which is what makes that acceptable here; do not
# point this script at a wallet holding anything of value.
# TODO: drop the argv exposure once core_wallet_import can take the mnemonic
# from an environment variable or stdin.

set -euo pipefail

DATA_DIR=""
NETWORK="testnet"
MNEMONIC_ENV="FIXTURE_WALLET_MNEMONIC"
DET_CLI=""
ALIAS="migration-fixture"
SPV_WAIT_SECS=900
ALLOW_EMPTY_DISCOVERY=0

die() {
    echo "::error::$*" >&2
    exit 1
}

note() { echo "==> $*"; }

usage() {
    cat <<'EOF'
usage: capture-headless.sh --data-dir DIR [options]

  --data-dir DIR           target DASH_EVO_DATA_DIR (created if absent)
  --network NAME           network to capture on            (default: testnet)
  --mnemonic-env VAR       env var holding the BIP-39 phrase (default: FIXTURE_WALLET_MNEMONIC)
  --det-cli PATH           det-cli binary to drive           (default: auto-detected)
  --alias NAME             wallet alias to import under      (default: migration-fixture)
  --spv-wait-secs N        budget for the SPV gate           (default: 900)
  --allow-empty-discovery  do not fail when no platform addresses are discovered
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
    --data-dir)
        DATA_DIR="${2:-}"
        shift
        ;;
    --network)
        NETWORK="${2:-}"
        shift
        ;;
    --mnemonic-env)
        MNEMONIC_ENV="${2:-}"
        shift
        ;;
    --det-cli)
        DET_CLI="${2:-}"
        shift
        ;;
    --alias)
        ALIAS="${2:-}"
        shift
        ;;
    --spv-wait-secs)
        SPV_WAIT_SECS="${2:-}"
        shift
        ;;
    --allow-empty-discovery) ALLOW_EMPTY_DISCOVERY=1 ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        usage >&2
        die "unknown argument '$1'"
        ;;
    esac
    shift
done

[ -n "$DATA_DIR" ] || {
    usage >&2
    die "--data-dir is required"
}
command -v jq >/dev/null 2>&1 || die "jq is required but not installed."

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd)"

# --------------------------------------------------------------------------
# Locate the binary.
# --------------------------------------------------------------------------
#
# Cargo's output directory is redirectable by CARGO_TARGET_DIR or by
# build.target-dir in .cargo/config.toml, so ./target is a guess, not a fact.
# Ask cargo, and only then fall back.
if [ -z "$DET_CLI" ]; then
    target_dir="${CARGO_TARGET_DIR:-}"
    if [ -z "$target_dir" ] && command -v cargo >/dev/null 2>&1; then
        target_dir="$(cd -- "$REPO_ROOT" && cargo metadata --format-version 1 --no-deps 2>/dev/null |
            jq -r '.target_directory // empty')"
    fi
    target_dir="${target_dir:-$REPO_ROOT/target}"
    for candidate in "$target_dir/release/det-cli" "$target_dir/debug/det-cli"; do
        if [ -x "$candidate" ]; then
            DET_CLI="$candidate"
            break
        fi
    done
fi
if [ -z "$DET_CLI" ] || [ ! -x "$DET_CLI" ]; then
    die "det-cli binary not found. Build it with 'cargo build --bin det-cli --features cli', or point at it with --det-cli PATH."
fi

MNEMONIC="${!MNEMONIC_ENV:-}"
[ -n "$MNEMONIC" ] || die "Environment variable '$MNEMONIC_ENV' is unset or empty. Export the fixture wallet's recovery phrase there, or name a different variable with --mnemonic-env."

# --------------------------------------------------------------------------
# Prepare the data directory.
# --------------------------------------------------------------------------

mkdir -p "$DATA_DIR"
DATA_DIR="$(cd -- "$DATA_DIR" && pwd)"
if [ ! -f "$DATA_DIR/.env" ]; then
    [ -f "$REPO_ROOT/.env.example" ] || die "No .env in $DATA_DIR and no .env.example at $REPO_ROOT to seed it from."
    cp "$REPO_ROOT/.env.example" "$DATA_DIR/.env"
    note "Seeded $DATA_DIR/.env from .env.example"
fi

note "Binary:   $DET_CLI"
note "Data dir: $DATA_DIR"
note "Network:  $NETWORK"
note "Alias:    $ALIAS"

# MCP_API_KEY is unset rather than merely empty so a key inherited from the
# operator's shell cannot silently redirect these calls into a running GUI —
# that would capture nothing and mutate the operator's real profile instead.
det() {
    env -u MCP_API_KEY \
        DASH_EVO_DATA_DIR="$DATA_DIR" \
        RUST_LOG="${RUST_LOG:-off}" \
        "$DET_CLI" "$@"
}

# --------------------------------------------------------------------------
# 1. Boot + lazy context init.
# --------------------------------------------------------------------------

note "[1/6] network-info"
det network-info >/dev/null || die "det-cli could not initialise an AppContext in $DATA_DIR. The data directory may be held by another DET process, or .env may be missing DAPI addresses for '$NETWORK'."

# --------------------------------------------------------------------------
# 2. Network switch (idempotent; no-op when already active).
# --------------------------------------------------------------------------

note "[2/6] network-switch network=$NETWORK"
switch_out="$(det network-switch "network=$NETWORK")" ||
    die "Failed to switch to network '$NETWORK'. Check that ${NETWORK^^}_dapi_addresses is populated in $DATA_DIR/.env."
echo "$switch_out" | jq -e '.active' >/dev/null 2>&1 ||
    die "network-switch returned no active network: $switch_out"

# --------------------------------------------------------------------------
# 3. Wallet import.
# --------------------------------------------------------------------------
#
# Output is discarded rather than echoed: it carries the seed hash, and the
# command line already exposes enough. Import is idempotent, so re-running the
# capture against an existing data dir is safe.

note "[3/6] core-wallet-import alias=$ALIAS (output suppressed)"
det core-wallet-import "mnemonic=$MNEMONIC" "network=$NETWORK" "alias=$ALIAS" >/dev/null ||
    die "Wallet import failed. Verify the phrase in \$$MNEMONIC_ENV is a valid BIP-39 mnemonic for '$NETWORK'."

# --------------------------------------------------------------------------
# 4. SPV gate.
# --------------------------------------------------------------------------
#
# core-balances-get is read-only and SPV-gated, which makes it the cheapest
# probe for "is the chain synced enough to answer anything". The tool has its
# own internal 10-minute sync wait; this loop exists on top of it so a sync that
# needs longer than one attempt retries instead of failing the capture, and so
# the wait is visible in the log rather than looking like a hang.

note "[4/6] waiting up to ${SPV_WAIT_SECS}s for SPV sync"
deadline=$((SECONDS + SPV_WAIT_SECS))
synced=0
attempt=0
while [ "$SECONDS" -lt "$deadline" ]; do
    attempt=$((attempt + 1))
    if det core-balances-get "wallet-id=$ALIAS" "network=$NETWORK" >/dev/null 2>&1; then
        synced=1
        note "SPV ready after $attempt attempt(s), ${SECONDS}s elapsed"
        break
    fi
    note "SPV not ready yet (attempt $attempt, ${SECONDS}s elapsed) — retrying in 15s"
    sleep 15
done
[ "$synced" -eq 1 ] ||
    die "SPV did not sync within ${SPV_WAIT_SECS}s. Raise --spv-wait-secs, or check the runner's outbound P2P access to the '$NETWORK' network."

# --------------------------------------------------------------------------
# 5. Address derivation.
# --------------------------------------------------------------------------

note "[5/6] core-address-create"
addr_out="$(det core-address-create "wallet-id=$ALIAS" "network=$NETWORK")" ||
    die "Address generation failed for wallet '$ALIAS'."
echo "$addr_out" | jq -e '.address' >/dev/null 2>&1 ||
    die "core-address-create returned no address: $addr_out"

# --------------------------------------------------------------------------
# 6. Identity / DPNS discovery.
# --------------------------------------------------------------------------

note "[6/6] platform-addresses-list (triggers identity discovery)"
plat_out="$(det platform-addresses-list "wallet-id=$ALIAS" "network=$NETWORK")" ||
    die "Platform address lookup failed for wallet '$ALIAS'."

discovered="$(echo "$plat_out" | jq -r '.balances | length' 2>/dev/null || echo 0)"
note "Discovered platform addresses: $discovered"

if [ "$discovered" -eq 0 ]; then
    if [ "$ALLOW_EMPTY_DISCOVERY" -eq 1 ]; then
        echo "::warning::No platform addresses discovered; packing anyway because --allow-empty-discovery was given." >&2
    else
        die "No platform addresses were discovered, so this profile has nothing for a migration to reshape and would pass as a fixture without testing anything.
The fixture wallet most likely has no registered identity on '$NETWORK' — register one with the current build first, or re-register after a testnet reset. Pass --allow-empty-discovery to capture regardless."
    fi
fi

note "Capture complete: $DATA_DIR"
note "Pack it with: scripts/migration-fixtures/pack.sh '$DATA_DIR' <out-prefix>"
