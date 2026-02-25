# Manual Test Scenarios: Platform Sync Simplification

**PR:** #635 (`zk-extract/platform-sync-simplification`)
**Date:** 2026-02-24
**Scope:** Removal of `PlatformSyncMode` enum (Auto/ForceFull/TerminalOnly), replacement of
full/terminal sync logic with SDK-managed incremental sync, simplified `RefreshMode` dropdown,
removal of `last_full_sync_balance` and `last_terminal_block` from persistence.

---

## Preconditions (all scenarios)

- Dash Evo Tool built from the `zk-extract/platform-sync-simplification` branch.
- Dash Core node running and reachable (or Regtest local node for Regtest scenarios).
- At least one wallet loaded with a known mnemonic / seed phrase.
- Platform (DAPI) endpoints reachable for the selected network.
- Developer mode enabled (Settings > Developer Mode) for scenarios that reference the
  refresh mode dropdown.

---

## Scenario 1: Fresh Wallet Sync (No Prior Sync State)

**Goal:** Verify that a brand-new wallet with no database sync state performs a complete
Platform address balance sync successfully.

### Steps

1. Create or import a new wallet that has never been synced.
2. Ensure the wallet is unlocked (seed phrase entered / wallet open).
3. On the Wallets screen, click the Refresh button (default mode "Core + Platform").
4. Wait for the sync to complete.

### Expected Results

- A loading/spinner indicator appears during sync.
- After completion, Platform address balances are displayed (may be zero for a truly new wallet).
- No errors or warnings appear in the UI.
- In the application logs (`RUST_LOG=info`), you see:
  - `Platform address sync start` (no "mode" field -- the old `mode: Auto/ForceFull/TerminalOnly`
    log line should be absent).
  - `Sync complete: duration=..., found=..., absent=..., checkpoint=..., new_sync_height=..., new_sync_timestamp=...`
  - `Platform address sync complete: total_duration=..., addresses_with_balance=...`
- The database `wallet` table is updated with non-zero `last_platform_full_sync` and
  `last_platform_sync_checkpoint` values for the wallet's seed hash.

---

## Scenario 2: Incremental Sync (Wallet Already Has Balances)

**Goal:** Verify that a wallet that was previously synced performs an incremental sync
(using `last_ts`) and picks up balance changes since the last sync.

### Steps

1. Start with a wallet that was already synced at least once (Scenario 1 completed).
2. From another tool or wallet, send a small amount of Dash credits to one of the
   wallet's Platform addresses (e.g., via asset lock or transfer).
3. Return to Dash Evo Tool. Click Refresh ("Core + Platform").
4. Wait for sync to complete.

### Expected Results

- The balance of the funded Platform address increases by the expected amount.
- Sync is noticeably faster than a full sync (the SDK receives `last_ts` > 0 and
  performs incremental catch-up rather than querying all addresses from scratch).
- Logs show `new_sync_height` and `new_sync_timestamp` are updated to newer values
  than the previous sync.
- No double-counting of balances (the old `apply_recent_balance_changes` two-pass
  approach is gone; the SDK handles deltas internally).

---

## Scenario 3: Refresh Mode Dropdown (Developer Mode)

**Goal:** Verify that the simplified refresh mode dropdown in developer mode offers
exactly three options and each triggers the correct behavior.

### Preconditions

- Developer mode is enabled.

### Steps

1. On the Wallets screen, locate the refresh dropdown next to the Refresh button.
2. Click the dropdown and observe the available options.
3. Select **"Core + Platform"** and click Refresh. Observe behavior.
4. Select **"Core Only"** and click Refresh. Observe behavior.
5. Select **"Platform Only"** and click Refresh. Observe behavior.

### Expected Results

- **Step 2:** Exactly three options are listed:
  - "Core + Platform" (default)
  - "Core Only"
  - "Platform Only"
  - The old options ("All (Auto)", "Platform (Full)", "Platform (Terminal)",
    "Core + Platform (Full)", "Core + Platform (Terminal)") must NOT appear.
- **Step 3 ("Core + Platform"):** Both Core wallet UTXOs/transactions and Platform
  address balances are refreshed. The loading indicator covers both operations.
- **Step 4 ("Core Only"):** Only Core wallet data refreshes. Platform address
  balances remain unchanged (no Platform network calls in logs).
- **Step 5 ("Platform Only"):** Only Platform address balances refresh. Core wallet
  balance and transaction list remain unchanged.

---

## Scenario 4: Platform Address Balance Updates After Funding

**Goal:** Verify balances update correctly after funding a Platform address via
asset lock or wallet UTXOs.

### Steps

1. Open a wallet with Core balance available.
2. Initiate "Fund Platform Address" (either from asset lock or from wallet UTXOs).
3. Enter an amount and confirm the transaction.
4. Wait for the operation to complete.

### Expected Results

- After funding completes, an automatic Platform balance refresh is triggered
  (the code calls `fetch_platform_address_balances(seed_hash)` without any mode argument).
- The funded Platform address shows the increased balance.
- The Core wallet balance decreases by the funded amount plus fees.
- No errors appear in the UI or logs.

---

## Scenario 5: Platform Address Balance Updates After Withdrawal

**Goal:** Verify balances update correctly after withdrawing from a Platform address.

### Steps

