# Scenario: "Max" across asset-lock funding and Transfer/Withdraw flows

**Verifies:** "Max" (and its dispatch-time validation) reflects what the
wallet can actually build/send, across every flow that offers a Max button:
Shield DASH, Fund Platform Address (Simple builder-driven form), Send
directly to an identity, Create Identity / Top Up Identity from wallet
balance or a received deposit, and identity Transfer/Withdraw. Must include
a near-empty ("dust") wallet variant and a rapid-UTXO-composition-change
variant specifically — these are real edge cases in this diff and are not
reliably described by the CHANGELOG text. (Risk area: #937, #927.)

**Tier justification:** Needs a live, synced SPV wallet with real UTXOs
(confirmed, unconfirmed, dust-only) and a real broadcast to prove Max
actually sends on the first try — the ceiling query and the fee-reserve
estimator both depend on genuine wallet/network state a no-display harness
can't fake.

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here). Describe what you observe
on each build without assuming which one is "correct."

Before any step that spends funds, consumes a deposit, or registers a
name, each build needs its own independently-funded equivalent fixture (or
a restored snapshot of the same starting state) — see [A/B build comparison
contract](../README.md#ab-build-comparison-contract).

## Prerequisites

- Network: testnet
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — funded testnet wallet
- A wallet with an unusually large number of small UTXOs, if available
  (otherwise send several small amounts to the test wallet first)
- A second, separate near-empty wallet (drained close to the network dust
  threshold) for the dust-wallet variant
- A registered, funded identity (for the Transfer/Withdraw half)
- Verify exact widget/button labels during execution before relying on the
  names used below

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/wallet-max-send.log"
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

## Procedure

1. **Checking → Max → send, each asset-lock flow.** For each of: Shield
   DASH, Fund Platform Address (Simple form), Send to identity, Create
   Identity (fund from wallet balance), Top Up Identity (fund from wallet
   balance) — open the flow, note whether/how the amount field indicates a
   check is in progress, press Max, then send. Record whether the send
   succeeds on the first attempt.
2. **Received-deposit ceiling.** On Create Identity or Top Up Identity, use
   "fund from a received deposit." Record what ceiling Max offers relative
   to what actually arrived at that specific deposit address versus the
   wallet's total balance elsewhere.
3. **Advanced manual-input path.** On the Advanced manual-input
   Platform-address flow, record what Max/validation is governed by (the
   Core inputs selected, or something else).
4. **Stale-quote behavior.** Start a Max check on any flow above, then —
   before sending — change the wallet's spendable funds from elsewhere
   (send some of the same wallet's funds out, or receive a new deposit).
   Record what the amount field does, and what the eventual send validates
   against.
5. **Check-failure UX.** If you can force the availability check to fail
   (e.g. briefly interrupt network during the check), record what the
   amount field shows and whether a retry path is offered, and whether you
   can still switch to a different funding method.
6. **Dust wallet.** Using the near-empty/dust wallet, open any flow above
   and press Max. Record the result and whether it settles or stays
   perpetually "checking."
7. **Rapid UTXO churn.** With a wallet whose balance is actively changing
   (mid-sync, or receiving several small deposits in quick succession), open
   a funding flow and watch the amount field for at least 30 seconds. Record
   how many times it re-checks automatically and whether it settles or
   keeps re-dispatching.
8. **Identity Transfer, Max.** Open Transfer credits between identities,
   press Max, confirm and send. Record success/failure and the reserved fee
   amount shown.
9. **Identity Withdraw, Max.** Open Withdraw to Core address, press Max,
   confirm and send. Same recording.
10. **Reserve accuracy.** After steps 8–9, compare the amount actually
    deducted (visible in the transaction/activity detail) to what Max
    reserved. Record whether they're a close, sane match or wildly
    different.

## Safety constraints specific to this scenario

- Cap every real send to the smallest amount that proves the behavior.
- Step 7 needs to run long enough (≥30s) to distinguish "one legitimate
  automatic retry" from "runs away re-dispatching" — don't conclude early.

## Expected outcome / pass criteria

Record the observed behavior for each build at every step, then apply this
rule:

- **BLOCKING** only if HEAD is worse than baseline in the happy flow (e.g.
  Max sends successfully on baseline but fails/rejects on HEAD for the same
  wallet state; HEAD's dust-wallet or rapid-churn case gets permanently
  stuck where baseline at least settled; HEAD's fee reserve is wildly less
  accurate than baseline's), or if a **data-loss** outcome appears on either
  build.
- **NOT blocking**: a check that legitimately runs longer on one build than
  the other without ever getting stuck; a UI wording difference with no
  functional consequence; an issue reproducing identically on both builds.
- If baseline doesn't have a Max button on a flow at all where HEAD does (or
  vice versa), record that as a feature-presence difference, not a
  pass/fail on that step.

## Known gotchas

- The probe can hold the wallet-manager write lock for up to ~5 seconds per
  call in a pathological case on HEAD — a real send attempted at the exact
  same moment may appear to briefly stall rather than fail; wait a few
  seconds before treating that as a hang.
- Issue #909 / `rust-dashcore#911` (Send Core→Core Max fails with
  `InsufficientFunds` when change would be zero) is a distinct, real,
  already-tracked, **still-open** upstream bug unrelated to this scenario's
  asset-lock/Transfer/Withdraw Max — if a plain Core-to-Core "Max send"
  reproduces it on both builds, that's pre-existing, not new.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
