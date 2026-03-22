# PR #604 — Manual Review Guide

Key files for manual review: architectural decisions, reusable patterns, and behavioral changes.
Files that merely apply patterns defined here are excluded (~57 files).

**Diff summary**: 83 files changed, 3,106 insertions, 3,739 deletions (net −633 lines).

---

## 1. Core Infrastructure

### [`src/ui/components/message_banner.rs`](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-5c97c5af2fd0a32afba3e66e5f6a5f7acafb9a73da0d3b89a92f8fc9fca06a35)

Heart of the PR. All other changes depend on patterns defined here.

| What | Notes |
|---|---|
| `set_global` idempotency — no longer resets existing banners | Subtle semantic change; all callers rely on this |
| `replace_global` — docs + empty-text semantics | Used for progress update sequences |
| `with_details` takes `impl Debug` instead of `&str` | API broadening |
| `ResultBannerExt` — `or_show_error()` on `Result<T, E: Display>` | Shows error banner, passes `self` through unchanged |
| `OptionBannerShowExt` — `or_show_error()` on `Option<T>` | Shows named error banner when `None`, passes `self` through |
| `OptionBannerExt` — `take_and_clear()` on `Option<BannerHandle>` | Clears progress banner without leaking the handle |
| SEC-003: Eviction log no longer includes message text | Privacy fix |

**Extension trait summary**:

```rust
// Result<T, E>  — show banner on Err, return self unchanged
result.or_show_error(ctx)

// Option<T>  — show named banner on None, return self unchanged
option.or_show_error(ctx, "message")

// Option<BannerHandle>  — take handle + clear banner atomically
self.refresh_banner.take_and_clear()
```

**Review focus**: Verify the idempotency change in `set_global` (old behavior reset existing banners, new behavior is a no-op). This subtle semantic change affects all callers.

---

## 2. Behavioral / Architectural Decisions

### [`src/ui/mod.rs` — ScreenLike trait](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-d64f1e2614b0de74ea77854bc2fb944deaaa5b40a4e37b91a81a94da9bd5ebf8)

`display_task_result` default changed from showing "Success" banner to **no-op**. Breaking behavioral change — screens that relied on the default now silently swallow success results. `display_message` contract is also clarified: screens only implement it for side-effects (e.g., clearing a progress banner); banner display is handled centrally by `AppState`.

| What | Notes |
|---|---|
| `display_task_result` default → no-op | AppState now owns success banner display |
| `display_message` contract clarified | Side-effects only; all 60+ screen impls are boilerplate no-ops |

**Review focus**: Verify that `AppState` handles success banners centrally (see `src/app.rs` below), and no screen depended on the old default showing "Success".

### [`src/app.rs` — Task result routing + connection banner](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-8c6f1be9c6b6eb6dc2c76e6a6f2706d76f81aad0ff222c5a9ef4eab78acee7b5)

Two distinct changes:

**Centralized task result dispatch** (around L1023):
- `TaskResult::Success(Message)` → `MessageBanner::set_global` + `display_task_result`
- `TaskResult::Success(_)` catch-all → delegates to screen's `display_task_result` only (no global banner)
- `TaskResult::Error` → `MessageBanner::set_global` + optional debug details in developer mode + `display_message` side-effect

**Connection banner state machine** (around L881–960):

| What | Notes |
|---|---|
| `previous_connection_state` + `connection_banner_handle` fields | Track state for FSM |
| `clear_all_global` on network switch | Stale banners cleared when user changes network |
| `update_connection_banner()` — Disconnected/Syncing/Synced FSM | Replaces ad-hoc string matching in connection_status.rs |

**Review focus**: The FSM uses `OverallConnectionState` equality to suppress redundant banner updates. Verify state transitions cover edge cases (rapid Disconnected→Syncing→Disconnected). Verify `clear_all_global` on network switch does not clear banners that should persist.

### [`src/context/connection_status.rs`](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-75d4306c0e7eca30d7e0dbb01b6f6b3d3b3af1e65a22e6de68aa6c92d9b3779f)

Removed the `contains("Failed to get best chain lock...")` error-string-matching handler. The connection banner FSM in `app.rs` now owns this responsibility.

