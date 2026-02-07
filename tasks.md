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

- [x] **1.1f Fix wallet-023: Replace panic on Dash-Qt spawn failure** (P2)
  In `src/backend_task/core/start_dash_qt.rs:64`, replace `.expect("Failed to spawn dash-qt process")` with proper error propagation using `map_err`.

- [x] **1.1g Fix wallet-015: Log silenced database errors in wallet operations** (P2)
  In `src/backend_task/core/send_single_key_wallet_payment.rs` lines 233, 238-240, replace `let _ =` with `if let Err(e) = ... { tracing::warn!(...) }` to log database errors instead of silently discarding them.

- [x] **1.2 [META] Triage identity & token bugs** (P0)
  Review:
  - GH#499 (Identity Create screen improvements)
  - GH#224 (Token creator only sees one key)
  - GH#273 (Unclaimed reward estimate wrong)
  - GH#478 (Identity top up max button)
  - `issues/identity-*.md` files (identity-001 through identity-020+)
  - `issues/ui-tokens-*.md` files (ui-tokens-001 through ui-tokens-024)
  - `issues/ui-identity-*.md` files (ui-identity-001 through ui-identity-013)
  Same process: validate against code, root-cause, create fix tasks, note already-fixed.

  **Triage Results:**

  **GitHub Issues:**
  - **GH#499 — PARTIALLY CONFIRMED.** (a) ContractBounds for keys in Identity Create: already implemented — default keys include ContractBounds, and the pipeline supports them. (b) Security level validation per key purpose: TRANSFER keys are correctly locked to CRITICAL, but ENCRYPTION/DECRYPTION keys lack UI enforcement of the MEDIUM requirement when manually adding keys. Fix: add validation in add_key flow.
  - **GH#224 — FALSE POSITIVE (by design).** Token creator simple mode auto-selects the first eligible key via `.find()`. Advanced mode properly shows all keys matching transaction requirements via `add_identity_key_chooser`. The "only sees one key" is a UX limitation in simple mode, not a bug. Users can switch to advanced mode.
  - **GH#273 — CANNOT CONFIRM.** Reward estimation logic delegates to SDK methods (`cycle_start()`, `current_interval()`, `max_cycle_moment()`). The DET code itself handles ranges correctly with `RangeInclusive`. The off-by-one may be in the SDK or in block timing. Cannot reproduce without test environment. Deferring.
  - **GH#478 — ALREADY FIXED by task 1.1c.** Wallet balance top-up max button now reserves estimated fees.

  **Issue Files (identity-001 through identity-014):**
  - **identity-001 (panic on unsupported key type)** — CONFIRMED. Two `panic!("need a ECDSA Key for now")` calls at `src/backend_task/identity/mod.rs:167,193` in `to_public_keys_map()`. Triggers on any KeyType other than ECDSA_SECP256K1 or ECDSA_HASH160 (e.g., BLS12_381, EDDSA_25519_HASH160). Fix: return error instead of panic.
  - **identity-002 (unwrap on try_into)** — CONFIRMED. `src/backend_task/identity/mod.rs:317` has `.try_into().unwrap()` on hex decode result. While input is length-checked at 64 chars, the unwrap is fragile. Fix: use `try_into().map_err(...)`.
  - **identity-003 (unchecked max unwrap)** — FALSE POSITIVE. Code uses `unwrap_or()` safely.
  - **identity-004 (expect on lock poisoning)** — CONFIRMED but deferred to task 2.5 (lock poisoning strategy). Multiple `.expect()` on `.lock()` in register_identity.rs and top_up_identity.rs.
  - **identity-005 (UTXO removal timing)** — CONFIRMED but LOW PRIORITY. UTXOs removed before proof confirmation; in practice the proof almost always arrives.
  - **identity-006 (refresh returns wrong identity)** — FALSE POSITIVE. Return value is a completion signal; actual identity is correctly updated in DB and memory before return.
  - **identity-007 (silently ignored wallet update errors)** — CONFIRMED. `let _ =` patterns in register_identity.rs:192,333 and top_up_identity.rs:206,319. Same pattern as wallet-015 (already fixed). Fix: add tracing::warn.
  - **identity-008 (inconsistent wallet update patterns)** — LOW PRIORITY. Inconsistent but functional code in mod.rs:612-680.
  - **identity-009 (unwrap on label to_str)** — FALSE POSITIVE. All uses have `.unwrap_or_default()` or `.ok()` fallbacks.
  - **identity-010 (duplicate DPNS fetch code)** — LOW PRIORITY. Code duplication across 6 files, belongs in refactoring (Section 3).
  - **identity-011 (unused wallet clone)** — LOW PRIORITY. Minor inefficiency.
  - **identity-012 (potential deadlock in transfer)** — FALSE POSITIVE. Operations use owned Vec, not shared locks.
  - **identity-013 (wallet clone in top-up)** — LOW PRIORITY. Minor inefficiency.
  - **identity-014 (missing wallet association in DPNS load)** — LOW PRIORITY. Edge case in data preservation.

  **Issue Files (ui-tokens-001 through ui-tokens-024):**
  - **ui-tokens-001 through ui-tokens-004** — LOW PRIORITY. Fragile `is_err()`/`is_some()` + `unwrap()` patterns. Technically safe due to prior checks but vulnerable to refactoring. Covered by task 2.2 audit.
  - **ui-tokens-005 (mutex lock unwrap)** — CONFIRMED but deferred to task 2.5 (lock poisoning strategy).
  - **ui-tokens-006 (expect on SystemTime)** — CONFIRMED but deferred to task 2.6 (SystemTime expects).
  - **ui-tokens-007 (silent parse errors)** — LOW PRIORITY. Silent `unwrap_or(0)` in build_distribution_rules is arguably acceptable for optional fields.
  - **ui-tokens-008 (expect on embedded images)** — CONFIRMED. Two `.expect()` calls in `load_formula_image()` at `tokens_screen/mod.rs:84-86`. These are on compile-time embedded images so LOW RISK, but still shouldn't panic.
  - **ui-tokens-009 (unwrap in UI style mutation)** — LOW PRIORITY. TextStyle::Body always exists in egui.
  - **ui-tokens-010 (expect on signing key)** — CONFIRMED. `.expect("No key selected")` in 8+ token action screens. Users could trigger this if no key is selected. Fix: validate key selection before submit, return error instead of panic.
  - **ui-tokens-011 through ui-tokens-013** — LOW PRIORITY. Unwrap chains that are mostly guarded by prior state.
  - **ui-tokens-014 (very large function)** — CONFIRMED but deferred to task 3.3 (tokens_screen refactoring).
  - **ui-tokens-015 (duplicate control rules UI)** — CONFIRMED but deferred to task 3.3. ~250 lines of duplicate code.
  - **ui-tokens-021 (commented-out reorder assignment)** — CONFIRMED. `reorder_vec_to()` in `tokens_screen/mod.rs:1799` builds reordered map but assignment `self.my_tokens = reordered` is commented out. Token reordering is completely non-functional.
  - **ui-tokens-022 (inconsistent field check logic)** — CONFIRMED. `build_distribution_rules()` in `tokens_screen/mod.rs` checks `step_decreasing_start_period_offset_input.is_empty()` for min_value and max_interval_count fields instead of their own inputs. Logic error causing incorrect distribution rule construction.
  - **ui-tokens-023 (TODO filtering not implemented)** — CONFIRMED. destroy_frozen_funds_screen and unfreeze_tokens_screen show all identities instead of filtering to only frozen ones. UX issue.
  - **ui-tokens-024 (test unwraps in production code)** — LOW PRIORITY. Test code mixed into production file.

  **Issue Files (ui-identity-001 through ui-identity-013):**
  - **ui-identity-001 (unwrap on identity refresh)** — CONFIRMED. `transfer_screen.rs:504-511` and `withdraw_screen.rs:320-329` both use `.unwrap()` after `.find()` to locate identity by ID. If identity was deleted during refresh, this panics. Fix: handle None case gracefully.
  - **ui-identity-002 (mutex poison handling)** — CONFIRMED but deferred to task 2.5.
  - **ui-identity-003 (missing wallet validation)** — FALSE POSITIVE. The `.expect()` on `ensure_correct_identity_keys()` is intentional error handling.
  - **ui-identity-004 (wallet unlock state not reset)** — LOW PRIORITY. UX annoyance, not a crash bug.
  - **ui-identity-005 (identity deletion errors ignored)** — CONFIRMED. `.ok()` in `identities_screen.rs:936-952` silently discards DB deletion errors. Fix: log errors via tracing::warn.
  - **ui-identity-006 (avatar cache unbounded)** — PARTIALLY CONFIRMED. contacts_list.rs correctly clears cache on identity switch, but profile_screen.rs intentionally preserves cache across changes without eviction policy. Could grow unbounded with many avatars.
  - **ui-identity-007 (avatar loading no cancellation)** — LOW PRIORITY. Tokio tasks for avatar loading have no cancellation, but they're short-lived.
  - **ui-identity-008 (database save errors ignored)** — CONFIRMED. Same pattern as ui-identity-005. Silent `.ok()` discards DB save errors in contacts_list.rs.
  - **ui-identity-009 (profile validation inconsistency)** — LOW PRIORITY. Bio length limit mismatch (250 vs 140) — cosmetic.
  - **ui-identity-010 (assume_checked address parsing)** — CONFIRMED but JUSTIFIED. Comment in transfer_screen.rs:271-281 explains DIP-18 requires `assume_checked()` because platform addresses share version bytes across testnet/devnet/regtest. Not a bug.
  - **ui-identity-011 (withdrawal address validation timing)** — LOW PRIORITY. UX flaw where validation occurs after confirmation dialog.
  - **ui-identity-012 (self-contact filtering)** — LOW PRIORITY. Edge case in one code path.
  - **ui-identity-013 (error message duplication)** — LOW PRIORITY. Cosmetic issue.

