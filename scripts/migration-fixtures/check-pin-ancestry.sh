#!/usr/bin/env bash
#
# Guard against a platform pin that forks the wallet-storage migration history.
#
# Two independent checks, both of which must pass:
#
#   (a) ANCESTRY  — the rev pinned in Cargo.toml is an ancestor of TARGET_BRANCH
#                   upstream. A pin taken from a side branch is how DET ended up
#                   needing the platform_compatibility bridge in the first place.
#
#   (b) FINGERPRINT — a hash over the contents of the pinned rev's migration
#                   sources, compared against a checked-in expected value.
#                   Ancestry alone is not enough: an upstream commit can change
#                   the SQL inside an already-released migration number without
#                   ever leaving the target branch, and every profile that
#                   already ran the old version would then carry a schema whose
#                   checksum the new code rejects.
#
# The fingerprint file is updated by a HUMAN, deliberately, as part of a repin:
#
#   scripts/migration-fixtures/check-pin-ancestry.sh --update
#
# Reviewing that diff is the point. If the fingerprint moves without the pin
# moving, upstream rewrote a released migration and DET needs a compatibility
# bridge, not a rubber stamp.

set -euo pipefail

# --------------------------------------------------------------------------
# Configuration — edit here when the upstream release line advances.
# --------------------------------------------------------------------------

# Branch the pin must descend from. Override with MIGRATION_PIN_TARGET_BRANCH to
# try a repin against a different line before editing this file.
TARGET_BRANCH="${MIGRATION_PIN_TARGET_BRANCH:-v4.2-dev}"

PLATFORM_REMOTE="${MIGRATION_PIN_REMOTE:-https://github.com/dashpay/platform.git}"

# Directory holding the refinery migration sources inside the platform repo.
MIGRATIONS_DIR="packages/rs-platform-wallet-storage/migrations"

# Cargo.toml dependencies that must all point at the same platform rev. These
# are the crates whose migrations touch on-disk user data.
PINNED_CRATES=(platform-wallet platform-wallet-storage)

# --------------------------------------------------------------------------

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
CARGO_TOML="$REPO_ROOT/Cargo.toml"
EXPECTED_FILE="$SCRIPT_DIR/expected-migration-fingerprint.txt"

# Clone reused across runs: $RUNNER_TEMP on GitHub Actions, /data/tmp locally
# (never /tmp — that is a RAM-backed tmpfs on the dev host).
CACHE_DIR="${MIGRATION_PIN_CACHE_DIR:-${RUNNER_TEMP:-/data/tmp}/platform-pin-cache}"

MODE=check
while [ $# -gt 0 ]; do
    case "$1" in
    --update) MODE=update ;;
    --print) MODE=print ;;
    -h | --help)
        sed -n '3,26p' "${BASH_SOURCE[0]}" | sed 's/^#\{1,2\} \{0,1\}//'
        exit 0
        ;;
    *)
        echo "error: unknown argument '$1' (expected --update, --print, or no argument)" >&2
        exit 2
        ;;
    esac
    shift
done

die() {
    echo "::error::$*" >&2
    exit 1
}

note() { echo "==> $*"; }

# --------------------------------------------------------------------------
# 1. Read the pinned rev out of Cargo.toml.
# --------------------------------------------------------------------------

[ -f "$CARGO_TOML" ] || die "Cargo.toml not found at $CARGO_TOML"

PIN=""
for crate in "${PINNED_CRATES[@]}"; do
    # Anchored on "<crate> =" so platform-wallet does not also match
    # platform-wallet-storage. The rev always sits on the crate's own line.
    rev="$(sed -nE "s/^${crate}[[:space:]]*=[[:space:]]*\{.*rev[[:space:]]*=[[:space:]]*\"([0-9a-fA-F]{7,40})\".*/\1/p" "$CARGO_TOML" | head -n1)"
    [ -n "$rev" ] || die "Could not read a git rev for '$crate' from $CARGO_TOML. Has the dependency been renamed, switched to a path/version dependency, or reformatted across multiple lines?"
    if [ -z "$PIN" ]; then
        PIN="$rev"
    elif [ "$PIN" != "$rev" ]; then
        die "Platform crates are pinned to different revs: expected all of ${PINNED_CRATES[*]} to share one rev, but found '$PIN' and '$rev'. A split pin means two different migration histories can reach the same database file."
    fi
done

note "Pinned platform rev: $PIN"
note "Target branch:       $TARGET_BRANCH"

# --------------------------------------------------------------------------
# 2. Fetch just enough of the platform repo.
# --------------------------------------------------------------------------
#
# --filter=tree:0 fetches the full commit graph and no trees/blobs: a few
# megabytes and a couple of seconds, versus a multi-gigabyte full clone.
# Ancestry needs only commits; the migration blobs are pulled on demand through
# the promisor remote when git cat-file asks for them below.
#
# Deliberately NOT --depth=1: a shallow fetch has no ancestry to check.

mkdir -p "$CACHE_DIR"
if [ ! -d "$CACHE_DIR/.git" ]; then
    git init -q "$CACHE_DIR"
    git -C "$CACHE_DIR" remote add origin "$PLATFORM_REMOTE"
else
    git -C "$CACHE_DIR" remote set-url origin "$PLATFORM_REMOTE"
fi

