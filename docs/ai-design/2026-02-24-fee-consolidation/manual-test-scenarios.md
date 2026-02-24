# Manual Test Scenarios: Consolidated Core and Platform Fee Estimation

**PR:** #651 (`fix/consolidate-core-fee-estimation`)
**Date:** 2026-02-24
**Components:**
- `src/model/fee_estimation.rs` (unified fee utilities)
- `src/backend_task/core/mod.rs` (RPC wallet payment with iterative fee, SPV guard)
- `src/model/wallet/asset_lock_transaction.rs` (asset lock with calculate_relay_fee)
- `src/ui/wallets/send_screen.rs` (Platform send with unified fee estimator)
- `src/backend_task/identity/register_identity.rs` (consolidated UTXO removal)
- `src/backend_task/identity/top_up_identity.rs` (consolidated UTXO removal)

## Background

PR #651 consolidates fee estimation across the entire app:

1. **Core tx fee estimation** — RPC wallet payment now uses iterative fee estimation
   (`estimate_p2pkh_tx_size` + `calculate_relay_fee`) instead of a hardcoded 1,000 duff fee.
   If the actual tx size after building exceeds the initial 5-input estimate, the fee is
   recalculated and the tx is rebuilt. SPV mode is guarded: fee re-estimation returns an error
   instead of silently losing UTXOs.

2. **Asset lock fee estimation** — the multi-input path (`asset_lock_transaction_from_private_key`)
   now uses `calculate_relay_fee(estimate_asset_lock_tx_size(...))` plus a retry loop. The
   single-UTXO path (`asset_lock_transaction_for_utxo_from_private_key`) also uses
   `calculate_asset_lock_fee` for consistency (replacing a former hardcoded 3,000 duff fee).

3. **Platform fee consolidation** — Platform-to-Platform transfers, Platform-to-Core
   withdrawals, Core-to-Platform asset locks, and identity creation/top-up from platform
   addresses all use the unified `PlatformFeeEstimator` with `max(legacy, transition)` logic.

4. **UTXO removal consistency** — `FundWithUtxo` paths in identity registration and top-up
   now call `remove_selected_utxos()` via the shared helper instead of manual `retain()`.

---

## Preconditions (all scenarios)

1. Dash Evo Tool is running and connected to **Testnet** (or a Devnet).
2. A wallet is loaded with known UTXO distribution (check wallet screen for balances).
3. The wallet has been synced recently (UTXOs are up to date).
4. Dash Core node is reachable and accepting transactions.

---

## Section A: Core Wallet Payment Fee Estimation

These scenarios verify the RPC wallet payment path which previously used a hardcoded 1,000 duff
fee and now uses iterative size-based estimation.

### A1: Single-Recipient Payment — Fee Covers Actual Size

**Purpose:** Verify that a normal send from the Core wallet uses a dynamically calculated fee
(not a hardcoded 1,000 duffs) and succeeds.

#### Preconditions
- Core wallet has at least one UTXO of 100,000 duffs.

#### Steps
1. Navigate to the Wallets screen.
2. Select a wallet and click Send (Core wallet payment).
3. Enter a valid Testnet recipient address and an amount of 50,000 duffs.
4. Leave "subtract fee from amount" unchecked.
5. Confirm the transaction.

#### Expected Results
- The transaction broadcasts successfully.
- The fee deducted is based on the actual tx size (not a fixed 1,000 duffs).
  With 1–2 inputs and 2 outputs: `8 + 148*n + 34*2 = ~224–372 bytes` → fee ~224–372 duffs.
- The wallet balance decreases by 50,000 + actual fee.
- No "min relay fee not met" rejection from the network.

---

### A2: Payment with Many Inputs — Iterative Fee Recalculation

**Purpose:** Verify that when the actual number of inputs exceeds the initial 5-input estimate,
the fee is recalculated and the tx is rebuilt with the correct fee.

#### Preconditions
- Core wallet has 10+ small UTXOs (e.g., 10 UTXOs of 6,000 duffs each).
- No single UTXO covers the full amount + fee alone.

#### Steps
1. Navigate to the Wallets screen → Send.
2. Enter a valid recipient address and an amount of 50,000 duffs.
3. Confirm the transaction.

#### Expected Results
- The transaction consumes multiple UTXOs.
- The fee reflects the actual input count (> 5 inputs → fee may be recalculated in the second
  pass).
