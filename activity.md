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
