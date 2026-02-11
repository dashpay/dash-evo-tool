#!/bin/bash
set -euo pipefail

# Only run in Claude Code remote environments
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

# Use sudo only if not already root
SUDO=""
if [ "$(id -u)" -ne 0 ]; then
  SUDO="sudo"
fi

# Install protoc (Protocol Buffers compiler) if not present
# Version aligned with CI workflows (.github/workflows/clippy.yml, tests.yml)
if ! command -v protoc &>/dev/null; then
  PROTOC_VERSION="25.2"
  PROTOC_ZIP="protoc-${PROTOC_VERSION}-linux-x86_64.zip"
  curl -fsSL "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/${PROTOC_ZIP}" -o "/tmp/${PROTOC_ZIP}"
  $SUDO unzip -o "/tmp/${PROTOC_ZIP}" -d /usr/local
  rm "/tmp/${PROTOC_ZIP}"
fi

# Install Rust components (clippy, rustfmt) if not present
rustup component add clippy rustfmt 2>/dev/null || true

# Install system dependencies if not already present
# Matches CI: build-essential, pkg-config, clang, cmake, libsqlite3-dev, libzmq3-dev
PACKAGES_TO_INSTALL=""
for pkg in build-essential pkg-config clang cmake libsqlite3-dev libzmq3-dev unzip; do
  if ! dpkg -s "$pkg" &>/dev/null; then
    PACKAGES_TO_INSTALL="$PACKAGES_TO_INSTALL $pkg"
  fi
done

if [ -n "$PACKAGES_TO_INSTALL" ]; then
  $SUDO apt-get update -qq || true
  $SUDO apt-get install -y -qq $PACKAGES_TO_INSTALL
fi

# Pre-download Tenderdash proto sources to avoid network fetch during cargo build.
#
# The tenderdash-proto build.rs (from rs-tenderdash-abci) downloads proto sources
# from GitHub during compilation. In sandboxed environments, ureq/rustls can't
# verify TLS certificates, causing the download to fail.
#
# Workaround: Pre-download and extract the sources, then configure Cargo env vars
# so the build script finds them without needing network access.
#
# The build script checks:
#   1. TENDERDASH_DIR - path to pre-extracted tenderdash sources
#   2. CARGO_TARGET_DIR - used as the zip archive cache directory
#
# Even with TENDERDASH_DIR set, a clean build has no state file, so the build
# script re-downloads. By also placing the zip in CARGO_TARGET_DIR (the cache dir),
# the build script finds the cached archive and skips the network download.
#
# Version must match DEFAULT_VERSION in rs-tenderdash-abci/proto/build.rs
TENDERDASH_VERSION="v1.5.3"
TENDERDASH_DIR="/tmp/tenderdash-${TENDERDASH_VERSION}"
TENDERDASH_CACHE_DIR="/tmp/tenderdash-cache"
TENDERDASH_ZIP="${TENDERDASH_CACHE_DIR}/tenderdash-${TENDERDASH_VERSION}.zip"

if [ ! -d "${TENDERDASH_DIR}/proto" ] || [ ! -f "${TENDERDASH_ZIP}" ]; then
  echo "[session-start] Downloading Tenderdash ${TENDERDASH_VERSION} proto sources..."
  mkdir -p "${TENDERDASH_CACHE_DIR}"
  DOWNLOAD_ZIP="/tmp/tenderdash-download-${TENDERDASH_VERSION}.zip"
  curl -fsSL "https://github.com/dashpay/tenderdash/archive/${TENDERDASH_VERSION}.zip" -o "${DOWNLOAD_ZIP}"

  # Keep a copy in the cache dir for the build script to find
  cp "${DOWNLOAD_ZIP}" "${TENDERDASH_ZIP}"

  # Extract to a known location for TENDERDASH_DIR
  if [ ! -d "${TENDERDASH_DIR}/proto" ]; then
    TMPDIR=$(mktemp -d)
    unzip -q "${DOWNLOAD_ZIP}" -d "${TMPDIR}"
    # The archive extracts to a subdirectory like tenderdash-v1.5.3/
    EXTRACTED_DIR=$(find "${TMPDIR}" -maxdepth 1 -type d -name "tenderdash-*" | head -1)
    if [ -z "${EXTRACTED_DIR}" ]; then
      echo "[session-start] ERROR: Could not find extracted tenderdash directory"
      rm -rf "${TMPDIR}" "${DOWNLOAD_ZIP}"
      exit 1
    fi
    rm -rf "${TENDERDASH_DIR}"
    mv "${EXTRACTED_DIR}" "${TENDERDASH_DIR}"
    rm -rf "${TMPDIR}"
  fi
  rm -f "${DOWNLOAD_ZIP}"
  echo "[session-start] Tenderdash proto sources installed to ${TENDERDASH_DIR}"
fi

# Set env vars in global Cargo config so build scripts pick them up.
# - TENDERDASH_DIR: tells the proto-compiler where pre-extracted sources are
# - CARGO_TARGET_DIR: tells the proto-compiler where to find the cached zip archive
#   (Note: Cargo's [env] table does NOT affect Cargo's own target dir behavior,
#    per Cargo docs. It only sets env vars for build scripts and rustc.)
# This avoids modifying the project's .cargo/config.toml.
CARGO_CONFIG_DIR="${HOME}/.cargo"
CARGO_CONFIG_FILE="${CARGO_CONFIG_DIR}/config.toml"
mkdir -p "${CARGO_CONFIG_DIR}"

if ! grep -q "TENDERDASH_DIR" "${CARGO_CONFIG_FILE}" 2>/dev/null; then
  # If the file already has an [env] section, append keys under it.
  # Otherwise, add a new [env] section. Avoids duplicate TOML table headers.
  if grep -q '^\[env\]' "${CARGO_CONFIG_FILE}" 2>/dev/null; then
    # Insert the keys right after the existing [env] line
    sed -i '/^\[env\]/a TENDERDASH_DIR = "'"${TENDERDASH_DIR}"'"\nCARGO_TARGET_DIR = "'"${TENDERDASH_CACHE_DIR}"'"' "${CARGO_CONFIG_FILE}"
  else
    cat >> "${CARGO_CONFIG_FILE}" <<EOF

[env]
TENDERDASH_DIR = "${TENDERDASH_DIR}"
CARGO_TARGET_DIR = "${TENDERDASH_CACHE_DIR}"
EOF
  fi
  echo "[session-start] Set TENDERDASH_DIR and CARGO_TARGET_DIR in ${CARGO_CONFIG_FILE}"
fi
