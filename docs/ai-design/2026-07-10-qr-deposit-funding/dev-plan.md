# Development Plan — QR/Receive-Deposit Identity Funding (IDN-014 restore)

Phase 1c. For `developer-bilby`. Restores the "fund by receiving a deposit"
method on Register-Identity (`add_new_identity_screen`) and Top-Up
(`top_up_identity_screen`), routed through the **existing** `FundWithWallet`
→ `AssetLockFunding::FromWalletBalance` path. Test IDs reference
`test-cases.md` (TC-QRFUND-01..17).

## Scope guardrails (read first)

- **NO upstream `platform-wallet` change. NO new `BackendTask`/`WalletTask`
  variant. NO new `TaskError` variant.** The new funding method reuses the
  exact dispatch `UseWalletBalance` already uses:
  `register_identity.rs:56` / `top_up_identity.rs:59`
  `FundWithWallet(..)` → `FromWalletBalance { amount_duffs, account_index: 0 }`.
- The receive-address QR reuses the existing
  `WalletTask::GenerateReceiveAddress { seed_hash }` →
  `BackendTaskSuccessResult::GeneratedReceiveAddress { seed_hash, address }`
  (`backend_task/wallet/generate_receive_address.rs`) — the SPV-watched pool
  address. Same task `create_asset_lock_screen` already consumes.
- Reference implementation to mirror for deposit detection:
  `create_asset_lock_screen.rs:679-697` — **single-address equality** against
  `self.funding_address`, NOT `known_addresses` membership (TC-QRFUND-06/07).
  Note the REG screen's *other* detection point at `mod.rs:1268-1290`
  (`WaitingForAssetLock` arm) uses `known_addresses` — that is asset-lock-tx
  surfacing, a different concern; do not copy it for deposit detection.

## Task 1 — shared pure helper + enum in `funding_common.rs`

Single file, unit-tested first (TDD). Covers the logic both screens share so
detection is testable without a live egui screen (TC-QRFUND-14, Marvin pt 3).

1. **`FundingMethod::ReceiveDeposit` variant** (l.19). `Display` (l.31) and
   `top_up_label` (l.47) are exhaustive `match` with no wildcard, so the
   compiler forces a label in both — supply jargon-free copy for each
   (TC-QRFUND-03). Suggested: Display `"Receive a new deposit"`; top_up_label
   identical (no "new identity" assumption — §6 parity). Extend the existing
   exhaustive tests `display_is_jargon_free_for_every_variant` and
   `top_up_label_differs_only_for_asset_lock` to include it (TC-QRFUND-02/03/16).
2. **`fn deposit_matches(funding_address: Option<&Address>, outputs:
   &[(OutPoint, TxOut, Address)]) -> u64`** — pure; sums `TxOut.value` of
   outputs whose `Address == funding_address` (equality, not membership);
   returns `0` when address is `None` or no match. This is the guard both
   screens call from `display_task_result`. Unit tests: match ≥ minimum
   (TC-QRFUND-04), different address → 0 (TC-QRFUND-06), empty/None → 0.
3. **`fn reset_to_choose() -> (FundingMethod, WalletFundedScreenStep)`** or reuse
   `default_funding_state` for the never-trap back-out target
   (`NoSelection`/`ChooseFundingMethod`), asserting `funding_address` clearing is
   the caller's job (TC-QRFUND-10). Keep it a pure returning-helper; the field
   write stays in the screen.

Running-total display (TC-QRFUND-05) reads wallet spendable in the UI (needs a
wallet read, not pure) — `deposit_matches` supplies the per-event threshold
decision; the cumulative figure shown comes from the wallet snapshot. Document
this split in a one-line comment.

## Task 2 — wire both screens (one Bilby pass)

Both screens are near-twins bound to the same helper; splitting them would
double the shared-helper churn. **Do both in one pass.** Order within the pass:
helper first (Task 1), then REG, then TOPUP (TOPUP already partially references
`ReceivedAvailableUTXOTransaction` at `mod.rs:512`).

Per screen:

1. **Chooser option** — add a `selectable_value` for `ReceiveDeposit` beside the
   existing three (REG `render_funding_method` ~l.560-611; TOPUP ~l.298-337). On
   select, set step to `WaitingOnFunds` (not `ReadyToCreate`) and dispatch
   `WalletTask::GenerateReceiveAddress { seed_hash }` to populate
   `funding_address`.
