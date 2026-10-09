# Scenario: Add funds to an identity — progress dialog, background continuation, in-flight tracking

**Verifies:** what the user sees while an identity top-up runs (dialog, form
state, warnings), what happens when the user leaves the screen or switches
network while it runs, and where the result is reported. (Risk area: #1054,
#1058.)

**Tier justification:** Needs a real top-up broadcast with real settlement
timing (tens of seconds), a real network switch, and a concurrent Send — none
reproducible without a display and a live network.

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here). Describe what you observe
on each build without assuming which one is "correct", and note where a
control or screen exists on only one build (a feature-presence difference,
not a step failure).

Every step marked **MUTATES** spends wallet funds (crediting an identity) or
identity credits (registering a username); each build needs its own independently-funded equivalent wallet and identity — see
the contract below.

## Prerequisites

- Network: testnet
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — funded testnet wallet (separate wallet per build)
  - `E2E_IDENTITY_ID` — identity to top up (separate identity per build)
- A second identity on the same wallet (for step 4); if none exists, record
  step 4's different-identity part as not exercised
- A second, previously unvisited network available for the switch (for
  example Devnet or Regtest configured in `.env`), or a note that it is not
- Verify exact labels during execution

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/identity-add-funds-progress.log"
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

1. **Open the screen.** From the identity's Home, open the "Add funds"
   action and choose the "From your wallet" method. Record the controls on the
   screen.
2. **Press the add-funds button (MUTATES).** Enter a small amount (at most
   about 10% of the balance) and press the button. Record immediately, and
   every few seconds: whether a dialog covers the app and its text; whether
   the form remains visible, editable or replaced by a notice; any "not enough
   Dash" warning; whether the button can be pressed a second time.
3. **Long transfer.** If the transfer lasts more than about 30 seconds, record
   any control that appears on the dialog (its label) and use it. Record the
   notice shown afterwards, and whether it stays while navigating to other
   screens.
4. **Reopen during transfer.** While the transfer is still pending, reopen the
   Add funds screen for the same identity and for a different identity. Record
   what each shows.
5. **Network switch during transfer (MUTATES).** Start a second small top-up,
   choose the background option, then switch to a network not visited before
   and back. Record whether a notice about funds being added remains, and what
   Add funds for that identity shows.
6. **Concurrent Send (MUTATES).** Start a small wallet Send, and while it
   shows its in-progress state let a top-up finish. Record what the Send screen
   shows when the top-up result arrives, and where the top-up result is
   reported.
7. **Completion text (MUTATES when repeating).** Record the exact completion message and whether it names
   the identity (by name and/or ID). Repeat with two top-ups of the same
   identity name if possible (each repetition is another top-up, within the
   same cap) and record whether the second confirmation is
   visible as a new message.
8. **Registration dialog (MUTATES: spends credits, registers a name).** Start
   a username registration and record whether its progress dialog names the
   username. Follow the registration safety and equivalent-fixture rules in
   [dpns-registration-flow.md](dpns-registration-flow.md), including a
   different available name for each build.

## Safety constraints specific to this scenario

- Cap each top-up, including every repetition in step 7, at about 10% of the
  funded balance.
- Step 8 spends identity credits and registers a name; testnet only, smallest
  fee name, and never the same name on both builds.
- Do not use mainnet.

## Expected outcome / pass criteria

Record the observed behavior for each build at every step, with screenshots
of the dialog, the pending notice and the Send screen at step 6.

- A Send screen that reports a top-up's outcome as its own payment's, or a
  form that accepts a second submit of the same pending top-up, is a
  difference to classify under the contract (it can mean double spend).

Apply the [A/B build comparison contract](../README.md#ab-build-comparison-contract):
its blocker rule decides what blocks, and its equivalent-fixture rule applies
to every step marked **MUTATES** below.

## Known gotchas

- Settlement time varies; if the top-up finishes before 30 seconds the
  background control may never appear. Record that and use a congested
  period or the network-switch step to exercise the background path.
- A top-up is a single backend task with no stage reporting; expect one
  sentence of dialog text, not per-stage text.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