**Review focus**: Confirm the removed code path is fully covered by `update_connection_banner()` in `app.rs`, and that `ChainLocks` task no longer bails out when all networks fail (it now returns all-None as a valid result).

### [`src/backend_task/core/mod.rs`](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-...) — `build_unsigned_payment_tx`

`WalletManager::create_unsigned_payment_transaction` was removed upstream. This PR introduces a local replacement using `TransactionBuilder` directly.

| What | Notes |
|---|---|
| `build_unsigned_payment_tx` helper | Replaces removed SDK method; uses `SelectionStrategy::OptimalConsolidation` |
| `FeeLevel::Normal` → `FeeRate::normal()` | SDK type rename |
| All-None ChainLocks result now valid | Removed the early-bail error; returns whatever networks succeeded |

**Review focus**: The new `build_unsigned_payment_tx` manually assembles a transaction (change address, UTXO selection, output creation). Verify the `WalletError` mapping on selection failure (insufficient funds vs. generic build error).

### [`src/spv/manager.rs`](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-...) — ArcSwapOption migration

`client_interface: Arc<RwLock<Option<DashSpvClientInterface>>>` replaced with `spv_client: ArcSwapOption<SpvClient>`.

| What | Notes |
|---|---|
| `ArcSwapOption` for `spv_client` | Wait-free reads; eliminates lock contention on quorum lookups |
| `get_quorum_public_key` uses `load_full()` | No lock held across the async `block_on` call |
| `stop()` comment — no explicit clear needed | Client is cleared asynchronously when it stops |
| `client.start()` removed; client wrapped in `Arc` before storing | Lifecycle change — verify start/stop symmetry |

**Review focus**: The old code called `client.start()` explicitly before storing the interface. The new code stores the `Arc<SpvClient>` without a separate start call. Confirm the SPV client starts implicitly (e.g., on construction) and that the stop path correctly sets `spv_client` to None.

### [`src/ui/identities/mod.rs` — `get_selected_wallet` API](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-...)

Out-param error pattern replaced with `Result`.

| Old signature | New signature |
|---|---|
| `fn get_selected_wallet(..., error_message: &mut Option<String>) -> Option<Arc<RwLock<Wallet>>>` | `fn get_selected_wallet(...) -> Result<Option<Arc<RwLock<Wallet>>>, String>` |

**Review focus**: All callers updated. Verify no call site silently drops the `Err` variant.

### [`src/ui/tokens/mod.rs` — Shared helpers](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-0e09ce3e1e56a10a8d2cef2e29e7addb8e5a4dff5f78b8e15c50cdc94ef53e06)

Three helpers shared across ~15 token screens, reducing duplication.

| Helper | Notes |
|---|---|
| `load_identities_with_banner()` | Load identities + show error banner on failure |
| `set_error_banner()` | Thin wrapper around `MessageBanner::set_global` with Error type |
| `validate_signing_key()` | Signature: `&Option<T>` → returns `Option<&T>` |

---

## 3. Key Behavioral Changes (High Impact)

### Constructor panics eliminated

All screen constructors previously used `.expect()` for DB and identity loads. These now use `or_show_error()` or `MessageBanner::set_global` + graceful degraded state (empty list / zero balance). No constructor returns an error — callers remain clean.

**Representative example**: `src/ui/tokens/claim_tokens_screen.rs` — single DB load + `or_show_error` + `.unwrap_or_default()` fallback.

### Fee-aware validation — `src/ui/identities/transfer_screen.rs`

Amount validation now checks estimated fee before allowing submission:

```rust
let estimated_fee = self.app_context.fee_estimator().estimate_credit_transfer();
let max_transferable = (identity.balance() as u128).saturating_sub(estimated_fee as u128);
if credits > max_transferable {
    // error banner: "Amount plus estimated fee exceeds available balance (max: ...)"
}
```

`TransferCreditsStatus` enum simplified: `WaitingForResult(TimestampMillis)` → `WaitingForResult` (timestamp removed), `ErrorMessage(String)` → `Error` (string moved to global banner).

### Transfer tokens refresh filters by contract+position — `src/ui/tokens/transfer_tokens_screen.rs`

After a successful transfer, the refresh now filters the returned identity list by `data_contract_id` AND `token_position` to find the correct updated balance, rather than a naive first-match.