- The transaction is accepted by the network without fee errors.
- The success dialog shows a valid txid.

---

### A3: Subtract Fee from Amount

**Purpose:** Verify that "subtract fee from amount" correctly deducts the dynamically computed
fee from the output, not a hardcoded value.

#### Preconditions
- Core wallet has a single UTXO of 50,000 duffs.

#### Steps
1. Navigate to the Wallets screen → Send.
2. Enter a valid recipient address and 50,000 duffs.
3. Check "subtract fee from amount".
4. Confirm.

#### Expected Results
- The transaction broadcasts successfully.
- The recipient receives 50,000 − (dynamic fee) duffs, not 50,000 − 1,000 duffs (old hardcoded).
- The wallet balance drops to 0 (or near 0 if dust remains).

---

### A4: Multiple Recipients

**Purpose:** Verify that the fee estimate accounts for multiple output scripts.

#### Preconditions
- Core wallet has sufficient balance (100,000+ duffs).

#### Steps
1. Navigate to the Wallets screen → Send (advanced mode if available).
2. Add two distinct Testnet recipient addresses, each receiving 20,000 duffs.
3. Confirm the transaction.

#### Expected Results
- The transaction has 3 outputs (2 recipients + 1 change).
- Fee is estimated as: `estimate_p2pkh_tx_size(inputs, 3)` × rate.
- Both recipients receive the exact amounts. Change is returned to wallet.
- Transaction accepted by the network.

---

## Section B: Asset Lock Fee Estimation

These scenarios cover the asset lock transaction path used for identity registration, top-up,
and generic platform address funding.

### B1: Single-Input Asset Lock — Minimum Fee Applied

**Purpose:** Verify a 1-input asset lock uses at least the 3,000 duff minimum fee.

With 1 input and 2 outputs: `10 + 148 + 68 + 60 = 286 bytes` → relay fee ~286 duffs, but the
minimum of 3,000 duffs applies.

#### Preconditions
- Wallet has a single UTXO of at least 50,000 duffs.

#### Steps
1. Navigate to the Identity screen.
2. Initiate a new identity registration with 10,000 duffs.
3. Confirm.

#### Expected Results
- The asset lock transaction is created and broadcast.
- Fee = 3,000 duffs (minimum, since size-derived fee is lower).
- Wallet balance decreases by 10,000 + 3,000 = 13,000 duffs.
- A change output is returned if the UTXO exceeds 13,000 duffs.

---

### B2: Multi-Input Asset Lock — Fee Scales with Input Count

**Purpose:** Verify that for many inputs, the dynamic fee exceeds the 3,000 minimum and the
transaction is still accepted.

#### Preconditions
- Wallet has 30+ small UTXOs (e.g., 30 UTXOs of 5,000 duffs each).
- No single UTXO covers the full amount.

#### Steps
1. Navigate to the Identity screen.
2. Initiate identity registration with 100,000 duffs.
3. Confirm.

#### Expected Results
- Transaction consumes ~21+ UTXOs.
- With ~21 inputs and 2 outputs: `10 + (21×148) + (2×34) + 60 = 3,246 bytes` → fee 3,246 duffs.
- Fee exceeds the 3,000 minimum; network accepts the transaction.
- Wallet balance decreases by 100,000 + ~3,246 duffs.

---

### B3: Asset Lock Fee Retry — Initial Estimate Insufficient

**Purpose:** Verify the fee-retry loop picks up additional UTXOs when the initial minimum-fee
estimate was too low for the actual input count.

#### Preconditions
- Wallet has many small UTXOs where the initial 3,000 duff fee estimate would require one fewer
  UTXO than the real fee (edge case — requires careful setup, best confirmed via unit test or
  with a wallet of ~30 UTXOs of just-over-100 duffs each).

#### Steps
1. Initiate identity registration with an amount that triggers the retry (amount ~= sum of
   available UTXOs minus fee).
2. Confirm.

#### Expected Results
- The transaction succeeds; no "not enough spendable funds" error.
- The retry loop internally recalculates the fee and selects the correct UTXOs.

---

### B4: Single-UTXO Identity Registration via UTXO Selection

