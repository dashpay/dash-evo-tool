# Manual Test Scenarios: Dynamic Asset Lock Fee Calculation

**PR:** #636 (`zk-extract/asset-lock-fee-fix`)
**Date:** 2026-02-24
**Component:** `src/model/wallet/asset_lock_transaction.rs`

## Background

This PR replaces the hardcoded 3000 duff fee in `asset_lock_transaction_from_private_key()` with a
dynamic fee calculated from the estimated transaction size:
- 10 bytes header + (inputs x 148 bytes) + (outputs x 34 bytes) + 60 bytes payload
- Minimum fee remains 3000 duffs (via `std::cmp::max`)
- Change output is recalculated after the real fee is determined
- Fee shortfall is handled gracefully when `allow_take_fee_from_amount` is set

The change affects three public entry points:
- `registration_asset_lock_transaction()` -- identity registration
- `top_up_asset_lock_transaction()` -- identity top-up
- `generic_asset_lock_transaction()` -- platform address funding

**Note:** `asset_lock_transaction_for_utxo_from_private_key()` (single-UTXO variant) is NOT changed
by this PR and still uses a hardcoded 3000 duff fee.

---

## Preconditions (all scenarios)

1. Dash Evo Tool is running and connected to **Testnet** (or a Devnet).
2. A wallet is loaded with known UTXO distribution (check wallet screen for balances).
3. The wallet has been synced recently (UTXOs are up to date).
4. Dash Core node is reachable and accepting transactions.

---

## Scenario 1: Single-Input Asset Lock (fee equals minimum)

**Purpose:** Verify that a transaction with 1 input still works correctly and the fee is at least
3000 duffs. With 1 input and 2 outputs: `10 + 148 + 68 + 60 = 286 bytes`, which is below 3000, so
the minimum fee of 3000 applies.

### Preconditions
- Wallet has a single UTXO of at least 50,000 duffs (0.0005 DASH).

### Steps
1. Navigate to the Identity screen.
2. Initiate a new identity registration with an amount of 10,000 duffs.
3. Confirm the transaction.

### Expected Results
- The asset lock transaction is created and broadcast successfully.
- The transaction is accepted by the network (no "min relay fee not met" error).
- The wallet balance decreases by approximately 10,000 + 3,000 = 13,000 duffs.
- If the UTXO was larger than the required amount, a change output is returned to the wallet.
- The identity registration proceeds to completion (or awaits proof confirmation).

---

## Scenario 2: Multiple-Input Asset Lock (fee scales with inputs)

**Purpose:** Verify that when many UTXOs are consumed, the fee scales upward with the number of
inputs, exceeding the 3000 duff minimum.

### Preconditions
- Wallet has many small UTXOs (e.g., 30 UTXOs of 5,000 duffs each = 150,000 duffs total).
- No single UTXO is large enough to cover the requested amount alone.

### Steps
1. Navigate to the Identity screen.
2. Initiate a new identity registration with an amount of 100,000 duffs (0.001 DASH).
3. Confirm the transaction.

### Expected Results
- The transaction consumes multiple UTXOs (approximately 21+ inputs needed for 100,000 + fee).
- With ~21 inputs: estimated size = `10 + (21 * 148) + (2 * 34) + 60 = 3246 bytes`, so fee should
  be 3246 duffs (above the 3000 minimum).
- The transaction is accepted by the network without "min relay fee not met" errors.
- The wallet balance decreases by the requested amount plus the dynamically calculated fee.
- Previous behavior with a hardcoded 3000 fee would have been rejected by the network for large
  input counts, so acceptance confirms the fix works.

---

## Scenario 3: Tight Balance with Fee Deduction from Amount

**Purpose:** Verify that when `allow_take_fee_from_amount` is true and the wallet balance barely
covers the amount, the fee is correctly deducted from the output amount.

### Preconditions
- Wallet has exactly one UTXO of 10,000 duffs.

### Steps
1. Navigate to the platform address funding screen (or identity top-up).
2. Initiate a funding/top-up for 10,000 duffs with the "deduct fee from amount" option enabled.
3. Confirm the transaction.

### Expected Results
- The transaction is created successfully.
- The actual funded amount is 10,000 - 3,000 = 7,000 duffs (fee deducted from amount).
- No change output is present in the transaction.
- The transaction is accepted by the network.
- The wallet balance drops to 0.

---

## Scenario 4: Insufficient Funds (no fee deduction allowed)

**Purpose:** Verify that a clear error message is shown when the wallet cannot cover the amount
plus fee and fee deduction is not allowed.

### Preconditions
- Wallet has exactly one UTXO of 10,000 duffs.

### Steps
1. Navigate to the platform address funding screen.
2. Initiate a funding for 10,000 duffs with the "deduct fee from amount" option DISABLED.
3. Confirm the transaction.

### Expected Results
- The operation fails with an error message similar to:
  `"Insufficient funds: need 10000 + 3000 fee, have 10000"`
- The error message includes the specific amounts (requested, fee, available).
- No transaction is broadcast to the network.
- The wallet UTXOs remain unchanged.

---

## Scenario 5: Insufficient Funds (fee deduction allowed but amount too small)

**Purpose:** Verify that when the entire balance would be consumed by the fee alone, a clear error
is returned even when fee deduction is allowed.

### Preconditions
- Wallet has exactly one UTXO of 2,500 duffs (less than the minimum 3000 fee).

