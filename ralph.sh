#!/usr/bin/env bash
set -euo pipefail

# Ralph Loop — repeatedly invokes Claude Code to work through tasks.md
# Stops when the agent signals all tasks are complete.

TIMEOUT=2400      # 40 minutes per run (META/complex tasks need more time)
MAX_RUNS=250      # Safety cap — migration is ~120-180 tasks total
LOG_DIR="ralph_logs"

mkdir -p "$LOG_DIR"

# Ensure we're on the right branch
BRANCH="react-native"
CURRENT=$(git branch --show-current)
if [ "$CURRENT" != "$BRANCH" ]; then
  echo "ERROR: Expected to be on branch $BRANCH but on $CURRENT" >&2
  exit 1
fi

RUN=1
STALE_COUNT=0
MAX_STALE=3        # Stop after this many consecutive runs with no tasks.md changes
PREV_TASKS_HASH=""

while [ "$RUN" -le "$MAX_RUNS" ]; do
  TIMESTAMP=$(date '+%Y-%m-%d_%H-%M-%S')
  LOG_FILE="$LOG_DIR/run_${RUN}_${TIMESTAMP}.log"

  echo "=== Ralph Run #$RUN (max $MAX_RUNS) — $(date) ==="

  # Re-read prompt each run so edits take effect without restarting
  PROMPT=$(cat prompt.md)

  START=$(date +%s)
  EXIT_CODE=0
  timeout --foreground "$TIMEOUT" claude -p --dangerously-skip-permissions "$PROMPT" > "$LOG_FILE" 2>&1 || EXIT_CODE=$?

  DURATION=$(( $(date +%s) - START ))
  echo "  Finished in ${DURATION}s (exit code: $EXIT_CODE)"

  # Exit cleanly on Ctrl+C / kill (128+signal: 130=SIGINT, 143=SIGTERM)
  if [ "$EXIT_CODE" -eq 130 ] || [ "$EXIT_CODE" -eq 143 ]; then
    echo "Interrupted. Stopping ralph loop."
    exit "$EXIT_CODE"
  fi

  if [ "$EXIT_CODE" -eq 124 ]; then
    echo "  WARNING: Run #$RUN timed out after ${TIMEOUT}s" >&2
    # Continue to next run — the agent can pick up where it left off
  elif [ "$EXIT_CODE" -ne 0 ]; then
    echo "  WARNING: Claude exited with code $EXIT_CODE (see $LOG_FILE)" >&2
  fi

  # Detect stale runs — if tasks.md hasn't changed, increment counter
  CURR_TASKS_HASH=$(md5 -q tasks.md 2>/dev/null || md5sum tasks.md 2>/dev/null | cut -d' ' -f1)
  if [ -n "$PREV_TASKS_HASH" ] && [ "$CURR_TASKS_HASH" = "$PREV_TASKS_HASH" ]; then
    STALE_COUNT=$((STALE_COUNT + 1))
    echo "  No changes to tasks.md ($STALE_COUNT/$MAX_STALE stale runs)"
    if [ "$STALE_COUNT" -ge "$MAX_STALE" ]; then
      echo "ERROR: tasks.md unchanged for $MAX_STALE consecutive runs. Stopping." >&2
      exit 1
    fi
  else
    STALE_COUNT=0
  fi
  PREV_TASKS_HASH="$CURR_TASKS_HASH"

  RUN=$((RUN + 1))
done

if [ "$RUN" -gt "$MAX_RUNS" ]; then
  echo "ERROR: Reached max runs ($MAX_RUNS) without completion" >&2
  exit 1
fi