1. Open a wallet that has Platform address balance > 0.
2. Initiate "Withdraw from Platform Address."
3. Enter an amount and confirm.
4. Wait for the operation to complete.

### Expected Results

- After withdrawal completes, an automatic Platform balance refresh is triggered.
- The Platform address balance decreases by the withdrawn amount.
- No errors appear in the UI or logs.

---

## Scenario 6: Wallet Lock/Unlock Behavior During Sync

**Goal:** Verify that sync correctly reports an error when the wallet is locked,
and succeeds after unlocking.

### Steps

1. Lock the wallet (close it / remove seed phrase from memory).
2. Click Refresh ("Core + Platform" or "Platform Only").
3. Observe the error.
4. Unlock the wallet (enter seed phrase / passphrase).
5. Click Refresh again.

### Expected Results

- **Step 3:** An error message appears: "Wallet is locked. Please unlock it first to refresh."
- **Step 5:** Sync completes successfully; Platform address balances are displayed.

---

## Scenario 7: Network Switching (Mainnet / Testnet / Regtest)

**Goal:** Verify that sync state is per-network and switching networks does not
corrupt balances.

### Steps

1. Connect to Testnet. Load a wallet and perform a full refresh. Note the balances.
2. Switch to Mainnet (or another network). Perform a refresh.
3. Switch back to Testnet.

### Expected Results

- **Step 1:** Testnet balances are displayed and persisted.
- **Step 2:** Mainnet balances are fetched independently; Testnet balances are not
  displayed or overwritten.
- **Step 3:** Testnet balances are restored from the database and match the values
  from Step 1 (unless on-chain state changed).
- The `platform_address_balances` table uses the `network` column to isolate data.

---

## Scenario 8: Regtest with Empty Balance Tree

**Goal:** Verify graceful handling when the Platform balance tree is empty (common
on fresh Regtest/devnet with no funded addresses).

### Preconditions

- Regtest or local devnet with no Platform balances funded yet.

### Steps

1. Connect to Regtest.
2. Load a wallet.
3. Click Refresh ("Core + Platform").

### Expected Results

- No crash or unhandled error.
- The sync completes with zero balances displayed.
- Logs show: `Platform address balance tree is empty. Returning empty sync result.`
- The `ban_failed_address: false` config is applied for Regtest (verified in logs
  or by the fact that DAPI addresses are not banned after the empty-tree response).

---

## Scenario 9: No DAPI Endpoints Available

**Goal:** Verify graceful error handling when Platform is unreachable.

### Steps

1. Disconnect from the network or configure invalid DAPI endpoints.
2. Click Refresh ("Core + Platform" or "Platform Only").

### Expected Results

- An error message is displayed to the user (e.g., "Failed to sync Platform addresses: ...").
- The application does not crash or freeze.
- Core-only refresh still works if selected separately ("Core Only" mode).
- Previously stored Platform balances remain visible from the database cache.

---

## Scenario 10: Database Migration / Upgrade from Previous Version

**Goal:** Verify that upgrading from a version with the old 3-tuple sync state
(`last_full_sync`, `checkpoint`, `last_terminal_block`) to the new 2-tuple
(`last_sync_timestamp`, `last_sync_height`) does not cause data loss or crashes.

### Steps

1. Run the previous version of the app (before this PR) and perform at least one
   Platform sync to populate the old database columns.
2. Upgrade to the build from this PR branch.
3. Launch the application.
4. Navigate to the Wallets screen. Observe that existing wallet data loads.
5. Perform a refresh.

### Expected Results

- The application starts without database errors.
- The old `last_terminal_block` column, if still present, is simply ignored
  (the new code reads only 2 columns from the query).
- Existing platform address balances load correctly from the database
  (the `last_full_sync_balance` column is no longer read but its presence
  does not cause errors).
- The first sync after upgrade behaves like a fresh sync (since the old
  checkpoint semantics differ from the new `sync_height`), and subsequent
  syncs are incremental.

---

## Scenario 11: Pending Platform Balance Refresh After Transfer

**Goal:** Verify that the pending refresh mechanism (triggered after credit
transfers between Platform addresses) works correctly with the simplified API.

### Steps

1. Open a wallet with at least two Platform addresses, both with balances.
2. Initiate a Platform credit transfer from one address to another.
3. Wait for the transfer to complete.

### Expected Results

- After the transfer completes, a pending platform balance refresh is automatically
  queued (`pending_platform_balance_refresh` fires).
- Both addresses update: sender balance decreases, receiver balance increases.
- The refresh uses `FetchPlatformAddressBalances { seed_hash }` with no mode
  parameter (verified in logs -- no "sync_mode" field).

---

## Edge Cases Checklist

| Edge Case | Expected Behavior |
|---|---|
| Wallet with 0 Platform addresses derived | Sync completes with no found addresses; no error |
| Very large number of Platform addresses (>100) | Sync completes; gap limit logic extends as needed |
| Sync interrupted mid-way (app closed) | Next launch performs a clean sync from stored state |
| Two wallets synced concurrently | Each wallet syncs independently by seed hash |
| `last_sync_timestamp` is 0 in DB | SDK receives `None` for `last_ts`, performs full sync |
| `last_sync_timestamp` > 0 in DB | SDK receives `Some(ts)`, performs incremental sync |
