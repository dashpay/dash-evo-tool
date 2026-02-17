#!/bin/bash
set -euo pipefail
#
# safe-cargo.sh — Run cargo without CI secrets leaking to build scripts.
#
# WHY THIS FILE EXISTS
# --------------------
# Cargo build scripts (build.rs / proc-macros) execute arbitrary code during
# compilation. In CI the runner environment contains secrets such as
# CLAUDE_CODE_OAUTH_TOKEN and GITHUB_TOKEN. A compromised or malicious
# dependency could read those variables and exfiltrate them.
#
# This wrapper uses `env -i` (an allowlist approach) so that cargo and every
# child process it spawns start with only the variables listed below.
# Any new secret added to CI in the future is automatically excluded without
# having to update a denylist.
#
# USAGE (GitHub Actions)
#   scripts/safe-cargo.sh build --all-features
#   scripts/safe-cargo.sh test  --all-features --workspace
#   scripts/safe-cargo.sh clippy --all-features --all-targets -- -D warnings
#   scripts/safe-cargo.sh +nightly fmt --all
#

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