### QR scanner correct result handling — `src/ui/dashpay/qr_scanner.rs`

`parse_qr_code` previously called `self.display_message(...)` for all outcomes. Now uses `MessageBanner::set_global` directly. The `message: Option<(String, MessageType)>` field and inline rendering are removed.

### Broadcast status screens — elapsed rendering migrated to banner

`register_contract_screen.rs`, `update_contract_screen.rs`, `document_action_screen.rs` no longer store timestamps in status enum variants. Elapsed time is rendered via the `BannerHandle::with_elapsed` mechanism on the progress banner.

### Database error handling restored — `src/ui/dashpay/contacts_list.rs`

Contact update errors that were previously silently dropped now surface via `MessageBanner::set_global` with `tracing::error!`.

---

## 4. Reusable Pattern Examples (Representative)

### [`src/ui/tokens/claim_tokens_screen.rs`](https://github.com/dashpay/dash-evo-tool/pull/604/files#...) — Constructor pattern

Best example of the panic-elimination pattern: DB load + `or_show_error` + `.unwrap_or_default()` + `WaitingForResult` enum simplification (timestamp dropped).

### [`src/ui/identities/add_existing_identity_screen.rs`](https://github.com/dashpay/dash-evo-tool/pull/604/files#...) — Status enum simplification

`AddIdentityStatus::Error` changed from `Error(String)` to unit variant `Error`. Error text lives in the global banner. Constructor init error uses `MessageBanner::set_global`.

### [`src/ui/wallets/send_screen.rs`](https://github.com/dashpay/dash-evo-tool/pull/604/files#...) — BannerHandle lifecycle

Most complete example of the `BannerHandle` lifecycle: progress banner with elapsed time, cleared on result via `take_and_clear()`, with a `set_send_progress_banner()` helper.

---

## 5. Conventions / Docs

### [`CLAUDE.md`](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-82d8aa1d8fd8d9b71c3cf5e7fcade1d8697cc47c6df2e1b0c77ed5b0f01e5e93)

Updated conventions affect all future development.

| What |
|---|
| Constructor error handling: `or_show_error` + degraded state, no `expect()` |
| `BannerHandle` lifecycle: `refresh_banner` field, `take_and_clear()` on task arrival |
| `display_message` contract: side-effects only |

---

## Summary: files to review carefully vs. files to skim

| Priority | File | What to verify |
|---|---|---|
| Critical | `src/ui/components/message_banner.rs` | `set_global` idempotency, extension trait contracts |
| Critical | `src/ui/mod.rs` | `display_task_result` default → no-op; `display_message` contract |
| Critical | `src/app.rs` | Task result dispatch logic; connection banner FSM; `clear_all_global` on switch |
| High | `src/backend_task/core/mod.rs` | `build_unsigned_payment_tx` correctness; WalletError mapping; all-None ChainLocks |
| High | `src/spv/manager.rs` | ArcSwapOption migration; SPV client start/stop lifecycle |
| High | `src/context/connection_status.rs` | Removed error-string handler — covered by FSM in app.rs? |
| High | `src/ui/identities/mod.rs` | `get_selected_wallet` API: no caller silently drops `Err` |
| Medium | `src/ui/tokens/mod.rs` | Shared helpers; `validate_signing_key` signature |
| Medium | `src/ui/identities/transfer_screen.rs` | Fee-aware validation; status enum simplification |
| Medium | `src/ui/tokens/transfer_tokens_screen.rs` | Refresh filter by contract+position |
| Medium | `src/ui/dashpay/contacts_list.rs` | DB error handling restored |
| Low | `src/ui/tokens/claim_tokens_screen.rs` | Constructor pattern example |
| Low | `src/ui/identities/add_existing_identity_screen.rs` | Status enum example |
| Low | `src/ui/wallets/send_screen.rs` | BannerHandle lifecycle example |
| Low | `CLAUDE.md` | Convention accuracy |
| Skip | ~57 remaining screen files | Mechanical application of the patterns above |

The ~57 skipped files all apply the same pattern: remove `error_message: Option<String>` field, remove inline error rendering, remove `check_message_expiration`, replace `display_message` with `MessageBanner::set_global` calls, and add a boilerplate `display_message` no-op. If the patterns in the files above are correct, those files are correct by construction.