- [x] **1.2a Fix identity-001: Replace panic on unsupported key types** (P0)
  In `src/backend_task/identity/mod.rs:167,193`, replace `panic!("need a ECDSA Key for now")` with proper error return (e.g., `return Err(format!("Unsupported key type: {:?}", key_type))`). Two locations in `to_public_keys_map()`.

- [x] **1.2b Fix ui-tokens-021: Uncomment reorder assignment** (P1)
  In `src/ui/tokens/tokens_screen/mod.rs`, the `reorder_vec_to()` function builds a reordered map but the assignment `self.my_tokens = reordered` is commented out (~line 1799). Uncomment or properly implement the assignment so token reordering works.

- [x] **1.2c Fix ui-tokens-022: Wrong field checks in build_distribution_rules** (P1)
  In `src/ui/tokens/tokens_screen/mod.rs` `build_distribution_rules()`, lines ~2044 and ~2058 check `step_decreasing_start_period_offset_input.is_empty()` for min_value and max_interval_count. Each should check its own corresponding input field instead.

- [x] **1.2d Fix ui-tokens-010: Replace expect on signing key in token screens** (P1)
  Replace `.expect("No key selected")` with proper validation/error handling in 8+ token action screens: transfer_tokens_screen.rs, freeze_tokens_screen.rs, unfreeze_tokens_screen.rs, mint_tokens_screen.rs, destroy_frozen_funds_screen.rs, claim_tokens_screen.rs, pause_tokens_screen.rs, resume_tokens_screen.rs.

- [x] **1.2e Fix ui-identity-001: Handle deleted identity in transfer/withdraw refresh** (P1)
  In `src/ui/identities/transfer_screen.rs:504-511` and `src/ui/identities/withdraw_screen.rs:320-329`, replace `.unwrap()` after `.find()` with graceful handling (e.g., show error message if identity not found instead of panicking).

- [x] **1.2f Fix identity-007: Log silenced wallet update errors in identity registration** (P2)
  In `src/backend_task/identity/register_identity.rs:192,333` and `src/backend_task/identity/top_up_identity.rs:206,319`, replace `let _ =` with `if let Err(e)` + `tracing::warn!` (same pattern as task 1.1g).

- [x] **1.2g Fix ui-identity-005/008: Log silenced DB errors in identity deletion and contact save** (P2)
  Replace `.ok()` with logged error handling in `src/ui/identities/identities_screen.rs:936-952` and `src/ui/dashpay/contacts_list.rs` DB save operations.

- [x] **1.2h Fix GH#499b: Add security level validation for ENCRYPTION/DECRYPTION keys** (P2)
  In the Identity Create add_key flow, enforce that ENCRYPTION and DECRYPTION key purposes use SecurityLevel::MEDIUM (as required by Platform). Currently only TRANSFER keys are locked to CRITICAL. Add UI validation or auto-lock for enc/dec keys.

