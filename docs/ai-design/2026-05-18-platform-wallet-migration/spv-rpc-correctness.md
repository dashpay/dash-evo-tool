# SPV / RPC Correctness

**Purpose:** Per-phase correctness analysis for SPV and RPC backend modes; identifies where each phase introduces new behavior and what gates close each risk.

[← back to README](README.md)

---

## D. SPV vs RPC Post-Migration Correctness

### Root Finding

Verified by grep: identity, credits, and DashPay code is backend-agnostic — it never branches on `CoreBackendMode`, never touches `SpvManager` or `WalletManager`. The only SPV/RPC coupling is the Core-funding side (asset-lock/UTXO/broadcast: `src/context/transaction_processing.rs:22`, `src/backend_task/core/mod.rs:500`), which already branches and is out of this migration's scope. The generic swap cannot reach the RPC payment path.

### Per-Phase Verdict Table

| Phase | SPV mode | RPC mode | Divergence / regression risk |
|---|---|---|---|
| **0 Spike** | Correct (no prod code) | Correct (no prod code) | None |
| **1 Newtype** | Correct — pure refactor | Correct and untouched — RPC never calls `wallet()` or the newtype (`send_wallet_payment_via_rpc` uses `core_client`, `src/backend_task/core/mod.rs:543`) | Low. QA gate: RPC-mode E2E to confirm no collateral change to shared code |
| **2 Adopt persister** | Correct — persister fed by reconcile + identity tasks | Correct but newly exercised — `spv_manager` is constructed even in RPC mode (`src/context/mod.rs:295`); identity rehydration now flows through the new persister in BOTH modes because identity persistence was always backend-agnostic | Medium, RPC-specific. RPC users were never on a persister-identity path; now they are. Correct by construction, but mandates a dedicated RPC-mode identity + DashPay E2E gate — do not infer from SPV passing |
| **3 Generic swap** | Correct — `IdentityManager` live; wallet rehydration still seed-driven (Gate G2) | Correct, same identity path as SPV; RPC payment flow provably unaffected (orthogonal to `WalletManager`) | Medium. RPC + Devnet identity discovery must route to the legacy fallback exactly as SPV + Devnet — branch on `network`, not `core_backend_mode` (`discover_identities.rs` is SDK-driven), so one branch serves both; verify no accidental mode-coupling |
| **4 Delete** | Correct if E.1 parity probe green (see [verification.md § E.1](verification.md#e1--dip-1415-dashpay-derivation-parity)) | Same risk profile — deleted code is backend-agnostic; sanity check runs mode-independently | Medium, mode-symmetric. RPC + Devnet + DashPay is the thinnest-tested corner — explicit QA lane required |

### The Mandatory RPC-Rehydration E2E Gate (Phase 2)

Phase 2 introduces one genuinely new RPC behavior: identity rehydration flowing through the upstream persister for the first time. This earns a dedicated gate, not a free pass off the SPV suite.

**Gate definition:** A backend-E2E test on testnet that, in RPC mode, loads a wallet with existing identities and established DashPay contacts through the new persister path and asserts:
1. Identity list matches pre-migration
2. Credit balances match pre-migration
3. Contact addresses match pre-migration

This test lives in `tests/backend-e2e/` and runs as part of the Phase-2 QA lane (see [migration-plan.md QA Matrix](migration-plan.md#qa-matrix)). It must pass before Phase 3 begins.
