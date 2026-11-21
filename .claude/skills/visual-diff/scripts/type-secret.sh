#!/usr/bin/env bash
# Types one named test credential into the focused field of the GUI on
# $DISPLAY without ever printing it.
# usage: type-secret.sh [--env-file FILE] VAR_NAME [WORD_INDEX]
# FILE defaults to .env at the repository root. WORD_INDEX (1..24) types only
# that word of E2E_WALLET_MNEMONIC, for forms with one field per word.
# The value goes from the env file to xdotool's stdin, one character at a
# time: never echoed, never an argument, never written to a file.
# An inherited xtrace (SHELLOPTS, BASH_ENV) would print the value.
{ set +x; } 2>/dev/null
set -euo pipefail
ENV_FILE=""
if [ "${1:-}" = "--env-file" ]; then
  ENV_FILE="${2:?--env-file needs a path}"
  shift 2
fi
[ -n "$ENV_FILE" ] || ENV_FILE="$(git rev-parse --show-toplevel)/.env"
: "${DISPLAY:?set DISPLAY to the GUI-testing display}"
case "${1:-}" in
  E2E_WALLET_MNEMONIC | E2E_MN_PROTX_HASH | E2E_MN_VOTING_KEY | E2E_MN_OWNER_KEY | E2E_IDENTITY_ID) ;;
  *)
    echo "usage: type-secret.sh [--env-file FILE] <E2E_WALLET_MNEMONIC|E2E_MN_PROTX_HASH|E2E_MN_VOTING_KEY|E2E_MN_OWNER_KEY|E2E_IDENTITY_ID> [WORD_INDEX]" >&2
    exit 2
    ;;
esac
# Reads the variable without printing it; nothing on any form it cannot read.
# The file is redirected, not named: awk would read a name like --source=...
# as an option, or name=value as an assignment.
env_value() { awk -v name="$1" -f "$(dirname "$0")/env-value.awk" <"$ENV_FILE"; }
if ! env_value "$1" >/dev/null; then
  echo "type-secret: $1 is not set, or not as NAME=value, NAME=\"value\" or NAME='value'" >&2
  exit 1
fi
word="${2:-}"
if [ -n "$word" ]; then
  if [ "$1" != E2E_WALLET_MNEMONIC ] || ! [[ "$word" =~ ^([1-9]|1[0-9]|2[0-4])$ ]]; then
    echo "type-secret: a word index (1..24) is valid only with E2E_WALLET_MNEMONIC" >&2
    exit 2
  fi
fi
# Only into the GUI under test, never a terminal or another app: the focused
# window must belong to dash-evo-tool and keep the focus for every character,
# so a focus change can catch at most the one character in flight.
wid="$(xdotool getactivewindow 2>/dev/null)" || wid=""
pid=""
[ -z "$wid" ] || pid="$(xdotool getwindowpid "$wid" 2>/dev/null)" || pid=""
if [ -z "$pid" ] || [ "$(cat "/proc/$pid/comm" 2>/dev/null)" != dash-evo-tool ]; then
  echo "type-secret: the focused window is not Dash Evo Tool; focus its field and retry" >&2
  exit 1
fi
value="$(env_value "$1" | awk -v i="$word" '{ if (i == "") printf "%s", $0; else printf "%s", $i }')"
for ((n = 0; n < ${#value}; n++)); do
  if [ "$(xdotool getactivewindow 2>/dev/null)" != "$wid" ]; then
    echo "type-secret: the focus left Dash Evo Tool after $n characters; clear the field and retry" >&2
    exit 1
  fi
  printf '%s' "${value:n:1}" | xdotool type --clearmodifiers --file - >/dev/null 2>&1
done
if [ -n "$word" ]; then
  echo "type-secret: typed word $word of $1 into the focused field"
else
  echo "type-secret: typed $1 into the focused field"
fi