- [x] **1.3 [META] Triage core/config/infrastructure bugs** (P1)
  Review:
  - GH#522 (UTXO loading - overlaps with wallet triage, focus on core/config aspects)
  - GH#333 (Inconsistent connection status - note PR#532 may address this)
  - GH#98 (Wallet file not specified error if multiple Core wallets open)
  - GH#77 (ZMQ crash on Load Identity)
  - `issues/core-001-panic-on-db-init.md` through `issues/core-020-large-update-function.md`
  - `issues/context-001-unwrap-panics-in-new.md` through `issues/context-023-missing-dashpay-in-reinit.md`
  - `issues/infra-*.md` files
  Same process: validate, root-cause, create fix tasks.

  **Triage Results:**

  **GitHub Issues:**
  - **GH#522 — ALREADY FIXED by task 1.1a.** Auto-refresh UTXOs on startup added.
  - **GH#333 — ADDRESSED BY PR#532.** PR#532 ("fix: connection status not clear") centralizes connection status monitoring with dynamic tooltips and backend-mode awareness. No additional DET work needed; defer to that PR.
  - **GH#98 — CONFIRMED.** Core RPC client is created in context.rs (line ~171) and context_provider.rs (line ~49) without any wallet specification (`rpcwallet` parameter). When Core has multiple wallets open, all RPC calls fail with "Wallet file not specified". No `listwallets` check, no wallet selection UI, no helpful error message. Fix: detect multi-wallet scenario and either auto-select or prompt user.
  - **GH#77 — LIKELY STALE.** Original crash (2023) was "illegal hardware instruction" (SIGILL), likely CPU incompatibility or old deserialization bug. Current ZMQ listener code handles all deserialization errors gracefully with error logging. Load Identity flow doesn't directly interact with ZMQ. CPU compatibility check added in cpu_compatibility.rs. Remaining risk: `.expect()` calls on ZMQ socket setup (covered by infra-002). Not creating a specific fix task; existing expect() calls covered by task 2.2/infra-002 findings.

  **Issue Files (core-001 through core-020):**
  - **core-001 (panic on DB init)** — CONFIRMED. `app.rs:170-172` has `expect()` on file path creation, `unwrap()` on Database::new and initialize. Fix: propagate errors to show user-friendly startup failure message.
  - **core-002 (RwLock poison panic)** — CONFIRMED but deferred to task 2.5 (lock poisoning strategy).
  - **core-003 (Mutex unwrap panic)** — CONFIRMED but deferred to task 2.5.
  - **core-004 (runtime creation panic)** — LOW PRIORITY. Tokio runtime creation only fails in extreme circumstances (resource exhaustion). The `expect()` is standard practice.
  - **core-005 (config expect on addresses)** — CONFIRMED. `config.rs:300,306` uses `expect()` on parsing DAPI addresses and Insight API URL. User-edited config with invalid values will panic at runtime. Fix: return Result instead of panicking.
  - **core-006 (ZMQ listener panic)** — CONFIRMED. `app.rs:413,440,467,494` uses `expect()` on CoreZMQListener::spawn_listener() for all 4 networks. If ZMQ endpoint is unreachable or port is in use, the entire app panics. Fix: handle errors gracefully and show connection error in UI.
  - **core-007 (network context panic)** — CONFIRMED. `app.rs:679-681` uses `expect()` on network context access. Panics if user switches to an unconfigured network. Fix: return Option or show error instead of panic.
  - **core-008 (icon load panic)** — LOW RISK. Embedded compile-time image, extremely unlikely to fail.
  - **core-009 (screen unwrap panic)** — CONFIRMED but LOW PRIORITY. BTreeMap screen access assumes initialization completed; would only fail if init logic changes.
  - **core-010 (SystemTime unwrap)** — CONFIRMED, deferred to task 2.6.
  - **core-011 (config load poor error)** — LOW PRIORITY. Confusing conditional logic but functional.
  - **core-012 (config save no sync)** — LOW PRIORITY. Missing fsync could lose data on crash, but this is a common pattern and low probability.
  - **core-013 (duplicate screen init)** — CONFIRMED but deferred to task 3.5 (context.rs refactoring) / code duplication.
  - **core-014 (logging panic on failure)** — CONFIRMED. `logging.rs:17-26` panics if log file creation fails. Fix: fall back to stderr logging.
  - **core-015 (CPU check dialog unwrap)** — LOW PRIORITY. Dialog `.show()` unwrap only panics in headless environments, which aren't target platforms.
  - **core-016 (config truncation danger)** — CONFIRMED. `config.rs:71-72` uses `File::create()` which truncates before writing. A partial write failure leaves corrupt config. Fix: write to temp file, then rename.
  - **core-017 (bundled write race)** — LOW PRIORITY. TOCTOU race on bundled resource files is unlikely in practice.
  - **core-018 (app dir filename validation)** — LOW PRIORITY. Only accepts filenames from internal code, not user input.
  - **core-019 (dead code todo/unimplemented)** — CONFIRMED. `app_dir.rs:61` and `app.rs:682` have `unimplemented!()` and `todo!()` macros. Fix: replace with proper error handling for unknown networks.
  - **core-020 (large update function)** — CONFIRMED but deferred to Section 3 refactoring.

  **Issue Files (context-001 through context-023):**
  - **context-001 (unwrap panics in new)** — CONFIRMED but deferred to task 2.3 (context.rs/database audit).
  - **context-002 (cookie path panic)** — CONFIRMED, part of context-001 scope.
  - **context-003 through context-007 (lock unwraps)** — CONFIRMED but deferred to task 2.5 (lock poisoning strategy).
  - **context-008 (cookie parsing unchecked indexing)** — CONFIRMED. `context_provider.rs:38-39` indexes cookie_parts[0] and [1] without bounds check. Malformed cookie file panics. Fix: validate split result.
  - **context-009 (missing DashPay contract check in SPV)** — LOW PRIORITY. Inconsistent but functional.
  - **context-010 (cookie newline issue)** — CONFIRMED. Cookie read via `read_to_string` with no `.trim()`, trailing newline gets included in password causing auth failures. Fix: trim the cookie string.
  - **context-011 (dead code)** — LOW PRIORITY. Unused function, possible future feature.
  - **context-012 (hardcoded platform version)** — LOW PRIORITY. TODO to use sdk.version() but not actionable without SDK support.
  - **context-013 (unreachable network panic)** — CONFIRMED, same as core-019.
  - **context-014 through context-015 (lock poisoning/race)** — Deferred to task 2.5.
  - **context-016 (unused password_info)** — LOW PRIORITY. Dead code with #[allow(dead_code)].
  - **context-017 (DB errors swallowed in reconcile)** — FALSE POSITIVE. Verified code actually logs errors with tracing::warn and propagates with ?.
  - **context-018 (very long reconcile function)** — CONFIRMED but deferred to Section 3 refactoring.
  - **context-019 (clone poisoned lock fallback)** — Deferred to task 2.5.
  - **context-020 (SDK builder expect)** — CONFIRMED. `sdk_wrapper.rs:31` has `.expect("Failed to build SDK")`. Fix: propagate error.
  - **context-021 (wallet classification incomplete)** — LOW PRIORITY. Functional for current use cases.
  - **context-022 (UTXO insert unchecked)** — LOW PRIORITY. All-or-nothing on error is acceptable behavior.
  - **context-023 (missing dashpay in reinit)** — LOW PRIORITY. Reinit doesn't reload system contracts but this rarely causes issues.

  **Issue Files (infra-001 through infra-028):**
  - **infra-001 (P2P panic on network)** — CONFIRMED. `core_p2p_handler.rs:54` panics on unsupported network. Same pattern as core-019. Fix: return error.
  - **infra-002 (ZMQ listener expect panics)** — CONFIRMED. Multiple `expect()` calls in ZMQ background thread. Covered by core-006 fix for setup; runtime expects need separate handling.
  - **infra-003 (SPV manager expect panic)** — LOW PRIORITY. Tokio runtime creation expect is standard practice.
  - **infra-004/005 (ZMQ Windows panics)** — LOW PRIORITY. Windows-specific code, and platform targets are limited.
  - **infra-006 (SPV wallet load busy wait)** — LOW PRIORITY. Inefficient but functional.
  - **infra-007 (task spawn resource leak)** — LOW PRIORITY. Documented known limitation.
  - **infra-008 (SPV lock error swallowing)** — Deferred to task 2.5.
  - **infra-009 (P2P header sync no limit)** — LOW PRIORITY. 1KB scan is bounded.
  - **infra-010 (unbounded channel)** — LOW PRIORITY. Unlikely to exhaust memory in practice.
  - **infra-011 (timeout not restored)** — LOW PRIORITY. Non-critical socket timeout.
  - **infra-012 (platform_info.rs expect panics)** — CONFIRMED. 29+ `expect()` calls on document property accessors. Malformed Platform data will crash the app. Fix: replace with proper error handling.
  - **infra-013/014 (println not tracing)** — CONFIRMED, deferred to task 6.3.
  - **infra-015 (SPV storage lock)** — LOW PRIORITY. Unlikely failure mode.
  - **infra-016 (quorum lookup no timeout)** — LOW PRIORITY. SPV-specific edge case.
  - **infra-017 (backend task lock unwraps)** — Deferred to task 2.5.
  - **infra-018 (test unwraps)** — LOW PRIORITY. Test code.
  - **infra-019/020 (DashPay unwraps)** — LOW PRIORITY. Cryptographic operations with well-formed inputs.
  - **infra-021 (asset lock unwrap)** — FALSE POSITIVE. Lines 70,73 don't have unwraps on confirmations.
  - **infra-022 (P2P slice unwrap)** — LOW PRIORITY. Slice sizes are correct in practice.
  - **infra-023 (mnlist lock unwrap)** — Deferred to task 2.5.
  - **infra-024 (mnlist unbounded loop)** — LOW PRIORITY. Range calculation is bounded by blockchain height.
  - **infra-025 (mnlist connection reuse)** — LOW PRIORITY. Single-use P2P connection is acceptable.
  - **infra-026/027 (task manager shutdown)** — LOW PRIORITY. Shutdown edge cases.
  - **infra-028 (SPV stop race)** — LOW PRIORITY. Unlikely to cause issues.