2. **`GeneratedReceiveAddress` handling** — in `display_task_result`, on that
   result for the selected wallet's `seed_hash`, store `funding_address`
   (mirror `create_asset_lock_screen.rs:665-674`). REG's `funding_address` field
   must be added if absent; TOPUP already has it (`mod.rs:54`).
3. **Revive `WaitingOnFunds` arm** (REG `mod.rs:1265` empty; TOPUP equivalent):
   on `CoreItem::ReceivedAvailableUTXOTransaction(_, outputs)`, call
   `deposit_matches`; if cumulative spendable ≥ minimum → set `FundsReceived`.
   Guard fires ONLY when `step == WaitingOnFunds` (TC-QRFUND-07).
4. **Revive `FundsReceived` arm** — pre-fill the amount input via
   `max_amount_after_fee_reserve(spendable_duffs, estimated_fee)` (already tested
   pure fn; TC-QRFUND-08); leave editable, clamped by existing `AmountInput`
   (TC-QRFUND-09). On confirm, dispatch the **existing**
   `FundWithWallet(amount_duffs, identity_index[, top_up_index])` and set
   `WaitingForAssetLock` — identical to the `UseWalletBalance` confirm arm
   (REG l.988-995; TOPUP l.390-403). From here the flow is already implemented
   (`WaitingForAssetLock` → `WaitingForPlatformAcceptance` → `Success`),
   TC-QRFUND-01.
5. **QR render** in the `WaitingOnFunds` view — `generate_qr_code_image(pay_uri)`
   (exists, l.254) with a `dash:<address>?amount=` URI, show address text +
   minimum-amount hint + running total (TC-QRFUND-05), and
   `request_repaint_after(1s)` (no timeout — TC-QRFUND-15).
6. **Never-trap affordance** — a "Choose a different funding method" button
   present in BOTH `WaitingOnFunds` and `FundsReceived`, resetting to
   `ChooseFundingMethod`/`NoSelection` and clearing `funding_address`
   (TC-QRFUND-10). No error banner on back-out.
7. **Failure reset** — reuse existing reset points (REG l.1222; TOPUP l.484):
   `WaitingForAssetLock`/`WaitingForPlatformAcceptance` failure → `ReadyToCreate`
   (TC-QRFUND-11/12). No new code path — the `FundWithWallet` error already
   surfaces via `AppState`.

## Error handling — no new variant (TC-QRFUND-11)

`ReceiveDeposit` funds land in the wallet balance first, then route through
`FundWithWallet` → `FromWalletBalance`. Any build/broadcast failure leaves the
deposit in the wallet — the "funds are safe" condition is structural. The
existing typed `FundWithWallet` failure surface (`error.rs`, e.g. the
payment-preparation / wallet-service variants) already covers it. **Soft
option only:** if reassurance wording ("your deposit is safe in your wallet")
is wanted, refine the *existing* wallet-funding failure variant's `#[error(..)]`
copy — do not add a variant. Flag to coordinator; not required for green tests.

## Task 3 — `docs/user-stories.md` IDN-014

Flip `[Removed — upstream-only funding]` → `[Implemented]`. Replacement (Diziet's
draft, refined): *"As an everyday user, I can fund a new identity or a top-up by
receiving a Dash deposit to an address the tool shows me as a QR code, so I can
pay from any wallet or exchange without first moving funds into this tool.
Acceptance: choosing 'Receive a new deposit' shows a scannable address; once
enough arrives the amount pre-fills and I confirm to create/top-up; I can switch
funding methods at any time; a failure leaves my deposit safe in the wallet."*
Non-code doc edit — fold into the same PR, its own commit.

## Ordering / dependencies

Task 1 → Task 2 (Task 2 depends on the helper + variant). Task 3 independent.
**One Bilby invocation** covers Tasks 1+2+3: ~150-250 lines across
`funding_common.rs` + two screens + the doc, tightly coupled through the shared
helper. Splitting screens across passes would fork the helper contract — do not.

## Test traceability

Unit (helper + `display_task_result`): 01-08, 11, 12, 14, 15, 16, 17.
kittest (widget/nav/lock-gate): 09, 10, 13, 17. Manual testnet (coordinator GUI):
05 accumulation, 12 asset-lock reuse, 14 funds-safe-on-chain. Extract
`deposit_matches`/reset helpers so 04/06/07/14 are unit-reachable without egui.
