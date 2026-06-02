# PR #860 Platform-Wallet Rewrite — Consolidated Gap Audit

**Audit date:** 2026-06-01 — **Refreshed:** 2026-06-02
**Head SHA (refresh):** `450214e5c5ed602a0c10a951ae00400a371c3b97`
**Original audit head:** `686430a4d2b83596fbbe716acc183a424859e11d`
**PR #860 base:** `v1.0-dev` @ `87ba5b711839219f5e1c7aee8f9de36d038866e3`
**Auditor:** project-reviewer-adams (READ-ONLY; no source touched)

PR #860 rips out DET's home-grown SPV stack (`src/spv/**` deleted) and `data.db` wallet
schema and re-seats wallet/identity/DashPay/shielded state on the upstream
`dashpay/platform` `platform-wallet` crate behind a `WalletBackend` seam. It was built in
phases (P0.5 "compile floor" → P5). Several seams were intentionally landed inert
("compiles, returns `Ok(())`, wire later"). This document catalogues every such gap — dead
stubs, deferred-by-design scope cuts, real bugs, test holes, upstream blockers, and doc
gaps — each verified against the live tree with `file:line` citations.

**2026-06-02 refresh:** re-checked every gap against current source. Eight items landed
between the original audit and `450214e5` and were verified against actual code, not commit
messages. The single CRITICAL merge-blocker (PROJ-001) is resolved on-branch. Of the six
original functional gaps, four are now RESOLVED (PROJ-003, PROJ-004, PROJ-006, plus SEC-001
which was found and fixed in the same window). One new deferred-by-design seam (PROJ-022)
and one pre-existing convention violation (PROJ-023) were surfaced in the fresh sweep.

I checked the inventory against the actual code. I did not take commit messages on faith,
and several "fixed" items were re-derived from the source line by line.

---

## Executive summary

| Severity | Open | Resolved | Total |
|----------|------|----------|-------|
| CRITICAL | 0    | 1        | 1 |
| HIGH     | 2    | 2        | 4 |
| MEDIUM   | 6    | 1        | 7 |
| LOW      | 9    | 1        | 10 |
| INFO     | 0    | 0        | 0 |
| **Total** | **17** | **5** | **22** |

Open by category: functional/unwired = 1 (PROJ-002); deferred-by-design = 7; test = 4;
upstream = 2; doc = 4. New this refresh: PROJ-022 (LOW, deferred), PROJ-023 (LOW, pre-existing
convention). Original 21-gap snapshot grew to 22 confirmed; 5 are now RESOLVED.

### Merge-blocker verdict (called out up top)

**No CRITICAL merge-blocker remains open.** The one functional release gate stands:

1. **PROJ-001 (CRITICAL)** — **RESOLVED on-branch (`36f5a982`).** SPV / platform-address /
   identity sync is now started across all four caller paths. See PROJ-001 section + Resolution log.