- [x] **1.3a Fix core-005: Replace expect() on config address parsing** (P1)
  In `src/config.rs:300,306`, replace `expect()` calls in `dapi_address_list()` and `insight_api_uri()` with `Result` returns, so invalid user-edited config values produce error messages instead of panics.

- [x] **1.3b Fix core-006: Replace expect() on ZMQ listener creation** (P1)
  In `src/app.rs:413,440,467,494`, replace `expect()` on `CoreZMQListener::spawn_listener()` with error handling that logs the failure and continues without ZMQ (degraded mode) instead of crashing the app. All four network listeners.

- [x] **1.3c Fix core-019/context-013/infra-001: Replace unimplemented!/todo! macros** (P1)
  Replace panic-inducing macros with proper error handling:
  - `src/app_dir.rs:61` — `unimplemented!()` for unknown network
  - `src/app.rs:682` — `todo!()` for unknown network in current_app_context
  - `src/components/core_p2p_handler.rs:54` — panic on unsupported network

- [x] **1.3d Fix context-008/context-010: Cookie parsing safety** (P1)
  In `src/context_provider.rs:34-40`:
  (1) Trim the cookie string after reading to remove trailing newlines (context-010)
  (2) Check that split(':') produces exactly 2 parts before indexing (context-008)
  Return an error instead of panicking on malformed cookie files.

- [x] **1.3e Fix core-016: Safe config save with atomic write** (P2)
  In `src/config.rs` `save()` method, write to a temporary file first, then rename/move to the target path. This prevents config corruption if a write fails partway through (currently `File::create()` truncates immediately).

- [x] **1.3f Fix core-014: Logging initialization should not panic** (P2)
  In `src/logging.rs:17-26`, replace `panic!`/`expect()` on log file creation and EnvFilter with fallback to stderr logging, so the app can still run even if log file setup fails.

- [x] **1.3g Fix core-001: Replace unwrap/expect on database initialization** (P2)
  In `src/app.rs:170-172`, replace `expect()` and `unwrap()` on database file path creation and initialization with proper error handling that shows a user-friendly error dialog instead of panicking.

- [x] **1.3h Fix infra-012: Replace expect() calls on document property access in platform_info.rs** (P2)
  In `src/backend_task/platform_info.rs`, replace 29+ `expect()` calls on document property accessors with proper error handling. Platform data may be malformed or have schema changes; these should produce errors, not panics.

- [x] **1.4 [META] Triage UI/UX bugs** (P1)
  Review:
  - GH#482 (Warning message does not fit the screen)
  - GH#147 (Confusing Withdraw vs Transfer naming)
  - GH#170 (Title missing version and double folder)
  - `issues/ui-core-*.md` files (ui-core-001 through ui-core-014)
  - `issues/ui-contracts-*.md` files
  - `issues/ui-dpns-*.md` files
  Same process: validate, root-cause, create fix tasks.

  **Triage Results:**

  **GitHub Issues:**
  - **GH#482 — CONFIRMED.** Warning/error messages overflow horizontally on smaller screens. Root cause: inconsistent use of text wrapping. The main wallets screen message display (`wallets_screen/mod.rs:3237-3266`) uses `.wrap()` correctly, but many other error display locations use `ui.colored_label()` or `ui.label()` without wrapping: `wallets_screen/mod.rs:3541` (SK unlock error), `import_mnemonic_screen.rs:479,609`, `add_new_wallet_screen.rs:901`, `send_screen.rs:1066`, `create_asset_lock_screen.rs:618`, `single_key_send_screen.rs:381,805`. Fix: add `.wrap()` to all error/warning label displays.
  - **GH#147 — CONFIRMED (UX issue).** The Withdraw vs Transfer naming is confusing to users. Problem 1: Withdrawal payout address shown only after pressing Withdraw button. Problem 2: Transfer from identity balance vs Transfer from wallet balance are on different screens with no clear distinction. Problem 3: User reported pending withdrawal that never arrived. This is primarily a UX/design issue requiring coordinated UI changes — deferring to task 4.1 (UX feature requests triage).
  - **GH#170 — CANNOT REPRODUCE.** The version is set via `env!("CARGO_PKG_VERSION")` in `lib.rs:19` and displayed in `main.rs:54` as `format!("Dash Evo Tool v{}", VERSION)`. The Cargo.toml version is `1.0.0-dev`. This should display correctly. The "double folder" issue on Windows under Roaming is likely due to the `directories` crate behavior or a legacy directory from a previous version — cannot reproduce on macOS. Deferring.

  **Issue Files (ui-core-001 through ui-core-014):**
  - **ui-core-001 (unwrap on wallet RwLock)** — CONFIRMED but deferred to task 2.5 (lock poisoning strategy). Multiple `.read().unwrap()` and `.write().unwrap()` on wallet RwLock in create_asset_lock_screen.rs and wallet_unlock.rs.
  - **ui-core-002 (unreachable! in screen creation)** — CONFIRMED but LOW RISK. Three `unreachable!()` in `ScreenType::create_screen()` at `ui/mod.rs:476,596,599`. NetworkChooser is pre-instantiated (never goes through `create_screen()`). ClaimTokensScreen and ViewTokenClaimsScreen have enum variants but no factory methods. These are guards against invalid code paths, not user-reachable crashes. LOW PRIORITY.
  - **ui-core-003 (ScreenType equality ignores Arc)** — LOW PRIORITY. Structural equality check; functional for current use.
  - **ui-core-004 (unwrap on settings access)** — PARTIALLY CONFIRMED. Wrong line numbers cited, but `network_chooser_screen.rs` has multiple `.expect("Expected to save db settings")` calls (lines 755, 775, 825) and `.unwrap()` on app_context (lines 172, 175, 178). The settings save expects are P2 — save failure shouldn't crash the app. The app_context unwraps are guarded by tab selection logic.
  - **ui-core-005 (screen_type unwrap)** — FALSE POSITIVE. Lines 943, 946 contain only match arm braces, no unwrap/expect calls.
  - **ui-core-006 (large screen match methods)** — LOW PRIORITY. Code duplication in match arms, belongs in Section 3 refactoring.
  - **ui-core-007 (theme detection error)** — FALSE POSITIVE. `detect_system_theme()` already returns `Result` and `resolve_theme_mode()` uses `.unwrap_or(ThemeMode::Light)` with error logging.
  - **ui-core-008 (state reset on network switch)** — PARTIALLY CONFIRMED but LOW RISK. Network switch updates context on all screens. Most screens re-fetch data on context change. MasternodeListDiffScreen explicitly clears state. Stale data in other screens is transient.
  - **ui-core-009 (filter_headers_stage_start never reset)** — LOW PRIORITY. SPV-specific internal state, doesn't affect user experience.
  - **ui-core-010 (AmountInput changed flag race)** — LOW PRIORITY. egui is single-threaded; no actual race condition possible.
  - **ui-core-011 (core_client RwLock expect)** — CONFIRMED but deferred to task 2.5. Three `.expect("Core client lock was poisoned")` in `create_asset_lock_screen.rs:118,132,140`.
  - **ui-core-012 (password not zeroized)** — CONFIRMED (Security). Password field is only zeroized in the `attempt_unlock` path. If user navigates away, switches screens, or closes dialog without unlocking, the password string remains in memory unzeroized. No `Drop` implementation wipes the password buffer. Fix: ensure zeroization on all exit paths.
  - **ui-core-013 (AmountInput unit mutation)** — LOW PRIORITY. Unit name could theoretically mismatch, but all callers use consistent units.
  - **ui-core-014 (KeyExchangeConfirmationScreen)** — FALSE POSITIVE. This screen type does not exist in the codebase.

  **Issue Files (ui-contracts-017, ui-contracts-018):**
  - **ui-contracts-017 (unwrap chain in document actions)** — PARTIALLY CONFIRMED. Unwraps on `Option<T>` fields (selected_contract, selected_identity, selected_key) in `document_action_screen.rs`. These are guarded by `can_broadcast()` check before the methods are called. MEDIUM risk — could panic if call paths change. Covered by task 2.2 audit.
  - **ui-contracts-018 (unwrap in register contract)** — PARTIALLY CONFIRMED. Mix of SystemTime unwraps (safe, covered by task 2.6) and field unwraps (lines 259-260) with "unwrap should be safe here" comments. LOW-MEDIUM risk.

  **Issue Files (ui-dpns-019):**
  - **ui-dpns-019 (Mutex unwrap in DPNS screen)** — CONFIRMED but deferred to task 2.5. 11 instances of `.lock().unwrap()` in `dpns_contested_names_screen.rs` on three Mutex fields. Same lock poisoning pattern.

