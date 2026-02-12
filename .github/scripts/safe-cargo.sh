#!/bin/bash
# Runs cargo in a sanitized environment without CI secrets.
# This prevents build.rs scripts and proc macros from accessing
# credentials via environment variables.
exec env \
    -u GITHUB_TOKEN \
    -u CLAUDE_CODE_OAUTH_TOKEN \
    -u CLAUDE_CODE_OAUTH_TOKEN_LKLIMEK \
    -u INPUT_CLAUDE_CODE_OAUTH_TOKEN \
    -u INPUT_ANTHROPIC_API_KEY \
    -u ANTHROPIC_API_KEY \
    -u ACTIONS_ID_TOKEN_REQUEST_TOKEN \
    -u ACTIONS_ID_TOKEN_REQUEST_URL \
    -u ACTIONS_RUNTIME_TOKEN \
    cargo "$@"
