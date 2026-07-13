# Test Case Specification — QR/Receive-Deposit Identity Funding (IDN-014 restore)

Phase 1b. Restores the removed "fund by receiving a deposit" method on the
Register-Identity (`add_new_identity_screen`) and Top-Up (`top_up_identity_screen`)
screens, routed through the existing `FundWithWallet` → `AssetLockFunding::FromWalletBalance`
backend path. Both screens are near-twins; cases apply to BOTH unless a
**[REG]**/**[TOPUP]** divergence is called out.

## Verified ground truth (corrections to the Phase-1a brief)

- **Detection is single-address equality, not `known_addresses`.** The reference
  (`create_asset_lock_screen.rs:685`) advances only when a received output's
  address `== self.funding_address`. Tests must encode equality against the ONE
  shown address, not membership in the wallet's address set.
- `funding_address: Option<Address>` already exists as a **per-screen** field in
  both screens (not in `funding_common.rs`).
- Deposit-state transitions are driven through `display_task_result()`; the
  existing kittest harness only renders `AppState` and cannot inject task
  results, so detection/state-machine cases are **unit-level** (call
  `display_task_result` with a synthesized `BackendTaskSuccessResult`), NOT
  egui_kittest. New pure helpers (see TC-QRFUND-14) should be extracted so the
  guard is testable without a live screen.
- No timeout exists (only `request_repaint_after(1s)`); "never errors while
  waiting" is structural, assert absence of error banner.
- `step`/`funding_address` init to `ChooseFundingMethod`/`None` on construction →
  waiting state does **not** survive reload.

## 1. State machine

**TC-QRFUND-01** — Happy path progression. Choose "Receive a new deposit" →
`WaitingOnFunds`; inject a `ReceivedAvailableUTXOTransaction` paying the shown
address ≥ minimum → `FundsReceived`; confirm amount → dispatch `FundWithWallet`,
step = `WaitingForAssetLock`; inject asset-lock result → `WaitingForPlatformAcceptance`;
inject acceptance → `Success`. Expected: exact ordered transitions, no skipped
state. Trace: journey 1–6. *Automatable (unit).*

**TC-QRFUND-02** — Dead-arm reactivation is non-regressive. With the four
already-live methods, `UseUnusedAssetLock`/`UseWalletBalance`/`UsePlatformAddress`
and `NoSelection` still reach their existing steps unchanged after the new
variant is added. Expected: existing transition table intact. Trace: journey 1.
*Automatable (unit — extend existing `funding_common` tests).*

**TC-QRFUND-03** — New `FundingMethod` variant forces copy decisions. The
existing exhaustive tests `display_is_jargon_free_for_every_variant` and
`top_up_label_differs_only_for_asset_lock` must be extended to include the new
variant. Expected: variant has a jargon-free `Display` label AND a `top_up_label`;
compile-time exhaustiveness prevents a silent `Debug` fallback. Trace: journey 1;
§8. *Automatable (unit).*

## 2. Deposit detection

**TC-QRFUND-04** — Single deposit ≥ minimum advances. `ReceivedAvailableUTXOTransaction`
with one output to the shown address, amount ≥ minimum → `FundsReceived`. Trace:
journey 2,4. *Automatable (unit).*

**TC-QRFUND-05** — Multiple partial deposits accumulate. Two sub-minimum deposits
to the shown address: after the first, step stays `WaitingOnFunds` and the running
total reflects deposit 1; after the second (cumulative ≥ minimum) → `FundsReceived`.
Expected: running total = sum; advance only on crossing minimum. Trace: journey 4.
*Automatable (unit) — but confirm cumulative balance source (SPV spendable) is
readable without live sync; the accumulation display itself needs **manual**
verification with real testnet deposits.*

**TC-QRFUND-06** — Deposit to a DIFFERENT address must NOT advance.
`ReceivedAvailableUTXOTransaction` whose output address ≠ `funding_address` →
step remains `WaitingOnFunds`, no total change. Trace: journey 2 (guard).
*Automatable (unit).* Encodes the single-address-equality correction.

**TC-QRFUND-07** — Guard scoped to active method/state. Inject a matching
`ReceivedAvailableUTXOTransaction` while the screen is in `ReadyToCreate` (a
different funding method selected) or `ChooseFundingMethod`. Expected: NO
spurious advance to `FundsReceived` — the match arm fires only when
step == `WaitingOnFunds`. Trace: journey 2 (guard). *Automatable (unit).*

## 3. Amount pre-fill

**TC-QRFUND-08** — Fee-reserve cap on pre-fill. On `FundsReceived`, the amount
field pre-fills `max_amount_after_fee_reserve(spendable_duffs, estimated_fee)`.
Expected: equals received-spendable-minus-fee; saturates to 0 when fee exceeds
balance. Trace: journey 5. *Automatable (unit — pure fn already tested; add a
test asserting the FundsReceived path calls it with received balance).*

**TC-QRFUND-09** — User may edit but not exceed received balance. Editing the
field is allowed; a value above spendable is rejected/clamped by the existing
`AmountInput` validation. Expected: confirm disabled / value clamped when
over-max. Trace: journey 5. *Automatable (kittest for the widget; unit for the
clamp rule).*

## 4. Never-trap

**TC-QRFUND-10** — "Choose a different funding method" reachable from every
waiting sub-state. From `WaitingOnFunds` and `FundsReceived`, the affordance is
present and returns to `ChooseFundingMethod` / `NoSelection`, clearing
`funding_address`. Expected: no dead end; no error banner on back-out. Trace:
journey 3. *Automatable (kittest — assert control exists & click resets step;
also unit for the reset helper).*

## 5. Failure / edge paths

**TC-QRFUND-11** — Build/broadcast failure resets safely. From `WaitingForAssetLock`,
inject a `FundWithWallet` failure `TaskError`. Expected: step resets to
`ReadyToCreate`; banner is a **typed** `TaskError` (not a string literal) stating
funds are safe in the wallet; [TOPUP] reuses existing 481–484 reset, [REG] the
1222 reset. Trace: journey 7. *Automatable (unit).*

**TC-QRFUND-12** — Platform rejection recoverable. From `WaitingForPlatformAcceptance`,
inject rejection. Expected: reset to `ReadyToCreate`, banner points user to the
existing `UseUnusedAssetLock` recovery path; funds not lost. Trace: journey 7.
*Automatable (unit) + **manual** confirm the asset lock is actually reusable on
testnet.*

**TC-QRFUND-13** — Wallet-locked gate unchanged. With a locked wallet, the
lock/secret gate fires BEFORE any Create dispatch, regardless of the new method.
Expected: identical gate behavior to existing methods. Trace: journey 7.
*Automatable (kittest — secret-prompt path) — see `tests/kittest/secret_prompt.rs`.*

**TC-QRFUND-14** — Reload while `WaitingOnFunds` resets gracefully. Reconstruct
the screen (simulating restart). Expected: step = `ChooseFundingMethod`,
`funding_address = None`, no crash; any deposit already sent remains in wallet
balance and is later usable via `UseWalletBalance`/`UseUnusedAssetLock`. This is
the SAME persistence model as the existing `WaitingFor*` states. Trace: journey
7 / §5. *Automatable (unit for the reset; **manual** for the on-chain-funds-safe
claim).*

**TC-QRFUND-15** — No error while idle-waiting. Remain in `WaitingOnFunds` with no
deposit across many frames. Expected: no error/timeout banner ever set; only
repaint scheduling. Trace: journey 3. *Automatable (unit — assert no banner).*

## 6. Register vs Top-Up parity — divergences to encode

- Labels: [REG] uses `Display`, [TOPUP] uses `top_up_label()` — new variant must
  supply BOTH (TC-QRFUND-03).
- [TOPUP] operates on an existing identity (has current balance/context); [REG]
  has none. Minimum-amount hint copy may differ; detection + state machine are
  identical. Flag any wording that assumes "new identity" in the shared copy.
- Otherwise detection, cap, and never-trap logic must be identical; a shared
  helper (extracted per TC-QRFUND-14) keeps them so.

## 7. Regression guard (existing methods — minimal re-run)

**TC-QRFUND-16** — Re-run existing `funding_common` unit suite (all
`default_funding_state`, `funding_method_after_switch`, label, and
`max_amount_after_fee_reserve` tests) unchanged-green. **TC-QRFUND-17** — Smoke:
each of `UseUnusedAssetLock`, `UseWalletBalance`, `UsePlatformAddress` still
selects → `ReadyToCreate` and dispatches its existing backend variant. *Automatable
(unit + existing kittest render smoke).*

## 8. i18n/UX string spot-check (flag only — do not rewrite)

Diziet's draft copy uses named placeholders (`{minimum_amount}`, `{received_amount}`)
and complete sentences — **passes** the CLAUDE.md convention on inspection. One
flag for the dev-plan stage: "Received {received_amount} so far. Waiting for at
least {minimum_amount}." is two sentences sharing a unit — fine, but ensure
`{received_amount}`/`{minimum_amount}` are pre-formatted amount strings (with unit),
not bare numbers, so no fragment concatenation leaks in. *Manual/review-time.*

## Automation summary

- **Unit (`display_task_result` + pure helpers):** 01–08, 11, 12, 14, 15, 16, 17
  — the core state-machine and detection coverage. Requires extracting the
  `WaitingOnFunds` guard into a testable helper.
- **kittest (widget/nav):** 09, 10, 13, 17 (render smoke).
- **Manual live-testnet only (coordinator's GUI pass):** real QR scan + deposit
  arrival (05 accumulation, 12 asset-lock reuse, 14 funds-safe-on-chain). These
  cannot be verified without funding a real address.