- [x] **1.4a Fix GH#482: Add text wrapping to error/warning message displays** (P1)
  Add `.wrap()` to all error/warning message labels that currently overflow horizontally. Affected locations:
  - `src/ui/wallets/wallets_screen/mod.rs:~3541` (SK unlock error)
  - `src/ui/wallets/import_mnemonic_screen.rs:479,609`
  - `src/ui/wallets/add_new_wallet_screen.rs:901`
  - `src/ui/wallets/send_screen.rs:1066`
  - `src/ui/wallets/create_asset_lock_screen.rs:618`
  - `src/ui/wallets/single_key_send_screen.rs:381,805`
  Replace `ui.colored_label(color, msg)` with `ui.add(egui::Label::new(egui::RichText::new(msg).color(color)).wrap())` pattern.

- [x] **1.4b Fix ui-core-012: Ensure wallet password zeroization on all exit paths** (P1)
  In `src/ui/wallets/wallet_unlock.rs`, ensure the password field is zeroized not just on unlock attempt, but also when the dialog is dismissed, the screen is navigated away from, or the component is dropped. Consider implementing `Drop` for the containing struct or adding zeroization in all non-unlock exit paths.

- [x] **1.4c Fix ui-core-004: Replace expect() on settings save in network chooser** (P2)
  In `src/ui/network_chooser_screen.rs`, replace `.expect("Expected to save db settings")` calls (lines ~755, ~775, ~825) with `if let Err(e)` + `tracing::warn!`. Settings save failure shouldn't crash the application.

---

## Section 2: Stability & Error Handling [Week 2-4]