note "Fetching $PLATFORM_REMOTE ($TARGET_BRANCH + pinned rev) into $CACHE_DIR"
if ! git -C "$CACHE_DIR" fetch -q --filter=tree:0 origin "$TARGET_BRANCH" "$PIN"; then
    die "Failed to fetch branch '$TARGET_BRANCH' and rev '$PIN' from $PLATFORM_REMOTE. Either the branch no longer exists (update TARGET_BRANCH at the top of this script), the pinned rev was force-pushed away upstream, or the runner has no network access."
fi

BRANCH_REF="refs/remotes/origin/$TARGET_BRANCH"
BRANCH_TIP="$(git -C "$CACHE_DIR" rev-parse "$BRANCH_REF")"
note "Branch tip:          $BRANCH_TIP"

# --------------------------------------------------------------------------
# 3. Ancestry.
# --------------------------------------------------------------------------

if ! git -C "$CACHE_DIR" merge-base --is-ancestor "$PIN" "$BRANCH_REF"; then
    die "Pinned platform rev $PIN is NOT an ancestor of $TARGET_BRANCH (tip $BRANCH_TIP).
The pin sits on a side branch, so its wallet-storage migration history can diverge from the released line — exactly the failure that forced the platform_compatibility bridge into DET.
Move the pin to a commit on $TARGET_BRANCH, or, if the release line itself has advanced, update TARGET_BRANCH at the top of this script."
fi
note "Ancestry OK: pin is on $TARGET_BRANCH"

# --------------------------------------------------------------------------
# 4. Fingerprint of the rendered migration sources.
# --------------------------------------------------------------------------

mapfile -t MIGRATION_FILES < <(
    git -C "$CACHE_DIR" ls-tree -r --name-only "$PIN" -- "$MIGRATIONS_DIR" | LC_ALL=C sort
)

if [ "${#MIGRATION_FILES[@]}" -eq 0 ]; then
    die "No migration files found under '$MIGRATIONS_DIR' at rev $PIN. Upstream most likely moved or renamed that directory, and fingerprinting an empty set would pass silently forever. Update MIGRATIONS_DIR at the top of this script."
fi

# Per-file digests rather than one digest over a concatenation: when the check
# fails, the listing names exactly which migration moved.
LISTING="$(
    for f in "${MIGRATION_FILES[@]}"; do
        digest="$(git -C "$CACHE_DIR" cat-file blob "$PIN:$f" | sha256sum | cut -d' ' -f1)"
        printf '%s  %s\n' "$digest" "$f"
    done
)"
FINGERPRINT="$(printf '%s\n' "$LISTING" | sha256sum | cut -d' ' -f1)"

note "Migration files:     ${#MIGRATION_FILES[@]}"
note "Fingerprint:         $FINGERPRINT"

if [ "$MODE" = print ]; then
    printf '%s\n' "$LISTING"
    exit 0
fi

if [ "$MODE" = update ]; then
    cat >"$EXPECTED_FILE" <<EOF
# Fingerprint of the platform-wallet-storage migration sources at the pinned rev.
#
# Regenerate ONLY as a deliberate part of a repin, and read the resulting diff:
#   scripts/migration-fixtures/check-pin-ancestry.sh --update
#
# fingerprint = sha256 of the LC_ALL=C-sorted "<sha256>  <path>" listing of every
# file under $MIGRATIONS_DIR at that rev.
pin=$PIN
target_branch=$TARGET_BRANCH
migration_count=${#MIGRATION_FILES[@]}
fingerprint=$FINGERPRINT
EOF
    note "Wrote $EXPECTED_FILE"
    exit 0
fi

[ -f "$EXPECTED_FILE" ] || die "Expected-fingerprint file is missing: $EXPECTED_FILE. Create the baseline with: $0 --update"

read_key() { sed -nE "s/^$1=(.*)$/\1/p" "$EXPECTED_FILE" | head -n1; }
EXPECTED_PIN="$(read_key pin)"
EXPECTED_FINGERPRINT="$(read_key fingerprint)"

[ -n "$EXPECTED_FINGERPRINT" ] || die "No 'fingerprint=' line in $EXPECTED_FILE. Recreate it with: $0 --update"

if [ "$FINGERPRINT" != "$EXPECTED_FINGERPRINT" ]; then
    printf '%s\n' "$LISTING" >&2
    if [ "$EXPECTED_PIN" = "$PIN" ]; then
        die "Migration fingerprint changed WITHOUT the platform pin changing (still $PIN).
Upstream rewrote the contents of an already-released migration in place. Every DET profile that ran the old version now holds a schema whose checksum the new code rejects, and repinning cannot undo that — it needs a compatibility bridge like src/wallet_backend/platform_compatibility/.
  expected $EXPECTED_FINGERPRINT
  actual   $FINGERPRINT
Per-file digests are listed above."
    fi
    die "Migration fingerprint does not match the recorded baseline.
  pin:         recorded $EXPECTED_PIN -> current $PIN
  fingerprint: recorded $EXPECTED_FINGERPRINT -> current $FINGERPRINT
If this repin is intentional, confirm the migration diff between the two revs is additive (new V0NN files only, no edits to existing ones), then record it:
  $0 --update
Per-file digests are listed above."
fi

note "Fingerprint OK: matches $EXPECTED_FILE"
note "Platform pin guard passed."
