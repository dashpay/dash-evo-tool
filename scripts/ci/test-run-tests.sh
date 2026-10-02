#!/usr/bin/env bash
# Verify suite concurrency, environment isolation and failure propagation offline.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/ci-suite-tests.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin"
cat > "$scratch/bin/cargo" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ " $* " == *' --all-features '* && " $* " == *' --locked '* ]]
printf '%s\n' "$*" >> "$CASE_DIR/calls"
echo "mock cargo: $*"
if [[ " $* " == *' --workspace '* ]]; then
    [[ -z "${MIGRATION_FIXTURES_DIR+x}" && -z "${MIGRATION_V093_WALLET_PASSWORD+x}" ]]
    touch "$CASE_DIR/tests-started"
    if [[ "$CASE_NAME" == download-failure ]]; then
        echo 'ordinary tests still ran after the download failed'
        exit 0
    fi
    for ((i=0; i<100; i++)); do
        [[ -e "$CASE_DIR/matrix-started" ]] && break
        sleep 0.02
    done
    [[ -e "$CASE_DIR/matrix-started" ]]
    echo 'ordinary tests and doctests ran'
    if [[ "$CASE_NAME" == unit-failure || "$CASE_NAME" == both-fail ]]; then
        exit 37
    fi
elif [[ " $* " == *' --test migration-matrix '* ]]; then
    [[ -d "$MIGRATION_FIXTURES_DIR" && "$MIGRATION_V093_WALLET_PASSWORD" == disposable-test-password ]]
    if [[ "$CASE_NAME" == fork ]]; then
        [[ "$MIGRATION_MATRIX_SKIP_PASSWORDS" == true ]]
    fi
    touch "$CASE_DIR/matrix-started"
    for ((i=0; i<100; i++)); do
        [[ -e "$CASE_DIR/tests-started" ]] && break
        sleep 0.02
    done
    [[ -e "$CASE_DIR/tests-started" ]]
    if [[ "$CASE_NAME" == matrix-failure || "$CASE_NAME" == both-fail ]]; then
        exit 42
    fi
    [[ "$CASE_NAME" == empty-matrix ]] || echo 'Running 3 migration fixture(s)'
    echo 'test result: ok. 1 passed; 0 failed;'
elif [[ " $* " == *' --lib platform_compatibility_upgrades_the_real '* ]]; then
    [[ "$CASE_NAME" != compatibility-failure ]]
    [[ "$CASE_NAME" != skipped-compatibility ]] || echo 'skipped: fixtures are missing'
    echo 'test result: ok. 1 passed; 0 failed;'
else
    echo "unexpected cargo invocation: $*" >&2
    exit 2
fi
MOCK
chmod +x "$scratch/bin/cargo"
for scenario in success fork unit-failure matrix-failure both-fail empty-matrix compatibility-failure skipped-compatibility download-failure; do
    case_dir="$scratch/$scenario"
    mkdir -p "$case_dir/fixtures"
    fixture_dir="$case_dir/fixtures"
    skip_passwords=false
    [[ "$scenario" != download-failure ]] || fixture_dir=''
    [[ "$scenario" != fork ]] || skip_passwords=true
    status=0
    env PATH="$scratch/bin:$PATH" CASE_DIR="$case_dir" CASE_NAME="$scenario" \
        MIGRATION_FIXTURES_DIR="$fixture_dir" MIGRATION_MATRIX_SKIP_PASSWORDS="$skip_passwords" \
        MIGRATION_FIXTURES_MANIFEST="$case_dir/manifest.json" \
        MIGRATION_V093_WALLET_PASSWORD=disposable-test-password \
        bash "$script_dir/run-tests.sh" "$case_dir/logs" > "$case_dir/output" 2>&1 || status=$?
    expected_status=1
    [[ "$scenario" != success && "$scenario" != fork ]] || expected_status=0
    if [[ "$status" != "$expected_status" ]]; then
        cat "$case_dir/output"
        echo "FAIL: $scenario returned $status" >&2
        exit 1
    fi
    [[ -e "$case_dir/tests-started" ]]
    if [[ "$scenario" == download-failure ]]; then
        [[ ! -e "$case_dir/matrix-started" ]]
    else
        [[ -e "$case_dir/matrix-started" ]]
    fi
    [[ -s "$case_dir/logs/tests.log" && -s "$case_dir/logs/migration.log" ]]
    case "$scenario" in
        unit-failure) grep -Fxq 'Suite exit codes: tests=37 migration=0' "$case_dir/output" ;;
        matrix-failure) grep -Fxq 'Suite exit codes: tests=0 migration=42' "$case_dir/output" ;;
        both-fail) grep -Fxq 'Suite exit codes: tests=37 migration=42' "$case_dir/output" ;;
    esac
    {
        echo '::group::Tests and doctests'
        cat "$case_dir/logs/tests.log"
        echo '::endgroup::'
        echo '::group::Archived migrations'
        cat "$case_dir/logs/migration.log"
        echo '::endgroup::'
    } > "$case_dir/expected-groups"
    sed '/^Suite exit codes:/,$d' "$case_dir/output" > "$case_dir/actual-groups"
    diff -u "$case_dir/expected-groups" "$case_dir/actual-groups"
    echo "PASS: $scenario"
done
