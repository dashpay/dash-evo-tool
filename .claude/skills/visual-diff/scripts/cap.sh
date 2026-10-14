#!/usr/bin/env bash
# usage: cap.sh DIR NAME   -> DIR/NAME.png, a full capture of $DISPLAY.
# Do not call while a secret field is filled; use an area capture below the
# fields instead (see SKILL.md, secret protocol).
set -euo pipefail
: "${DISPLAY:?set DISPLAY to the GUI-testing display}"
[ $# -eq 2 ] || { echo "usage: cap.sh DIR NAME" >&2; exit 2; }
mkdir -p "$1"
scrot -o "$1/$2.png" && echo "saved $1/$2.png"
