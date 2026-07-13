# Scenario: <short descriptive name>

**Verifies:** one sentence — the specific behavior/assumption this scenario proves or disproves.

**Tier justification:** why this can't be a `kittest` test (e.g. needs live
network timing, needs a real broadcast, spans a flow `kittest` doesn't cover).

## Prerequisites

- Network: <testnet / mainnet — default testnet, justify if not>
- Environment variables (names only — see the project's `.env`/`tests/backend-e2e/README.md`
  for where real values come from):
  - `VAR_NAME` — what it is
- Any other precondition (e.g. "a registered masternode with a nonzero credit balance")

## Setup

```bash
# Isolated data dir — never point at a real user's default location
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
# ... any .env adjustments this scenario needs ...

# Confirm no conflicting instance is already using this display/data dir
pgrep -af dash-evo-tool

DISPLAY=:99 DASH_EVO_DATA_DIR="$DATADIR" nohup /data/target/debug/dash-evo-tool >/tmp/<scenario-slug>.log 2>&1 &
```

## Procedure

Describe steps by what to look for (label/role), not pixel coordinates —
scenarios should survive minor layout changes.

1. ...
2. ...

## Safety constraints specific to this scenario

- (e.g. "cap withdrawal amount at 10% of available balance")
- (e.g. "use the owner-key path so the destination is consensus-forced, never a typed address")

## Expected outcome / pass criteria

What "it worked" looks like — exact banner text, a specific screen state, a
specific log line. Be precise enough that a future run can tell pass from
fail without guessing.

## Known gotchas

Anything non-obvious discovered while running this — timing quirks, an
unexpected intermediate screen, a naming mismatch between the plan and the
real UI. Keep this section updated; it's the most valuable part of the file
after the first real run.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
