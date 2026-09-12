#!/usr/bin/env bash
#
# Pack a DASH_EVO_DATA_DIR-style data directory into a migration fixture archive.
#
#   scripts/migration-fixtures/pack.sh <data-dir> <out-prefix>
#
# <out-prefix> is a path WITHOUT extension; the script appends .tar.zst when
# zstd is available and .tar.gz otherwise, and writes a matching .sha256 next to
# it. The final archive path is echoed on stdout as the last line, and exported
# as archive/sha256/format to $GITHUB_OUTPUT when running under Actions.
#
# Excluded from the archive:
#   spv/      chain data — hundreds of MB, re-synced from the network anyway
#   backups/  DET's own pre-migration copies; a fixture must capture the state
#             the app is asked to migrate, not a previous migration's leftovers
#   *.log     runtime noise, and the most likely place for stray secrets
#
# Archives are built with reproducible tar flags so that re-packing an unchanged
# directory produces a byte-identical file and the .sha256 stays meaningful.

set -euo pipefail

die() {
    echo "::error::$*" >&2
    exit 1
}

note() { echo "==> $*"; }

[ $# -eq 2 ] || die "usage: $0 <data-dir> <out-prefix>
  <data-dir>    a DASH_EVO_DATA_DIR to pack (must contain .env)
  <out-prefix>  output path without extension, e.g. out/fixture-v0.9.3-testnet"

DATA_DIR="$1"
OUT_PREFIX="$2"

[ -d "$DATA_DIR" ] || die "Data directory does not exist: $DATA_DIR"
DATA_DIR="$(cd -- "$DATA_DIR" && pwd)"

# A data dir with no .env is almost always a typo'd path or a capture that never
# got as far as initialising the app. Packing it would produce a fixture that
# fails at boot for reasons unrelated to migration.
[ -f "$DATA_DIR/.env" ] || die "No .env in $DATA_DIR — this does not look like an initialised DET data directory. Run the capture script first."

# --------------------------------------------------------------------------
# Secret check.
# --------------------------------------------------------------------------
#
# Fixtures are published as build artifacts that anyone with read access to the
# repo can download. MCP_API_KEY authorises control of a running DET instance,
# so shipping a live one inside a fixture is a credential leak. The fixture
# wallet's own seed is a different matter: it is a deliberately public
# testnet-only wallet, and the vault is the thing under test.
api_key="$(sed -nE 's/^[[:space:]]*MCP_API_KEY[[:space:]]*=[[:space:]]*(.*)$/\1/p' "$DATA_DIR/.env" | tail -n1 | tr -d '"'"'"' \t\r')"
if [ -n "$api_key" ]; then
    die "Refusing to pack: $DATA_DIR/.env sets a non-empty MCP_API_KEY.
Fixture archives are published as build artifacts and anyone who can read the repo can download them, so a live key inside one is a credential leak.
Blank the key (MCP_API_KEY=) and re-run. Standalone det-cli, which is what the capture script uses, needs it empty anyway."
fi

# --------------------------------------------------------------------------
# Format selection.
# --------------------------------------------------------------------------

if command -v zstd >/dev/null 2>&1; then
    FORMAT="tar.zst"
    COMPRESS=(--zstd)
else
    note "zstd not found — falling back to gzip"
    FORMAT="tar.gz"
    COMPRESS=(--gzip)
fi

ARCHIVE="${OUT_PREFIX}.${FORMAT}"
OUT_DIR="$(dirname -- "$ARCHIVE")"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd -- "$OUT_DIR" && pwd)"
ARCHIVE="$OUT_DIR/$(basename -- "$ARCHIVE")"

# --------------------------------------------------------------------------
# Pack.
# --------------------------------------------------------------------------
#
# --sort=name, a fixed --mtime and zeroed ownership make the archive a pure
# function of the directory contents. Without them the same data dir packs to a
# different checksum on every run and the .sha256 only proves "nothing was
# corrupted in transit", not "this is the fixture the manifest names".
#
# -C "$DATA_DIR" . stores paths relative to the data dir, so unpacking with
# `tar -xf archive -C "$target"` yields a usable DASH_EVO_DATA_DIR directly.

note "Packing $DATA_DIR -> $ARCHIVE"
tar \
    --create \
    "${COMPRESS[@]}" \
    --file "$ARCHIVE" \
    --directory "$DATA_DIR" \
    --exclude="./spv" \
    --exclude="./backups" \
    --exclude="*.log" \
    --sort=name \
    --mtime="@0" \
    --owner=0 --group=0 --numeric-owner \
    .

SHA="$(sha256sum "$ARCHIVE" | cut -d' ' -f1)"
# Basename-relative so `cd "$(dirname archive)" && sha256sum -c archive.sha256`
# verifies wherever the pair is later downloaded to.
printf '%s  %s\n' "$SHA" "$(basename -- "$ARCHIVE")" >"$ARCHIVE.sha256"

note "Size:   $(du -h "$ARCHIVE" | cut -f1)"
note "sha256: $SHA"

if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
        echo "archive=$ARCHIVE"
        echo "sha256=$SHA"
        echo "format=$FORMAT"
    } >>"$GITHUB_OUTPUT"
fi

echo "$ARCHIVE"
