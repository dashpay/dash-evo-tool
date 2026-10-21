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
    network-info)
        if [ -f "$DASH_EVO_DATA_DIR/network-choice" ]; then
            echo '{"active":"testnet"}'
        else
            echo '{"active":"mainnet"}'
        fi ;;

    core-wallet-import | core-balances-get) echo '{}' ;;
    network-switch)
        if [ "${FORGET_NETWORK:-0}" != 1 ]; then
            touch "$DASH_EVO_DATA_DIR/network-choice"
        fi
        echo '{"active":"testnet"}' ;;

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
if FORGET_NETWORK=1 FIXTURE_WALLET_MNEMONIC="$RANDOM-$RANDOM" bash "$SCRIPT_DIR/capture-headless.sh" \
    --data-dir "$scratch/forget-network" --det-cli "$scratch/bin/det-cli" > "$scratch/forget-network.log" 2>&1; then
    fail 'capture accepted a network switch lost on restart'
fi
grep -q 'did not survive restart' "$scratch/forget-network.log" || fail 'capture did not report lost network selection'
echo 'PASS: capture rejects a network switch lost on restart'


# TEST_RELEASES lines are TAG[|ASSET,ASSET...[|draft]]; TEST_ASSETS holds the
# files of each release and of the stored artifact, plus the names that fail
# attestation.
cat > "$scratch/bin/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$GH_CALL_LOG"
query='' dir='' patterns=()
for ((i = 1; i <= $#; i++)); do
    next=$((i + 1))
    case "${!i}" in
        --jq) query="${!next}" ;;
        --dir) dir="${!next}" ;;
        --pattern) patterns+=("${!next}") ;;
    esac
done
emit() { if [ -n "$query" ]; then jq -r "$query"; else cat; fi; }
case "$*" in
    'api '*'/releases'*)
        printf '%s\n' "$TEST_RELEASES" | jq -Rn '[inputs | select(length > 0) | split("|")
            | {tag_name: .[0], draft: (.[2] == "draft"),
               assets: ((.[1] // "") | split(",") | map({name: .}))}]' | emit ;;
    'api '*'/tags'*) printf '%s\n' "$TEST_RELEASES" | cut -d'|' -f1 ;;
    'api '*'/artifacts'*) jq -n '{artifacts: [{name: "stored-fixture", expired: false}]}' | emit ;;
    'run download '*) cp "$TEST_ASSETS/stored-fixture/"* "$dir/" ;;
    'release download '*)
        for pattern in "${patterns[@]}"; do cp "$TEST_ASSETS/$3/$pattern" "$dir/"; done ;;
    'attestation verify '*)
        [[ "$*" == *' --repo owner/repo '* && "$*" == *' --source-ref refs/heads/v1.0-dev'* &&
            "$*" == *' --signer-workflow owner/repo/.github/workflows/migration-fixture-capture.yml '* ]]
        ! grep -Fxq "$(basename "$3")" "$TEST_ASSETS/unattested" ;;
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
check_coverage v1.0.0-weekly.20260908 $'test-fix-directory-searching\nv0.9-Developer-Preview-3\nv1.0.0-weekly.20260908' covered
grep -q 'Skipping non-SemVer release tag: test-fix-directory-searching' "$scratch/coverage.log" || fail 'historical tag was not reported'
check_coverage v1.0.0-weekly.20260908 $'test-fix-directory-searching\nv1.0.0-weekly.20260915' missing
grep -q '^  v1.0.0-weekly.20260915$' "$scratch/coverage.log" || fail 'missing weekly fixture was not reported'
# A release is also covered by the fixture attached to it: archive and metadata.
pair() { printf 'migration-fixture-%s-wallet-only.tar.zst,migration-fixture-%s-wallet-only.json' "$1" "$1"; }
check_coverage v1.0.0 "v1.0.1|$(pair v1.0.1)" covered
check_coverage v1.0.0 'v1.0.1|migration-fixture-v1.0.1-wallet-only.tar.zst' missing
check_coverage v1.0.0 "v1.0.1|$(pair v1.0.0)" missing
check_coverage v1.0.0 $'v1.0.1\nv1.0.2|'"$(pair v1.0.2)" missing
grep -q '^  v1.0.1$' "$scratch/coverage.log" || fail 'an attached fixture hid an older uncovered release'
! grep -q '^  v1.0.2$' "$scratch/coverage.log" || fail 'a release with an attached fixture was reported'

jq -n '{fixtures:[{git_tag:"invalid-fixture-tag"}]}' > "$scratch/manifest.json"
if bash "$SCRIPT_DIR/check-coverage.sh" --manifest "$scratch/manifest.json" > "$scratch/coverage.log" 2>&1; then
    fail 'coverage accepted an invalid fixture tag'
