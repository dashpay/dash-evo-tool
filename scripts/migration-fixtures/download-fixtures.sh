#!/usr/bin/env bash
#
# Fetch migration fixture archives named by tests/migration-fixtures/manifest.json
# into a local directory, for MIGRATION_FIXTURES_DIR to point at.
#
#   scripts/migration-fixtures/download-fixtures.sh --dest "$RUNNER_TEMP/fixtures"
#
# WHY THIS IS NOT `actions/download-artifact`
# -------------------------------------------
# Manifest fixtures are stored as GitHub Actions build artifacts. The consequence
# is that the artifact a matrix run needs was almost never produced by that run:
# a hand capture such as the v0.9.3 baseline is uploaded once and then consumed
# by every future run.
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
# FIXTURES ATTACHED TO RELEASES
# -----------------------------
# migration-fixture-capture.yml attaches a release's fixture to that release: an
# archive and its manifest entry, both attested. No commit records them, so with
# --merged-manifest this script finds them on the published releases, verifies
# them, and writes the manifest plus their entries for the matrix to read. The
# attestation is the only thing that vouches for an attached fixture — anyone
# with write access can replace a release asset — so one that fails it is never
# used. A release the manifest already covers is not looked at.
#
# The capture itself needs one fixture, the newest older than the release it
# captures. --source-for resolves exactly that one, from the manifest or from a
# release, however many newer fixtures exist.
#
# ARTIFACT RETENTION is the standing hazard: artifacts expire (90 days by
# default). An entry whose artifact has expired resolves to nothing and is
# reported as such rather than silently skipped — see --strict.

set -euo pipefail

MANIFEST=""
DEST=""
REPO="${GITHUB_REPOSITORY:-dashpay/dash-evo-tool}"
STRICT=0
MERGED_MANIFEST=""
SOURCE_FOR=""
# Only fixtures the capture workflow attested from this branch are trusted.
TRUSTED_REF="refs/heads/v1.0-dev"
# Attached fixtures used per run, newest first. Provisional: every weekly adds
# one, and the matrix boots each, so the number bounds CI time. It does not
# bound --source-for.
RELEASE_FIXTURE_LIMIT=8

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
  --merged-manifest PATH
                     also resolve the fixtures attached to published releases
                     and write the manifest plus their entries to PATH
  --source-for TAG   with --merged-manifest: resolve only the fixture a capture
                     of release TAG starts from and write that entry alone to
                     PATH; implies --strict
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
    --merged-manifest)
        MERGED_MANIFEST="${2:-}"
        shift
        ;;
    --source-for)
        SOURCE_FOR="${2:-}"
        STRICT=1
        shift
        ;;
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

only=""
if [ -n "$MERGED_MANIFEST" ]; then
    releases="$(gh api "repos/$REPO/releases" --paginate | jq -s 'add // []')" ||
        die "Could not list releases for $REPO."
fi
if [ -n "$SOURCE_FOR" ]; then
    [ -n "$MERGED_MANIFEST" ] || die "--source-for needs --merged-manifest"
    only="$(python3 "$SCRIPT_DIR/release_fixture.py" source --tag "$SOURCE_FOR" \
        --manifest "$MANIFEST" --releases <(printf '%s' "$releases"))" ||
        die "No fixture to start the capture of $SOURCE_FOR from."
    ENTRIES="$(jq -c --arg only "$only" 'select(.git_tag == $only)' <<<"$ENTRIES")"
fi

resolved=0
failed=0

