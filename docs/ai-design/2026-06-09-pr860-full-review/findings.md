# PR #860 Full Review Findings

**Reviewed commit**: `c18da455`
**Review methodology**: 36 reviewer agents + 6 cross-cutting dimensions; adversarially verified (2 verifiers per finding); 133 confirmed findings synthesized to 54.
**Overall risk**: HIGH
**Merge recommendation** (synthesis verbatim):

> DO NOT MERGE as-is. There is a confirmed CRITICAL fresh-install regression (F77) that bricks wallet
> creation for every new user, plus a cluster of HIGH/MEDIUM funds-safety, secret-lifecycle, and
> migration-lifecycle defects (F131, F62, F140/F141, F60, F17, F20, F78). These must be fixed and
> regression-tested (a test that drives the real `Database::initialize` fresh path then
> `register_wallet`, and a first-boot migrate->hydrate->assert-non-empty test) before merge. PROJ-005
> (re-pin platform deps to a tagged release) remains a hard release gate G1. Once the top blockers
> are resolved, the remaining LOW/INFO findings (convention/docs/dead-code) can be addressed as
> fast-follows and should not individually block merge.

---

## Resolved Blockers

All of the following were confirmed fixed and verified by Marvin (combined tree green: clippy clean,
lib 709 tests, kittest 86 tests) and Smythe ("ship it", 0 new defects). QA-reconfirmed.

| ID | Severity | Title | Fix commit |
|----|----------|-------|------------|
| F77 | CRITICAL | Fresh installs cannot create or import any wallet — legacy `wallet` table is never created | `dc9a0c3b` |
| F78 | MEDIUM | `clear_network_data` deletes from non-existent legacy tables on fresh installs — "Clear data" fails silently | `cacbf6c4` |
| F60 | HIGH | `clear_network_database` does not clear authoritative wallet state (sidecars + seed vault + shielded tree) | `f6d2ecf7` |
| F17 | MEDIUM | Removing an HD wallet leaves its encrypted seed envelope (and shielded notes) orphaned on disk | `f6d2ecf7` |
| F20 | MEDIUM | Wallet removal and network reset do not clear the shielded-notes sidecar — orphaned plaintext notes | `f6d2ecf7` |
| F54 | MEDIUM | Swallowed shielded-note DB insert can permanently lose a spendable note | `415b4826` |
| F131 | HIGH | "Lock" gesture does not wipe the session-cached plaintext seed; locked wallet still signs without a prompt | `2304d063` |
| F62 | HIGH | `register_wallet` swallows seed-envelope persist failure, causing silent wallet/funds loss on next restart | `88c21c96` |
| F140 | HIGH | Migrated wallets are invisible until a second restart — hydration runs before migration populates sidecars | `36f77562` |
| F134 | MEDIUM | `Wallet`/`WalletSeed`/`ClosedKeyItem` derive `Debug` without redaction; a live `debug!` log sink writes seed material | `c15048f7` |
| F37 | MEDIUM | Failed wallet-funded identity registration persists an all-zeros placeholder identity | `3a1bb2f6` |
| F89 | MEDIUM | Advanced single-key import dialog never inserts the key into `single_key_wallets`; imported key is lost | `34543c0d` |
| F95 | MEDIUM | `ContactRequests::display_task_error` is unreachable; embedded DashPay tabs lose typed error routing | `d92dcf34` |
| F118 | MEDIUM | `event_bridge_live` e2e test uses `#[tokio::test]` instead of the mandatory shared runtime | `cdfa4311` |
| SEC-001 | — | QA-surfaced security gap (resolved) | `6300f27b` |
| SEC-002 | — | QA-surfaced security gap (resolved) | `4ec7b5e9` |
| QA-001 | — | Private-key bytes unredacted in `PrivateKeyData` `Debug`/`Display` | `b5ad862b` |

**Resolved count**: 17 (14 synthesis findings + 3 QA-found gaps)

---

## Open — Release Gate

