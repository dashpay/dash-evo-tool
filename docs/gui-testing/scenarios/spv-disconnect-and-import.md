# Scenario: Manual Disconnect, network switching and import-time identity discovery

**Verifies:** that a manual Disconnect keeps the app offline across network
switches, that Disconnect (or stopping the startup sync) abandons a connection
that is still starting, and that importing a wallet while the app is still
connecting finds the wallet's identities. (Risk area: #1055, #1056, #1057.)

**Tier justification:** Needs real connection startup timing, real network
switches and a real wallet import against live masternode-list sync.

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here). Describe what you observe
on each build without assuming which one is "correct", and note where a
control or screen exists on only one build (a feature-presence difference,
not a step failure).

All steps are read-only on-chain. Equivalent fixtures are only needed so
both builds import a wallet owning the same identities.

## Prerequisites

- Network: testnet, plus a second network selectable in the network chooser
- "Auto-start SPV on startup" enabled (the default) for steps 1-4 (record the
  setting's state on each build)
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — testnet wallet that already owns at least one
    identity (for step 6; enter it through the masked import field only)
- Verify exact labels during execution

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

# Build in the checkout under test and resolve BIN from Cargo's effective
# target directory, as in the README ("How to run a scenario").
BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/spv-disconnect-and-import.log"
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

1. **Disconnect then switch.** With the app connected, open the Network
   screen, press "Disconnect", then choose another network. Record the
   connection indicator over the next minute and whether the app connects on
   its own.
2. **Connect again.** Press "Connect". Record that it connects, and then switch
   back and forth once more while connected.
3. **Disconnect while starting.** Restart the app (auto-start on). Within the
   first seconds, press "Disconnect" on the Network screen, or on the startup
   sync screen choose Cancel and then "Stop syncing". Record the indicator for
   a minute: does it come online anyway?
4. **Connect right after Disconnect.** Press "Disconnect" and immediately
   "Connect". Record the end state.
5. **Background network.** Start a connection on network A, switch to B
   without disconnecting, then press "Disconnect". Record whether A's
   connection continues (peer counts/log lines).
6. **Import while connecting.** Use a fresh data directory. As soon as the
   app starts connecting, import `E2E_WALLET_MNEMONIC` through the import screen,
   choosing a small number of identities to check. Record the Identities list
   immediately and for several minutes, without pressing any manual load
   control. Then record whether a password prompt appears. Note that the first
   balance shown right after an import can be partial until sync completes.
7. **Import when already synced.** Repeat on a second fresh data directory
   after sync completes; record the time until identities appear.

## Safety constraints specific to this scenario

- Enter the mnemonic only through the app's masked import field; never on a
  command line and never in a screenshot while populated.

## Expected outcome / pass criteria

Record the connection indicator and the identity list per build per step.

- An app that goes online after the user pressed Disconnect, or an empty
  identity list that never fills for a wallet owning identities, on the build
  where the other build behaves correctly, is a difference to classify under
  the contract.

Apply the [A/B build comparison contract](../README.md#ab-build-comparison-contract):
its blocker rule decides what blocks, and its equivalent-fixture rule applies
to every step marked **MUTATES** below.

## Known gotchas

- Whether wallet identities appear on their own after an import, and when,
  is what this scenario records; do not assume either outcome beforehand.
- Steps 3-5 are timing-sensitive; repeat several times per build before
  concluding. Intermittency only changes how many attempts a repro needs.
- Disconnect is session-only; restarting the app connects again when the
  setting is on, on both builds.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
