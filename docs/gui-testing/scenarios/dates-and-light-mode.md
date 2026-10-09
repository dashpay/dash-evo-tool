# Scenario: Dates in local time and light-mode readability

**Verifies:** how dates and times are presented across screens (zone, format)
and whether all text is readable in light and dark theme. (Risk area: #1052,
#1053.)

**Tier justification:** Needs real rendering in both themes and a non-UTC host
time zone; pixel colours cannot be judged in the in-process harness.

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here). Describe what you observe
on each build without assuming which one is "correct", and note where a
control or screen exists on only one build (a feature-presence difference,
not a step failure).

All steps are read-only.

## Prerequisites

- Network: testnet, with data that has dates: wallet transactions, an
  identity with token claims or withdrawals, a proof-verification result in the
  GroveSTARK tool, and (optional) a contested name or request
- Host time zone set to a non-UTC zone for the launch (for example
  `TZ=Pacific/Auckland`); record it
- Verify exact labels during execution

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/dates-and-light-mode.log"
: "${DISPLAY:?Set DISPLAY to the desktop used for GUI testing}"
xdpyinfo >/dev/null
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

Read the network indicator after launch. A fresh data directory starts on
Mainnet, so if it is not **Testnet**, select Testnet in Settings ▸ Networks
and confirm the indicator before continuing.

Resize the window (see the README's "Known UI/environment quirks") before
judging layout. Check `det-stderr.log` / `det.log` in `$DATADIR` for panics
after each run.

## Procedure

1. **Dates.** Open each screen that shows dates: wallet transactions, token
   claims, withdrawals, the GroveSTARK proof result, username/request views,
   contest deadlines. For each record the exact date text, the zone
   indication and the order of fields.
2. **Typed time.** In the vote confirmation's specific-time option (see the
   votes-hub scenario) record the zone wording and offset shown next to the
   time field.
3. **Light theme.** Switch to light theme in Settings. Visit the Identities
   Usernames list, a username payment review, the Masternodes vote view and a
   node detail. Record any heading, name column, summary or button label that is
   not visible, and spinners that are invisible.
4. **Dark theme.** Repeat step 3 in dark theme.
5. **Control glyphs.** In light theme, hover/press checkboxes, radio buttons
   and focused controls; record whether tick or dot marks are visible.

## Safety constraints specific to this scenario

- Read-only.

## Expected outcome / pass criteria

Record screenshots from each build for steps 1, 3 and 5 side by side.

- Invisible or low-contrast text, or a control mark that became invisible,
  on one build only, is a difference to classify under the contract; so is a
  date shown in a different zone on the two builds for the same instant.

Apply the [A/B build comparison contract](../README.md#ab-build-comparison-contract):
its blocker rule decides what blocks, and its equivalent-fixture rule applies
to every step marked **MUTATES** below.

## Known gotchas

- Mixed UTC and local dates on the older build are expected to be recorded as
  observations, not judged in advance.
- Daylight-saving transitions make a time ambiguous or nonexistent; avoid
  those dates when comparing.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
