#!/bin/bash
set -euo pipefail
# Runs cargo in a sanitized environment without CI secrets.
# Uses env -i (allowlist) instead of env -u (denylist) so that
# any new secrets added in the future are stripped automatically.

# Build the environment allowlist. Only pass variables that are set
# to avoid empty values confusing tools (e.g. PROTOC="" breaks prost).
ENV_ARGS=(
    HOME="$HOME"
    PATH="$PATH"
    CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
    RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
    TMPDIR="${TMPDIR:-/tmp}"
    LANG="${LANG:-C.UTF-8}"
    TERM="${TERM:-dumb}"
)

# Conditionally pass optional variables only if they are set and non-empty.
for var in PROTOC CC CXX PKG_CONFIG_PATH USER SHELL; do
    if [ -n "${!var:-}" ]; then
        ENV_ARGS+=("$var=${!var}")
    fi
done

exec env -i "${ENV_ARGS[@]}" cargo "$@"
