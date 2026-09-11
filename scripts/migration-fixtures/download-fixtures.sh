#!/usr/bin/env bash
#
# Fetch migration fixture archives named by tests/migration-fixtures/manifest.json
# into a local directory, for MIGRATION_FIXTURES_DIR to point at.
#
#   scripts/migration-fixtures/download-fixtures.sh --dest "$RUNNER_TEMP/fixtures"
#
# WHY THIS IS NOT `actions/download-artifact`
# -------------------------------------------
# Fixtures are stored as GitHub Actions build artifacts, not Release assets (a
# deliberate decision in the migration-matrix plan). The consequence is that the
# artifact a matrix run needs was almost never produced by that run: the v0.9.3
# baseline is uploaded once and then consumed by every future run, and each
# release's fixture is uploaded by the capture job that follows that release.
# `actions/download-artifact` without a `run-id` only sees the CURRENT run's
# artifacts, so it finds nothing here. It does accept `run-id` + `github-token`
# for cross-run downloads, but only one artifact per step and only when the id
# is already known at YAML-authoring time, which it is not — it lives in the
# manifest.
#
# So resolution happens here, in order:
#   1. the manifest entry carries an explicit run id -> download from that exact
#      run. No guessing; the manifest is the durable pointer, which is the whole
#      reason it is committed to the repo.
#   2. otherwise -> search the newest successful run of the named workflow for
#      an artifact of the given name. Weaker (a re-run can shift which run holds
#      it) but it keeps a manifest entry usable before anyone records a run id.
#
# `dawidd6/action-download-artifact` is the well-known third-party action for
# case 2 and would work. It is avoided deliberately: this job downloads data
# that later gets executed against a real binary, so the fewer third parties in
# that path the better, and `gh` gives exact run-id resolution that a
# name-pattern search cannot. If a maintainer later prefers the action, it is a
# drop-in replacement for this script's fallback branch only.
#
# ARTIFACT RETENTION is the standing hazard: artifacts expire (90 days by
# default). An entry whose artifact has expired resolves to nothing and is
# reported as such rather than silently skipped — see --strict.

set -euo pipefail

MANIFEST=""
DEST=""
REPO="${GITHUB_REPOSITORY:-dashpay/dash-evo-tool}"
STRICT=0

die() {
    echo "::error::$*" >&2
    exit 1
}
warn() { echo "::warning::$*" >&2; }
note() { echo "==> $*"; }

usage() {
    cat <<'EOF'
usage: download-fixtures.sh --dest DIR [options]

  --dest DIR         directory to download and unpack fixtures into (required)
  --manifest PATH    manifest to read (default: tests/migration-fixtures/manifest.json)
  --repo OWNER/NAME  repository owning the artifacts (default: $GITHUB_REPOSITORY)
  --strict           exit non-zero when the manifest is empty or any entry
                     cannot be resolved (default: warn and continue)
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
    --dest)
        DEST="${2:-}"
        shift
        ;;
    --manifest)
        MANIFEST="${2:-}"
        shift
        ;;
    --repo)
        REPO="${2:-}"
        shift
        ;;
    --strict) STRICT=1 ;;
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

[ -n "$DEST" ] || {
    usage >&2
    die "--dest is required"
}
command -v jq >/dev/null 2>&1 || die "jq is required but not installed."
command -v gh >/dev/null 2>&1 || die "the GitHub CLI (gh) is required but not installed."

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
MANIFEST="${MANIFEST:-$REPO_ROOT/tests/migration-fixtures/manifest.json}"

mkdir -p "$DEST"
DEST="$(cd -- "$DEST" && pwd)"

# The manifest is owned by the fixture-capture workstream and may legitimately
# not exist yet. Treat absent and empty the same way.
if [ ! -f "$MANIFEST" ]; then
    if [ "$STRICT" -eq 1 ]; then
        die "Manifest not found: $MANIFEST"
    fi
    warn "No fixture manifest at $MANIFEST — nothing to download."
    exit 0
fi

# Accept either a bare array or an object with a "fixtures" array, so the schema
# can settle without breaking this reader.
ENTRIES="$(jq -c 'if type == "array" then . else (.fixtures // []) end | .[]' "$MANIFEST" 2>/dev/null)" ||
    die "Manifest $MANIFEST is not valid JSON."

if [ -z "$ENTRIES" ]; then
    if [ "$STRICT" -eq 1 ]; then
        die "Manifest $MANIFEST lists no fixtures."
    fi
    warn "Manifest $MANIFEST lists no fixtures yet — nothing to download."
    exit 0
fi

resolved=0
failed=0

