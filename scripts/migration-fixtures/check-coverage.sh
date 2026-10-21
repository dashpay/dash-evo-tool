#!/usr/bin/env bash
#
# Fail when a published release newer than the newest captured fixture has no
# fixture of its own.
#
#   scripts/migration-fixtures/check-coverage.sh
#
# The migration matrix is only worth anything if the chain of fixtures keeps up
# with the chain of releases: the rule from the plan is that every release from
# the next one onwards gets a fixture, so that the N -> N+1 pair is always
# testable. Nothing enforces that at release time, and a gap is invisible — the
# matrix stays green while quietly testing an ever-older pair. This check is the
# enforcement, and it runs on the weekly schedule rather than per-PR because a
# gap is created by cutting a release, not by editing code.
#
# A release is covered by a manifest entry, or by the fixture attached to it:
# the archive and metadata assets migration-fixture-capture.yml uploads. Only
# their presence is checked here; download-fixtures.sh verifies them.
#
# Releases BEFORE the newest manifest fixture are not reported. Backfilling
# history is a deliberate decision, not an oversight to nag about; only forward
# gaps matter. An attached fixture covers its own release and nothing older.
#
# Source of truth is the releases API filtered to draft == false, intersected
# with the tags API. Tags alone would false-positive: weekly-build.yml pushes
# the tag first and publishes the release at the end of the run, so a tag can
# exist for a release that was never published (and, on build failure, gets
# deleted again).

set -euo pipefail

# --------------------------------------------------------------------------
# Configuration
# --------------------------------------------------------------------------

# Enforcement starts with the first release published AFTER this tag; this tag
# and everything before it are exempt.
#
# The agreed baseline today is v0.9.3 alone, and the weeklies already published
# are deliberately not backfilled: upgrading from v0.9.3 predates the
# platform-wallet rewrite, so it always takes the "unwire" path and builds the
# persister from scratch, which is the one case structurally immune to the pin
# divergence this whole effort exists to catch. Without this knob the check
# would open by demanding five fixtures nobody agreed to capture.
#
# Move this forward only to write off a release on purpose.
ENFORCE_AFTER="${MIGRATION_COVERAGE_ENFORCE_AFTER:-v1.0.0-weekly.20260908}"

# --------------------------------------------------------------------------

REPO="${GITHUB_REPOSITORY:-dashpay/dash-evo-tool}"
MANIFEST=""

die() {
    echo "::error::$*" >&2
    exit 1
}
warn() { echo "::warning::$*" >&2; }
note() { echo "==> $*"; }

usage() {
    cat <<'EOF'
usage: check-coverage.sh [options]

  --manifest PATH      manifest to read (default: tests/migration-fixtures/manifest.json)
  --repo OWNER/NAME    repository whose releases to check (default: $GITHUB_REPOSITORY)
  --enforce-after TAG  exempt this release and everything before it
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
    --manifest)
        MANIFEST="${2:-}"
        shift
        ;;
    --repo)
        REPO="${2:-}"
        shift
        ;;
    --enforce-after)
        ENFORCE_AFTER="${2:-}"
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

command -v jq >/dev/null 2>&1 || die "jq is required but not installed."
command -v gh >/dev/null 2>&1 || die "the GitHub CLI (gh) is required but not installed."

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
MANIFEST="${MANIFEST:-$REPO_ROOT/tests/migration-fixtures/manifest.json}"

# --------------------------------------------------------------------------
# Fixture tags from the manifest.
# --------------------------------------------------------------------------

if [ ! -f "$MANIFEST" ]; then
    warn "No fixture manifest at $MANIFEST — coverage cannot be assessed yet."
    exit 0
fi

# Tolerant of both a bare array and { "fixtures": [...] }.
FIXTURE_TAGS="$(jq -r 'if type == "array" then . else (.fixtures // []) end | .[] | .git_tag // empty' "$MANIFEST" 2>/dev/null)" ||
    die "Manifest $MANIFEST is not valid JSON."

if [ -z "$FIXTURE_TAGS" ]; then
    # Bootstrap state: with no fixtures at all there is no "newest fixture" to
    # measure forward gaps against, so there is nothing to fail on.
    warn "Manifest $MANIFEST lists no fixtures with a git_tag yet — skipping the coverage check. It starts enforcing once the first fixture is recorded."
    exit 0