while IFS= read -r entry; do
    [ -n "$entry" ] || continue

    get() { echo "$entry" | jq -r "$1 // empty"; }

    id="$(get '.id')"
    id="${id:-$(get '.git_tag')}"
    id="${id:-unnamed}"
    # The `artifact.*` keys documented in tests/migration-fixtures/README.md
    # come first; the rest are shapes this reader accepted before that schema
    # settled, kept so an older entry still resolves.
    artifact_name="$(get '.artifact.artifact_name')"
    artifact_name="${artifact_name:-$(get '.artifact.name')}"
    artifact_name="${artifact_name:-$(get '.artifact_name')}"
    run_id="$(get '.artifact.workflow_run_id')"
    run_id="${run_id:-$(get '.artifact.run_id')}"
    run_id="${run_id:-$(get '.run_id')}"
    workflow="$(get '.artifact.workflow_name')"
    workflow="${workflow:-$(get '.artifact.workflow')}"
    workflow="${workflow:-$(get '.workflow')}"
    expected_sha="$(get '.artifact.sha256')"
    expected_sha="${expected_sha:-$(get '.archive.sha256')}"
    expected_sha="${expected_sha:-$(get '.sha256')}"
    expected_file="$(get '.artifact.archive_filename')"

    note "Fixture '$id'"

    if [ "$STRICT" -eq 1 ] && [[ ! "$expected_sha" =~ ^[0-9a-fA-F]{64}$ ]]; then
        warn "Fixture '$id' needs a valid sha256 in strict mode — skipping."
        failed=$((failed + 1))
        continue
    fi
    expected_sha="${expected_sha,,}"

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

    # The harness locates the archive by this name, so a mismatch here would
    # only resurface later as a less specific "archive not found".
    if [ -n "$expected_file" ] && [ "$(basename -- "$archive_path")" != "$expected_file" ]; then
        warn "Fixture '$id': artifact '$artifact_name' holds $(basename -- "$archive_path"), but the manifest names $expected_file."
        rm -rf "$stage"
        failed=$((failed + 1))
        continue
    fi

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

# --------------------------------------------------------------------------
# Fixtures attached to releases.
# --------------------------------------------------------------------------

if [ -n "$MERGED_MANIFEST" ]; then
    release_tags="$(jq -L "$SCRIPT_DIR" -r --argjson limit "$RELEASE_FIXTURE_LIMIT" \
        --arg only "$only" --slurpfile manifest "$MANIFEST" 'include "semver";
        [$manifest[0] | if type == "array" then . else (.fixtures // []) end | .[].git_tag] as $covered
        | [.[] | select(.draft == false) | .tag_name as $tag
           | "migration-fixture-\($tag)-wallet-only" as $fixture
           | select(any(.assets[].name; . == "\($fixture).tar.zst")
                    and any(.assets[].name; . == "\($fixture).json"))
           | $tag | select(try (semver_key | true) catch false)]
        | sort_by(semver_key) | reverse
        | if $only == "" then .[:$limit] else map(select(. == $only)) end
        | map(select(IN($covered[]) | not)) | .[]' <<<"$releases")" ||
        die "Could not read the release list of $REPO."

    entries="$DEST/.release-fixtures.jsonl"
    : >"$entries"
    while IFS= read -r tag; do
        [ -n "$tag" ] || continue
        note "Fixture attached to release '$tag'"
        fixture="migration-fixture-$tag-wallet-only"
        stage="$DEST/.staging-release-$tag"
        rm -rf "$stage"
        mkdir -p "$stage"
        # The entry is what the attestation vouches for; it pins the archive's
        # size and digest, which release_fixture.py then holds the archive to.
        if gh release download "$tag" --repo "$REPO" --dir "$stage" \
            --pattern "$fixture.tar.zst" --pattern "$fixture.json" &&
            gh attestation verify "$stage/$fixture.json" --repo "$REPO" \
                --signer-workflow "$REPO/.github/workflows/migration-fixture-capture.yml" \
                --source-ref "$TRUSTED_REF" >/dev/null &&
            python3 "$SCRIPT_DIR/release_fixture.py" check --tag "$tag" \
                --metadata "$stage/$fixture.json" --archive "$stage/$fixture.tar.zst"; then
            jq -c . "$stage/$fixture.json" >>"$entries"
            mv "$stage/$fixture.tar.zst" "$DEST/"
            resolved=$((resolved + 1))
            note "  attestation and checksum OK"
        else
            warn "Fixture attached to release '$tag' could not be downloaded or verified as attested by migration-fixture-capture.yml on $TRUSTED_REF — not used."
            failed=$((failed + 1))
        fi
        rm -rf "$stage"
    done <<<"$release_tags"
    jq --arg only "$only" --slurpfile attached "$entries" \
        'if type == "array" then {fixtures: .} else . end
         | if $only == "" then . else .fixtures |= map(select(.git_tag == $only)) end
         | .fixtures += $attached' "$MANIFEST" >"$MERGED_MANIFEST"
    rm -f "$entries"
fi

note "Resolved $resolved fixture(s) into $DEST; $failed unresolved."

if [ "$failed" -gt 0 ] && [ "$STRICT" -eq 1 ]; then
    die "$failed fixture(s) in $MANIFEST could not be resolved."
fi
if [ "$resolved" -eq 0 ] && [ "$STRICT" -eq 1 ]; then
    die "No fixtures could be resolved from $MANIFEST."
fi
