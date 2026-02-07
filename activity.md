# Ralph Loop Activity Log

## Run 1 — 2026-02-07
**Task:** 1.1 [META] Triage wallet bugs
**What was done:** Triaged 6 GitHub issues and 24 auto-generated issue files against current codebase. Verified each claim by reading relevant source code. Created 7 specific sub-tasks for confirmed bugs.
**Files changed:** tasks.md, activity.md
**Sub-tasks created:** 7 (1.1a through 1.1g)

**Summary of findings:**
- GH#522 CONFIRMED: No auto-refresh of UTXOs on startup
- GH#476 CONFIRMED: Hardcoded ReduceOutput(0) fee strategy ignores user's "deduct from input" selection
- GH#475 NOT CONFIRMED: Multiple platform inputs already handled correctly
- GH#478 PARTIALLY CONFIRMED: Wallet balance top-up max button missing fee reservation
- GH#485 ALREADY FIXED: Platform addresses show "N/A" for UTXOs
- GH#85 CONFIRMED: Address reuse due to skip_known_addresses_with_no_funds=false in 4 locations
- wallet-008 CONFIRMED: Infinite loop waiting for asset lock proof (no timeout)
- wallet-023 CONFIRMED: Panic on Dash-Qt spawn failure
- wallet-015 CONFIRMED: Silently ignored database errors
- wallet-001, wallet-009, wallet-024: FALSE POSITIVE
- Remaining wallet issues: LOW PRIORITY or covered by other tasks

## Run 2 — 2026-02-07
**Task:** 1.1a Fix GH#522: Auto-refresh UTXOs on app startup
**What was done:** Added automatic UTXO refresh on startup in `bootstrap_loaded_wallets()`. In RPC mode, spawns background tasks to call `refresh_wallet_info` for each HD wallet and `refresh_single_key_wallet_info` for each single-key wallet. SPV mode is unaffected (handles UTXOs via reconciliation). This ensures wallet balances are current without requiring the user to manually click Refresh.
**Files changed:** src/context.rs

## Run 3 — 2026-02-07
**Task:** 1.1b Fix GH#476: Hardcoded fee strategy in platform address funding
**What was done:** Fixed the platform fee strategy in `fund_platform_address_from_wallet_utxos()` to respect the `fee_deduct_from_output` flag. When the user selects "deduct from input" (fee_deduct_from_output=false), the code now creates two outputs: the destination with an explicit `Some(amount_credits)` for the exact requested amount, and a change address (derived from the asset lock one-time key) as the `None` remainder recipient. The fee is deducted from the change output via `ReduceOutput(change_index)`, so the destination receives the exact amount. Previously, the fee was always hardcoded as `ReduceOutput(0)` against a single `None` output, causing the destination to always receive less than requested regardless of the fee strategy setting.
**Files changed:** src/backend_task/wallet/fund_platform_address_from_wallet_utxos.rs

## Run 4 — 2026-02-07
**Task:** 1.1c Fix GH#478: Wallet balance top-up max button doesn't reserve fees
**What was done:** In `top_up_funding_amount_input()` in `mod.rs`, the "UseWalletBalance" max amount calculation now subtracts the estimated identity top-up fee before setting the max. Previously, the max was set to `total_balance_duffs * 1000` (raw credits) with no fee buffer, causing the top-up to fail with "Insufficient combined address balances" when the user clicked Max. Now uses `fee_estimator.estimate_identity_topup()` to reserve fees, matching the pattern already used by `by_platform_address.rs`. Also added a `set_max_exceeded_hint` showing the reserved fee amount.
**Files changed:** src/ui/identities/top_up_identity_screen/mod.rs

## Run 5 — 2026-02-07
**Task:** 1.1d Fix GH#85: Funding address reuse across identities
**What was done:** Changed `receive_address()` calls from `skip_known_addresses_with_no_funds=false` to `true` in 4 locations: add_new_identity_screen/by_wallet_qr_code.rs, top_up_identity_screen/by_wallet_qr_code.rs, create_asset_lock_screen.rs, and generate_receive_address.rs. Previously, these all passed `false`, which allowed returning a previously-used address that happened to have zero balance, causing the same funding address to be reused across multiple identity registrations and top-ups. With `true`, the wallet skips known zero-balance addresses and generates a fresh one each time.
**Files changed:** src/ui/identities/add_new_identity_screen/by_wallet_qr_code.rs, src/ui/identities/top_up_identity_screen/by_wallet_qr_code.rs, src/ui/wallets/create_asset_lock_screen.rs, src/backend_task/wallet/generate_receive_address.rs

## Run 6 — 2026-02-07
**Task:** 1.1e Fix wallet-008: Add timeout to asset lock proof wait loop
**What was done:** Replaced the infinite busy-loop waiting for an asset lock proof (InstantLock or ChainLock) with a `tokio::select!`-based loop that times out after 5 minutes. If the proof never arrives (due to network issues, rejected transaction, ZMQ listener failure, or quorum problems), the function now returns a descriptive error instead of hanging forever. The timeout branch also cleans up the finality tracking entry to prevent map leaks.
**Files changed:** src/backend_task/wallet/fund_platform_address_from_wallet_utxos.rs