### Steps
1. Navigate to the identity registration screen.
2. Initiate a registration with an amount of 2,500 duffs, with fee deduction from amount enabled.
3. Confirm the transaction.

### Expected Results
- The operation fails with `"Insufficient funds for transaction fee"` or the UTXO selection
  returns None (displayed as an appropriate error in the UI).
- No transaction is broadcast.
- Wallet UTXOs remain unchanged.

---

## Scenario 6: Change Output Presence and Correctness

**Purpose:** Verify that the change output is correctly calculated after the dynamic fee is applied,
not the initial estimate.

### Preconditions
- Wallet has a single UTXO of 100,000 duffs.

### Steps
1. Navigate to the Identity screen.
2. Initiate a new identity registration with an amount of 50,000 duffs.
3. Confirm the transaction.

### Expected Results
- The transaction has exactly 2 outputs: the burn output (50,000 duffs) and a change output.
- Change = 100,000 - 50,000 - fee. With 1 input, 2 outputs: fee = max(3000, 10+148+68+60) = 3000.
- Change should be 100,000 - 50,000 - 3,000 = 47,000 duffs.
- The change output goes to a wallet-controlled change address.
- After the transaction confirms, the wallet shows approximately 47,000 duffs remaining.

---

## Scenario 7: No Change Output (exact amount consumed)

**Purpose:** Verify that when inputs exactly equal amount + fee, no change output is created.

### Preconditions
- Wallet has a single UTXO of exactly 53,000 duffs.

### Steps
1. Navigate to the Identity screen.
2. Initiate a new identity registration with an amount of 50,000 duffs.
3. Confirm the transaction.

### Expected Results
- The transaction has exactly 1 output (the burn output for 50,000 duffs). No change output.
- Fee = 3,000 duffs (1 input, 1 output: `10 + 148 + 34 + 60 = 252`, min 3000 applies).
- 53,000 - 50,000 - 3,000 = 0, so no change.
- The wallet balance drops to 0 after confirmation.

---

## Scenario 8: Transaction Acceptance by Network (regression)

**Purpose:** This is the core regression test -- confirm that transactions which previously failed
with "min relay fee not met" now succeed.

### Preconditions
- Wallet has 50+ small UTXOs (e.g., from prior dust consolidation or faucet drips).

### Steps
1. Attempt to register an identity or top up an identity using an amount that requires consuming
   many UTXOs (e.g., 200,000 duffs spread across 50+ UTXOs of 4,000 duffs each).
2. Confirm the transaction.

### Expected Results
- The transaction is broadcast successfully (no RPC error).
- The transaction is accepted into the mempool (no rejection).
- With 50 inputs: estimated size = `10 + (50 * 148) + (2 * 34) + 60 = 7538 bytes`.
- Fee should be 7,538 duffs (well above the old hardcoded 3,000).
- The identity registration or top-up completes normally after proof confirmation.

---

## Scenario 9: Identity Top-Up with Dynamic Fee

**Purpose:** Verify the fix works for identity top-up flows (not just registration).

### Preconditions
- An identity already exists in the wallet.
- Wallet has sufficient balance (e.g., 100,000 duffs).

### Steps
1. Navigate to the identity detail screen.
2. Initiate a top-up of 20,000 duffs.
3. Confirm the transaction.

### Expected Results
- The asset lock transaction is created with a dynamically calculated fee.
- The transaction is accepted by the network.
- The identity balance increases by approximately 20,000 duffs (in credits).
- Wallet balance decreases by 20,000 + fee.

---

## Scenario 10: Platform Address Funding with Dynamic Fee

**Purpose:** Verify the fix works for the generic platform address funding flow.

### Preconditions
- Wallet has sufficient balance (e.g., 100,000 duffs).
- A valid platform address is available to fund.

### Steps
1. Navigate to the platform address funding screen.
2. Enter a destination platform address and amount of 30,000 duffs.
3. Choose "deduct fee from amount" = disabled.
4. Confirm the transaction.

### Expected Results
- The asset lock transaction is created with a dynamically calculated fee.
- The transaction is accepted by the network.
- The destination platform address receives exactly 30,000 duffs worth of credits (minus platform
  fees, but not minus Core tx fees).
- Wallet balance decreases by more than 30,000 duffs (amount + estimated platform fee + Core fee).

---

## Edge Cases

### E1: Very Large Number of Inputs
- If a wallet has 100+ tiny UTXOs and attempts to spend them all, the fee could exceed 15,000 duffs.
  Verify the transaction is still accepted and the fee does not consume an unreasonable portion of
  the amount.

### E2: UTXO Selection Crosses Initial Estimate Threshold
- The initial estimate of 3000 duffs is used for UTXO selection. If the real fee is higher and
  causes a shortfall (total inputs < amount + real fee), the code should handle this via the
  `allow_take_fee_from_amount` path or return an error. Verify no panic or silent data loss occurs.

### E3: Database Connection Change (secondary change in PR)
- The PR also removes `Arc` wrapping from the `Database.conn` field and removes the
  `shared_connection()` method. Verify that all database operations (wallet persistence, UTXO
  tracking, identity storage) continue to work correctly after this change. This is a structural
  refactor and should not affect behavior.

### E4: Concurrent Asset Lock Creation
- If two asset lock transactions are created in quick succession (e.g., rapid identity registration
  attempts), verify that UTXO double-spend is prevented and the second attempt either uses different
  UTXOs or fails gracefully.
