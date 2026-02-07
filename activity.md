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
