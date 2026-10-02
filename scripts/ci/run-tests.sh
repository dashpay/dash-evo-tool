#!/usr/bin/env bash
# Run both suites against the all-features build already present in this checkout.
set -euo pipefail
log_dir="${1:?Usage: run-tests.sh LOG_DIRECTORY}"
mkdir -p "$log_dir"

run_migrations() {
    : "${MIGRATION_FIXTURES_DIR:?Migration fixture download must succeed}"
    : "${MIGRATION_FIXTURES_MANIFEST:?Migration manifest must be supplied}"
    if [[ "${MIGRATION_MATRIX_SKIP_PASSWORDS:-false}" == true ]]; then
        echo '::notice::Fork PR: running password-free migration scenarios only.'
        if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
            echo 'Migration coverage: password-free scenarios only (fork PR).' >> "$GITHUB_STEP_SUMMARY"
        fi
    fi
    cargo test --locked --all-features --test migration-matrix -- migration_matrix --exact --nocapture \
        2>&1 | tee "$log_dir/archived-profiles.log"
    if ! grep -Eq 'Running [1-9][0-9]* migration fixture\(s\)' "$log_dir/archived-profiles.log" || \
       ! grep -q 'test result: ok. 1 passed; 0 failed;' "$log_dir/archived-profiles.log"; then
        echo '::error::The migration matrix did not execute the downloaded fixtures.'
        return 1
    fi
    cargo test --locked --all-features --lib platform_compatibility_upgrades_the_real -- --nocapture \
        2>&1 | tee "$log_dir/platform-compatibility.log"
    if ! grep -q 'test result: ok. 1 passed; 0 failed;' "$log_dir/platform-compatibility.log" || \
       grep -q '^skipped:' "$log_dir/platform-compatibility.log"; then
        echo '::error::The real-data compatibility test did not execute the downloaded fixture.'
        return 1
    fi
}

# The normal suite covers the bundled fixtures and doctests, but must not run
# archived profiles a second time or inherit their password.
env -u MIGRATION_FIXTURES_DIR -u MIGRATION_FIXTURES_MANIFEST \
    -u MIGRATION_V093_WALLET_PASSWORD -u MIGRATION_MATRIX_SKIP_PASSWORDS \
    cargo test --locked --all-features --workspace 2>&1 | tee "$log_dir/tests.log" &
tests_pid=$!
run_migrations 2>&1 | tee "$log_dir/migration.log" &
migrations_pid=$!

# Wait for both even when one fails; neither result may hide the other's failure.
tests_status=0
migrations_status=0
wait "$tests_pid" || tests_status=$?
wait "$migrations_pid" || migrations_status=$?
printf 'Suite exit codes: tests=%s migration=%s\n' "$tests_status" "$migrations_status"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    printf '| Suite | Exit code |\n|---|---|\n| Tests and doctests | %s |\n| Archived migrations | %s |\n' \
        "$tests_status" "$migrations_status" >> "$GITHUB_STEP_SUMMARY"
fi
[[ "$tests_status" == 0 && "$migrations_status" == 0 ]]