## Run 7 — 2026-02-07
**Task:** 1.1f Fix wallet-023: Replace panic on Dash-Qt spawn failure
**What was done:** Replaced `.expect("Failed to spawn dash-qt process")` with a `match` that logs the error via `tracing::error!` and returns early from the async block instead of panicking. This prevents the entire application from crashing if the Dash-Qt binary cannot be spawned (e.g., binary not found, permission denied, or path misconfigured). The error was already being logged by `inspect_err` but was then immediately followed by a panic via `expect`.
**Files changed:** src/backend_task/core/start_dash_qt.rs

## Run 8 — 2026-02-07
**Task:** 1.1g Fix wallet-015: Log silenced database errors in wallet operations
**What was done:** Replaced two `let _ =` patterns in `send_single_key_wallet_payment.rs` with `if let Err(e)` blocks that log warnings via `tracing::warn!`. The first logs when dropping spent UTXOs from the database fails (line 233), and the second logs when updating single key wallet balances fails (lines 238-240). Previously these database errors were silently discarded, making it impossible to diagnose post-payment DB inconsistencies.
**Files changed:** src/backend_task/core/send_single_key_wallet_payment.rs

## Run 9 — 2026-02-07
**Task:** 1.2 [META] Triage identity & token bugs
**What was done:** Triaged 4 GitHub issues (GH#499, GH#224, GH#273, GH#478) and 46 auto-generated issue files (identity-001 through identity-014, ui-tokens-001 through ui-tokens-024, ui-identity-001 through ui-identity-013) against the current codebase. Verified each claim by reading relevant source code. Created 8 specific sub-tasks for confirmed bugs.
**Files changed:** tasks.md, activity.md
**Sub-tasks created:** 8 (1.2a through 1.2h)

**Summary of findings:**
- GH#499(a) ALREADY IMPLEMENTED: ContractBounds supported in Identity Create
- GH#499(b) PARTIALLY CONFIRMED: ENCRYPTION/DECRYPTION keys lack security level enforcement in manual add flow
- GH#224 FALSE POSITIVE: Intentional simple mode behavior; advanced mode shows all keys
- GH#273 CANNOT CONFIRM: Off-by-one may be in SDK, not DET code
- GH#478 ALREADY FIXED by task 1.1c
- identity-001 CONFIRMED: Two panic!() calls on unsupported key types (P0)
- identity-002 CONFIRMED: Unwrap on try_into for hex decode (covered by 2.2 audit)
- ui-tokens-021 CONFIRMED: Token reordering completely non-functional (assignment commented out)
- ui-tokens-022 CONFIRMED: Wrong field checks in build_distribution_rules (logic error)
- ui-tokens-010 CONFIRMED: expect("No key selected") in 8+ token screens
- ui-identity-001 CONFIRMED: Unwrap panics if identity deleted during refresh
- identity-007 CONFIRMED: Silenced wallet update errors in register/top-up
- ui-identity-005/008 CONFIRMED: Silenced DB errors in identity deletion and contact save
- 15 issues classified as FALSE POSITIVE or LOW PRIORITY
- 4 issues deferred to existing tasks (2.5 lock poisoning, 2.6 SystemTime, 3.3 refactoring)

## Run 10 — 2026-02-07
**Task:** 1.2a Fix identity-001: Replace panic on unsupported key types
**What was done:** Changed `to_public_keys_map()` in `src/backend_task/identity/mod.rs` to return `Result<BTreeMap<KeyID, IdentityPublicKey>, String>` instead of panicking on unsupported key types. Replaced both `panic!("need a ECDSA Key for now")` calls (lines 167, 193) with descriptive error returns. Converted the inner `.map()` closure to a `for` loop to enable early `return Err(...)`. Updated both callers in `register_identity.rs` (lines 365, 720) to propagate the error with `?`.
**Files changed:** src/backend_task/identity/mod.rs, src/backend_task/identity/register_identity.rs

## Run 11 — 2026-02-07
**Task:** 1.2b Fix ui-tokens-021: Uncomment reorder assignment
**What was done:** Uncommented `self.my_tokens = reordered;` in `reorder_vec_to()` at `src/ui/tokens/tokens_screen/mod.rs:1799` so token reordering actually takes effect. Also added a safety loop to append any tokens not present in the saved order (e.g., newly added tokens since the last save) to the end, preventing data loss. Previously, the function built the reordered map but never assigned it, making the drag-to-reorder and saved-order features completely non-functional.
**Files changed:** src/ui/tokens/tokens_screen/mod.rs

## Run 12 — 2026-02-07
**Task:** 1.2c Fix ui-tokens-022: Wrong field checks in build_distribution_rules
**What was done:** Fixed two incorrect emptiness checks in `build_distribution_rules()` in `src/ui/tokens/tokens_screen/mod.rs`. Lines 2051 and 2065 both checked `step_decreasing_start_period_offset_input.is_empty()` to gate `min_value` and `max_interval_count`, but should have checked their own corresponding input fields. Changed to `step_decreasing_min_value_input.is_empty()` and `step_decreasing_max_interval_count_input.is_empty()` respectively. Previously, if the start_period_offset was empty but min_value or max_interval_count had values, those values would be silently set to None. Conversely, if start_period_offset was non-empty but min_value/max_interval_count were empty, the code would attempt to parse empty strings and return an error.
**Files changed:** src/ui/tokens/tokens_screen/mod.rs
