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
