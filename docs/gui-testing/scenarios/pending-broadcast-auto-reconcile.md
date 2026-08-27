# Scenario: pending-broadcast auto-reconcile fires only on a real network verdict

**Verifies:** when a Send-screen payment's broadcast returns `TransactionConfirmationUnknown`,
the auto-reconciling banner (`PendingConfirmation` in `src/app/reconcilers.rs`) flips to
"Your transaction is confirmed." only once the transaction is genuinely InstantSend-locked or
mined — never merely because dash-spv injected it into its own local mempool before any peer
verdict (the phantom-balance shape in project memory, `0d38d2d4`).

**Tier justification:** needs a real broadcast against a live testnet peer set and real
InstantSend/block timing; `kittest` has no network and cannot produce a genuine ambiguous
broadcast outcome (`TransactionBroadcastUnconfirmed`) or a real confirmation event.

## Prerequisites

- Network: testnet
- Environment variables:
  - none beyond the default `.env.example` (testnet DAPI/SPV endpoints)
- A funded testnet wallet with enough balance for at least two small sends
- Ability to induce the ambiguous-broadcast path (e.g. brief network throttling around the
  30s SPV acceptance timeout in `platform-wallet`'s `SpvBroadcaster::broadcast`), or patience
  to catch it occurring naturally

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"

pgrep -af dash-evo-tool

TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | \
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
BIN="$TARGET_DIR/debug/dash-evo-tool"
test -x "$BIN"
LOG="$DATADIR/pending-broadcast-auto-reconcile.log"
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

## Procedure

1. Open the Send screen for a funded testnet wallet.
2. Send a small amount to any valid testnet address, inducing (or waiting for) an ambiguous
   broadcast outcome — the error banner should read: *"Your transaction was sent but the
   confirmation could not be verified. Wait a moment, then refresh your balance before
   sending it again."*
3. Without touching Send again, navigate away to another screen (confirms the watch survives
   navigation — it lives in `AppState`, not screen state).
4. Watch for the transaction to reach an InstantSend lock or a mined block (check the
   wallet's transaction-history row for ground truth, independent of the banner).
5. Confirm the banner replaces itself with *"Your transaction is confirmed."* at (or shortly
   after) the moment the history row shows `⚡ InstantSend` or `Confirmed @h` — not before.
6. Separately, repeat steps 1-2 for a send whose transaction has **no wallet-owned change
   output** (spend the account down to as close to zero as fee math allows), to close R-4:
   confirm the watch still resolves for this shape (a purely-outgoing send some upstream event
   paths might not report identically to a send-with-change).

## Safety constraints specific to this scenario

- Use small testnet amounts only; testnet Dash has no real value but keep sends minimal to
  avoid needlessly draining the shared testnet faucet balance.
- Do not run this against mainnet.

## Expected outcome / pass criteria

- The banner never flips to the confirmed message while the transaction-history row still
  shows `Pending` with no InstantSend lock and no block height.
- The banner does flip to the confirmed message once the history row shows a genuine
  InstantSend lock or a mined height — within a few seconds of that (the watch polls every
  2s per `PENDING_POLL_INTERVAL`).
- The no-change-output send in step 6 resolves the same way as an ordinary send (closes R-4
  from the architecture plan at the time this scenario was written).

## Known gotchas

(fill in on first real run)

- This closes **R-1** from the `fix/auto-reconcile-broadcast-status` architecture plan: the
  open question of whether a locally-injected, never-relayed transaction can appear at a
  `Confirmed` tier in DET's snapshot without ever being accepted by the network. If this
  scenario ever shows the banner confirming before the history row shows a real height, that
  is a live bug in `network_took_transaction` (`src/app.rs`) or in the upstream snapshot data
  it reads, not a scenario problem — do not weaken the predicate to make this scenario "pass".

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