fi
grep -q 'Invalid release tag: invalid-fixture-tag' "$scratch/coverage.log" || fail 'fixture tag validation failed for an unrelated reason'
echo 'PASS: release precedence coverage'

# Fixtures attached to releases: discovered, verified, merged into the manifest.
export TEST_ASSETS="$scratch/assets"
mkdir -p "$TEST_ASSETS/stored-fixture"
printf 'stored profile\n' > "$TEST_ASSETS/stored-fixture/stored.tar.zst"
attach_fixture() { # RELEASE_TAG [TAG_THE_METADATA_DESCRIBES]
    local dir="$TEST_ASSETS/$1" base="migration-fixture-$1-wallet-only" tag="${2:-$1}" sha
    mkdir -p "$dir"
    printf 'profile of %s\n' "$tag" > "$dir/$base.tar.zst"
    sha="$(sha256sum "$dir/$base.tar.zst" | cut -d' ' -f1)"
    jq -n --arg tag "$tag" --arg sha "$sha" --argjson bytes "$(stat -c %s "$dir/$base.tar.zst")" \
        '{git_tag: $tag, sha256: $sha, bytes: $bytes, binary_sha256: $sha,
          source: {git_tag: "v1.0.0", archive: "stored.tar.zst", sha256: $sha},
          wallets: [{seed_hash: $sha, alias: "weekly-fixture"}]}' > "$dir/receipt.json"
    [ "$tag" = "$1" ] || cp "$dir/$base.tar.zst" "$dir/migration-fixture-$tag-wallet-only.tar.zst"
    python3 "$SCRIPT_DIR/release_fixture.py" build --receipt "$dir/receipt.json" --tag "$tag" \
        --archive "$dir/migration-fixture-$tag-wallet-only.tar.zst" --det-version "${tag#v}+abcdef012" \
        --platform-rev "$(printf '%040d' 0)" --run-id 7 --out "$dir/$base.json"
}
# download EXPECTED_STATUS STORED_TAG [OPTIONS...]; leaves $merged and $scratch/download.log
download() {
    local expected="$1" stored_tag="$2" status=0
    shift 2
    merged="$scratch/merged.json"
    rm -rf "$scratch/download" "$merged" "$GH_CALL_LOG"
    jq -n --arg tag "$stored_tag" --arg sha "$(sha256sum "$TEST_ASSETS/stored-fixture/stored.tar.zst" | cut -d' ' -f1)" \
        '{schema_version: 1, fixtures: [{id: "\($tag)-wallet-only", git_tag: $tag, artifact: {artifact_name: "stored-fixture",
          workflow_run_id: 1, archive_filename: "stored.tar.zst", sha256: $sha}}]}' > "$scratch/manifest.json"
    bash "$SCRIPT_DIR/download-fixtures.sh" --manifest "$scratch/manifest.json" --repo owner/repo \
        --dest "$scratch/download" "$@" > "$scratch/download.log" 2>&1 || status=$?
    [ "$status" -eq "$expected" ] || { cat "$scratch/download.log" >&2; fail "download exited $status, expected $expected ($*)"; }
}
merged_ids() { jq -r '[.fixtures[].id] | join(" ")' "$merged"; }
release_archive() { echo "$scratch/download/migration-fixture-$1-wallet-only.tar.zst"; }

: > "$TEST_ASSETS/unattested"
attach_fixture v1.0.1
attach_fixture v1.0.2
TEST_RELEASES=$'v1.0.0\nv1.0.1|'"$(pair v1.0.1)"$'\nv1.0.2|'"$(pair v1.0.2)|draft"
download 0 v1.0.0 --strict --merged-manifest "$scratch/merged.json"
[ "$(merged_ids)" = 'v1.0.0-wallet-only v1.0.1-wallet-only' ] || fail "merged manifest lists: $(merged_ids)"
[ -f "$scratch/download/stored.tar.zst" ] || fail 'the stored archive is missing'
[ -f "$(release_archive v1.0.1)" ] || fail 'the attached archive is missing'
! grep -q 'release download v1.0.2' "$GH_CALL_LOG" || fail 'a draft release was read'
jq -e '.schema_version == 1' "$merged" > /dev/null || fail 'the merged manifest lost the manifest header'
echo 'PASS: attached fixtures are discovered and merged; drafts are skipped'

download 0 v1.0.0 --strict
! grep -q '/releases' "$GH_CALL_LOG" || fail 'releases were read without --merged-manifest'
echo 'PASS: releases are read only on request'

