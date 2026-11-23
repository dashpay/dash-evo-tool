#!/usr/bin/env bash
#
# Verify the migration fixture archives committed with
# tests/migration-fixtures/manifest.json and, with --merged-manifest, fetch the
# fixtures attached to releases into a local directory for
# MIGRATION_FIXTURES_DIR.
#
#   scripts/migration-fixtures/download-fixtures.sh --dest "$RUNNER_TEMP/fixtures" \
#       --merged-manifest "$RUNNER_TEMP/fixtures/manifest.json"
#
# COMMITTED ARCHIVES
# ------------------
# A manifest entry names its archive by `archive.path`, relative to the
# manifest's directory, with its `sha256` and `bytes`. The archive stays where
# it is; this script holds it to the entry, and a mismatch always fails.
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
# A fixture that cannot be used. --strict stops at the first one, so the step
# fails at once with the reason and fetches nothing further; otherwise warn and
# count it.
unresolved() {
    [ "$STRICT" -eq 0 ] || die "$*"
    warn "$*"
    failed=$((failed + 1))
}

usage() {
    cat <<'EOF'
usage: download-fixtures.sh --dest DIR [options]

  --dest DIR         directory to download attached fixtures into (required)
  --manifest PATH    manifest to read (default: tests/migration-fixtures/manifest.json)
  --repo OWNER/NAME  repository owning the releases (default: $GITHUB_REPOSITORY)
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

# Archive paths are relative to the manifest's directory.
FIXTURES_ROOT="$(cd -- "$(dirname -- "$MANIFEST")" && pwd)"

while IFS= read -r entry; do
    [ -n "$entry" ] || continue

    get() { jq -r "$1 // empty" <<<"$entry"; }

    id="$(get '.id')"
    id="${id:-$(get '.git_tag')}"
    id="${id:-unnamed}"
    path="$(get '.archive.path')"
    expected_sha="$(get '.archive.sha256')"
    expected_bytes="$(get '.archive.bytes')"

    note "Fixture '$id'"

    if [[ -z "$path" || "$path" == /* || "/$path/" == */../* || "/$path/" == */./* ]]; then
        unresolved "Fixture '$id' names no archive path below $FIXTURES_ROOT."
        continue
    fi
    if [[ ! "$expected_sha" =~ ^[0-9a-f]{64}$ || ! "$expected_bytes" =~ ^[0-9]+$ ]]; then
        unresolved "Fixture '$id' needs a lowercase hex sha256 and a byte size."
        continue
    fi
    archive="$FIXTURES_ROOT/$path"
    if [ ! -f "$archive" ]; then
        unresolved "Fixture '$id': archive $path is not in $FIXTURES_ROOT."
        continue
    fi

    actual_bytes="$(stat -c %s "$archive")"
    actual_sha="$(sha256sum "$archive" | cut -d' ' -f1)"
    if [ "$actual_bytes" != "$expected_bytes" ] || [ "$actual_sha" != "$expected_sha" ]; then
        die "Fixture '$id': $path does not match its manifest entry.
  manifest $expected_sha, $expected_bytes bytes
  actual   $actual_sha, $actual_bytes bytes
Refusing to run migration tests against unverified fixture data."
    fi
    note "  size and checksum OK"

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
            unresolved "Fixture attached to release '$tag' could not be downloaded or verified as attested by migration-fixture-capture.yml on $TRUSTED_REF — not used."
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
