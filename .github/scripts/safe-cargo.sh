#!/bin/bash
set -euo pipefail
# Runs cargo in a sanitized environment without CI secrets.
# Uses env -i (allowlist) instead of env -u (denylist) so that
# any new secrets added in the future are stripped automatically.
exec env -i \
    HOME="$HOME" \
    PATH="$PATH" \
    USER="${USER:-}" \
    SHELL="${SHELL:-/bin/bash}" \
    TMPDIR="${TMPDIR:-/tmp}" \
    LANG="${LANG:-C.UTF-8}" \
    TERM="${TERM:-dumb}" \
    CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
    RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
    PROTOC="${PROTOC:-}" \
    CC="${CC:-}" \
    CXX="${CXX:-}" \
    PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-}" \
    cargo "$@"