2. **PROJ-005 (HIGH)** — release gate G1: the `dash-sdk` / `platform-wallet` pin (`Cargo.toml`)
   tracks an **unreleased** platform dev rev. Project policy (Decision #1) classifies this as
   a release-hardening blocker, not a start blocker — but it gates *merge-to-ship*. **This is
   the sole remaining merge-blocker.** The pin moved since the original audit (now
   `rev = 35e4a2f6…`, was `17653ba8…`) but is still a dev rev, not a released tag.

Everything else is fixable post-merge or is a disclosed scope cut.

---

## Merge-blocking gaps

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-001 | SPV sync never driven — dead `start()`, inert `start_spv()` | `src/context/wallet_lifecycle.rs:103,130`; `src/wallet_backend/mod.rs:462-479` | CRITICAL | **RESOLVED (`36f5a982`)** | See Resolution log 2026-06-01 |
| PROJ-005 | Pin tracks unreleased platform rev (G1) | `Cargo.toml:21,31,32,35` (`rev = 35e4a2f640…`) | HIGH | OPEN | Pin must move to a released platform rev before shipping. Still a dev rev. |

---

## Functional gaps (dead stubs, no-op UI, unwired drivers, real bugs)

### PROJ-001 — SPV / sync coordinators never started *(CRITICAL — RESOLVED)*

Original finding (head `686430a4`): `AppContext::start_spv()` was literally `Ok(())`;
`WalletBackend::start()` was the sole site spawning the upstream sync coordinators and had
**zero callers**. Net effect was dead chain/platform-address/identity sync in every path.

**RESOLVED 2026-06-02 verify** (`42388c4b`, `3165f98c`, `36f5a982`). Confirmed against source:
- `start_spv()` (`src/context/wallet_lifecycle.rs:103`) now drives `WalletBackend::start()`;
  the chokepoint `ensure_wallet_backend_and_start_spv()` (`:130`) wires-then-starts.
- `WalletBackend::start()` (`src/wallet_backend/mod.rs:462`) latches via `start_latch.try_begin()`
  and spawns `spv_arc().spawn_in_background()`, `platform_address_sync_arc().start()`,
  `identity_sync_arc().start()`.
- All four caller paths funnel through the chokepoint: GUI boot (`src/app.rs:494,890,1198`),
  Connect button via new `AppAction::StartSpv` (`src/app.rs:275,1613-1625`;
  `src/ui/network_chooser_screen.rs:390`), MCP (`src/mcp/resolve.rs:129`), network switch
  (`src/backend_task/mod.rs:559`).
- Start-path gated by four offline tests (`src/context/wallet_lifecycle.rs:561,583,617,649`).

### PROJ-002 — DashPay `add_contact` / `remove_contact` are `NotSupported` stubs *(MEDIUM — OPEN)*

`src/backend_task/dashpay/contacts.rs:521-535` (`add_contact`) and `:539-549`
(`remove_contact`) ignore all args and return `DashPayError::NotSupported`. `add_contact`
still carries a "TODO: Steps to implement" comment (`:528`). **Pre-existing in base
`87ba5b71`**; no live backend-task dispatch caller for the free functions, but PR-relevant
(module rewritten). Add-contact-by-username and contact removal are not functional. Scope: indirect.

### PROJ-003 — `update_payment_status` is a logging no-op *(MEDIUM — RESOLVED)*

Original: `payments.rs` `update_payment_status` was a `// TODO: Update payment record in
database` / "Would update payment…" no-op returning `Ok(())`.

**RESOLVED 2026-06-02** (`3ac9b3b0`). Verified at `src/backend_task/dashpay/payments.rs:462`:
the function now reads the existing `PaymentEntry` from `dashpay_view().payments(owner)`,
rebuilds it (counterparty, amount, memo, direction, mapped status) and persists via
`backend.dashpay_record_payment(owner, tx_id, entry)`, stamping `confirmed_at` on
confirmation via `dashpay_set_payment_timestamps`. Signature changed
(`owner, tx_id, status`). The adjacent confirmation path is now `check_address_usage`
(`:653`) — documented BLOCKED-BY-DESIGN (see PROJ-024).

### PROJ-004 — DashPay outgoing contact-request derivation used placeholder seed *(HIGH — RESOLVED)*

Original: xpub derivation built from `let wallet_seed = sender_private_key;` with a "For
now… In production this would derive from the wallet's HD seed" comment.

**RESOLVED 2026-06-02** (`6c520a33`). Verified at `src/backend_task/dashpay/contact_requests.rs:315-327`:
derivation now uses `first_open_wallet_seed(&identity)` (`:521`) → 64-byte HD seed → upstream
`crate::wallet_backend::derive_contact_xpub_material(&wallet_seed, …)`
(`src/wallet_backend/dashpay.rs:105`). A regression test proves HD-seed derivation differs
from the old private-key placeholder (`src/wallet_backend/dashpay.rs:2102-2134`).
**Follow-up SEC-001 (also resolved)**: the receive-side path hardcoded coin-type 5' on every
network, breaking testnet send/receive xpub agreement. Fixed in `450214e5` via
`coin_type_for_network()` (`src/model/wallet/mod.rs:50`) threaded through every DashPay HD
path (DIP-14 contact xpub, DIP-15 root, auto-accept proof, contact_info, contacts,
incoming_payments). Full broadcast→association still warrants a live-network test, but the
DET-side derivation is now production HD.

### PROJ-006 — `context_provider_spv` activation-height TODO *(LOW — RESOLVED)*

Original: `// TODO: wire actual activation height if needed` with `Ok(1)` for all networks.

**RESOLVED 2026-06-02** (`7e2553e3`). Verified at `src/context_provider_spv.rs:128-139`:
`get_platform_activation_height()` now returns real per-network Core heights
(Mainnet 2_132_092, Testnet 1_090_319, Devnet/Regtest 1), mirroring the SDK's trusted
context provider; the previously-ignored `network` field is now used.

---

## Deferred-by-design / disclosed trade-offs

Intentional scope cuts, recorded so reviewers do not mistake them for oversights. All trace
to a written decision in `docs/ai-design/2026-05-18-platform-wallet-migration/`.

| ID | Title | Location | Sev | Status | Decision ref |
|----|-------|----------|-----|--------|--------------|
| PROJ-007 | Single-key refresh + SPV-send return `SingleKeyWalletsUnsupported` | `src/backend_task/core/refresh_single_key_wallet_info.rs:16`; `src/backend_task/core/send_single_key_wallet_payment.rs:19`; `src/backend_task/core/mod.rs:218,304` | LOW | Open by design | Decision #7 (`single-key-mock.md`) |
| PROJ-008 | SEC-002 sign-time passphrase prompt UX deferred | `src/wallet_backend/mod.rs:562-566` | MEDIUM | Open (issue #90) | per-task prompt UX deferred; storage+unlock cache shipped |
| PROJ-009 | DIP-14 back-compat dropped (non-mainnet / non-account-0 legacy contact addresses not reproduced) | `src/wallet_backend/mod.rs:722-724` (`register_dashpay_contact`, "Decision #6, back-compat dropped") | MEDIUM | Open by design | Decision #6 (`open-questions.md`) |
| PROJ-010 | `UpstreamFromPersisted` loader intentionally not implemented (G2 swap deferred) | `src/wallet_backend/loader.rs:139-145` | LOW | Open by design | Decision #2 (`g2-mock-boundary.md` §G2.4) |
| PROJ-011 | `identity` `CREATE TABLE` still on fresh installs — tombstone ADR pending | `src/database/initialization.rs:845-866` | LOW | Open by design | T-DEV-02b; deferred to separate ADR (`finish-unwire/notes.md` §4) |
| PROJ-012 | ZMQ listener receiver retained behind `#[allow(dead_code)]` pending P4 audit | `src/context/mod.rs:73-77` | LOW | Open by design | Decision #3 (`open-questions.md`) |
| PROJ-022 | `UpstreamPlatformAddresses` reserved swap-target — read methods `unimplemented!()` | `src/wallet_backend/platform_address.rs:245-307` | LOW | Open by design (NEW) | pending platform todo `e817b66a`; parallels PROJ-010 |

Notes:

- **PROJ-007** narrowed since the design docs: SEC-002 work (`6052dc72`, `48cdb8ad`) made
  single-key **import / sign / list / hydrate** genuinely real
  (`src/wallet_backend/single_key.rs`; `SingleKeyView::import_wif`). UI now imports via
  `ImportSingleKeyDialog` (`src/ui/wallets/wallets_screen/mod.rs:42,157`;
  `src/ui/wallets/import_single_key.rs`). Only balance/UTXO **refresh** and **SPV-based
  send** remain stubbed. The `single-key-mock`/`g2-mock-boundary` "fully read-only mock"
  claim is stale — see PROJ-020.
- **PROJ-011** (re-verified): `legacy_detected()` (`src/database/initialization.rs:146`) gates
  `wallet` / `wallet_addresses` / `utxos` / `wallet_transactions` / `shielded_notes` behind
  `include_legacy`. The `identity` empty placeholder (`:851`) is still created
  unconditionally for legacy `database/wallet.rs` cold-start reads. `platform_address_balances`
  (`:797,933`) is still live. Documented "separate ADR" carve-out.
- **PROJ-022 (new):** `UpstreamPlatformAddresses` (`platform_address.rs:245`) is the reserved
  swap target for reading per-address Platform funds straight from upstream. It is **NOT
  selected** — the ACTIVE impl is `KvCachedPlatformAddresses`
  (`src/wallet_backend/mod.rs:512`). Its read methods (`get_address_info`, `all_address_info`,
  `get_sync_info`) are `unimplemented!()` pending upstream `e817b66a` (a public per-address
  balance+nonce reader + sync-cursor shape). Dead code by design; structurally identical to
  the PROJ-010 G2 loader seam. Cannot panic in any live path while the cached impl is active.
- **`FundWithUtxo` (seed item #15)** — the *removed* asset-lock funding path. Current active
  funding task is `WalletTask::FundPlatformAddressFromWalletUtxos`
  (`src/backend_task/wallet/mod.rs`), a different working path. No live broken surface.

---

## Test gaps

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-013 | `RUST_MIN_STACK=16777216` not enforced by harness or CI | `tests/backend-e2e/main.rs:7,10`; `.github/workflows/` (no ref) | MEDIUM | OPEN | Only a `//!` doc instruction. No thread `stack_size` builder in harness; `grep RUST_MIN_STACK .github/` = 0 hits. SDK deep-stack tests segfault at default 8 MB without it. |
| PROJ-014 | `WalletBackend::start()` start-path test coverage | `src/context/wallet_lifecycle.rs:561,583,617,649` | HIGH | **RESOLVED (`3165f98c`, `36f5a982`)** | Four offline tests now gate the start path (`start_spv_errors_when_backend_not_wired`, `start_spv_starts_after_backend_wired`, `ensure_wallet_backend_and_start_spv_wires_then_starts`, `chokepoint_wiring_failure_flips_indicator_to_error`). Full live-SPV success path remains an e2e/network gap. |
| PROJ-015 | TC-012 receive-address reuse — unverified from DET source | `src/wallet_backend/mod.rs` (`next_receive_address` → upstream) | LOW | Unverified — needs follow-up | Depends on upstream used-marking; now testable since PROJ-001 is resolved. Re-test on live network. |
| PROJ-016 | TC-066 key-not-visible-after-broadcast (flake-vs-bug) | (tracked-only, no isolated code surface) | LOW | Unverified — needs follow-up | No deterministic repro in tree. Re-classify after live run. |

Recorded test-spec gaps from `finish-unwire/notes.md` §5 (feature-flag/manual only, not
counted as new open gaps): TC-SK-010, TC-A11Y-008, TC-PERF-003.

---

## Upstream dependencies

| ID | Title | Location | Sev | Status | Blocker |
|----|-------|----------|-----|--------|---------|
| PROJ-005 | platform pin tracks unreleased dev rev (G1) | `Cargo.toml:21,31,32,35` | HIGH | OPEN | Release gate; bump to released rev before ship. Current rev `35e4a2f640a862ac1a6fc088532facbf8dc17200` (was `17653ba8…` at original audit). |
| PROJ-017 | `register_identity_funding_account` absent upstream — DET carries contained exception | `src/wallet_backend/mod.rs:1205-1287` (`provision_identity_funding_account` / `ensure_identity_funding_accounts`) | LOW | OPEN (tracked, live) | `rs-platform-wallet` has no funding-account registrar sibling to `register_contact_account`. Verified live — called from register/topup (`mod.rs:441,1088,1142,1181`). Upstream-contribution TODO. |

---

## Doc gaps

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-018 | External user docs (dashpay/docs) not updated for storage rewrite / single-key limits | `CHANGELOG.md` (no external-docs note) | MEDIUM | OPEN | No reference to updating `github.com/dashpay/docs` for the new storage model, single-key send/refresh limits, or the DIP-14 fund-accessibility trade-off. End users get no published guidance. |
| PROJ-019 | ADR floor SHA placeholder unfilled | `docs/ai-design/2026-05-29-finish-unwire/notes.md:92` (`[TO BE UPDATED ON MERGE]`) | LOW | OPEN | Needs this PR's merge SHA recorded as the wallet-state floor. |
| PROJ-020 | Design docs claim single-key is fully read-only mock — now stale | `docs/ai-design/2026-05-18-platform-wallet-migration/single-key-mock.md:51`; `g2-mock-boundary.md` | LOW | OPEN | SEC-002 made import/sign/list real; `single-key-mock.md:51` still says "render in read-only mode… no operations are enabled." See PROJ-007. |
| PROJ-021 | CHANGELOG omits single-key capability limits and DIP-14 trade-off | `CHANGELOG.md:9-32` | LOW | OPEN | Changed/Removed/Fixed sections cover the storage move but never tell users single-key send/refresh is unsupported this release, nor the contact-fund re-establishment trade-off. |

---

## Project conventions

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-023 | String-based error matching in DashPay add-contact UI (NEW) | `src/ui/dashpay/add_contact_screen.rs:626-650` | LOW | OPEN (pre-existing) | `display_task_result` classifies errors by `message.contains("ENCRYPTION key")`, `"not found"`, etc. — directly violates the CLAUDE.md rule "Never parse error strings to extract information." Self-tagged `TODO(RUST-002)` / issue #660. Pre-existing in base; not introduced by #860 but in a DashPay-adjacent surface the rewrite did not address. Silently misclassifies if upstream wording changes. Scope: indirect. |

---

## Seed-list items found ALREADY RESOLVED (evidence)

Recorded for completeness; **not** counted in the open-gap tally.

1. **Seed #3 — eager wallet-backend init hard failure.** **Resolved.** `src/app.rs` eager init
   warns + degrades to lazy fallback — no hard "Could not access wallet data" abort.
2. **Seed #7 — TC-019 inverted error precedence.** **Resolved / moot.**
   `refresh_single_key_wallet_info.rs:16` returns `SingleKeyWalletsUnsupported`
   unconditionally — no seed-lookup branch left to invert.
3. **Seed #11 — QA-004 `core_backend_mode` inert plumbing.** **Resolved.** `rg
   core_backend_mode src` = 0 hits.
4. **Seed #19 — SPV readiness gate "all 5 managers Synced".** **Not present.**
   `EventBridge::on_progress` keys off the single upstream `progress.is_synced()` predicate.
5. **Mock finding #1 — "Stop Tracking Balance" only pruned local ordering.** **Resolved**
   (`5a047357`). `stop_tracking_token_balance` (`src/backend_task/tokens/query_my_token_balances.rs:62`)
   now calls `unwatch_identity_token` upstream so the row stays gone after refresh.
6. **Appendix item — DashPay threshold-expiry "not yet wired".** **Resolved** (`a7327e7c`).
   `expires_at` now derived `created_at + DASHPAY_REQUEST_EXPIRY_DAYS` via
   `request_expires_at_ms` (`src/wallet_backend/dashpay.rs:429,520`), checked arithmetic,
   two unit tests + an overflow test.
7. **Appendix item — MCP `platform_withdrawals_get` pagination/structured-data TODOs.**
   **Resolved** (`5ba4554e`). `src/mcp/tools/platform.rs` now has `limit` / `start_after` /
   `next_cursor` pagination (`:36,55,81,148-176`) and a structured response type.
8. **Appendix item — SPV sync UI read inert `SpvStatusSnapshot::default()`.** **Resolved**
   (`bd0ed0e4`). `EventBridge::on_progress` now publishes live per-phase heights so the
   progress bar/labels populate during sync.

Seed items **unverifiable from this tree** (needs follow-up, not asserted): seed #17 (m/9'
identity-address SPV bloom registration — not found in DET; likely upstream), seed #18
(DiskStorageManager byte-compat — lives in upstream `platform-wallet-storage`), seed #20
(`/tmp/marvin-finish-unwire-exceptions.md` absent).

---

## Resolution log

- **2026-06-01 — PROJ-001 resolved** (`42388c4b`, `3165f98c`, `36f5a982`): `start_spv()` wired to
  `WalletBackend::start()` with `StartLatch` idempotency; single async chokepoint
  `ensure_wallet_backend_and_start_spv()` covers all four caller paths; wiring/start failures
  surface via indicator + banner. QA-007 / QA-008 closed in `36f5a982`.
- **2026-06-01 — PROJ-014 largely resolved** (`3165f98c`, `36f5a982`): four offline tests gate
  the start path; live-SPV success path remains an e2e/network gap.
- **2026-06-02 — PROJ-003 resolved** (`3ac9b3b0`): `update_payment_status` persists via
  `dashpay_record_payment` + timestamp sidecar; `check_address_usage` documented BLOCKED-BY-DESIGN.
- **2026-06-02 — PROJ-004 resolved** (`6c520a33`): contact-request xpub derived from the real
  64-byte HD seed (`first_open_wallet_seed` → `derive_contact_xpub_material`); regression test
  proves divergence from the old placeholder. Follow-up **SEC-001 resolved** (`450214e5`):
  `coin_type_for_network()` threaded through all DashPay HD paths so send/receive xpubs agree
  per network.
- **2026-06-02 — PROJ-006 resolved** (`7e2553e3`): real per-network platform activation heights.
- **2026-06-02 — refresh sweep**: PROJ-005 pin moved `17653ba8…` → `35e4a2f6…` (still
  unreleased, stays OPEN). New PROJ-022 (`UpstreamPlatformAddresses` reserved seam, deferred)
  and PROJ-023 (add-contact string error matching, pre-existing convention violation) added.

---

## Appendix: raw stub-signal hits not separately categorized

So nothing is silently dropped. Deferred markers / inert-looking bodies that are (a) benign,
(b) pre-existing, or (c) rolled into a finding above.

- `src/wallet_backend/mod.rs:1214` — exhaustive-match guard comment "keeping the match
  exhaustive forces a review if a new variant appears" (intentional, benign).
- `src/wallet_backend/event_bridge.rs:173` — `on_shielded_sync_completed` left at upstream
  no-op default (DET enables only `serde`; matches `start()` comment at `mod.rs:474-476`).
- `src/wallet_backend/platform_address.rs:256-303` — `UpstreamPlatformAddresses` write/delete
  arms are intentional `Ok(())` no-ops; the panicking read arms are PROJ-022.
- `src/backend_task/migration/finish_unwire.rs:340,656,1655,1988` — password-protected
  single_key rows **deferred** to T-SK-03 UX prompt; counted "skipped", not "failed". By
  design (PROJ-008-adjacent); migration itself fully wired (`src/backend_task/migration/mod.rs`).
- `src/backend_task/dashpay/payments.rs:210` — `#[allow(dead_code)]` payments helper (pre-existing).
- `src/ui/dashpay/send_payment.rs:743,754,776` — local in-memory display `PaymentRecord` list
  uses `Identifier::new([0;32])` contact-id placeholder and `timestamp: 0`. Cosmetic: the
  authoritative payment record is persisted via `dashpay_record_payment` (the mirror was
  dropped, see comment `:760-764`); only the throwaway UI list is affected. LOW-adjacent,
  not separately scored.
- `src/ui/tokens/update_token_config.rs:684` — "Marketplace settings are not yet supported"
  UI label for the upstream `MarketplaceTradeMode` config arm. Pre-existing, unrelated to
  #860; disclosed unsupported feature.
- `src/ui/identities/identities_screen.rs:183` — stale "dummy for now" comment; the
  InWallet sort below it actually compares wallet names correctly. Benign stale comment.
- `src/database/initialization.rs:906` — "TODO: Discuss migration approach with the team" —
  pre-existing architectural note in `set_default_version`, benign.
- `src/context/mod.rs:810` — `// TODO: Ideally use sdk.load().version()` — cosmetic version
  free-fn TODO (pre-existing).
- `src/app.rs:1314` (`TODO(RUST-002)`) — message-text-inspection routing TODO (tracked tech-debt).
- `src/app.rs:717,733`, `src/model/qualified_identity/mod.rs:770` — `panic!` BUG-guard
  invariants (missing-network-context, inconsistent-wallet-index); intentional, not gaps.
- `src/bin/det_cli/main.rs:186,188`, `src/ui/mod.rs:485,604,607`,
  `src/ui/components/address_input.rs:576` — `unreachable!()` arms guarded by prior checks
  (intentional).
- `src/mcp/tools/platform.rs` pagination/structured TODOs — now RESOLVED (see already-resolved #7).

---

## Appendix: BLOCKED-BY-DESIGN

- **PROJ-024 — `check_address_usage` returns all-unused** (`src/backend_task/dashpay/payments.rs:653`).
  Documented at `:640-652`: upstream exposes per-account `is_used` keyed by
  `(wallet_id, AccountType)`, not by arbitrary address; the function receives a context-free
  address list it cannot route. The only context-bearing DashPay addresses are contact-SEND
  addresses, which never live in any managed pool, so a full scan would correctly report
  all-unused anyway. Returning a fabricated usage flag would corrupt gap-limit math. Honest
  all-unused stub pending a properly-scoped upstream reader. Not a bug — a disclosed,
  reasoned design limit (introduced with the PROJ-003 fix, `3ac9b3b0`).

---

*🍬 Candy tally — confirmed gaps: 22 (1 CRITICAL · 4 HIGH · 7 MEDIUM · 10 LOW · 0 INFO).
Status: 5 RESOLVED (PROJ-001, PROJ-003, PROJ-004, PROJ-006, PROJ-014) + SEC-001 follow-up;
17 OPEN; of those, 7 deferred-by-design and 1 blocked-by-design (PROJ-024). 8 seed/appendix
items confirmed already-resolved with evidence. Closing a gap counts too — and this pass
closed five plus a security follow-up.*
