# Scenario: Funding minimums — Add funds, Create identity, Send, Shield, existing fundings

**Verifies:** every place that moves Dash from the Core wallet to Platform
states and enforces a smallest accepted amount (or fee) before anything is
sent, and that Max, deposit requests and existing-funding lists agree with it.
Complements [`wallet-max-send-asset-lock.md`](wallet-max-send-asset-lock.md)
(which covers Max/ceiling behaviour); this file covers the lower bound, the
displayed fee and the funding lists. (Risk area: #1059, #1063, #1064, #1065.)

**Tier justification:** Needs a live synced wallet with a small controlled
balance, real fee data for the connected protocol version and real network
refusal behaviour.

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here). Describe what you observe
on each build without assuming which one is "correct", and note where a
control or screen exists on only one build (a feature-presence difference,
not a step failure).

Steps marked **MUTATES** broadcast a transfer and need per-build equivalent
wallets (same starting balance and UTXOs) — see the contract below. Every
other step stops before the final confirm and moves nothing.

## Prerequisites

- Network: testnet
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — funded wallet; for the small-balance variant a
    second wallet drained to about 0.001–0.003 DASH
  - `E2E_IDENTITY_ID` — identity used as a top-up / Send target
- A wallet with an unfinished (unused) funding of a very small amount, if
  one can be made; otherwise record the lists as "no small funding available"
- Expert mode on (Shield and Platform-address Send need it)
- The fixture wallet can really broadcast a Core transaction (check before
  planning steps 9-10); otherwise use a temporary funded wallet as described in
  the README's "Procedure lessons for A/B campaigns"
- Verify exact captions during execution

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/funding-minimums.log"
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

Resize the window (see the README's "Known UI/environment quirks") before
judging layout. Check `det-stderr.log` / `det.log` in `$DATADIR` for panics
after each run.

## Procedure

For each flow record, **without sending**: the caption under the amount field
(smallest accepted amount, fee text), the value Max produces, the validation
state of a very small amount (0.00005 DASH), and what is shown when the
wallet cannot cover the minimum at all.

1. **Add funds → From your wallet.** Identity Home → "Add funds".
2. **Add funds → receive a deposit.** Record the amount the deposit request
   asks for, and whether a smaller arriving deposit still lets the form open
   with an amount.
3. **Create identity → from wallet.** Record the minimum with 1 key and after
   adding keys up to 6, and the message when the wallet is too small.
4. **Create identity → deposit request.** Record the amount requested.
5. **Wallets → Send → Core to an identity ID.** The "Send to" box accepts a
   pasted identity ID; clicking an entry in its suggestion list fills the
   field, so check which ID landed there. Record the minimum, caption,
   and the refusal text on a too-small amount.
6. **Wallets → Send → Core to a Platform address.** Record the caption, Max
   and, in advanced mode, the "fee from wallet" variant with a small amount.
7. **Shield.** Shielding has its own screen: Wallets ▸ Shielded tab ▸
   "Shield" button (separate from Send; Send to your own shielded address
   is a second entry). Enter 0.01 DASH and
   record the fee text shown, then 0.001 DASH, then Max on the small wallet
   and the message when the fee cannot be covered. Record the confirmation dialog's
   wording, including whether the destination is named. A wallet created in
   this session cannot shield until the app is restarted.
8. **Existing fundings.** With a small unfinished funding, open the list in
   Add funds, in Create identity, and the Asset Locks table on the Wallets
   screen. Record whether the entry can be selected ("Select"/"Fund"), the
   "Usable" column value, the explanation text and any end-of-list note.
9. **Tiny send (MUTATES).** On the small wallet, send the smallest accepted
   amount for one flow (Add funds or Send → identity) and record the result
   message and the wallet balance before/after.
10. **Shield (MUTATES).** Shield 0.01 DASH from the Core wallet through the
    Shielded tab's "Shield" screen. Record the
    amount named in the result message, the shielded balance after the next
    sync, and the wallet balance change. Compare with the fee displayed in
    step 7.

## Safety constraints specific to this scenario

- Never send more than the smallest accepted amount in steps 9-10 (shield at
  most 0.01 DASH).
- Do not press the final send/create control in steps 1-8.

## Expected outcome / pass criteria

Record the observed behavior for each build at every step, as a table of
flow → minimum, fee shown, Max, small-amount validation.

- Dash that leaves the wallet and is then refused by the network, a shielded
  amount different from what the result message states, or a displayed fee
  that differs from the fee actually charged, on one build only, are
  differences to classify under the contract (they can mean lost funds).

Apply the [A/B build comparison contract](../README.md#ab-build-comparison-contract):
its blocker rule decides what blocks, and its equivalent-fixture rule applies
to every step marked **MUTATES** below.

## Known gotchas

- Fee figures depend on the protocol version of the connected network; compare
  builds on the same network at the same time.
- A funding's remaining amount after partial use is not known to the app; a
  partially used funding may still be refused on either build.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