fi

newest_tag() {
    jq -L "$SCRIPT_DIR" -Rsr 'include "semver"; split("\n") | map(select(length > 0)) | max_by(semver_key)'
}
NEWEST_FIXTURE="$(printf '%s\n' "$FIXTURE_TAGS" | newest_tag)"
note "Newest fixture tag: $NEWEST_FIXTURE"

# --------------------------------------------------------------------------
# Published release tags.
# --------------------------------------------------------------------------

# One line per published release: its tag, and whether it carries both assets.
# shellcheck disable=SC2016 # jq variables, not shell expansions
RELEASES="$(gh api "repos/$REPO/releases" --paginate --jq '.[] | select(.draft == false)
    | "migration-fixture-\(.tag_name)-wallet-only" as $fixture
    | [.tag_name, (any(.assets[].name; . == "\($fixture).tar.zst")
                   and any(.assets[].name; . == "\($fixture).json"))] | @tsv' 2>/dev/null)" ||
    die "Could not list releases for $REPO. Check that GH_TOKEN is set and has read access."
PUBLISHED="$(cut -f1 <<<"$RELEASES")"
ATTACHED="$(awk -F'\t' '$2 == "true" { print $1 }' <<<"$RELEASES")"

ALL_TAGS="$(gh api "repos/$REPO/tags" --paginate --jq '.[].name' 2>/dev/null)" ||
    die "Could not list tags for $REPO. Check that GH_TOKEN is set and has read access."

# Intersection: a tag that exists AND whose release is published.
CANDIDATES="$(comm -12 \
    <(printf '%s\n' "$PUBLISHED" | LC_ALL=C sort -u) \
    <(printf '%s\n' "$ALL_TAGS" | LC_ALL=C sort -u))"

if [ -z "$CANDIDATES" ]; then
    warn "No published releases found for $REPO — nothing to check coverage against."
    exit 0
fi

# --------------------------------------------------------------------------
# Forward gaps.
# --------------------------------------------------------------------------

# Only releases newer than BOTH the newest fixture and the exemption tag count.
# Taking the greater of the two keeps the two rules from cancelling each other:
# the exemption must not silence a genuine gap above it, and a newer fixture
# must not resurrect the releases the exemption wrote off.
BASELINE="$(printf '%s\n%s\n' "$NEWEST_FIXTURE" "$ENFORCE_AFTER" | newest_tag)"
if [ "$BASELINE" != "$NEWEST_FIXTURE" ]; then
    note "Enforcement baseline: $BASELINE (exempt by configuration, newer than the newest fixture)"
else
    note "Enforcement baseline: $BASELINE"
fi

missing=()
while IFS= read -r tag; do
    [ -n "$tag" ] || continue
    # Historical test/preview tags do not follow SemVer and have no ordering.
    if ! key="$(jq -L "$SCRIPT_DIR" -en --arg tag "$tag" \
        'include "semver"; $tag | semver_key' 2>/dev/null)"; then
        warn "Skipping non-SemVer release tag: $tag"
        continue
    fi
    newer="$(jq -L "$SCRIPT_DIR" -nr --argjson key "$key" --arg baseline "$BASELINE" \
        'include "semver"; $key > ($baseline | semver_key)')"
    [ "$newer" = true ] || continue
    # Already covered?
    if printf '%s\n' "$FIXTURE_TAGS" "$ATTACHED" | grep -Fxq "$tag"; then
        continue
    fi
    missing+=("$tag")
done <<<"$CANDIDATES"

if [ "${#missing[@]}" -eq 0 ]; then
    note "Fixture coverage OK: no published release newer than $BASELINE is missing a fixture."
    exit 0
fi

printf 'Published releases with no migration fixture:\n' >&2
printf '  %s\n' "${missing[@]}" >&2
die "${#missing[@]} published release(s) newer than $BASELINE have neither an attached fixture nor an entry in $MANIFEST.
Until each is captured, the migration matrix silently tests an older version pair than the one users are actually upgrading across.
Capture each, oldest first: gh workflow run migration-fixture-capture.yml -f tag=<tag> -f attach=true
A release that workflow cannot capture needs a manual capture and a manifest entry; see tests/migration-fixtures/README.md."
