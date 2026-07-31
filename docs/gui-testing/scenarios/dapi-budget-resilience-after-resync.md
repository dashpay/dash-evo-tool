# Scenario: Connection budget survives repeated resync cycles

**Verifies:** Cycling the SPV connection state between `Syncing` and `Synced`
repeatedly does not exhaust the app's shared DAPI request budget, and does not
cause an unrelated Platform action (identity top-up) to fail with a generic
"servers are temporarily unreachable" banner. (Risk area: #950, building on
#936/#938.)

**Tier justification:** The failure this scenario targets is a real,
live-network request-budget exhaustion that only manifests after several
actual `Syncing`→`Synced` transitions against a real DAPI connection — it
cannot be simulated in `kittest` (no network) or isolated to a single
`backend_task` call (the bug is about a background task's cumulative effect
across transitions, not any single call's return value).

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here).

## Prerequisites

- Network: testnet
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — funded testnet wallet
  - `E2E_IDENTITY_ID` (or an identity created ad hoc) — an identity to top up
- Verify exact banner/error wording during execution rather than assuming the
  text below is literal

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/dapi-budget-resilience.log"
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

## Procedure

1. **Baseline Platform Info check.** Open Platform Info (Developer/Advanced
   view). Record the exact "Current Epoch Information" text, in particular
   whether the fee-multiplier line reads "unavailable" or states it is "a
   fixed value, not read from the network" (or neither, if built against a
   version predating both #936 and #950).
2. **Force repeated `Syncing`↔`Synced` transitions.** With the app running
   and initial sync complete, force several resync cycles in the same
   session — e.g. toggle network connectivity briefly (airplane-mode-style
   interruption, or a short `iptables`/firewall block on the DAPI/Core
   ports if available) 4–6 times over a few minutes, or use "Clear cached
   SPV data to force a resync" (NET-020) repeatedly. Each cycle should visibly
   pass back through a `Syncing` state before returning to `Synced`.
3. **Attempt identity top-up immediately after the resync cycles.** Top up
   an identity's credits (any small amount) right after step 2's last
   transition back to `Synced`. Record whether it succeeds, and if it fails,
   capture the exact banner text.
4. **Repeat the top-up attempt once more** a short time later (a minute or
   two) without additional resync cycling, to distinguish "budget recovers
   on its own" from "budget stays exhausted."
5. **Check the logs.** Search `det.log`/`det-stderr.log` for
   `DapiAllAddressesExhausted` or similar rate-limit/address-pool exhaustion
   markers appearing near the resync cycles from step 2, independent of
   whether the UI showed an error.
6. **Re-check Platform Info.** Reopen Platform Info after the resync cycling
   and top-up attempts; record whether the epoch/fee-multiplier text is
   still consistent with step 1's observation (no crash, no stuck "loading"
   state).

## Safety constraints specific to this scenario

- Step 3's top-up moves real (small) testnet funds — keep amounts minimal,
  per the standing fund-movement cap.
- If simulating a network interruption via firewall rules, restore
  connectivity immediately after each toggle and confirm via `pgrep`/process
  liveness that the app itself never needed restarting.

## Expected outcome / pass criteria

Record the observed behavior for each build at every step, then apply this
rule:

- **BLOCKING** only if HEAD is worse than baseline in the happy flow — e.g.
  the identity top-up in step 3 succeeds on baseline after repeated resync
  cycling but fails with a generic "servers are temporarily unreachable"
  banner on HEAD, or `DapiAllAddressesExhausted` (or equivalent) appears in
  HEAD's logs where it did not on baseline — or a **data-loss** outcome on
  either build.
- **NOT blocking**: the reverse direction is the *expected improvement* this
  PR ships — if baseline's top-up fails after repeated resync cycling (the
  bug #950 fixes) and HEAD's succeeds, that's the intended fix working, not
  a baseline "failure" to flag beyond noting it factually. A wording-only
  difference in the Platform Info fee-multiplier line (`unavailable` vs `a
  fixed value, not read from the network`) is expected and not itself a
  defect on either build.
- If neither build reproduces an exhaustion-triggered top-up failure within
  the attempted resync-cycle budget, record that plainly as "not reproduced
  this run" rather than asserting the fix is confirmed — a live-network
  race like this can be timing-sensitive.

## Known gotchas

- Simulating repeated `Syncing`→`Synced` transitions without real network
  interruption tooling may require several minutes of patience — SPV
  reconnect timing is not instantaneous. Budget real wall-clock time for
  this scenario rather than expecting each cycle to complete in seconds.
- The DAPI request-budget exhaustion this scenario targets is a *shared,
  per-client* resource — if another tool/process is also hitting the same
  SDK client concurrently (e.g. an `det-cli` smoke test running in
  parallel), the observed exhaustion timing may differ from a clean,
  single-consumer session. Run this scenario in isolation from other DAPI
  traffic against the same data directory.
- This is the only scenario in the library that deliberately drives
  multiple resync cycles in one session — if a *different*, unrelated
  defect surfaces only under repeated resync (not specific to #950), note
  it here rather than assuming it belongs to another scenario file.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
