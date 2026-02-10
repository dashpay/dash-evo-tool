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
for pkg in build-essential pkg-config clang cmake libsqlite3-dev libzmq3-dev; do
  if ! dpkg -s "$pkg" &>/dev/null; then
    PACKAGES_TO_INSTALL="$PACKAGES_TO_INSTALL $pkg"
  fi
done

if [ -n "$PACKAGES_TO_INSTALL" ]; then
  $SUDO apt-get update -qq || true
  $SUDO apt-get install -y -qq $PACKAGES_TO_INSTALL
fi
