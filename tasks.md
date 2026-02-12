# Dash Evo Tool — Tauri Migration Meta Review

> **Branch:** `react-native`
> **Purpose:** Comprehensive audit comparing egui and Tauri implementations
> **Priority:** P0 = blocks functionality, P1 = likely to cause bugs, P2 = correctness/quality, P3 = polish

---

## Backend / Frontend Bridge Issues

- [ ] **B.1 GroupStateTransitionInfoStatus parsing always returns None** (P0)
  `src-tauri/src/commands/token.rs:387-394` — `parse_group_info()` has a TODO and always returns `Ok(None)`. This means **all group-based token operations silently ignore group info** from the frontend.
  **Affected operations (9):** token_mint, token_burn, token_destroy_frozen_funds, token_freeze, token_unfreeze, token_pause, token_resume, token_set_direct_purchase_price, token_update_config.
  **Impact:** Group signing flows are broken — any multi-sig / group-authorized token operation will fail or behave incorrectly.
  **Fix:** Implement manual parsing of `GroupStateTransitionInfoStatus` from JSON since it doesn't derive `Deserialize`.

- [ ] **B.2 token_save_locally command is stubbed** (P0)
  `src-tauri/src/commands/token.rs` — The command returns `Err("token_save_locally: TokenInfo deserialization not yet implemented...")`. Frontend's `tokenStore.saveTokenLocally()` will always fail.
  **Impact:** Cannot programmatically save fetched tokens to the local database.
  **Fix:** Implement `TokenInfo` deserialization from the JSON value, or restructure to pass individual fields.

- [ ] **B.3 Token payment conversion not implemented in document commands** (P1)
  `src-tauri/src/commands/document.rs` lines 346, 376, 408, 438, 468, 498 — All six document operation commands have `let token_payment = None; // TODO:` stubs.
  **Affected commands:** document_broadcast, document_delete, document_replace, document_transfer, document_purchase, document_set_price.
  **Impact:** Any document operation that requires token payment will silently drop the payment info. The frontend has `TokenPaymentInfoDto` in its input types but it's never used.
  **Fix:** Implement `TokenPaymentInfoDto` → `TokenPaymentInfo` conversion in each command.

- [ ] **B.4 GroveSTARK key resolution blocks the Tauri main thread** (P1)
  `src-tauri/src/commands/system.rs:629-704` — The `grovestark_generate_proof` command is synchronous but performs heavy work: loads all qualified identities, iterates with Base58 string comparison, resolves private keys from wallet (disk I/O). This blocks the threadpool and can freeze the UI.
  **Fix:** Convert to an async task dispatched via `task_dispatcher` like other heavy operations, or at minimum convert the command to `async`.

- [ ] **B.5 Settings commands don't invalidate in-memory context cache** (P1)
  `src-tauri/src/commands/settings.rs` — Several settings write to the database but don't update the in-memory `AppContext` cache. For example, `settings_update_onboarding_completed` writes to DB but if another command reads from context cache, it gets stale data.
  **Fix:** Audit all settings update commands and ensure they also update the corresponding in-memory state, or ensure reads always hit DB.

- [ ] **B.6 Core backend mode switch has a race condition** (P2)
  `src-tauri/src/commands/settings.rs:400-421` — `context_set_core_backend_mode` sets the mode to RPC then calls `stop_spv()` non-atomically. Between these two calls, a concurrent command could see RPC mode but SPV still running.
  **Fix:** Use a lock or perform both operations atomically.

- [ ] **B.7 Proof log pagination integer overflow** (P2)
  `src-tauri/src/commands/proof_log.rs:173` — `input.page * input.items_per_page` can overflow u64 if the frontend sends large values. Debug mode will panic, release mode wraps.
  **Fix:** Use `saturating_mul` or add input validation.

- [ ] **B.8 DashPay avatar bytes stored without size validation** (P2)
  `src-tauri/src/commands/dashpay.rs:762-773` — `dashpay_db_save_avatar_bytes` accepts `Option<Vec<u8>>` with no maximum size. Frontend could send arbitrarily large data.
  **Fix:** Add a size limit (e.g., 5MB max) before writing to SQLite.

---

## Frontend / Store Issues

- [ ] **F.1 Verify identity store handles all operation results** (P1)
  The `identityStore` handles list/select/alias/delete but does NOT have actions for RegisterIdentity, TopUpIdentity, AddKey, DisableKeys, or ReplaceKey. These appear to be handled directly by screen components. Verify that task result events from these operations properly update the store state (e.g., identity list is refreshed after registration).

- [ ] **F.2 Wallet store requires explicit reload after mutations** (P2)
  After operations like address generation, the Tauri frontend must explicitly call `walletGetHd()` to refetch wallet state. In egui, results came back in the task result payload. Verify that all mutation flows properly trigger a store reload.

- [ ] **F.3 Asset lock index-based lookup race condition** (P2)
  `src-tauri/src/commands/wallet.rs` — `FundPlatformFromAssetLockInput` uses an index into `wallet.unused_asset_locks`. If the wallet state changes between the frontend reading the list and the backend processing the command, the index could be stale.
  **Fix:** Consider using txid-based lookup instead of positional index, or validate the expected txid matches.

---

## Cross-Cutting Verification

- [ ] **V.1 End-to-end test: wallet creation → identity registration → DPNS name** (P0)
  Full flow test with real Tauri backend. This exercises wallet create, identity register (with FundWithWallet), and DPNS name registration — the most common user journey.

- [ ] **V.2 End-to-end test: token operations with group signing** (P0)
  Since group signing is currently broken (B.1), once fixed, verify that mint/burn/freeze with group authorization works end-to-end.

- [ ] **V.3 End-to-end test: document CRUD with token payment** (P1)
  Once B.3 is fixed, verify document operations that require token payment work correctly.

- [ ] **V.4 Verify network switching preserves state correctly** (P1)
  Switch networks and verify: wallet list updates, identity list clears/reloads, contracts refresh, SPV status resets. The Tauri event system must properly handle context changes.

- [ ] **V.5 Verify SPV lifecycle across wallet lock/unlock** (P1)
  Test: create wallet → lock → unlock → verify SPV resumes. The Tauri version uses explicit `wallet_notify_unlocked`/`wallet_notify_locked` commands vs egui's implicit context-based approach.

---

## Notes

### What's Working Well
- **Identity domain**: Full parity plus enhancements (DisableKeys, ReplaceKey, MessageSigning)
- **Wallet domain**: ~95% parity, better error messages, explicit SPV control
- **Contract/Document domain**: Tree view, query, CRUD all present
- **DPNS/Contests**: All voting operations, scheduled votes, contest viewing present
- **DashPay**: Profile, contacts, payments, search all present
- **Tools**: All 8 tool screens implemented with feature parity
- **Settings**: All settings available via granular API endpoints
- **Platform Info**: All 8 query types available
- **Security**: GroveSTARK keys resolved server-side (improvement over egui)

### Architecture Differences (Not Bugs)
- Tauri uses Zustand stores for centralized state vs egui's per-screen state
- Tauri uses task event system (`taskResultEvent`/`taskErrorEvent`) vs egui's MPSC channels
- Tauri passes wallet identifiers (seed hash hex) vs egui's `Arc<RwLock<Wallet>>`
- Tauri uses index-based asset lock lookup vs egui's full proof passing (more efficient IPC)
- Tauri exposes SPV operations as explicit commands vs egui's implicit context behavior
