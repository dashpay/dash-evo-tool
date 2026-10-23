#!/usr/bin/env bash
# Types one named test credential into the focused field of the GUI on
# $DISPLAY without ever printing it.
# usage: type-secret.sh [--env-file FILE] VAR_NAME [WORD_INDEX]
# FILE defaults to .env at the repository root. WORD_INDEX (1..24) types only
# that word of E2E_WALLET_MNEMONIC, for forms with one field per word.
# The value goes from the env file straight to xdotool's stdin: never echoed,
# never an argument, never written to a file.
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
if ! grep -q "^$1=..*" "$ENV_FILE"; then
  echo "type-secret: $1 is not set" >&2
  exit 1
fi
word="${2:-}"
if [ -n "$word" ]; then
  if [ "$1" != E2E_WALLET_MNEMONIC ] || ! [[ "$word" =~ ^([1-9]|1[0-9]|2[0-4])$ ]]; then
    echo "type-secret: a word index (1..24) is valid only with E2E_WALLET_MNEMONIC" >&2
    exit 2
  fi
fi
# Strip the name, optional surrounding quotes and the trailing newline.
grep -m1 "^$1=" "$ENV_FILE" | cut -d= -f2- | sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\(.*\)'\$/\1/" | tr -d '\n' |
  awk -v i="$word" '{ if (i == "") printf "%s", $0; else printf "%s", $i }' |
  xdotool type --clearmodifiers --delay 15 --file - >/dev/null 2>&1
if [ -n "$word" ]; then
  echo "type-secret: typed word $word of $1 into the focused field"
else
  echo "type-secret: typed $1 into the focused field"
fi