download 0 v1.0.1 --strict --merged-manifest "$scratch/merged.json"
[ "$(merged_ids)" = 'v1.0.1-wallet-only' ] || fail "manifest entry did not win: $(merged_ids)"
! grep -q 'release download' "$GH_CALL_LOG" || fail 'an attached fixture was fetched for a tag the manifest covers'
echo 'PASS: a manifest entry wins over an attached fixture'

# reject NAME: the prepared v1.0.1 fixture must fail --strict and never be used.
reject() {
    download 1 v1.0.0 --strict --merged-manifest "$scratch/merged.json"
    download 0 v1.0.0 --merged-manifest "$scratch/merged.json"
    [ "$(merged_ids)" = 'v1.0.0-wallet-only' ] || fail "$1 fixture entered the manifest: $(merged_ids)"
    [ ! -e "$(release_archive v1.0.1)" ] || fail "$1 fixture archive was kept"
    echo "PASS: $1 fixture is rejected"
}
TEST_RELEASES=$'v1.0.0\nv1.0.1|'"$(pair v1.0.1)"
echo 'migration-fixture-v1.0.1-wallet-only.json' > "$TEST_ASSETS/unattested"
reject 'an unattested'
: > "$TEST_ASSETS/unattested"
printf 'replaced\n' > "$TEST_ASSETS/v1.0.1/migration-fixture-v1.0.1-wallet-only.tar.zst"
reject 'a replaced-archive'
rm -rf "$TEST_ASSETS/v1.0.1"
attach_fixture v1.0.1 v1.0.2
reject 'another release'"'"'s'

rm -rf "$TEST_ASSETS/v1.0.1"
releases='v1.0.0'
for patch in 1 2 3 4 5 6 7 8 9 10; do
    attach_fixture "v1.1.$patch"
    releases+=$'\n'"v1.1.$patch|$(pair "v1.1.$patch")"
done
TEST_RELEASES="$releases"
download 0 v1.0.0 --strict --merged-manifest "$scratch/merged.json"
expected='v1.0.0-wallet-only'
for patch in 10 9 8 7 6 5 4 3; do expected+=" v1.1.$patch-wallet-only"; done
[ "$(merged_ids)" = "$expected" ] || fail "expected the 8 newest attached fixtures, got: $(merged_ids)"
echo 'PASS: only the 8 newest attached fixtures are used'

# A capture resolves the one fixture it starts from, outside that limit too.
# source_for EXPECTED_STATUS TAG: leaves the fetch counts in $fetched.
source_for() {
    download "$1" v1.0.0 --merged-manifest "$scratch/merged.json" --source-for "$2"
    fetched="$(grep -c 'run download' "$GH_CALL_LOG" || true) stored, $(grep -c 'release download' "$GH_CALL_LOG" || true) attached, $(grep -c 'attestation verify' "$GH_CALL_LOG" || true) verified"
}
source_for 0 v1.1.11
[ "$(merged_ids)" = 'v1.1.10-wallet-only' ] || fail "the capture of v1.1.11 starts from: $(merged_ids)"
[ "$fetched" = '0 stored, 1 attached, 1 verified' ] || fail "the capture of v1.1.11 fetched: $fetched"
[ -f "$(release_archive v1.1.10)" ] || fail 'the source archive is missing'
TEST_RELEASES="$(grep -v '^v1\.1\.2|' <<<"$releases")"$'\nv1.1.2'
source_for 0 v1.1.2
[ "$(merged_ids)" = 'v1.1.1-wallet-only' ] || fail "the capture of v1.1.2 starts from: $(merged_ids)"
[ "$fetched" = '0 stored, 1 attached, 1 verified' ] || fail "the capture of v1.1.2 fetched: $fetched"
echo 'migration-fixture-v1.1.1-wallet-only.json' > "$TEST_ASSETS/unattested"
source_for 1 v1.1.2
: > "$TEST_ASSETS/unattested"
TEST_RELEASES=$'v1.0.0\nv1.1.1'
source_for 1 v1.1.2
grep -q 'no fixture: v1.1.1' "$scratch/download.log" || fail 'the capture of v1.1.2 was not refused for skipping v1.1.1'
[ "$fetched" = '0 stored, 0 attached, 0 verified' ] || fail "a refused capture fetched: $fetched"
source_for 0 v1.1.1
[ "$(merged_ids)" = 'v1.0.0-wallet-only' ] || fail "the capture of v1.1.1 starts from: $(merged_ids)"
[ "$fetched" = '1 stored, 0 attached, 0 verified' ] || fail "the capture of v1.1.1 fetched: $fetched"
echo 'PASS: a capture resolves only the fixture it starts from, however many are newer'

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s "$SCRIPT_DIR" -p test_release_fixture.py -v
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s "$SCRIPT_DIR" -p test_chain.py -v