**Purpose:** Verify the single-UTXO asset lock path (selected from the wallet's UTXO list) uses
the fee calculation helper rather than a hardcoded value, and removes the UTXO correctly.

#### Preconditions
- Wallet has a known UTXO (visible in the wallet UTXO list).

#### Steps
1. Navigate to the wallet UTXO list.
2. Select one UTXO and choose "Register Identity with this UTXO".
3. Confirm.

#### Expected Results
- The asset lock transaction is created with fee = `calculate_asset_lock_fee(utxo.value, ...)`.
- The transaction is broadcast and accepted.
- The UTXO is removed from the wallet UTXO list after the transaction (not before or on failure).
- The identity registration proceeds.

---

### B5: Single-UTXO Identity Top-Up via UTXO Selection

**Purpose:** Same as B4 but for identity top-up.

#### Preconditions
- An existing identity is registered in the wallet.
- Wallet has a known UTXO.

#### Steps
1. Navigate to the wallet UTXO list.
2. Select a UTXO and choose "Top Up Identity with this UTXO".
3. Confirm.

#### Expected Results
- Asset lock is created and broadcast.
- UTXO is removed from the wallet after confirmation.
- Identity balance increases by (UTXO value − fee) × credits_per_duff.

---

### B6: Tight Balance — Fee Deducted from Amount

**Purpose:** Verify that when the wallet barely covers the amount, the fee is deducted from the
output rather than failing, when the option is enabled.

#### Preconditions
- Wallet has exactly one UTXO of 10,000 duffs.

#### Steps
1. Navigate to platform address funding or identity top-up.
2. Set amount to 10,000 duffs with "deduct fee from amount" enabled.
3. Confirm.

#### Expected Results
- Transaction succeeds; actual credited amount = 10,000 − 3,000 = 7,000 duffs.
- No change output. Wallet balance drops to 0.

---

### B7: Insufficient Funds — Clear Error Message

**Purpose:** Verify a clear error is shown when the wallet cannot cover the amount + fee and fee
deduction is disabled.

#### Preconditions
- Wallet has exactly one UTXO of 10,000 duffs.

#### Steps
1. Navigate to platform address funding.
2. Set amount to 10,000 duffs with "deduct fee from amount" DISABLED.
3. Confirm.

#### Expected Results
- Error message includes the specific amounts: `"Insufficient funds: need 10000 + 3000 fee, have 10000"` (or similar).
- No transaction broadcast. Wallet UTXOs unchanged.

---

## Section C: Platform Fee Consolidation

These scenarios verify that Platform-side operations use the unified `PlatformFeeEstimator`
with `max(legacy, transition)` fee logic.

### C1: Platform-to-Platform Transfer — Fee Display and Deduction

**Purpose:** Verify that the Send screen correctly estimates and displays the Platform fee for
a Platform-to-Platform transfer, using the unified estimator.

#### Preconditions
- Wallet has at least one platform address with a balance of 500,000+ credits.
- A second valid platform address is available as destination.

#### Steps
1. Navigate to Wallets → Send.
2. Select "Platform Addresses" as the source.
3. Enter a destination platform address and an amount of 200,000 credits.
4. Observe the estimated fee shown in the UI before confirming.
5. Confirm the transfer.

#### Expected Results
- The estimated fee is shown in the UI before submission.
- The fee is computed as `max(legacy_estimate, transition_based_estimate)` — the higher of the
  two estimators is used.
- After the transition completes, the source address balance decreases by amount + fee.
- The destination address balance increases by the transferred amount.
- No "insufficient funds" or fee-related errors.

---

### C2: Platform-to-Core Withdrawal — Unified Fee

**Purpose:** Verify that a Platform-to-Core withdrawal uses the unified fee estimator.

#### Preconditions
- Wallet has a platform address with at least 1,000,000 credits (to cover withdrawal fee).
- A valid Core wallet address is available for the destination.

#### Steps
1. Navigate to Wallets → Send (or the dedicated withdrawal screen).
2. Select a Platform address as source and a Core wallet address as destination.
3. Enter a withdrawal amount.
4. Note the displayed fee estimate.
5. Confirm the withdrawal.

#### Expected Results
- The fee estimate uses the unified `PlatformFeeEstimator` (withdrawal transition fee).
- The transition is submitted successfully.
- After finality, the Core wallet receives the withdrawal amount (minus fee).
- The platform address balance decreases by the amount + fee.

---

### C3: Core-to-Platform Address Funding — Asset Lock with Platform Fee

**Purpose:** Verify that funding a platform address from Core wallet UTXOs correctly estimates
and includes the Platform fee in the asset lock amount.

#### Preconditions
- Core wallet has at least 100,000 duffs.
- A destination platform address is available.

#### Steps
1. Navigate to Wallets → Fund Platform Address (or the platform funding screen).
2. Enter a destination platform address and an amount of 50,000 duffs.
3. Leave "deduct fee from amount" unchecked.
4. Confirm.

#### Expected Results
- The asset lock amount is `50,000 + estimated_platform_fee_duffs` (fee paid from extra wallet
  balance, not deducted from the recipient amount).
- The destination platform address receives ~50,000 duffs worth of credits.
- The Core wallet balance decreases by more than 50,000 duffs (amount + Core fee + Platform fee).
- The transaction is accepted by the network.

---

### C4: Core-to-Platform Address Funding — Fee Deducted from Amount

**Purpose:** Verify that when "deduct fee from amount" is enabled, the recipient receives less
than the nominal amount but the Core fee is correctly reflected.

#### Preconditions
- Core wallet has at least 50,000 duffs.

#### Steps
1. Navigate to Wallets → Fund Platform Address.
2. Enter a destination and 50,000 duffs with "deduct fee from amount" enabled.
3. Confirm.

#### Expected Results
- The asset lock amount is 50,000 duffs (no Platform fee added to the lock).
- The Core fee is deducted from the amount, so the recipient receives 50,000 − fee credits.
- Wallet balance drops by exactly 50,000 duffs.

---

### C5: Identity Creation from Platform Address — Unified Fee

**Purpose:** Verify that creating an identity funded from a platform address uses the unified
Platform fee estimator.

#### Preconditions
- Wallet has a platform address with sufficient balance (≥ 500,000 credits).

#### Steps
1. Navigate to the Identity registration screen.
2. Select "Fund from Platform Address" (or equivalent).
3. Choose the platform address as the funding source.
4. Initiate registration.

#### Expected Results
- The estimated fee shown before confirmation uses `max(legacy, transition)` logic.
- Identity creation transition is submitted successfully.
- The platform address balance decreases by the registration amount + fee.
- The new identity appears in the identity list.

---

### C6: Identity Top-Up from Platform Address — Unified Fee

**Purpose:** Verify that topping up an identity funded from a platform address uses the unified
fee estimator.

#### Preconditions
- An existing identity is in the wallet.
- A platform address has at least 200,000 credits.

#### Steps
1. Navigate to the identity detail screen.
2. Initiate a top-up using the platform address as the funding source.
3. Enter 100,000 credits as the top-up amount.
4. Confirm.

#### Expected Results
- Fee is calculated using the unified `PlatformFeeEstimator`.
- The identity balance increases by approximately 100,000 credits (minus Platform fee).
- The platform address balance decreases accordingly.

---

## Section D: UTXO Removal Consistency

These scenarios verify that UTXOs are removed exactly once, after a transaction is fully built
and signed, and that the `remove_selected_utxos()` helper is used consistently.

### D1: Identity Registration Removes UTXO Exactly Once

**Purpose:** Verify that after a successful identity registration, the consumed UTXO is no
longer shown in the wallet, and that no duplicate or premature removal occurs.

#### Preconditions
- Wallet has 2–3 UTXOs of known values.
- Record the UTXO list before the test.

#### Steps
1. Navigate to the Identity registration screen.
2. Initiate registration using one of the known UTXOs.
3. Wait for the transaction to broadcast (or fail).
4. Navigate back to the wallet UTXO list.

#### Expected Results (success path)
- The consumed UTXO(s) are no longer in the list.
- Remaining UTXOs are intact.
- If a change output was created, the new UTXO appears after the wallet refreshes.

#### Expected Results (failure path — e.g., network error before broadcast)
- No UTXOs are removed from the wallet list.
- The wallet state is unchanged.

---

### D2: Identity Top-Up Removes UTXO Exactly Once

**Purpose:** Same as D1 but for identity top-up.

#### Preconditions
- An existing identity is in the wallet.
- Wallet has 2–3 known UTXOs.

#### Steps
1. Navigate to the identity detail screen → Top Up.
2. Initiate top-up using the wallet's UTXOs.
3. Wait for broadcast.
4. Check the UTXO list.

#### Expected Results (success path)
- Consumed UTXOs are removed from the list.
- Remaining UTXOs are intact.

#### Expected Results (failure path)
- UTXOs remain in the wallet list; no phantom removal.

---

### D3: No Double-Spend on Rapid Consecutive Operations

**Purpose:** Verify that if two operations are initiated in quick succession, the second does
not attempt to spend UTXOs already consumed by the first.

#### Preconditions
- Wallet has exactly 2 UTXOs of similar value.

#### Steps
1. Initiate identity registration (operation A) — do not wait for confirmation.
2. Immediately initiate a wallet send (operation B) for a different amount.
3. Observe both operations.

#### Expected Results
- Operation A and B each use distinct UTXOs (no overlap).
- Both succeed, or the second fails gracefully with "insufficient funds" if UTXOs are exhausted.
- No panic, no silent UTXO duplication.

---

## Section E: Edge Cases

### E1: SPV Mode — Fee Re-Estimation Error

**Purpose:** Verify that in SPV mode, if the initial fee estimate is too low and UTXOs cannot
be reloaded, an error is returned rather than silently losing UTXOs or panicking.

#### Preconditions
- App is running in SPV mode (no Dash Core node configured).
- Wallet has UTXOs loaded via SPV sync.

#### Steps
1. In SPV mode, attempt a wallet payment where the actual tx input count exceeds the 5-input
   estimate (e.g., 8+ small UTXOs needed).
2. Observe the result.

#### Expected Results
- If the initial fee was sufficient, the payment succeeds normally.
- If the initial fee was insufficient AND the UTXOs cannot be reloaded (SPV mode),
  the error message is:
  `"Fee re-estimation failed: cannot reload UTXOs in SPV mode"`.
- No UTXOs are lost from the wallet state.
- The UI displays the error in the message banner.

---

### E2: SPV Mode — Override Fee Retry

**Purpose:** Verify that a payment rejected by the network due to low fee can be retried with
an explicit fee override (if this UI flow is exposed).

#### Preconditions
- App in SPV mode. Wallet has funds.

#### Steps
1. Attempt a payment that is rejected by the network with "min relay fee not met".
2. If the UI shows a "retry with higher fee" option, confirm the retry.

#### Expected Results
- The retry uses the `override_fee` field, bypassing re-estimation.
- The retried transaction is accepted by the network.
- Only the override fee amount is deducted, not a double fee.

---

### E3: Poisoned Lock — Graceful Error (not a manual test)

**Note:** The poisoned-lock fix (review finding FEE-005) ensures that `core_client.read()` and
`wallets.read()` return a descriptive error string instead of panicking when a lock is poisoned.
This scenario cannot be triggered manually in normal usage, but is verified by the fact that
all lock accesses now use `.map_err(|e| e.to_string())` or equivalent. Confirm via code review
that no `.unwrap()` or `.expect()` calls remain on these locks in the modified files.

---

### E4: Very Large Number of Inputs

**Purpose:** Confirm that transactions with 100+ inputs are still accepted after the fee
consolidation.

#### Preconditions
- Wallet has 100+ dust/tiny UTXOs (e.g., from faucet drips).

#### Steps
1. Attempt a payment or identity registration that consumes 100+ UTXOs.
2. Confirm.

#### Expected Results
- With 100 inputs: estimated size ≈ `10 + (100×148) + (2×34) + 60 = 15,198 bytes`.
  Fee ≈ 15,198 duffs. The transaction is accepted by the network.
- No overflow or panic from large input count arithmetic.

---

### E5: Minimum-Balance Wallet — Dust Check Prevents Invalid Output

**Purpose:** Verify that when the change amount falls below the dust threshold (546 duffs),
no dust change output is created.

#### Preconditions
- Wallet has exactly one UTXO of 53,500 duffs.

#### Steps
1. Attempt identity registration with 50,000 duffs.
2. With 1 input: fee = 3,000 duffs, change = 53,500 − 50,000 − 3,000 = 500 duffs (below 546).
3. Confirm.

#### Expected Results
- No change output is created (500 duffs < 546 duff dust threshold).
- The 500 duffs are added to the fee instead.
- Transaction has 1 output (the burn output). Accepted by the network.
- Wallet balance drops to 0.
