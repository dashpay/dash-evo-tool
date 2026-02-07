# Dash Evo Tool - Task Backlog

> **Branch:** `ralph/improvements` (from `v1.0-dev`)
> **Sources:** GitHub issues (GH#), `issues/` directory files, `validated_issues/` directory files, direct code inspection
> **Convention:** `[META]` tasks produce sub-tasks only (no code changes). All other tasks produce code changes + commits.
> **Priority:** P0 = critical/crash, P1 = important bug, P2 = quality/refactor, P3 = nice-to-have

---

## Section 1: Bug Triage & Fixes [Week 1-2]

These META tasks validate reported bugs against the current codebase before any fixes are attempted.

- [x] **1.1 [META] Triage wallet bugs** (P0)
  Review the following against current code on `v1.0-dev`:
  - GH#522 (UTXOs not loaded correctly on DET start)
  - GH#476 (Advanced send: fee deducted from output despite selecting deduct from input)
  - GH#475 (Only 1 input considered when validating amount on send from platform addresses)
  - GH#478 (Identity top up "max" button not including all fees)
  - GH#485 (UTXOs counter always 0 for platform addresses)
  - GH#85 (Same funding address for multiple identities)
  - `issues/wallet-001-arithmetic-underflow-risk.md` through `issues/wallet-024-signature-length-overflow-risk.md`
  For each: (1) verify the bug still exists in code, (2) identify root cause if valid, (3) create specific fix tasks as new checkboxes in this section, (4) mark already-fixed issues. Update this file with findings.

  **Triage Results:**

  **GitHub Issues:**
  - **GH#522 — CONFIRMED.** UTXOs not loaded on startup. Root cause: `bootstrap_loaded_wallets()` in context.rs only bootstraps addresses, never calls `reload_utxos()`. The app relies on stale DB-cached UTXOs until user clicks Refresh. Fix: trigger automatic UTXO refresh on startup after wallet load.
  - **GH#476 — CONFIRMED.** Fee always deducted from output. Root cause: `fund_platform_address_from_wallet_utxos.rs:174` hardcodes `ReduceOutput(0)` fee strategy regardless of `fee_deduct_from_output` flag. The flag only affects asset lock amount calculation (lines 30-39), not the SDK fee strategy. Fix: conditionally use `DeductFromInput(0)` vs `ReduceOutput(0)` based on the flag.
  - **GH#475 — NOT CONFIRMED (already fixed).** The `allocate_platform_addresses_with_fee()` function in `send_screen.rs:134-234` correctly handles up to 16 platform address inputs with iterative fee estimation. Both simple and advanced modes properly collect multiple inputs.
  - **GH#478 — PARTIALLY CONFIRMED.** The "Platform Address" funding method for top-up correctly reserves estimated fees via `saturating_sub(estimated_fee)` in `by_platform_address.rs:104-105`. However, the "Wallet Balance" funding method in `mod.rs:374-381` sets max to `total_balance_duffs * 1000` (credits) with NO fee reservation — clicking Max and submitting will fail because fees aren't accounted for.
  - **GH#485 — ALREADY FIXED.** Platform addresses now show "N/A" in the UTXOs column (wallets_screen/mod.rs:970-975) since platform addresses don't hold Core UTXOs.
  - **GH#85 — CONFIRMED.** `receive_address()` called with `skip_known_addresses_with_no_funds=false` in 4 locations: `add_new_identity_screen/by_wallet_qr_code.rs:26`, `top_up_identity_screen/by_wallet_qr_code.rs:20`, `create_asset_lock_screen.rs:110`, `generate_receive_address.rs:38`. This causes reuse of zero-balance addresses across identity registrations.

  **Issue Files (wallet-001 through wallet-024):**
  - **wallet-001 (arithmetic underflow)** — FALSE POSITIVE. Subtraction at line ~143 is guarded by balance check at line 107.
  - **wallet-002 (total output mismatch)** — LOW PRIORITY, needs deeper analysis of edge case.
  - **wallet-003 (UTXO double-spend race)** — CONFIRMED but LOW RISK. `take_unspent_utxos_for` takes `&mut self` (write lock), but in `send_single_key_wallet_payment` the read→write gap between lines 52-207 could theoretically allow concurrent selection. In practice, UI serializes user actions.
  - **wallet-004 (inconsistent balance after broadcast failure)** — CONFIRMED. UTXOs removed from wallet after broadcast attempt even if broadcast fails.
  - **wallet-005 (missing balance rollback)** — LOW PRIORITY, relates to DB failure after in-memory update.
  - **wallet-006 (unwrap on height check)** — LOW PRIORITY, technically safe but fragile pattern.
  - **wallet-007 (lock poisoning)** — CONFIRMED, covered by task 2.5.
  - **wallet-008 (infinite loop on proof wait)** — CONFIRMED. `fund_platform_address_from_wallet_utxos.rs:139-148` loops indefinitely with no timeout waiting for asset lock proof.
  - **wallet-009 (fee estimation mismatch)** — FALSE POSITIVE. Fee is recalculated as UTXO count changes.
  - **wallet-010 (change output detection)** — LOW PRIORITY, fragile but works for current patterns.
  - **wallet-011 through wallet-014** — LOW PRIORITY edge cases.
  - **wallet-015 (silently ignored DB errors)** — CONFIRMED. Multiple `let _ =` patterns in send_single_key_wallet_payment.rs (lines 233, 238-240) and context.rs silently discard DB errors.
  - **wallet-016 through wallet-022** — LOW PRIORITY or FALSE POSITIVE after review.
  - **wallet-023 (Dash-Qt spawn panic)** — CONFIRMED. `start_dash_qt.rs:64` uses `.expect()` on spawn, will panic if binary not found.
  - **wallet-024 (signature length overflow)** — FALSE POSITIVE. DER signatures and pubkeys are always well under 255 bytes.

- [x] **1.1a Fix GH#522: Auto-refresh UTXOs on app startup** (P0)
  In `src/context.rs` `bootstrap_loaded_wallets()` (or equivalent startup path), trigger a background `reload_utxos()` call for each loaded wallet after initialization, so UTXOs reflect current Core state without manual Refresh.

- [x] **1.1b Fix GH#476: Hardcoded fee strategy in platform address funding** (P0)
  In `src/backend_task/wallet/fund_platform_address_from_wallet_utxos.rs:174`, replace the hardcoded `ReduceOutput(0)` with conditional logic: use `ReduceOutput(0)` when `fee_deduct_from_output` is true, use a non-reducing strategy when false (fees were already added to the asset lock amount at lines 30-39).

- [x] **1.1c Fix GH#478: Wallet balance top-up max button doesn't reserve fees** (P1)
  In `src/ui/identities/top_up_identity_screen/mod.rs:374-381`, the "UseWalletBalance" max amount calculation should subtract estimated fees (similar to how `by_platform_address.rs:104-105` does it). Currently sets max to raw `total_balance_duffs * 1000` with no fee buffer.

- [x] **1.1d Fix GH#85: Funding address reuse across identities** (P1)
  Change `receive_address()` calls from `skip_known_addresses_with_no_funds=false` to `true` in these 4 files:
  - `src/ui/identities/add_new_identity_screen/by_wallet_qr_code.rs:26`
  - `src/ui/identities/top_up_identity_screen/by_wallet_qr_code.rs:20`
  - `src/ui/wallets/create_asset_lock_screen.rs:110`
  - `src/backend_task/wallet/generate_receive_address.rs:38`

- [x] **1.1e Fix wallet-008: Add timeout to asset lock proof wait loop** (P1)
  In `src/backend_task/wallet/fund_platform_address_from_wallet_utxos.rs:139-148`, add a timeout (e.g., 5 minutes) to the proof-wait loop so the task doesn't hang indefinitely if the proof never arrives.

- [ ] **1.1f Fix wallet-023: Replace panic on Dash-Qt spawn failure** (P2)
  In `src/backend_task/core/start_dash_qt.rs:64`, replace `.expect("Failed to spawn dash-qt process")` with proper error propagation using `map_err`.

- [ ] **1.1g Fix wallet-015: Log silenced database errors in wallet operations** (P2)
  In `src/backend_task/core/send_single_key_wallet_payment.rs` lines 233, 238-240, replace `let _ =` with `if let Err(e) = ... { tracing::warn!(...) }` to log database errors instead of silently discarding them.

- [ ] **1.2 [META] Triage identity & token bugs** (P0)
  Review:
  - GH#499 (Identity Create screen improvements)
  - GH#224 (Token creator only sees one key)
  - GH#273 (Unclaimed reward estimate wrong)
  - GH#478 (Identity top up max button)
  - `issues/identity-*.md` files (identity-001 through identity-020+)
  - `issues/ui-tokens-*.md` files (ui-tokens-001 through ui-tokens-024)
  - `issues/ui-identity-*.md` files (ui-identity-001 through ui-identity-013)
  Same process: validate against code, root-cause, create fix tasks, note already-fixed.

- [ ] **1.3 [META] Triage core/config/infrastructure bugs** (P1)
  Review:
  - GH#522 (UTXO loading - overlaps with wallet triage, focus on core/config aspects)
  - GH#333 (Inconsistent connection status - note PR#532 may address this)
  - GH#98 (Wallet file not specified error if multiple Core wallets open)
  - GH#77 (ZMQ crash on Load Identity)
  - `issues/core-001-panic-on-db-init.md` through `issues/core-020-large-update-function.md`
  - `issues/context-001-unwrap-panics-in-new.md` through `issues/context-023-missing-dashpay-in-reinit.md`
  - `issues/infra-*.md` files
  Same process: validate, root-cause, create fix tasks.

- [ ] **1.4 [META] Triage UI/UX bugs** (P1)
  Review:
  - GH#482 (Warning message does not fit the screen)
  - GH#147 (Confusing Withdraw vs Transfer naming)
  - GH#170 (Title missing version and double folder)
  - `issues/ui-core-*.md` files (ui-core-001 through ui-core-014)
  - `issues/ui-contracts-*.md` files
  - `issues/ui-dpns-*.md` files
  Same process: validate, root-cause, create fix tasks.

---

## Section 2: Stability & Error Handling [Week 2-4]

- [ ] **2.1 [META] Audit all `panic!()` calls in production code** (P0)
  Run `grep -rn "panic!" src/` and examine every instance. For each:
  (1) determine if it's reachable in production, (2) assess severity, (3) create specific removal/replacement tasks.
  Known instances: `src/backend_task/identity/mod.rs` lines 167, 193 ("need a ECDSA Key for now").

- [ ] **2.2 [META] Audit `unwrap()`/`expect()` in `src/backend_task/`** (P1)
  Categorize every `unwrap()`/`expect()` call in the backend_task directory as:
  - **Safe**: value is guaranteed (e.g., regex compile of literal, `Some` just checked)
  - **Unsafe**: can actually panic in production
  Create fix tasks for all unsafe instances. Prioritize by crash likelihood.

- [ ] **2.3 [META] Audit `unwrap()`/`expect()` in `src/context.rs` and `src/database/`** (P1)
  Same categorization approach as 2.2. These are critical infrastructure files.
  Reference: `issues/context-001` through `context-023`, `issues/db-*.md`.

- [ ] **2.4 [META] Validate critical issue file claims** (P0)
  Read and verify these specific high-severity issue reports against actual code:
  - `issues/wallet-003-utxo-double-spend-race-condition.md`
  - `issues/wallet-008-infinite-loop-on-proof-wait.md`
  - `issues/core-016-config-file-truncate-danger.md`
  - `issues/context-014-lock-poisoning-cascade-risk.md`
  - `issues/wallet-001-arithmetic-underflow-risk.md`
  For each confirmed issue, create a specific fix task.

- [ ] **2.5 Design and implement lock poisoning recovery strategy** (P1)
  Currently the codebase uses `.lock().unwrap()` pervasively. Design a consistent approach:
  - Option A: Use `.lock().unwrap_or_else(|e| e.into_inner())` where safe
  - Option B: Create a helper that logs and recovers
  - Option C: Use parking_lot mutexes (no poisoning)
  Implement the chosen strategy in `src/context.rs` first as a template, then apply elsewhere.

- [ ] **2.6 Fix SystemTime expect panics** (P1)
  Replace `SystemTime::now().duration_since(UNIX_EPOCH).expect(...)` with `.unwrap_or_default()` across the codebase.
  Reference: `issues/core-010-unix-timestamp-unwrap.md`, `issues/ui-tokens-006-expect-on-time-operations.md`.

---

## Section 3: Code Structure Refactoring [Week 3-6]

- [ ] **3.1 [META] Review masternode_list_diff_screen.rs (4406 lines)** (P2)
  Note: PR#520 already refactors this. First review PR#520 (`gh pr view 520`, `gh pr diff 520`).
  Then identify remaining work after that PR: further split points, extracted components, shared utilities.
  Create sub-tasks for remaining refactoring only.

- [ ] **3.2 [META] Review wallets_screen/mod.rs (3813 lines)** (P2)
  Identify logical split points in this file. Look for:
  - Independent UI sections that could be separate files/modules
  - State that could be grouped into sub-structs
  - Helper functions that belong in utilities
  Create specific sub-tasks with line ranges and proposed module names.

- [ ] **3.3 [META] Review tokens_screen/mod.rs (3707 lines)** (P2)
  Same approach as 3.2. Token listing, creation, and configuration are likely separable concerns.
  Reference: `issues/ui-tokens-014-very-large-function.md`, `issues/ui-tokens-015-duplicate-control-rules-ui-code.md`.

- [ ] **3.4 [META] Review send_screen.rs (2744 lines) and single_key_send_screen.rs (1042 lines)** (P2)
  Identify shared code between these two files for extraction into common utilities.
  Focus on: fee estimation logic, address validation, recipient management, transaction building.
  Create sub-tasks for specific extractions.

- [ ] **3.5 [META] Review context.rs (1754 lines, 40+ fields)** (P2)
  Identify a module split strategy. Possible groupings:
  - Network/SDK configuration
  - Wallet management
  - Database access
  - UI state coordination
  Create sub-tasks with specific field groupings and proposed module boundaries.

- [ ] **3.6 [META] Review BackendTaskSuccessResult enum (60+ variants)** (P2)
  This enum in `src/backend_task/mod.rs` has grown unwieldy. Design a simplification:
  - Group related variants into sub-enums?
  - Use trait objects for result handling?
  - Other approach?
  Create implementation sub-tasks.

- [ ] **3.7 [META] Identify and catalog code duplication** (P3)
  Systematically identify duplicated code across the codebase. Key known areas:
  - Fee calculation (3+ implementations)
  - Send screen logic (2 files)
  - Error handling patterns
  - UI layout boilerplate
  Create deduplication sub-tasks ordered by impact.

---

## Section 4: UI/UX Improvements [Week 3-6]

- [ ] **4.1 [META] Triage UX feature requests from GitHub** (P2)
  Review and assess feasibility of:
  - GH#471 (Hide zero balances)
  - GH#473 (Display pending funds on wallet page)
  - GH#474 (Add identity to send sources)
  - GH#482 (Warning message overflow)
  - GH#333 (Connection status clarity - check if PR#532 addresses this)
  - GH#369 (Import Wallet suggestions)
  - GH#368 (Create Wallet suggestions)
  - GH#367 (Wallet UX & documentation issues)
  For each: validate relevance, assess effort, create implementation tasks for approved ones.

- [ ] **4.2 [META] Audit UI screens for component design pattern compliance** (P3)
  Reference: `doc/COMPONENT_DESIGN_PATTERN.md`. Check all screens in `src/ui/` for:
  - Public mutable fields (should be private)
  - Missing builder methods
  - Missing Response structs with ComponentResponse trait
  - Eager initialization (should be lazy)
  Create fix tasks for non-compliant components.

- [ ] **4.3 [META] Review error display patterns across all screens** (P2)
  Identify where raw error messages (including Rust debug output) are shown to users.
  Create tasks to add user-friendly error messages with optional "show details" expansion.

- [ ] **4.4 [META] Review input validation across all form screens** (P2)
  Check all input fields across the app for missing validation:
  - Amount inputs (overflow, negative, too many decimals)
  - Address inputs (format, network mismatch)
  - Name inputs (length limits, character restrictions)
  - Fee inputs
  Reference: `issues/ui-identity-009-profile-validation-inconsistency.md`, `issues/ui-identity-011-withdrawal-address-validation-timing.md`.
  Create fix tasks for missing validation.

---

## Section 5: Architecture Improvements [Week 5-8]

- [ ] **5.1 Design crate-level error type hierarchy** (P2)
  Currently errors are `String` throughout (`Result<T, String>`). Design a proper error hierarchy using `thiserror`:
  - Define error types per module (wallet, identity, network, database)
  - Map to user-friendly display messages
  - Preserve error chains for debugging
  Start with `src/backend_task/` as the first module to convert.

- [ ] **5.2 Replace deprecated serde_yaml dependency** (P2)
  `serde_yaml = "0.9.34-deprecated"` in Cargo.toml. Evaluate alternatives:
  - `serde_yml` (maintained fork)
  - Remove YAML support if not needed
  - Other serialization format
  Check what actually uses YAML in the codebase and make the minimal change.

- [ ] **5.3 [META] Evaluate workspace structure feasibility** (P3)
  Analyze the dependency graph between modules. Could the project benefit from a Cargo workspace with separate crates (e.g., `ui`, `backend`, `model`, `database`)?
  Estimate effort, identify circular dependencies that would block this, and create a migration plan if feasible.

- [ ] **5.4 [META] Review module boundaries and shared utility opportunities** (P3)
  Identify code that's currently scattered across modules but could be centralized:
  - Common UI widgets/helpers
  - Shared validation logic
  - Platform protocol helpers
  Create extraction tasks.

---

## Section 6: Testing & Quality [Throughout]

- [ ] **6.1 [META] Assess test coverage gaps** (P1)
  Run existing tests, identify what's covered vs. not. Focus on:
  - Backend task flows (identity, wallet, document operations)
  - Error paths
  - Edge cases in fee calculations
  - Database operations
  Create specific test-writing tasks ordered by risk.

- [ ] **6.2 Run clippy and fix all warnings** (P2)
  Run `cargo clippy --all-features --all-targets -- -D warnings` and fix everything.
  This may be a large task - if so, split by module.

- [ ] **6.3 Replace println!/eprintln! with tracing macros** (P3)
  Find all `println!` and `eprintln!` in `src/` and replace with appropriate `tracing::info!`, `tracing::warn!`, `tracing::error!`, etc.
  Reference: `issues/core-014-logging-panic-on-failure.md`.

- [ ] **6.4 [META] Review and triage all TODO/FIXME comments** (P2)
  Find all TODO/FIXME comments in the codebase (approximately 51). For each:
  - If it's still relevant: create a task
  - If it's stale or done: remove the comment
  - If it's a known limitation: document it
  Update this file with new tasks.

- [ ] **6.5 Add config save/load roundtrip tests** (P2)
  Write tests that verify configuration can be saved and loaded without data loss.
  Reference: `issues/core-012-config-save-file-not-synced.md`, `issues/core-016-config-file-truncate-danger.md`.

- [ ] **6.6 Add basic wallet payment flow tests** (P2)
  Write unit tests for the core wallet payment construction logic:
  - UTXO selection
  - Fee calculation
  - Change output generation
  - Amount validation

---

## Section 7: Feature Completion [Week 4-8]

- [ ] **7.1 [META] Triage feature requests** (P2)
  Review and assess:
  - GH#498 (Replace master key)
  - GH#497 (Disable keys)
  - GH#88 (Export private key from DET wallet)
  - GH#468 (Importing wallet from mobile Dashpay wallets)
  - GH#283 (Optional proof verification bypass mode)
  - GH#491 (Wrapper around dashpay.io contracts for Register Contract screen)
  For each: assess feasibility, complexity, and user impact. Create implementation tasks for approved features.

- [ ] **7.2 [META] Review DashPay subsystem completeness** (P2)
  Check `src/ui/dashpay/` for unfinished features. Known TODOs:
  - Cancel outgoing contact request
  - Resolve username from identity
  - Fetch display name from profile
  Reference: `issues/dashpay-*.md` files.
  Create tasks for completing or properly deferring each feature.

- [ ] **7.3 [META] Review SPV manager for production readiness** (P2)
  Note: PR#525 is active SPV work. Review current SPV code for:
  - Error handling and recovery
  - Timeout handling
  - Connection management
  Reference: `issues/wallet-013-spv-transaction-build-fee-calculation-loop.md`, `issues/wallet-016-spv-address-registration-error-ignored.md`.
  Create hardening tasks.

- [ ] **7.4 [META] Review token system for completeness** (P2)
  Check token-related screens and backend for:
  - GH#224 (Token creator key visibility)
  - Frozen identity filtering
  - Token transfer edge cases
  Reference: `issues/ui-tokens-*.md` files, `issues/contracts-*.md` files.
  Create completion tasks.

- [ ] **7.5 [META] Review database layer** (P3)
  Check `src/database/` for:
  - Missing indexes on frequently queried columns
  - Migration strategy (how are schema changes handled?)
  - Error handling (are DB errors properly surfaced?)
  Reference: `issues/db-*.md` files, `issues/context-017-database-execute-error-swallowed.md`.
  Create improvement tasks.

---

## Section 8: Security Hardening [Week 6-8]

- [ ] **8.1 [META] Security audit** (P1)
  Review these security-sensitive areas:
  - DashPay encryption implementation
  - Private key handling and zeroization (`issues/ui-core-012-password-field-zeroize-timing.md`)
  - SQL construction (any raw SQL that could be injectable?)
  - Credential storage
  - External data parsing (could malicious Platform data crash the app?)
  Create specific fix tasks for each finding.

- [ ] **8.2 Add HTTP timeout for all external fetches** (P1)
  Avatar loading and any other HTTP requests should have reasonable timeouts to prevent hangs.
  Reference: `issues/ui-identity-006-avatar-loading-memory-leak.md`.

---

## Section 9: Upstream PR Submission [When Ready]

> **Goal:** Cherry-pick completed work from `ralph/improvements` into clean branches off `v1.0-dev` and open draft PRs upstream. Limit to 5-10 PRs max. Prioritize changes that are important, easy to review, trivial, and merge cleanly.

- [ ] **9.1 [META] Review all changes on `ralph/improvements` and select PR candidates** (P1)
  Compare `ralph/improvements` against `v1.0-dev` (`git log --oneline v1.0-dev..ralph/improvements`).
  For each commit or logical group of commits, evaluate:
  1. **Importance:** Does it fix a real bug, improve stability, or add clear value?
  2. **Reviewability:** Is the diff small and self-contained? Can a reviewer understand it quickly?
  3. **Merge cleanliness:** Does it apply cleanly to `v1.0-dev` HEAD without conflicts?
  4. **Risk:** Could it introduce regressions? Lower risk = higher priority for PR.
  Select 5-10 candidates and create a numbered sub-task (9.2, 9.3, ...) for each one below.
  For each candidate, note: commit hash(es), summary, estimated diff size, and target PR title.

- [ ] **9.2–9.N PR submission tasks** *(created by 9.1)*
  Each sub-task follows this exact process:
  1. `git fetch origin && git checkout -b pr/<short-name> origin/v1.0-dev`
  2. `git cherry-pick <commit-hash>` (resolve conflicts if any; if conflicts are non-trivial, skip this PR and note why)
  3. **Review the diff carefully** before pushing:
     - `git diff origin/v1.0-dev..HEAD` — verify only intended changes are included
     - No task-management files (tasks.md, activity.md, prompt.md, ralph.sh) should be in the diff
     - No unrelated changes leaked in
     - Code compiles (`cargo build 2>&1 | tail -5`)
     - Clippy passes (`cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -5`)
  4. `git push -u origin pr/<short-name>`
  5. Create draft PR:
     ```
     gh pr create --draft --base v1.0-dev \
       --title "<concise title>" \
       --body "$(cat <<'EOF'
     ## Summary
     <1-3 bullet points describing the change>

     ## Review Notes
     - Cherry-picked from branch `ralph/improvements` (commit `<hash>`)
     - This PR was created via an automated process by Claude Code
     - Please review carefully before merging

     ## Test Plan
     <How to verify this change>

     🤖 Generated with [Claude Code](https://claude.com/claude-code)
     EOF
     )"
     ```
  6. Record the PR URL in this file next to the task checkbox.

---

## Progress Tracking

**Total tasks:** 49 (24 META + 25 direct)
**Note:** META tasks will expand this list significantly as they produce sub-tasks.

| Section | Tasks | Completed |
|---------|-------|-----------|
| 1. Bug Triage | 11 | 6 |
| 2. Stability | 6 | 0 |
| 3. Refactoring | 7 | 0 |
| 4. UI/UX | 4 | 0 |
| 5. Architecture | 4 | 0 |
| 6. Testing | 6 | 0 |
| 7. Features | 5 | 0 |
| 8. Security | 2 | 0 |
| 9. Upstream PRs | 2+ | 0 |