- [x] **2.1 [META] Audit all `panic!()` calls in production code** (P0)
  Run `grep -rn "panic!" src/` and examine every instance. For each:
  (1) determine if it's reachable in production, (2) assess severity, (3) create specific removal/replacement tasks.
  Known instances: `src/backend_task/identity/mod.rs` lines 167, 193 ("need a ECDSA Key for now").

  **Audit Results:**

  **Production `panic!()` — CONFIRMED REACHABLE (fix tasks created):**
  - **database/initialization.rs:41-44** — `panic!` on DB migration failure. If migration has a bug or DB is corrupt, the app crashes instead of showing an error. The DB was already backed up before migration, so recovery is possible, but user sees a panic instead of a helpful message. Fix: return error instead of panic.
  - **database/wallet.rs:684,685,691,698** — Four panicking calls in asset lock transaction loading: `expect("Seed should be 64 bytes")`, `expect("Failed to deserialize transaction")`, `panic!("Expected AssetLockPayloadType")`, `expect("Expected at least one credit output")`. All reachable if DB has corrupt data. App crashes at startup when loading wallets. Fix: return Err from the query_map closure.
  - **app.rs:691-693** — Three `expect()` calls on `as_ref()` for testnet/devnet/local AppContext. If a network context failed to initialize (returned None), selecting that network tab crashes the app. Fix: return a user-friendly error or disable the tab.

  **Production `panic!()` — LOW RISK (sub-task created):**
  - **context.rs:1799** — `panic!("unsupported network")` in `default_platform_version()`. Matches all 4 known Network variants. Only reachable if the external `Network` enum adds a new variant (it's `#[non_exhaustive]`). Fix: return a compile-safe default or error.
  - **update_token_config.rs:678** — `unimplemented!("marketplace settings not implemented yet")`. The `MarketplaceTradeMode` variant is not selectable through the UI. Only reachable if future code sets it or if loaded from an external token config. Fix: show "not yet supported" in UI instead of panicking.

  **Production `panic!()` — JUSTIFIED (no fix task):**
  - **model/qualified_identity/mod.rs:762-765** — Intentional defensive panic for inconsistent wallet index. Comment explicitly states "non-recoverable error... to avoid unexpected behavior and loss of access to private keys." Data integrity guard — appropriate to keep as panic.

  **ALREADY FIXED by prior tasks:**
  - **backend_task/identity/mod.rs:167,193** — Fixed by task 1.2a (replaced with error return).

  **Test-only `panic!()` — SAFE (no action needed):**
  - `payments.rs:444,454` — In `#[test]` functions
  - `tokens_screen/mod.rs:3422,3481,3489,3498,3501,3510,3515,3644,3654,3657` — All in `#[cfg(test)]` module
  - `add_new_identity_screen/mod.rs:1502,1508,1548,1554` — In `#[cfg(test)]` module

  **Commented-out `panic!()` — SAFE:**
  - `core_p2p_handler.rs:127,218,237` — All commented out

  **`unreachable!()`/`unimplemented!()` calls:**
  - `app.rs:694` — After matching all 4 Network variants. JUSTIFIED.
  - `scheduled_votes.rs:158` — On boolean DB column. JUSTIFIED.
  - `ui/mod.rs:476,596,599` — Guards on screen types never constructed via `create_screen()`. JUSTIFIED (LOW RISK).

- [x] **2.1a Fix DB migration failure panic** (P1)
  In `src/database/initialization.rs:41-44`, replace `panic!` on migration failure with proper error propagation. The function already returns `rusqlite::Result<()>`, so convert the panic to a `Err(rusqlite::Error::QueryReturnedNoRows)` or a custom error message that includes the version info. The caller in `app.rs` already handles this via `?`.

- [x] **2.1b Fix asset lock loading panics in database/wallet.rs** (P1)
  In `src/database/wallet.rs:676-698`, replace 4 panicking calls inside the `query_map` closure with proper error handling:
  - Line 684: `expect("Seed should be 64 bytes")` → `map_err` to rusqlite error
  - Line 685: `expect("Failed to deserialize transaction")` → `map_err`
  - Line 691: `panic!("Expected AssetLockPayloadType")` → return Err
  - Line 698: `expect("Expected at least one credit output")` → return Err
  These are in a closure returning `rusqlite::Result`, so convert to `Err(rusqlite::Error::InvalidParameterName(...))` or similar.

- [x] **2.1c Fix network context expect() in app.rs current_app_context** (P1)
  In `src/app.rs:691-693`, the three `expect()` calls on `as_ref()` for testnet/devnet/local contexts will panic if the context failed to initialize. Options:
  - Return `Option<&Arc<AppContext>>` or `Result<&Arc<AppContext>, String>` and handle gracefully in callers
  - Or fall back to mainnet context with a warning (less ideal)
  This requires updating all callers of `current_app_context()`.

- [x] **2.1d Fix remaining low-risk panics (context.rs, update_token_config.rs)** (P2)
  Two low-risk fixes:
  (1) `src/context.rs:1799` — Replace `panic!("unsupported network")` in `default_platform_version()` with a safe fallback (e.g., return the latest platform version for unknown variants, since this is a const fn).
  (2) `src/ui/tokens/update_token_config.rs:678` — Replace `unimplemented!("marketplace settings")` with `ui.label("Marketplace settings are not yet supported.")` so it shows a message instead of crashing.

- [x] **2.2 [META] Audit `unwrap()`/`expect()` in `src/backend_task/`** (P1)
  Categorize every `unwrap()`/`expect()` call in the backend_task directory as:
  - **Safe**: value is guaranteed (e.g., regex compile of literal, `Some` just checked)
  - **Unsafe**: can actually panic in production
  Create fix tasks for all unsafe instances. Prioritize by crash likelihood.

  **Audit Results:**

  **Total calls found:** ~194
  **Lock unwraps (`.read().unwrap()`, `.write().unwrap()`, `.lock().unwrap()`, `.expect("...lock was poisoned")`):** ~80 instances across all files. ALL DEFERRED to task 2.5 (lock poisoning strategy).
  **Test-only unwraps (`#[cfg(test)]` / `#[test]`):** ~65 instances in dashpay tests (encryption.rs, avatar_processing.rs, dip14_derivation.rs, hd_derivation.rs, encryption_tests.rs, payments.rs test module). ALL SAFE.

  **UNSAFE production calls requiring fixes:**

  **P1 — Contract/document type expects (crash on SDK data mismatch):**
  - `contract.rs:105` — `.expect("Expected to get token configuration")` on `expected_token_configuration()`. Panics if contract tokens map is inconsistent.
  - `query_dpns_contested_resources.rs:24` — `.expect("expected document type")` on `document_type_for_name("domain")`. Panics if DPNS contract lacks "domain" type.
  - `query_dpns_vote_contenders.rs:25` — `.expect("expected document type")` — same pattern.
  - `vote_on_dpns_name.rs:39` — `.expect("expected document type")` — same pattern.
  - `query_dpns_contested_resources.rs:135` — `.expect("expected str")` on `Value::as_str()`. Panics if contested resource is not a string.
  - `query_dpns_contested_resources.rs:140` — `.last().unwrap()` on list that was just checked non-empty. SAFE (guarded by break on line 124).

  **P1 — Identity registration/top-up transition panics:**
  - `register_identity.rs:405` — `.expect("expected to make identity")` on `Identity::new_with_id_and_keys()`. Only in fallback when identity not fetched.
  - `register_identity.rs:695` — `.expect("expected to make transition")` on `IdentityCreateTransition::try_from_identity_with_signer()`. In error-reporting path but still panics.
  - `top_up_identity.rs:532` — `.expect("expected to make transition")` on `IdentityTopUpTransition::try_from_identity()`. Same pattern.

  **P1 — Channel send panics in spawned tasks:**
  - `query_dpns_contested_resources.rs:175,208` — `semaphore.acquire_owned().await.unwrap()`. Panics if semaphore closed during shutdown.
  - `query_dpns_contested_resources.rs:183,190,220,227` — `.expect("expected to send ...")` on channel `sender.send()`. Panics if UI receiver dropped.

  **P2 — Data conversion panics:**
  - `identity/mod.rs:327` — `.try_into().unwrap()` on hex decode result. Input is length-checked at 64 chars (line 323), so decoded output is always 32 bytes. SAFE.
  - `contacts.rs:263` — `Identifier::from_bytes(to_id_bytes).unwrap()`. Data from `Value::Identifier` match arm — should always be 32 bytes. LOW RISK.
  - `contacts.rs:307` — `Identifier::from_bytes(&decrypted_id).unwrap()`. Decrypted data could be wrong length if decryption fails silently. MEDIUM RISK.
  - `load_identity_from_wallet.rs:80,82` — `.expect("queried public key/wallet key index should exist...")`. Assumes control flow guarantees. LOW RISK.
  - `discover_identities.rs:97` — `existing.unwrap()` after `is_ok()` check on same binding. SAFE (single-threaded evaluation).
  - `contact_info.rs:495` — `.expect("entropy should be 32 bytes")` on Bytes32 try_into. SAFE (Bytes32 is always 32 bytes).
  - `query_token_non_claimed_perpetual_distribution_rewards.rs:139` — `.expect("epoch so far in future")` on u16 try_into. SAFE (epoch index is small).

  **P2 — SystemTime panics (deferred to task 2.6):**
  - `payments.rs:299,312` — `duration_since(UNIX_EPOCH).unwrap()`. Covered by task 2.6.

  **SAFE calls (no action needed):**
  - `incoming_payments.rs:228-235` — `ChildNumber::from_hardened_idx(9/5/15/0).unwrap()` and `from_normal_idx(hash).unwrap()`. All hardcoded values < 2^31, and `hash_identifier_to_u32()` masks with `0x7FFFFFFF`. SAFE.
  - `payments.rs:187` — `ChildNumber::from_normal_idx(0).unwrap()`. SAFE (0 < 2^31).
  - All test-only code in dashpay/, identity/ test modules. SAFE.

- [x] **2.2a Fix document type expect() calls in contested names** (P1)
  Replace `.expect("expected document type")` on `document_type_for_name("domain")` with `?` error propagation in 3 files:
  - `src/backend_task/contested_names/query_dpns_contested_resources.rs:24`
  - `src/backend_task/contested_names/query_dpns_vote_contenders.rs:25`
  - `src/backend_task/contested_names/vote_on_dpns_name.rs:39`
  Also replace `.expect("expected str")` on `Value::as_str()` at `query_dpns_contested_resources.rs:135` with `ok_or()?.to_string()`.

- [x] **2.2b Fix channel send/semaphore panics in contested resources query** (P1)
  In `src/backend_task/contested_names/query_dpns_contested_resources.rs`:
  - Lines 175, 208: Replace `semaphore.acquire_owned().await.unwrap()` with `.map_err()?` or graceful return.
  - Lines 183, 190, 220, 227: Replace `.expect("expected to send ...")` on `sender.send().await` with `if let Err(e)` that logs and returns (receiver may be dropped during shutdown).

- [x] **2.2c Fix identity/top-up transition expect() calls** (P1)
  Replace `.expect()` with `?` error propagation in 3 locations:
  - `src/backend_task/identity/register_identity.rs:405` — `.expect("expected to make identity")`
  - `src/backend_task/identity/register_identity.rs:695` — `.expect("expected to make transition")`
  - `src/backend_task/identity/top_up_identity.rs:532` — `.expect("expected to make transition")`

- [x] **2.2d Fix token configuration expect() in contract.rs** (P1)
  In `src/backend_task/contract.rs:105`, replace `.expect("Expected to get token configuration")` with `?` error propagation. If a token position exists in `contract.tokens()` but has no matching configuration, log a warning and skip that token instead of panicking.

- [x] **2.2e Fix Identifier::from_bytes unwrap in contacts.rs** (P2)
  In `src/backend_task/dashpay/contacts.rs`:
  - Line 263: Replace `.unwrap()` on `Identifier::from_bytes(to_id_bytes)` with `.map_err()?`.
  - Line 307: Replace `.unwrap()` on `Identifier::from_bytes(&decrypted_id)` with `.map_err()?` or `continue` on error (decrypted data may be invalid).

- [x] **2.3 [META] Audit `unwrap()`/`expect()` in `src/context.rs` and `src/database/`** (P1)
  Same categorization approach as 2.2. These are critical infrastructure files.
  Reference: `issues/context-001` through `context-023`, `issues/db-*.md`.

  **Audit Results:**

  **Total calls found:** ~95 (context.rs: ~38, database/: ~57)
  **Lock unwraps (`.read().unwrap()`, `.write().unwrap()`, `.lock().unwrap()`):** ~64 instances across all files. ALL DEFERRED to task 2.5 (lock poisoning strategy).
  **SystemTime unwraps:** 2 instances in contested_names.rs (lines 85, 219). DEFERRED to task 2.6.
  **Test-only unwraps:** ~30+ instances in database test modules. ALL SAFE.

  **UNSAFE production calls requiring fixes:**

  **P1 — context.rs initialization expects (crash on startup configuration issues):**
  - `context.rs:136` — `.expect("Failed to initialize SPV provider")` on SpvProvider::new(). Panics if DB or network config is bad.
  - `context.rs:138` — `.expect("Failed to initialize RPC provider")` on RpcProvider::new(). Panics on network config issues.
  - `context.rs:151,155,159,163,167` — Five `.expect()` on `load_system_data_contract()` for DPNS, Withdrawals, TokenHistory, KeywordSearch, Dashpay. Panics if platform version doesn't support a contract.
  - `context.rs:174` — `.expect("expected to get cookie path")` on core_cookie_path(). Panics on filesystem/config issues.
  - `context.rs:194` — `.expect("Failed to create CoreClient")` after cookie+userpass fallback. Panics if both auth methods fail.
  - `context.rs:198` — `.expect("expected to get wallets")` on DB query. Panics on schema mismatch/corruption.
  - `context.rs:205` — `.expect("expected to get single key wallets")` on DB query. Same risk.

  **P2 — context.rs asset lock processing expects:**
  - `context.rs:1641` — `.expect("Expected at least one credit output")` on `payload.credit_outputs.first()`. Panics on malformed asset lock transaction data.
  - `context.rs:1644` — `.expect("expected an address")` on `Address::from_script()`. Panics on corrupted script_pubkey.

  **P1 — database/wallet.rs data loading expects (crash on DB corruption):**
  - `wallet.rs:144` — `.expect("Expected address to be valid for network")` on check_address_for_network(). Panics if stored address doesn't match network.
  - `wallet.rs:440` — `.expect("Failed to decode ExtendedPubKey")` on decode. Panics on corrupted pubkey data.
  - `wallet.rs:457` — `.expect("expected to decrypt seed with no password")` on try_into. Panics if stored seed data is wrong length.
  - `wallet.rs:551` — `.expect("Invalid address format")` on Address::from_str(). Panics on corrupted address data.
  - `wallet.rs:556` — `.expect("Expected to convert to derivation path")` on DerivationPath::from_str(). Panics on corrupted path.
  - `wallet.rs:643` — `.expect("Invalid address format")` on UTXO address parsing. Same risk.
  - `wallet.rs:647` — `.expect("Invalid txid")` on Txid::from_slice(). Panics on corrupted txid.
  - `wallet.rs:780` — `.expect("Invalid txid bytes")` on transaction txid. Same risk.
  - `wallet.rs:782` — `.expect("Failed to deserialize transaction")` on raw tx bytes. Panics on corrupt tx data.
  - `wallet.rs:785` — `.expect("Invalid block hash")` on BlockHash::from_slice(). Panics on corrupt hash data.

  **SAFE calls (justified, no action):**
  - `wallet.rs:443,779,827,868` — `.expect("Seed hash should be 32 bytes")` on try_into. Stored as 32-byte BLOB in DB, type-guaranteed by schema.

  **P1 — database/contested_names.rs Identifier expects (crash on DB corruption):**
  - `contested_names.rs:77,211` — `.expect("Expected 32 bytes for awarded_to")` on Identifier::from_bytes(). Panics if stored BLOB is wrong length.
  - `contested_names.rs:118,252` — `.expect("Expected 32 bytes for identity_id")` on Identifier::from_bytes(). Same risk.
  - `contested_names.rs:126,260` — `.expect("Expected 32 bytes for document_id")` on Identifier::from_bytes(). Same risk.

  **P2 — database/tokens.rs:**
  - `tokens.rs:242` — `.expect("Failed to parse token ID")` on Identifier::from_vec(). Panics on corrupted token ID data.

  **ALREADY FIXED by prior tasks:**
  - `initialization.rs` migration panic — Fixed by task 2.1a
  - `wallet.rs:684-698` asset lock loading panics — Fixed by task 2.1b

- [x] **2.3a Fix context.rs initialization expects** (P1)
  In `src/context.rs:136-205`, replace 9 `.expect()` calls in `AppContext::new()` with error handling that returns `None` (the function already returns `Option<Self>`). For each:
  - Lines 136,138: SpvProvider/RpcProvider initialization → log error, return None
  - Lines 151,155,159,163,167: System data contract loads → log error, return None
  - Line 174: Cookie path → log error, return None
  - Line 194: CoreClient creation → log error, return None
  - Lines 198,205: Database wallet queries → log error, return None

- [x] **2.3b Fix context.rs asset lock processing expects** (P2)
  In `src/context.rs:1641,1644`, replace `.expect()` on credit output access and address derivation with `?` error propagation. The enclosing function `received_asset_lock_finality()` already returns `Result<(), String>`.

- [x] **2.3c Fix database/wallet.rs data loading expects** (P1)
  In `src/database/wallet.rs`, replace ~10 `.expect()` calls in query_map closures with `map_err` to rusqlite errors. Affected lines: 144, 440, 457, 551, 556, 643, 647, 780, 782, 785. These are all inside closures that return `rusqlite::Result`, so convert with `map_err(|e| rusqlite::Error::InvalidParameterName(format!(...)))`.

- [x] **2.3d Fix database/contested_names.rs Identifier expects** (P1)
  In `src/database/contested_names.rs`, replace 6 `.expect()` on `Identifier::from_bytes()` with `map_err` in the query_map closures at lines 77, 118, 126, 211, 252, 260. These are inside closures returning `rusqlite::Result`, so convert to `Err(rusqlite::Error::InvalidParameterName(...))`.

- [x] **2.3e Fix database/tokens.rs token ID expect** (P2)
  In `src/database/tokens.rs:242`, replace `.expect("Failed to parse token ID")` on `Identifier::from_vec()` with `map_err` to rusqlite error.

- [x] **2.4 [META] Validate critical issue file claims** (P0)
  Read and verify these specific high-severity issue reports against actual code:
  - `issues/wallet-003-utxo-double-spend-race-condition.md`
  - `issues/wallet-008-infinite-loop-on-proof-wait.md`
  - `issues/core-016-config-file-truncate-danger.md`
  - `issues/context-014-lock-poisoning-cascade-risk.md`
  - `issues/wallet-001-arithmetic-underflow-risk.md`
  For each confirmed issue, create a specific fix task.

  **Validation Results:**

  - **wallet-003 (UTXO double-spend race)** — CONFIRMED but LOW RISK. The race window exists: `send_single_key_wallet_payment_via_rpc()` reads UTXOs with a read lock (line 52), releases it, builds/signs/broadcasts the transaction, then later acquires a write lock (line 207) to remove spent UTXOs. However, the UI serializes user actions — only one payment can be initiated at a time via the GUI. The `Arc<RwLock<SingleKeyWallet>>` is per-wallet. The fix (hold write lock during selection, or mark UTXOs as "pending") would be correct but adds complexity for a race that cannot be triggered through the current UI. Deferring — the risk is theoretical, not practical. Would become relevant if batch/automated payments are added.
  - **wallet-008 (infinite loop on proof wait)** — ALREADY FIXED by task 1.1e. Code now uses `tokio::select!` with 5-minute timeout (lines 140-161 of `fund_platform_address_from_wallet_utxos.rs`).
  - **core-016 (config file truncate danger)** — ALREADY FIXED by task 1.3e. Code now uses atomic write via temp file + rename (lines 73-185 of `config.rs`).
  - **context-014 (lock poisoning cascade)** — FALSE POSITIVE. The issue claims returning `None` leaves the SDK and providers in an inconsistent state, but this is incorrect. The `Arc<AppContext>` is created at line 349 and if `None` is returned, no external reference exists — the Arc refcount drops to 0 and all resources (SDK, providers, DB connections) are cleaned up by Drop. The provider binding code (lines 352-388) already handles lock poisoning gracefully with `map_err(|_| "... lock poisoned")`. No inconsistent state leak.
  - **wallet-001 (arithmetic underflow risk)** — FALSE POSITIVE. The subtraction `total_input - total_output - fee` at line 143 is guarded by the check at line 107: `selected_total < total_output + final_fee`. The `fee` at line 125-127 and `final_fee` at line 103-105 use identical parameters (`estimate_p2pkh_tx_size(selected_utxos.len(), outputs.len() + 1)`) and `total_input` (line 129) equals `selected_total`. So `total_input >= total_output + fee` is mathematically guaranteed. No underflow possible.

  **No new sub-tasks created.** Both confirmed issues were already fixed by prior tasks. The UTXO race is low risk and deferred.

- [x] **2.5 Design and implement lock poisoning recovery strategy** (P1)
  Currently the codebase uses `.lock().unwrap()` pervasively. Design a consistent approach:
  - Option A: Use `.lock().unwrap_or_else(|e| e.into_inner())` where safe
  - Option B: Create a helper that logs and recovers
  - Option C: Use parking_lot mutexes (no poisoning)
  Implement the chosen strategy in `src/context.rs` first as a template, then apply elsewhere.

  **Implementation: Option B chosen and already applied codebase-wide.**
  Created `src/lock_helper.rs` with extension traits `MutexExt` (providing `lock_or_recover()`) and `RwLockExt` (providing `read_or_recover()` and `write_or_recover()`). These use `unwrap_or_else(|poisoned| poisoned.into_inner())` with `tracing::warn!` logging. All ~80+ production lock access sites across 71 files have been migrated to use these helpers. Zero `.lock().unwrap()`, `.read().unwrap()`, or `.write().unwrap()` calls remain in production code (18 remaining instances are exclusively in `#[test]` functions where panicking is acceptable).

- [x] **2.6 Fix SystemTime expect panics** (P1)
  Replace `SystemTime::now().duration_since(UNIX_EPOCH).expect(...)` with `.unwrap_or_default()` across the codebase.
  Reference: `issues/core-010-unix-timestamp-unwrap.md`, `issues/ui-tokens-006-expect-on-time-operations.md`.

---

## Section 3: Code Structure Refactoring [Week 3-6]

- [x] **3.1 [META] Review masternode_list_diff_screen.rs (4406 lines)** (P2)
  Note: PR#520 already refactors this. First review PR#520 (`gh pr view 520`, `gh pr diff 520`).
  Then identify remaining work after that PR: further split points, extracted components, shared utilities.
  Create sub-tasks for remaining refactoring only.

  **Review Results:**

  **PR#520 Status:** Open, targets `v1.0-dev`, +850/-799 lines. Reorganizes the 30 flat fields into 7 focused sub-structs (`InputState`, `UiState`, `TaskState`, `MnListData`, `CacheState`, `SelectionState`, `IncomingState`). Extracts 3 small helpers (`render_message_banner`, `render_error_banner`, `render_pending_status`). Good state organization work but does NOT reduce the file's line count (still ~4400 lines after PR#520).

  **Current file analysis (4392 lines, 69 functions):**
  - 13 functions over 100 lines, largest is `fetch_single_dml()` at 239 lines
  - 126 lines of commented-out dead code (`fetch_range_dml`, lines 971-1096)
  - 11 selection state fields with reset logic duplicated across 9+ locations
  - 5 cache structures for height/hash lookups (~9 functions, ~300 lines total)
  - Detail renderers share common layout patterns (collapsible sections, key-value grids) but aren't abstracted
  - `display_task_result()` is 197 lines of match arms handling 4+ backend result variants
  - File I/O (`FileDialog`) mixed into `render_qr_info()` rendering function

  **Remaining refactoring after PR#520 (sub-tasks below):**
  PR#520 handles the state decomposition well. The remaining work is extracting **rendering logic** and **data-layer functions** into separate files/modules, plus dead code cleanup.

- [x] **3.1a Remove commented-out `fetch_range_dml` dead code** (P2)
  Delete 126 lines of commented-out code at lines 971-1096. This is an abandoned function with no callers. If it's ever needed again, it exists in git history.

- [x] **3.1b Extract height/hash resolution and caching into a helper module** (P2)
  Extract these 9 functions (~300 lines) into a new `masternode_list_diff_screen/cache_helpers.rs` module (or similar):
  - `get_height()`, `get_height_or_error_as_string()`, `get_height_and_cache()`, `get_height_and_cache_or_error_as_string()`
  - `get_block_hash()`, `get_block_hash_and_cache()`
  - `get_chain_lock_sig()`, `get_chain_lock_sig_and_cache()`
  - `feed_qr_info_block_heights()`, `feed_mn_list_diff_heights()`, `feed_quorum_entry_height()`
  These all operate on the cache fields (`block_height_cache`, `block_hash_cache`, `chain_lock_sig_cache`, etc.) and can be grouped as methods on a `CacheState` struct (building on PR#520's struct).

- [x] **3.1c Extract QR info rendering into a separate file** (P2)
  Move these QR-info-related rendering functions (~400 lines) into `masternode_list_diff_screen/qr_info_tab.rs`:
  - `render_qr_info()` (167 lines, includes FileDialog I/O)
  - `render_quorum_snapshots()` (38 lines)
  - `render_mn_list_diffs()` (86 lines)
  - `render_last_commitments()` (54 lines)
  - `render_quorum_snapshot_list()` (15 lines)
  - `render_mn_list_diff_list()` (15 lines)
  - `show_mn_list_diff_heights_as_string()` (37 lines)

- [x] **3.1d Extract quorum viewer rendering into a separate file** (P2)
  Move quorum-viewer-related functions (~550 lines) into `masternode_list_diff_screen/quorum_viewer_tab.rs`:
  - `render_quorums()` (151 lines)
  - `render_quorum_details()` (201 lines)
  - `render_selected_quorum_entry()` (180 lines, static)
  - `required_cl_sig_heights()` (16 lines)

- [x] **3.1e Extract core items / chain-lock / instant-send rendering** (P2)
  Move these functions (~400 lines) into `masternode_list_diff_screen/core_items_tab.rs`:
  - `render_core_items()` (94 lines)
  - `render_chain_lock_details()` (85 lines)
  - `render_instant_send_details()` (105 lines)
  - `render_selected_item_details()` (9 lines)
  - `attempt_verify_chain_lock()` (7 lines)
  - `attempt_verify_transaction_lock()` (7 lines)
  - `received_new_block()` (47 lines)

- [ ] **3.1f Split display_task_result into per-variant handlers** (P3)
  In `display_task_result()` (197 lines, line 4074), each match arm handles a different `BackendTaskSuccessResult` variant with 20-50 lines of inline logic. Extract each arm into a named method (e.g., `handle_mn_list_diff_result()`, `handle_qr_info_result()`, etc.) to improve readability.

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

**Total tasks:** 65 (24 META + 41 direct)
**Note:** META tasks will expand this list significantly as they produce sub-tasks.

| Section | Tasks | Completed |
|---------|-------|-----------|
| 1. Bug Triage | 30 | 30 |
| 2. Stability | 20 | 20 |
| 3. Refactoring | 13 | 6 |
| 4. UI/UX | 4 | 0 |
| 5. Architecture | 4 | 0 |
| 6. Testing | 6 | 0 |
| 7. Features | 5 | 0 |
| 8. Security | 2 | 0 |
| 9. Upstream PRs | 2+ | 0 |
