#!/usr/bin/env bash
# Offline regression checks; all inputs and artifacts are disposable.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
scratch="$(mktemp -d "${TMPDIR:-${RUNNER_TEMP:-/data/tmp}}/migration-script-tests.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/data" "$scratch/bin"

fail() { echo "FAIL: $*" >&2; exit 1; }
printf 'MCP_API_KEY=\nTESTNET_core_rpc_password=""\nTESTNET_core_rpc_user=\n' > "$scratch/data/.env"
bash "$SCRIPT_DIR/pack.sh" "$scratch/data" "$scratch/clean" > "$scratch/pack.log"
for key in MCP_API_KEY TESTNET_core_rpc_password MAINNET_core_rpc_user LOCAL_wallet_private_key; do
    # Generated disposable text, never an operator credential.
    printf '%s=%s\n' "$key" "$RANDOM-$RANDOM" > "$scratch/data/.env"
    if bash "$SCRIPT_DIR/pack.sh" "$scratch/data" "$scratch/rejected" > "$scratch/pack.log" 2>&1; then
        fail "pack accepted $key"
    fi
    grep -Fq "$key" "$scratch/pack.log" || fail "pack did not identify $key"
done
printf ' export TESTNET_core_rpc_password = "" # empty\nMCP_API_KEY=\n' > "$scratch/data/.env"
bash "$SCRIPT_DIR/pack.sh" "$scratch/data" "$scratch/clean-export" > "$scratch/pack.log"
echo 'PASS: fixture credential checks'

if bash "$SCRIPT_DIR/capture-headless.sh" --network mainnet --data-dir "$scratch/capture" > "$scratch/capture.log" 2>&1; then
    fail 'capture accepted mainnet'
fi
grep -q 'must be captured on testnet' "$scratch/capture.log" || fail 'testnet guard did not run first'
echo 'PASS: capture testnet guard'

cat > "$scratch/bin/det-cli" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = --standalone ] || exit 1
shift
case "$1" in
    network-info | core-wallet-import | core-balances-get) echo '{}' ;;
    network-switch) echo '{"active":"testnet"}' ;;
    core-address-create) echo '{"address":"mock-address"}' ;;
    platform-addresses-list) echo '{"balances":[{}]}' ;;
    *) exit 1 ;;
esac
EOF
chmod +x "$scratch/bin/det-cli"
mkdir -p "$scratch/capture"
printf 'MCP_API_KEY=%s\n' "$RANDOM-$RANDOM" > "$scratch/capture/.env"
FIXTURE_WALLET_MNEMONIC="$RANDOM-$RANDOM" bash "$SCRIPT_DIR/capture-headless.sh" \
    --data-dir "$scratch/capture" --det-cli "$scratch/bin/det-cli" > "$scratch/capture.log" 2>&1 || fail 'capture did not force standalone mode'
echo 'PASS: capture standalone mode'

cat > "$scratch/bin/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'called\n' >> "$GH_CALL_LOG"
case "$*" in
    *'/releases'*) printf '%s\n' "$TEST_RELEASES" ;;
    *'/tags'*) printf '%s\n' "$TEST_RELEASES" ;;
    *) exit 1 ;;
esac
EOF
chmod +x "$scratch/bin/gh"
export PATH="$scratch/bin:$PATH" GH_CALL_LOG="$scratch/gh.log"
for digest in '' invalid; do
    jq -n --arg sha "$digest" '{fixtures:[{id:"test",artifact:{artifact_name:"fixture",workflow_run_id:1,sha256:$sha}}]}' > "$scratch/manifest.json"
    if bash "$SCRIPT_DIR/download-fixtures.sh" --manifest "$scratch/manifest.json" --dest "$scratch/download" --strict > "$scratch/download.log" 2>&1; then
        fail 'strict downloader accepted invalid digest'
    fi
    grep -q 'valid sha256' "$scratch/download.log" || fail 'digest validation did not fail first'
    [ ! -e "$GH_CALL_LOG" ] || fail 'invalid digest reached the network'
done
echo 'PASS: strict digest validation before network access'

check_coverage() {
    local baseline="$1" release="$2" expected="$3"
    jq -n --arg tag "$baseline" '{fixtures:[{git_tag:$tag}]}' > "$scratch/manifest.json"
    export TEST_RELEASES="$release"
    local result=0
    bash "$SCRIPT_DIR/check-coverage.sh" --manifest "$scratch/manifest.json" --enforce-after "$baseline" > "$scratch/coverage.log" 2>&1 || result=$?
    if [ "$expected" = missing ]; then
        [ "$result" -ne 0 ] || fail "coverage missed $release after $baseline"
        grep -q 'Published releases with no migration fixture' "$scratch/coverage.log" || fail 'coverage failed for an unrelated reason'
    else
        [ "$result" -eq 0 ] || fail "coverage incorrectly requires $release after $baseline"
    fi
}
check_coverage v1.0.0-weekly.20260908 v1.0.0 missing
check_coverage v1.0.0 v1.0.0-weekly.20260908 covered
check_coverage v1.0.0-weekly.9 v1.0.0-weekly.10 missing
check_coverage v1.0.0-beta.11 v1.0.0-rc.1 missing
check_coverage v1.9.0 v1.10.0 missing
check_coverage v1.0.0+one v1.0.0+two covered
echo 'PASS: release precedence coverage'
