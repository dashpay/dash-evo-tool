# Phase 7.5 E2E Coverage Completeness Audit

**Run:** 159
**Date:** 2026-02-10
**Task:** 7.5.4

## Summary

**Grade: B+** — Strong coverage across routes, stores, and journey tests. 17 E2E integration test failures need fixing. 54 IPC commands lack functional test coverage (mostly in unimplemented DashPay/Tools screens, which is expected). All 7 Zustand stores have 100% action coverage.

## Test Inventory

| Layer | Files | Tests | Status |
|---|---|---|---|
| Component (Vitest) | 115 | 3,710 | All pass |
| E2E Integration (Playwright) | 10 | 365 | 348 pass, 17 fail |
| Full E2E (WebdriverIO) | 7 | 61 | Cannot run on macOS (requires Linux WebKit2GTK) |
| Journey tests | 1 | 19 | All pass |
| **Total** | **133** | **4,155** | **4,077 pass, 17 fail** |

## Route Coverage

**68 total routes defined in `routes.tsx`.**

All 68 routes have at least one E2E integration test that navigates to them and verifies basic rendering. Test coverage by phase:

| Phase | Routes | Tests | Coverage |
|---|---|---|---|
| Phase 2 (Shell) | 3 | 25 | Full |
| Phase 3 (Wallets) | 8 | 46 | Full |
| Phase 4 (Identities) | 1 | 39 | Full |
| Phase 5 (DPNS) | 5 | 37 | Full |
| Phase 6 (Contracts) | 12 | 55 | Full |
| Phase 7 (Tokens) | 18 | 46 | Full (17 tests failing) |
| Phase 8 (DashPay) | 5 | 9 | Placeholder routes only |
| Phase 9 (Tools) | 10 | 44 | Full (2 tests failing) |
| Settings | 1 | included in Phase 2 | Full |

## IPC Command Coverage

**165 total IPC commands in `bindings.ts`.**

- **127 commands (77%)** have functional test coverage (invoked/asserted in store tests, component tests, or E2E integration tests)
- **38 commands (23%)** have only default mock coverage (registered in mock-ipc.ts but not functionally tested)

### Untested IPC Commands by Category

**Expected gaps (screens not yet implemented):**
- Masternode list (5): `mnlistFetchChainLocks`, `mnlistFetchDiff`, `mnlistFetchDiffsChain`, `mnlistFetchQrInfo`, `mnlistFetchQrInfoWithDmls`
- GroveSTARK (2): `grovestarkGenerateProof`, `grovestarkVerifyProof`
- Proof/transition parsing (pending): `broadcastStateTransition`
- DashPay advanced (3): `dashpayDbSaveAvatarBytes`, `dashpayRegisterAddresses`, `dashpaySendContactRequestWithProof`

**Gaps worth addressing:**
- Identity operations (6): `identityListSummaries`, `identitySearchByDpnsName`, `identitySearchFromWallet`, `identitySearchUpToIndex`, `identitySignMessage`, `identityTopUpFromPlatformAddresses`
- Wallet platform ops (5): `walletBootstrapAddresses`, `walletClearSpvData`, `walletFetchPlatformAddressBalances`, `walletFundPlatformFromAssetLock`, `walletStartSpv`
- Core ops (3): `coreGetBestChainLock`, `coreGetBestChainLocks`, `coreRecoverAssetLocks`
- Settings (4): `settingsUpdateAutoStartSpv`, `settingsUpdatePassword`, `settingsUpdateShowEvonodeTools`, `settingsUpdateUserMode`
- Context (3): `contextGetFeeMultiplier`, `contextSetCoreBackendMode`, `contextSetFeeMultiplier`
- Misc (2): `contractGetByTokenId`, `tokenQueryIdentityBalance`

## Zustand Store Coverage

**7 stores, 113 actions — 100% tested.**

| Store | Actions | Tests |
|---|---|---|
| walletStore | 16 | 43 |
| identityStore | 12 | 45 |
| contestStore | 18 | 63 |
| contractStore | 10 | 31 |
| documentStore | 17 | 40 |
| tokenStore | 12 | 36 |
| dashpayStore | 28 | 66 |

## Multi-Screen Journey Coverage

**5 journeys covering the 5 most common user workflows — all passing.**

1. **New User Onboarding** (3 tests): Welcome → Create Wallet → Create Identity → Register DPNS Name
2. **Token Creation & Transfer** (3 tests): Token Creator (Simple + Advanced) → My Tokens → Transfer
3. **Contract & Document Workflow** (4 tests): Add Contract → Expand Tree → Create Document → Delete Document
4. **Wallet Operations** (4 tests): Receive (QR) → Send → Asset Locks → Import Wallet
5. **Identity Management** (5 tests): View Detail → View Keys → Top Up → Withdraw → Transfer → Cross-section nav

## Failing Tests

### 17 E2E Integration Failures

**Token screens (15 failures):** All token action screen tests checking "Back to Tokens" button are failing. Likely a UI regression in navigation/button rendering on token operation screens.

**Tools landing page (2 failures):** "renders all 9 tool cards" and "renders tool descriptions on cards" fail with `waitForInit` timeout in mock IPC setup (`fixtures.ts:44`). Likely a mock initialization race condition.

## Recommendations

### Fix Tasks (P1-P2)

1. **Fix 15 token E2E failures** — investigate "Back to Tokens" button rendering issue in token operation screens
2. **Fix 2 tools E2E failures** — fix mock IPC `waitForInit` timeout for tools landing page
3. **Add IPC assertion tests for 14 commonly-used commands** — identity search/sign, wallet platform ops, core chain lock ops

### Non-Blocking Observations

- DashPay screens are placeholder-only (Phase 8 in progress), so DashPay IPC gaps are expected
- Tools screens pending implementation (9.1f-9.1n) account for masternode/grovestark/proof IPC gaps
- Full E2E tests (Layer 3) can only be verified on Linux — CI job is configured but untested
- The mock-ipc infrastructure successfully covers all 165 commands with default handlers