while IFS= read -r entry; do
    [ -n "$entry" ] || continue

    get() { echo "$entry" | jq -r "$1 // empty"; }

    id="$(get '.id')"
    id="${id:-$(get '.git_tag')}"
    id="${id:-unnamed}"
    artifact_name="$(get '.artifact.name')"
    artifact_name="${artifact_name:-$(get '.artifact_name')}"
    run_id="$(get '.artifact.run_id')"
    run_id="${run_id:-$(get '.run_id')}"
    workflow="$(get '.artifact.workflow')"
    workflow="${workflow:-$(get '.workflow')}"
    expected_sha="$(get '.archive.sha256')"
    expected_sha="${expected_sha:-$(get '.sha256')}"

    note "Fixture '$id'"

    if [ -z "$artifact_name" ]; then
        warn "Fixture '$id' has no artifact name in the manifest — skipping."
        failed=$((failed + 1))
        continue
    fi

    # --- resolve the run holding the artifact -----------------------------
    if [ -z "$run_id" ]; then
        if [ -z "$workflow" ]; then
            warn "Fixture '$id' gives neither an artifact run id nor a workflow to search — skipping."
            failed=$((failed + 1))
            continue
        fi
        note "  no run id recorded; searching newest successful run of $workflow"
        run_id="$(gh api "repos/$REPO/actions/workflows/$workflow/runs?status=success&per_page=50" \
            --jq '.workflow_runs | sort_by(.run_started_at) | reverse | .[0].id // empty' 2>/dev/null || true)"
        if [ -z "$run_id" ]; then
            warn "Fixture '$id': no successful run of $workflow found in $REPO — skipping."
            failed=$((failed + 1))
            continue
        fi
    fi

    # An expired artifact still appears in the API with expired == true, and
    # downloading it fails with an unhelpful 410. Say so plainly instead.
    expired="$(gh api "repos/$REPO/actions/runs/$run_id/artifacts?per_page=100" \
        --jq ".artifacts[] | select(.name == \"$artifact_name\") | .expired" 2>/dev/null | head -n1 || true)"
    if [ -z "$expired" ]; then
        warn "Fixture '$id': artifact '$artifact_name' is not present in run $run_id of $REPO. It may have been deleted, or the run id in the manifest may be stale."
        failed=$((failed + 1))
        continue
    fi
    if [ "$expired" = "true" ]; then
        warn "Fixture '$id': artifact '$artifact_name' in run $run_id has EXPIRED. Re-capture it, or re-upload it to refresh its retention window."
        failed=$((failed + 1))
        continue
    fi

    # Download into a per-fixture staging directory rather than straight into
    # $DEST: it keeps two fixtures from colliding on a shared file name, and it
    # makes "which file did THIS entry produce" an exact answer rather than a
    # guess based on modification times.
    safe_id="$(printf '%s' "$id" | tr -c 'A-Za-z0-9._-' '-')"
    stage="$DEST/.staging-$safe_id"
    rm -rf "$stage"
    mkdir -p "$stage"

    note "  downloading '$artifact_name' from run $run_id"
    if ! gh run download "$run_id" --repo "$REPO" --name "$artifact_name" --dir "$stage"; then
        warn "Fixture '$id': download of '$artifact_name' from run $run_id failed."
        rm -rf "$stage"
        failed=$((failed + 1))
        continue
    fi

    mapfile -t archives < <(find "$stage" -type f \( -name '*.tar.zst' -o -name '*.tar.gz' \) | LC_ALL=C sort)
    if [ "${#archives[@]}" -ne 1 ]; then
        warn "Fixture '$id': expected exactly one .tar.zst/.tar.gz inside artifact '$artifact_name', found ${#archives[@]}."
        rm -rf "$stage"
        failed=$((failed + 1))
        continue
    fi
    archive_path="${archives[0]}"

    # --- integrity --------------------------------------------------------
    if [ -n "$expected_sha" ]; then
        actual_sha="$(sha256sum "$archive_path" | cut -d' ' -f1)"
        if [ "$actual_sha" != "$expected_sha" ]; then
            die "Fixture '$id': checksum mismatch on $(basename -- "$archive_path").
  manifest $expected_sha
  actual   $actual_sha
The artifact does not match what the manifest describes. Refusing to run migration tests against unverified fixture data."
        fi
        note "  checksum OK"
    else
        warn "Fixture '$id' records no sha256 — downloaded archive is unverified."
    fi

    mv "$archive_path" "$DEST/"
    # Keep the .sha256 sidecar pack.sh emitted, if the artifact carried it.
    find "$stage" -type f -name '*.sha256' -exec mv -t "$DEST/" {} +
    rm -rf "$stage"

    resolved=$((resolved + 1))
done <<<"$ENTRIES"

note "Resolved $resolved fixture(s) into $DEST; $failed unresolved."

if [ "$failed" -gt 0 ] && [ "$STRICT" -eq 1 ]; then
    die "$failed fixture(s) in $MANIFEST could not be resolved."
fi
if [ "$resolved" -eq 0 ] && [ "$STRICT" -eq 1 ]; then
    die "No fixtures could be resolved from $MANIFEST."
fi