| ID | Severity | Title | Location | Status |
|----|----------|-------|----------|--------|
| F121 / PROJ-005 | HIGH | Platform git deps pinned to unreleased feature-branch HEAD `9e1248cb` (feat/platform-wallet-rehydration, open draft PR #3692); declared version `4.0.0-beta.2` does not match tag SHA; build is non-reproducible | `Cargo.toml:21,31,32,35` | Upstream-gated. Re-pin all `dashpay/platform` deps to a tagged release commit before merge. Transitively-pulled `rust-dashcore` rev and vendored OpenSSL must also reconcile against the released lockfile. |

---

## Fast-Follow Backlog

Non-blocking convention, docs, dead-code, and latent-edge-case items. None individually blocks merge.
Grouped by the synthesis systemic themes where natural.

**Tail outcome (range `0196b129..HEAD`)**: the fast-follow tail landed ~32 of these. Each row below
carries a `Status` of **RESOLVED** (with the fix commit), **SKIPPED** (judged inert — no real defect),
or **DEFERRED** (cross-file / upstream-gated / out of this round's scope). The Summary table at the
bottom is updated to match.

> **Pre-existing note (not in this finding set): SEC-006** — MEDIUM, surfaced by Smythe.
> `wallets_screen/mod.rs:2189` keeps a plaintext `SingleKeyWallet` in a long-lived in-memory map.
> Pre-existing, outside the PR diff; tracked separately, not a tail fast-follow.

### Systemic themes (verbatim from synthesis)

1. Migration was not fully swept: dead/vestigial code, orphaned stub methods, write-only state, and stale doc comments/tombstones referencing removed RPC/seedless/legacy machinery (F28, F32, F35, F36, F49, F71, F72, F73, F83, F104, F105, F109, F110, F111, F112).
2. Fresh-install vs legacy-schema mismatch: T-DEV-01 gated legacy tables behind `include_legacy` but several production write/clear paths still assumed those tables exist (F77 CRITICAL, F78, F60 — all resolved).
3. Migration lifecycle / hydration ordering is fragile: hydration runs before sidecars are populated, nothing re-hydrates on migration success, no-op handlers fail to clear secrets, migration banner/empty-state UI present contradictory guidance (F140 — resolved; F141, F113, F114, F142, F143, F51, F31 — open).
4. Typed-error convention drift: error-string control flow (`msg.contains` / `e.to_string()` parsing) and stringified errors in `String` fields instead of typed `#[source]` variants (F43, F59, F66, F111, F129, F139, F5).
5. Raw upstream error text leaks into user-facing strings (seam boundary 3 / no-jargon rule), inconsistent with the correct `with_details()` pattern used elsewhere (F23, F87, F91, F97, F101, F137).
6. Secret-lifecycle hygiene gaps: session-cached seed not wiped on lock, derived keys and imported WIF bytes left non-zeroized, unredacted `Debug` on wallet types with a live `debug!` log sink, orphaned encrypted seed envelopes after deletion (F131, F62, F17, F9, F92, F100, F134, F135 — F131/F62/F134 resolved).
7. UI auto-fetch/loading-flag inconsistency: attempted-flags set only on success cause retry storms or permanent stuck-loading states; embedded DashPay tabs lose error routing (`display_task_error` never forwarded) (F94, F96, F95 — F95 resolved).
8. `bincode` non-self-describing encoding + `#[serde(default)]` gives a false forward-compat guarantee for evolving sidecar structs routed through `DetKv`; misleading `SCHEMA_VERSION` guidance (F25, F26).
9. Panic-on-fallible-input on the at-rest/decode boundary: unguarded `copy_from_slice`/`Nonce::from_slice` on bincode-decoded vault and sidecar bytes can panic (one poisons a long-lived mutex) instead of returning typed errors (F12, F21, F133).
10. Unreleased upstream dependency pin and enlarged native/secret-storage build surface (F121 — release gate; F122, F123 — transitive consequences).

---

### Theme 1 — Migration sweep: dead code, vestigial stubs, stale docs

| ID | Sev | Title | Location | Recommendation | Status |
|----|-----|-------|----------|----------------|--------|
| F28 | INFO | Migration left dead/vestigial code and stale docs across many modules | `src/wallet_backend/platform_address.rs:245-306`; `src/backend_task/core/{refresh,send}_single_key_wallet_payment.rs`; `src/context/transaction_processing.rs:33-95`; `src/context_provider*.rs`; `src/backend_task/error.rs:1033` | Sweep: delete unreachable/vestigial code and fields, remove the stray `EDIT-PROBE-MARKER`, fix broken intra-doc link, drain or remove the ZMQ status channel, correct stale doc comments. | **RESOLVED** `d9f99838` — single-key stub files deleted, dead asset-lock loop dropped, stale doc comments and broken intra-doc link fixed, `EDIT-PROBE-MARKER` removed. ZMQ status channel left for a follow-up that owns `context/mod.rs`. |
| F32 | INFO | Migration design/audit docs describe never-shipped or contradicted mechanisms; CHANGELOG/grep evidence inaccurate | `docs/ai-design/2026-06-02-rehydration-rewire/design.md:51-58`; `docs/ai-design/2026-05-18-platform-wallet-migration/data-model-and-migration.md`; `CHANGELOG.md:20` | Add `SUPERSEDED` banners to affected design sections; fix false grep evidence in the gap audit; correct the CHANGELOG vault path; cross-link the live `FinishUnwire`/`kv-keys.md` mechanism. | **RESOLVED** `a6322da1` — SUPERSEDED banners added, false grep evidence corrected, CHANGELOG vault path fixed, live mechanism cross-linked. |
| F49 | INFO | Successful asset-lock top-up no longer untracks the consumed lock; stale lock keeps appearing as fundable | `src/backend_task/wallet/fund_platform_address_from_asset_lock.rs:87-123` | After a successful top-up, mark/untrack the consumed lock (or filter consumed locks from the picker) so it stops appearing as fundable. | **DEFERRED** — upstream `consume_asset_lock` is `pub(crate)`; untracking the consumed lock is upstream-gated. Revisit when the upstream API is exposed. |

### Theme 3 — Migration lifecycle / banner UX

| ID | Sev | Title | Location | Recommendation | Status |
|----|-----|-------|----------|----------------|--------|
| F51 | LOW | `ShieldedTask` excluded from the lazy-build set; a first shielded task before backend wiring surfaces a misleading 'restart to finish migration' error | `src/backend_task/mod.rs:475-514` | Add `BackendTask::ShieldedTask(_)` to the lazy-build `matches!`, or make `legacy_shielded_present_but_sidecar_empty` treat an unwired backend as 'cannot gate yet' rather than mapping to the migration error. | **RESOLVED** `ff98ef89` — shielded wired into the lazy-build set; terminal storage errors surfaced. |
| F113 | LOW | Migration banner UX: spurious 'Storage update complete' on every launch/switch, and two contradictory error banners on failure | `src/app.rs:1102-1150`; `src/backend_task/migration/finish_unwire.rs:207,223,272` | Surface the Success banner only when migration actually moved data (carry a `did_work` flag); suppress the generic `TaskResult::Error` banner for `TaskError::MigrationFailed` since it already emits its own. | **RESOLVED** `52edbaf8` — `did_work` flag added so Success shows only when data moved; generic error banner suppressed for `MigrationFailed`. (Same commit also resolves F30.) |

### Theme 4 — Typed-error convention drift

| ID | Sev | Title | Location | Recommendation | Status |
|----|-----|-------|----------|----------------|--------|
| F43 | LOW | Typed errors stringified into `TaskError` `String` fields and error-string control flow across DashPay/migration/balance paths | `src/backend_task/dashpay/contact_requests.rs:290`; `src/backend_task/wallet/fetch_platform_address_balances.rs:87`; `src/context/identity_db.rs:57-65`; `src/backend_task/error.rs:936` | Replace `String` detail fields with typed `#[source]` variants (`Box<SdkError>` for SDK errors); replace the proof-error string parse with a structural match; use `Box::new(e)` where a typed source already exists. | **RESOLVED** `bed6f8a0` + `18b9f65f` — stringified errors replaced with typed `#[source]` variants. |
| F45 | INFO | Wallet seed-unavailable mapped to a DashPay-contact-specific error message in unrelated balance-sync/pubkey-warming tasks | `src/backend_task/wallet/fetch_platform_address_balances.rs:43-45`; `src/backend_task/wallet/warm_identity_auth_pubkeys.rs:54-57` | Use `TaskError::WalletLocked` in both tasks for consistency; confine `ContactWalletSeedUnavailable` to DashPay contact flows where its wording is correct. | **RESOLVED** `18b9f65f` — seed-unavailable mapped to `WalletLocked` in both tasks. |

### Theme 5 — Raw error text in user-facing strings

| ID | Sev | Title | Location | Recommendation | Status |
|----|-----|-------|----------|----------------|--------|
| F23 | LOW | Raw upstream SPV/sync-manager error text and internal manager id rendered verbatim in the connection-status panel | `src/ui/network_chooser_screen.rs:420-428`; `src/wallet_backend/event_bridge.rs:90-93,159-162` | Render a fixed user-facing label ('Sync error — open Settings for details') for the SPV line and attach the raw `spv_last_error` only as a tooltip/details affordance, mirroring the `with_details()` pattern used elsewhere. | **RESOLVED** `6d9dbe9a` — raw SPV/unlock error text kept out of user-facing copy. |
| F50 | LOW | Storage-open errors (`WalletDataTooNew`/`WalletDataIncompatible`) are logged-and-discarded at dispatch; user sees a misleading generic banner | `src/backend_task/mod.rs:475-484`; `src/context/mod.rs:777-781` | Cache the build error in the context so `wallet_backend()` returns it instead of the generic variant, or propagate `Err(e)` for storage-open failures so the banner shows actionable copy. | **RESOLVED** `ff98ef89` — terminal storage-open errors are surfaced instead of collapsing to the generic banner. |
| F101 | INFO | `try_open_wallet_no_password`/unlock surface raw `String` errors with jargon and collapse all failures to 'Incorrect password' | `src/ui/components/wallet_unlock_popup.rs:124-185` | Map the no-password size error to a calm jargon-free message via `with_details`; add an explicit next step to the 'Incorrect password' message. | **RESOLVED** `6d9dbe9a` — unlock errors mapped to calm jargon-free copy with details. |
| F103 | LOW | DAPI endpoint refresh shows a spurious 'Core RPC password saved successfully' banner | `src/ui/network_chooser_screen.rs:1748-1770` | Remove the vestigial `CoreClientReinitialized` password-success handler and dead `config_save_failed`/`reinit_banner` plumbing; if a DAPI-reinit confirmation is wanted, give it an accurate message. | **RESOLVED** `6d9dbe9a` — spurious password-success banner and dead plumbing removed. |

### Theme 6 — Secret-lifecycle hygiene

| ID | Sev | Title | Location | Recommendation | Status |
|----|-----|-------|----------|----------------|--------|
| F9 | INFO | Derived encryption keys and imported WIF bytes escape the secret chokepoint without zeroization | `src/wallet_backend/dashpay.rs:154-209`; `src/wallet_backend/single_key.rs:179-194` | Wrap derived encryption keys and extracted WIF bytes in `Zeroizing`; also wrap the remembered-passphrase copy in `WalletUnlock`. | **RESOLVED (partial)** `499947e5` — extracted single-key WIF bytes wrapped in `Zeroizing`. **DEFERRED:** the derived DashPay encryption keys (`dashpay.rs`) and the remembered-passphrase copy in `WalletUnlock` are cross-boundary secret sub-parts left for a follow-up. |
| F92 | INFO | `ImportSingleKeyRequest` holds WIF/passphrase as plain `String` and derives `Debug` | `src/ui/wallets/import_single_key.rs:43-57` | Use `Secret<String>`/`Zeroizing<String>` for `wif`/`passphrase`; avoid deriving `Debug` (or implement a redacting `Debug`), reusing `PasswordInput::take_secret()`. | **RESOLVED (partial)** `499947e5` — derived `Debug` dropped for a hand-written redacting impl (presence+length only); WIF held as `Zeroizing<String>`; a test asserts neither secret leaks. **DEFERRED:** wrapping the `passphrase` field itself in `Zeroizing`/`Secret`. |

### Theme 7 — UI auto-fetch / loading-flag inconsistency

| ID | Sev | Title | Location | Recommendation | Status |
|----|-----|-------|----------|----------------|--------|
| F94 | LOW | Auto-fetch retry storm / stuck-loading from attempted-flag set only on success (`ContactRequests`, `PaymentHistory`, `ContactsList`) | `src/ui/dashpay/contact_requests.rs:234,290,843,873`; `src/ui/dashpay/contacts_list.rs`; `src/ui/dashpay/send_payment.rs` | Set the attempted flag at dispatch time (mirror `ProfileScreen`) and reset `loading=false` in `display_message`/`display_task_error` handlers so a failed load fires once and stops. | **RESOLVED** `33927899` (`ContactRequests`, `ContactsList`) + this tail (`PaymentHistory` in `send_payment.rs`: `has_searched` set at dispatch, `loading` settled in `display_message`). |

### Theme 8 — bincode / sidecar forward-compat

| ID | Sev | Title | Location | Recommendation | Status |
|----|-----|-------|----------|----------------|--------|
| F25 | LOW | `bincode` `#[serde(default)]` gives a false forward-compat guarantee for evolving `DetKv` sidecar structs; misleading `SCHEMA_VERSION` guidance | `src/model/wallet/meta.rs:45-52`; `src/wallet_backend/kv.rs:46-51` | Correct both comments to state bincode standard config is positional/non-self-describing — adding/removing/reordering fields is format-breaking for stored blobs; document the required migration (envelope versioning or msgpack). | **RESOLVED** `858fc63c` — comments corrected to present-state truth (positional/non-self-describing; field changes are format-breaking). |

### Theme 9 — Panic-on-fallible-input at the at-rest boundary

| ID | Sev | Title | Location | Recommendation | Status |
|----|-----|-------|----------|----------------|--------|
| F12 | LOW | Wrong-length nonce / cmx blob panics on the at-rest decode boundary instead of returning a typed error (one poisons a long-lived mutex) | `src/wallet_backend/single_key_entry.rs:174`; `src/wallet_backend/shielded.rs:319-335`; `src/wallet_backend/secret_access.rs:615` | Replace `copy_from_slice`/`Nonce::from_slice` with checked `try_into` conversions returning typed errors (`SingleKeyCryptoFailure`/`SecretDecryptFailed`/`MalformedVault`) on length mismatch. | **RESOLVED** `90cc22cc` — checked conversions return typed errors at the at-rest decode boundary; no more panics. |

### Remaining LOW items (no strong theme cluster)

| ID | Sev | Title | Location | Recommendation | Status |
|----|-----|-------|----------|----------------|--------|
| F90 | LOW | Two divergent single-key import flows with contradictory passphrase/hex/error behavior | `src/ui/wallets/import_mnemonic_screen.rs:108-189`; `src/ui/wallets/import_single_key.rs` | Consolidate on one dialog/component and the typed backend path; remove the 'Private Key' tab from Import Wallet or have it delegate to `ImportSingleKeyDialog`. | **DEFERRED** — single-key import consolidation not implemented this round; both flows still present. Tracked for a follow-up that owns the import UI. |
| F3 | LOW | DashPay derivation seam: no pinned-vector tests for `account_reference` or contact-info encryption keys | `src/wallet_backend/dashpay.rs:127-128,154-209,419-450` | Add positive pinned-vector tests and a testnet!=mainnet assertion; have `first_associated_wallet_seed_hash` delegate to `identity.dashpay_wallet_seed_hash()`. | **RESOLVED** `74cbeb10` — contact-derivation seam vectors pinned. **DEFERRED:** the `first_associated_wallet_seed_hash` delegation (cross-boundary) is left for a follow-up. |
| F63 | LOW | Latent concurrency edges: non-atomic lazy backend init, settings-cache lost update, non-atomic blob+index writes | `src/context/mod.rs:712-746`; `src/context/settings_db.rs:97-136`; `src/context/identity_db.rs` | Serialize backend construction with a `tokio::Mutex`/`OnceCell`; hold the settings write lock across the cache-miss path; make blob+index inserts atomic or document ordering. | **RESOLVED** `24842219` (+ `18b9f65f`) — lazy backend build serialized and settings-cache fill made atomic under the write lock. |
| F10 | LOW | Uncompressed-WIF and cross-network single-key import edges (cold-boot address/label divergence) | `src/wallet_backend/single_key.rs:168-194,450-464,502-556` | Persist the compression flag in `ImportedKey`/`SingleKeyEntry` and reconstruct with original compression on rebuild, or normalize/reject uncompressed WIFs explicitly; add a round-trip test. | **RESOLVED** `499947e5` — uncompressed WIFs rejected at import via typed `UncompressedWifUnsupported`; round-trip test proves compressed imports rebuild to the same address. |
| F40 | LOW | Contact-request rejection/blocked markers are global, not scoped per owner identity — cross-identity status bleed | `src/backend_task/dashpay/contact_requests.rs:738-739`; `src/wallet_backend/dashpay.rs:674-679,834-839` | Scope `rejected`/`blocked` markers per owner: thread the acting identity id into `dashpay_mark_rejected/blocked` and the `kv_contains` read, using `DetScope::Identity(&owner)`. | **RESOLVED** `bed6f8a0` — blocked/rejected markers scoped per owner identity. |
| F47 | LOW | `sign_message` produces a non-recoverable signature with a hardcoded recovery header; ~50% external-verify failure | `src/backend_task/wallet/sign_message_with_key.rs:72-78`; `src/ui/identities/keys/key_info_screen.rs:755-765` | Use `sign_ecdsa_recoverable`, compute the real header `27+recId(+4 for compressed)` and serialize the 65-byte recoverable form; fix both the backend task and the `sign_ecdsa_local` UI helper together. | **RESOLVED** `253a4c6d` — recoverable signatures produced for signed messages. |
| F48 | LOW | Rewritten change-address (fee-from-wallet) funding branch has no automated coverage | `tests/backend-e2e/wallet_tasks.rs:126-139,365-373`; `src/backend_task/wallet/fund_platform_address_from_wallet_utxos.rs:78-97` | Add a focused unit test over the output-map/fee-strategy construction asserting the destination receives exactly `amount_credits` and the fee `ReduceOutput` index resolves to the change output. | **RESOLVED** `da7110f1` — fee-from-wallet platform funding output map covered by a unit test. |
| F55 | LOW | Structured Withdrawals query returns all statuses for `status=queued` despite the documented in-queue filter | `src/backend_task/platform_info.rs:843-866`; `src/mcp/tools/platform.rs:110-112` | Add a `WhereClause` filtering status `In [QUEUED, POOLED, BROADCASTED]` for `completed=false`, or correct the docs to state queued returns all. | **RESOLVED** `86e0d2ba` — in-queue withdrawals filtered to QUEUED/POOLED/BROADCASTED. |
| F57 | LOW | Batch shield build bridges async via `futures::executor::block_on` and serializes N passphrase prompts | `src/backend_task/shielded/bundle.rs:134-156`; `src/ui/wallets/shield_screen.rs:323-369` | Keep the build async and `await` on the tokio runtime instead of `block_on`; pre-resolve/cache the passphrase for the batch before entering the loop. | **RESOLVED** `5617ae38` — batch shield build kept async; passphrase prompted once per batch. |
| F58 | LOW | User-selected funding source address for shield-from-core is silently ignored | `src/backend_task/shielded/mod.rs:44-50`; `src/context/shielded.rs:242-252`; `src/ui/wallets/shield_screen.rs:956-970` | Remove the source-address selection UI for the shield-from-core path (and drop the dead `source_address` field), or honor the user's selection by passing it through to upstream coin selection. | **RESOLVED (behavioral)** — shield-from-asset-lock now ignores `source_address` by design and delegates coin selection to the upstream wallet's authoritative UTXO set (`shielded.rs:247-249`); UI passes `source_address: None`. **DEFERRED:** removing the now-vestigial `source_address` enum field (cross-file). |
| F69 | LOW | New k/v storage modules ship with no unit tests | `src/context/{contested_names_db.rs, contract_token_db.rs, platform_address_db.rs, settings_db.rs}` | Add in-memory `KvStore` tests (reuse `InMemoryKv` pattern from `identity_db.rs`) covering at minimum: contest winner/locked/no-winner branches, token registry round-trip, settings round-trip. | **RESOLVED** `90853953` (+ `24842219`) — k/v round-trip unit tests added for contested-names, token registry, and platform-address stores. |
| F107 | LOW | Developer 'Clear Platform Addresses' DB-clear failure is silent (no user feedback, leaves UI inconsistent) | `src/ui/network_chooser_screen.rs:878-880` | Surface a `MessageBanner` error (`with_details`) on the `Err` arm; perform in-memory map clears independently of the best-effort file unlink. | **RESOLVED** `6d9dbe9a` — clear failure surfaces a banner with details; in-memory clears decoupled from the file unlink. |
| F61 | LOW | `clear_spv_data()` is a no-op that reports success; SPV persistor never cleared | `src/context/wallet_lifecycle.rs:23-25`; `src/ui/network_chooser_screen.rs:1371-1377` | Implement the clear against the upstream persistor (delete/recreate `platform-wallet.sqlite` + shielded tree with the backend stopped), or return a typed `NotImplemented`/`Unavailable` error until that work lands. | **RESOLVED** `1ac57395` — cached chain data is actually cleared instead of faking success. |
| F88 | LOW | `dash_qt_path` autodetect no longer re-runs after settings are persisted (behavioral regression) | `src/model/settings.rs` | Re-apply the fallback in the deserialization path (`dash_qt_path: w.dash_qt_path.map(PathBuf::from).or_else(detect_dash_qt_path)`), or document the sticky behavior as deliberate. | **RESOLVED** `837fc6bd` — autodetect re-runs on deserialize when the stored path is unset; explicit paths preserved. |
| F93 | LOW | Address table 'Total Received' column now duplicates 'Balance' (mislabeled metric) | `src/ui/wallets/wallets_screen/address_table.rs:141-205` | Drop the 'Total Received' column for HD accounts until a real cumulative-receipts source exists, or relabel it clearly as the current balance. | **RESOLVED** `c33773e2` — duplicate 'Total Received' column (and its sort key) dropped for HD accounts. |
| F102 | LOW | Passphrase modal and JIT secret prompt share one overlay id and both react to Escape without consuming it | `src/ui/components/passphrase_modal.rs:70-75,162-166` | Consume the Escape key (`input_mut().consume_key`) when the modal claims it, or derive the overlay id from a per-modal salt. | **RESOLVED** `c33773e2` — overlay id salted per modal title; Escape consumed when the modal claims it. |

### Remaining INFO items

| ID | Sev | Title | Location | Recommendation | Status |
|----|-----|-------|----------|----------------|--------|
| F37b | INFO | Contested/unreliable funds-safety claims — resolved to no real defect | `src/backend_task/dashpay/payments.rs:290-293`; `src/model/wallet/mod.rs:667-690` | No merge action. Optionally adopt explicit discriminators as defensive hygiene. | **SKIPPED** — judged inert; no real defect. No merge action taken. |
| F1 | INFO | Wallet-skip and seed-length errors are swallowed to log-only; user not told a wallet was skipped/relabeled | `src/wallet_backend/hydration.rs:74-82`; `src/wallet_backend/single_key.rs:168-223` | Where a typed error promises user visibility, carry the skip/relabel reason out to a `MessageBanner` once an egui context is reachable, or downgrade the doc to log-only. | **RESOLVED** `858fc63c` — hydration-skip doc corrected to present-state (log-only) truth. |
| F2 | INFO | Doc/contract overstatements and convention nits in storage/error layers (no behavioral defect) | `src/wallet_backend/{mod.rs:457-461, shielded.rs:3-8/106-142, kv.rs:71-72, single_key.rs:7,636}` | Batch-fix doc/label inaccuracies to present-state truth; dedupe the passphrase validator into `model/`; narrow the refinery-error mapping to divergent-history only. | **RESOLVED** `858fc63c` — doc/label inaccuracies corrected to present-state. **DEFERRED:** dedupe the passphrase validator into `model/` and the refinery-error narrowing (cross-boundary). |
| F34 | INFO | Backend-authoritative input validation dropped from `SendWalletPayment` (empty-list/zero-amount) | `src/backend_task/core/mod.rs:257-301`; `src/backend_task/dashpay/payments.rs:239,271` | Re-add backend-authoritative validation via a shared `model/` validator: reject empty recipient list and `amount_duffs==0` with a typed `TaskError` before calling `send_payment`. | **RESOLVED** `938a8436` — backend-authoritative recipient validation restored. |
| F19 | INFO | Latent unguarded-arithmetic and defensive-coding hazards (unreachable today) | `src/wallet_backend/secret_access.rs:441`; `src/context_provider_spv.rs:90-126` | Optional hardening: use `Instant::now().checked_add(duration)` for `RememberPolicy::For`; capture a stored `Handle` and use `try_current()` returning a typed error in `ContextProviderSpv`. | **RESOLVED** `8c49cb27` — secret-expiry arithmetic hardened (`checked_add`) and runtime-handle lookup made fallible. |
| F30 | INFO | Migration xpub column read uses `unwrap_or_default()`, swallowing read errors to empty | `src/backend_task/migration/finish_unwire.rs:913,1096` | Use `row.get(N)?` for the xpub column for consistency with seed_hash handling, or probe via `pragma_table_info` if a legacy schema could legitimately lack it. | **RESOLVED** `52edbaf8` — xpub reads use `row.get(N)?` so genuine read errors surface instead of yielding empty. |
| F70 | INFO | User-identity load now attaches the full wallet map instead of only the owning wallet (verified inert) | `src/context/identity_db.rs` | No action required. Optionally restrict the map to the identity's stored `wallet_hash` for clarity. | **SKIPPED** — verified inert; no action required. |

---

## Summary

Counts reflect the fast-follow tail (`0196b129..HEAD`). Several INFO/LOW rows resolved a behavioral
or doc sub-part while deferring a cross-file/cross-boundary remainder — those are counted under
**Resolved** with the deferred remainder annotated inline in the row.

| State | Count |
|-------|-------|
| Resolved blockers (synthesis findings) | 14 |
| Resolved blockers (QA-found gaps) | 3 |
| Open — release gate (F121/PROJ-005) | 1 |
| Fast-follow — resolved (LOW) | 24 |
| Fast-follow — resolved (INFO) | 11 |
| Fast-follow — skipped (judged inert: F37b, F70) | 2 |
| Fast-follow — deferred (F49 upstream-gated, F90 consolidation) | 2 |
| **Fast-follow subtotal** | **39** |
| **Total synthesis findings** | **54** |

**Tail resolution rate**: 35 of 39 fast-follow findings landed (24 LOW + 11 INFO), with 2 deferred and
2 skipped. Cross-boundary secret sub-parts still open as follow-ups: F9a (derived DashPay encryption
keys + remembered-passphrase copy), F92b (passphrase field as `Zeroizing`/`Secret`), F3b
(`first_associated_wallet_seed_hash` delegation), F2b (passphrase-validator dedupe into `model/`,
refinery-error narrowing), and the F58 `source_address` enum-field removal.

**Out of this finding set (pre-existing)**: SEC-006 (MEDIUM, Smythe) — plaintext `SingleKeyWallet`
held in a long-lived in-memory map at `wallets_screen/mod.rs:2189`. Tracked separately.
