# PR #860 Platform-Wallet Rewrite — Consolidated Gap Audit

**Audit date:** 2026-06-01 — **Refreshed:** 2026-06-10 — **Triage pass:** 2026-06-11
**Head SHA (refresh):** `a0d5034a0b573847b0786e3d538a335ef57e1281`
**Prior refresh heads:** `954ea3f8` (2026-06-08), `39e459ff`
**Original audit head:** `686430a4d2b83596fbbe716acc183a424859e11d`
**PR #860 base:** `v1.0-dev` @ `87ba5b711839219f5e1c7aee8f9de36d038866e3`
**Baseline (expanded 2026-06-10):** `origin/v0.10-dev` @ `b93b6b17` ≈ `v1.0-dev` — the
2026-06-10 pass swept **every** v0.10-dev user-facing feature for parity, not just the
seeded stub inventory. Four domain audits (wallets/core, identity/DPNS/contracts,
DashPay/tokens/shielded, MCP/settings/withdrawals) were consolidated into this document.
**Auditor:** project-reviewer-adams (READ-ONLY; this refresh touches gaps.md only)

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

**2026-06-02 disposition update:** PROJ-002 (dead `add_contact` / `remove_contact` free
functions + orphaned `NotSupported` variant) is now RESOLVED — removed by a sibling commit.
PROJ-012 was re-filed: it was mis-scoped as a benign deferred-by-design seam, but the source
shows a live wiring gap (ZMQ connection-health events flow into a void), so it moved to the
functional-gaps section and was bumped LOW → MEDIUM.

**2026-06-03 refresh:** five more items verified RESOLVED against source at `f39b085d`,
each re-derived line by line (not taken on commit-message faith):

- **PROJ-008** (sign-time passphrase prompt UX, issue #90) — RESOLVED. The JIT secret-access
  refactor (commit range `2272bae0..43f412cf`, 24 commits) landed a per-secret prompt seam:
  `src/wallet_backend/secret_prompt.rs` (the `SecretPrompt` trait + `SecretScope` /
  `RememberPolicy` / retry types), `src/ui/components/secret_prompt_host.rs` (egui host), and
  `src/ui/components/passphrase_modal.rs` (modal). The passphrase is requested just-in-time per
  operation, keyed by scope (`HdSeed { seed_hash }` / `SingleKey { address }`), with an
  optional "keep unlocked until app close" remember policy. The HD seed is decrypted on demand
  and dropped immediately.
- **PROJ-013** (large-stack e2e harness) — RESOLVED (`2a9161d3`, `93a20769`). 32 MB-stack runtime
  confirmed load-bearing; `#[stack_size]` rejected (recursion runs on tokio threads); `RUST_MIN_STACK` moot.
- **PROJ-020 / PROJ-021** (single-key-mock prose / CHANGELOG known-limits) — RESOLVED (`f39b085d`).
- **PROJ-023** (add-contact string error matching) — RESOLVED (`d852ce99`). A sibling
  occurrence of the same anti-pattern survives in `contact_requests.rs` and is filed as the
  new PROJ-025.

PROJ-018 / PROJ-019 stay OPEN (partials that can only close at/after merge). PROJ-005 remains
the sole open merge-blocker. The tally below is recomputed **from the actual body entries**,
which are the verifiable ground truth.

**2026-06-10 feature-parity pass (head `a0d5034a`):** four domain agents diffed the full
v0.10-dev feature surface against the working tree; every finding below was re-verified
against live source before recording. Net adds: **3 HIGH** (PROJ-026 asset-lock QR
soft-lock, PROJ-027 incoming-DashPay-payment detection gone, PROJ-028 nullifier-cursor
unit-mismatch **regression introduced by the 2026-06-10 #3828 re-pin**, commits
`4247c360`+`a0d5034a`), **7 MEDIUM** (PROJ-029..034, DOC-001), **9 LOW**
(PROJ-035..041, DOC-002, DOC-003). Corrections: PROJ-012 re-scoped (the *whole* ZMQ chain
is dead, not just health events), PROJ-007 extended + bumped LOW→MEDIUM (password-protected
single-key wallets silently vanish post-upgrade), PROJ-009 flagged incomplete
(`register_dashpay_contact` has zero callers — see PROJ-027), seed #15 cross-referenced to
the deleted QR UI tabs, and the disclosed-removal set is now itemised (RecoverAssetLocks,
ListCoreWallets, SPV peer source, Proof Log).

---

## Executive summary

| Severity | Open | Resolved | Total |
|----------|------|----------|-------|
| CRITICAL | 0    | 1        | 1 |
| HIGH     | 1    | 6        | 7 |
| MEDIUM   | 4    | 11       | 15 |
| LOW      | 5    | 15       | 20 |
| INFO     | 0    | 0        | 0 |
| **Total** | **10** | **33** | **43** |

Open by category: upstream/release-gate = 1 (PROJ-005);
functional/unwired = 1 (PROJ-041);
deferred/partial = 2 (PROJ-007 PARTIAL, PROJ-022 accepted);
deferred-with-TODO = 1 (PROJ-034 OPEN — confirmed real data loss per v0.9.3 cross-check);
test = 2 (PROJ-015, PROJ-016);
doc = 3 (PROJ-018, PROJ-019, DOC-003 deferred-with-TODO).
Sum = 10 open (PROJ-032 CLOSED/N-A: DashPay was never persisted in any v0.9.3 release — net count unchanged, PROJ-032 removed from open and added to resolved).
(2026-06-15: PROJ-042's platform-address half moved PARTIAL → RESOLVED in PR — wallet-owned
destinations now route through the upstream orchestrator; non-owned destinations keep the
manual path by design. The finding was already in the resolved tally for its actionable
shield-half fix, so the counts are unchanged. Follow-up same day: the fee-from-wallet residual
on owned in-pool destinations is now CLOSED — that case also routes through the orchestrator
using an in-pool, watched platform-payment change recipient. Only the by-design
foreign-destination footgun and a narrow no-spare-change edge remain on the manual path; counts
unchanged.)
(2026-06-15: PROJ-043 moved OPEN → RESOLVED in PR — the four sibling shielded spend fns now
propagate the post-broadcast confirmation failure as dedicated typed `*ConfirmationUnknown`
errors via `?` instead of swallowing it, so the caller no longer marks notes spent (or bumps
the platform-address nonce) on an unverified spend. LOW open 6 → 5; total open 11 → 10.)
(2026-06-11 triage pass: 17 findings moved to RESOLVED/WONTFIX — PROJ-026/027/029/031/040/017/012/033/011/035/036/037/038/039/009/DOC-001/DOC-002;
PROJ-007 PARTIAL; PROJ-022 accepted-risk. See Resolution log.)

(Note: the pre-2026-06-10 table under-counted HIGH at 3/13-LOW — PROJ-010 (HIGH) had been
mis-bucketed as LOW. Recomputed from body entries: 1 CRITICAL + 4 HIGH resolved-or-open
existing + the new findings above.)

### Merge-blocker verdict (called out up top)

**No CRITICAL merge-blocker remains open.** The release gate stands, joined by two open
HIGH parity regressions that should be fixed before ship (PROJ-028 RESOLVED 2026-06-10):

1. **PROJ-001 (CRITICAL)** — **RESOLVED on-branch (`36f5a982`).** SPV / platform-address /
   identity sync is now started across all four caller paths. See PROJ-001 section + Resolution log.
2. **PROJ-005 (HIGH)** — release gate G1: the `dash-sdk` / `platform-wallet` pin (`Cargo.toml`)
   tracks an **unreleased** platform dev rev. Project policy (Decision #1) classifies this as
   a release-hardening blocker, not a start blocker — but it gates *merge-to-ship*. The pin
   moved again — now `rev = 4f432c9baf10eeb051e70bc0370b1b7505b7d9c5` (the 2026-06-10 re-pin
   to dashpay/platform#3828, `4247c360`; was `9e1248cb…`, `ddfa66ed…`, `35e4a2f6…`,
   originally `17653ba8…`) — still a dev rev, not a released tag.
3. **PROJ-026 / PROJ-027 (HIGH, 2026-06-10)** — **RESOLVED 2026-06-11**: PROJ-026 (`fe01febb`
   + `26c13385`) and PROJ-027 (`910f8833` + `dc94bba6`). The third sibling, **PROJ-028**
   (#3828 re-pin's shielded nullifier-cursor unit mismatch), was **RESOLVED 2026-06-10**
   (`39433dac`). No open HIGH parity regressions remain.

Everything else is fixable post-merge or is a disclosed scope cut.

---

## Merge-blocking gaps

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-001 | SPV sync never driven — dead `start()`, inert `start_spv()` | `src/context/wallet_lifecycle.rs:103,130`; `src/wallet_backend/mod.rs:462-479` | CRITICAL | **RESOLVED (`36f5a982`)** | See Resolution log 2026-06-01 |
| PROJ-005 | Pin tracks unreleased platform rev (G1) | `Cargo.toml:21,31,32,35` (`rev = 9e1248cb…`) | HIGH | OPEN | Pin must move to a released platform rev before shipping. Still a dev rev. |

(PROJ-017 — `register_identity_funding_account` absent upstream — **RESOLVED 2026-06-11** (`26675766`): load() comments scoped to per-index top-up; upstream-contribution path documented in code. Moved from this table.)

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

### PROJ-002 — DashPay `add_contact` / `remove_contact` dead free functions *(MEDIUM — RESOLVED (removed))*

Original finding: `src/backend_task/dashpay/contacts.rs` carried two free functions —
`add_contact` and `remove_contact` — that ignored all args and returned
`DashPayError::NotSupported`, with a stale "TODO: Steps to implement" comment.

**RESOLVED (removed 2026-06-02, sibling commit).** These were dead orphans from PR #464
(`82399a26`): they had **zero callers** in the live tree and were superseded by the real
`DashPayTask::SendContactRequest` dispatch path. The sibling commit deletes both free
functions from `src/backend_task/dashpay/contacts.rs` along with the now-orphaned
`DashPayError::NotSupported` variant in `src/backend_task/dashpay/errors.rs` — the variant
had no other producers once the stubs were gone. No functional surface is lost: there was no
backend-task dispatch wired to either function. (Removal SHA omitted; the lead will reconcile
against the actual sibling commit if needed.)

### PROJ-012 — entire ZMQ subsystem is dead code; "Disable ZMQ" checkbox is a placebo *(MEDIUM — RESOLVED 2026-06-11; `255aa018` + `23b81718`)*

**RESOLVED 2026-06-11** (`255aa018` + `23b81718`): the dead ZMQ subsystem (listener, channel pair, placebo checkbox, dead consumption loop), the Dash-Qt launcher, and the legacy identity table were all removed. CHANGELOG updated in `23b81718`. Original finding follows.

**Correction (2026-06-10):** the earlier description ("the ZMQ status producer is live but
health events flow into a void") was stale — the *whole* producer→consumer chain is dead,
not just the health-event leg:

- **Listener never spawns:** `spawn_zmq_listener` returns `None` before spawning because
  `FeatureGate::RpcBackend.is_available()` is hardcoded `false`
  (`src/app.rs:845-847`; `src/model/feature_gate.rs:78` — "never active … retained as a
  gate so RPC/ZMQ-only UI is hidden"). The `sx_zmq_status` clone at `src/app.rs:852` is
  therefore unreachable; no `Connected` / `Disconnected` event is ever produced.
- **Unread receiver:** `rx_zmq_status` (`src/context/mod.rs:79`) is stored on `AppContext`
  (`:348`) but never drained — no `recv` / `try_recv` anywhere in the tree.
- **Zero-caller setter:** `ConnectionStatus::set_zmq_status`
  (`src/context/connection_status.rs:159`) has zero callers.
- **Dead consumption loop:** the islock/chainlock CoreItem consumption loop
  (`src/app.rs:1502-1540`) can never receive events.
- **Placebo setting:** the "Disable ZMQ (requires restart)" checkbox is still rendered and
  persisted (`src/ui/network_chooser_screen.rs:764-775`; `AppSettings.disable_zmq`,
  `src/model/settings.rs:72`) but controls nothing (was GAPCMP-D-04).

The channel pair is constructed as a unit at `src/context/mod.rs:204`. The binary fix is
unchanged in shape but wider in scope: either **wire** the chain when/if an RPC backend
returns, **or remove** the whole producer → channel → setter chain *plus* the placebo
checkbox and the dead consumption loop. This also feeds PROJ-026: with the listener gated
off, no `CoreItem` event producer exists at all.

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

### PROJ-026 — Create-Asset-Lock QR funding flow soft-locks at "Waiting for funds…" *(HIGH — RESOLVED 2026-06-11; `fe01febb` + `26c13385`)*

**RESOLVED 2026-06-11** (`fe01febb` + QA fix `26c13385`): `ReceivedAvailableUTXOTransaction` is now emitted from the core module so the asset-lock funding flow advances on fund arrival. The QA fix also reserves Max against spendable balance. Original finding follows.

- **v0.10-dev:** `src/ui/wallets/create_asset_lock_screen.rs:519` (OLD) polled
  `funding_common::capture_qr_funding_utxo_if_available` (OLD
  `src/ui/identities/funding_common.rs:56-94`) every frame; the helper watched
  `wallet.utxos` at the displayed funding address and flipped the step machine
  `WaitingOnFunds → FundsReceived`, which dispatched
  `CoreTask::CreateRegistrationAssetLock` / `CreateTopUpAssetLock`.
- **PR #860:** the poll and helper are deleted. The **only** remaining
  `WaitingOnFunds → FundsReceived` transition is the `display_task_result` arm matching
  `CoreItem::ReceivedAvailableUTXOTransaction`
  (`src/ui/wallets/create_asset_lock_screen.rs:650-666`, flip at `:659`) — and that
  `CoreItem` variant has **zero producers** in the tree (`src/backend_task/core/mod.rs:106`
  is definition-only; `received_transaction_finality` always returns `Ok(Vec::new())`,
  `src/context/transaction_processing.rs:17-30`; the ZMQ listener that once produced
  CoreItems never spawns — see PROJ-012). The screen is reachable: Asset Locks tab →
  "Create Asset Lock" (`src/ui/wallets/wallets_screen/asset_locks.rs:69`). Net effect:
  after entering an amount, the user is shown a QR + "Waiting for funds..."
  (`create_asset_lock_screen.rs:558-560`) **forever** — even after they send real DASH to
  the shown address. Funds are recoverable (they land in the wallet's normal SPV balance);
  only the flow dead-ends, after a real payment was solicited.
- **Fix direction:** detect the arriving UTXO from the `WalletBackend` snapshot at the
  funding address (the pattern `add_new_wallet_screen.rs` already uses via
  `snapshot_balance`), or emit the event from `EventBridge`.
- **Related dead code:** sibling match arms on the same producer-less `CoreItem` survive in
  `src/ui/identities/add_new_identity_screen/mod.rs:1144` and
  `src/ui/identities/top_up_identity_screen/mod.rs:488` — unreachable but harmless there,
  since the identity QR-funding tabs were removed (seed #15).
- **Disclosure status:** NOT covered by the CHANGELOG "QR-code wallet import flow" removal
  line — this screen kept the QR UI and lost only the detection plumbing.

### PROJ-027 — Incoming DashPay contact payments are never detected, recorded, or credited *(HIGH — RESOLVED 2026-06-11; `910f8833` + `dc94bba6`)*

**RESOLVED 2026-06-11** (`910f8833` + `dc94bba6`): incoming contact payment detection wired and recording implemented; per-output payment keying and related QA fixes in `dc94bba6`. Original finding follows.

- **v0.10-dev:** the ZMQ tx-finality path auto-detected payments to DashPay contact receive
  addresses, credited the UTXO, advanced the receive index, and recorded a "received"
  payment-history row — OLD `src/context/transaction_processing.rs:183` (`insert_utxo`),
  `:192` (`add_to_address_balance`), `:213` (`get_dashpay_address_mapping`), `:219`
  (`update_highest_receive_index`), `:227` (`save_payment(..., "received")`), driven from
  OLD `src/app.rs:1170,1188`.
- **PR #860:** the detection chain exists only as dead code with **zero callers**:
  `process_incoming_payment` (`src/backend_task/dashpay/incoming_payments.rs:349`) and its
  callee `mirror_incoming_payment_to_backend` (`src/backend_task/dashpay/payments.rs:592`)
  are never invoked. The new `received_transaction_finality`
  (`src/context/transaction_processing.rs:17-30`) handles asset-lock waiters only; the
  upstream `EventBridge` has no DashPay hook (`src/wallet_backend/event_bridge.rs:121-160`).
  `RegisterDashPayAddresses` writes contact receive addresses only into DET's **in-memory**
  wallet model (`incoming_payments.rs:298-314`), which nothing feeds into the upstream SPV
  watch set; and `WalletBackend::register_dashpay_contact` (`src/wallet_backend/mod.rs:1250`)
  — the seam meant to register contact accounts upstream (Decision #6) — has **zero
  callers**.
- **User impact:** funds a contact sends arrive at addresses DET no longer watches —
  invisible in wallet balance and absent from payment history, on **all** networks and
  accounts. Strictly wider than the disclosed PROJ-009 DIP-14 carve-out
  (non-mainnet/non-account-0 only). Funds are not destroyed (derivable from seed) but are
  invisible/unspendable in this version. Contradicts the design docs, which state
  incoming-payment detection is retained DET-side
  (`docs/ai-design/2026-05-18-platform-wallet-migration/backendtask-contract.md:46`,
  `open-questions.md:56`).
- **Fix direction:** call `register_dashpay_contact` when contacts are established (so
  upstream watches the DIP-15 account) and wire a finality/EventBridge hook to
  `process_incoming_payment`, or delegate detection fully upstream and mirror results.

### PROJ-028 — Shielded nullifier-cursor unit mismatch: #3828 re-pin regression breaks spend detection for migrated wallets *(HIGH — RESOLVED; 2026-06-10, was GAPCMP-C-2)*

**RESOLVED 2026-06-10** (`39433dac`; doc follow-up this commit). The migration no longer
carries the legacy block-height cursor: `finish_unwire.rs` now resets
`last_nullifier_sync_height` to 0 for migrated wallets (`SELECT … 0, 0`), so the next scan
re-walks the note tree from position 0 and re-derives the spent set. `tc_sh_002` flipped to
assert the reset; new `migrated_cursor_reset_lets_scan_flip_spent_note` pins the end-to-end
fix. Smythe QA: SHIP, funds-safe (reset-to-0 only flips notes to spent and is idempotent).

**This is a regression introduced on-branch by the 2026-06-10 #3828 re-pin**
(`4247c360` + `a0d5034a`), NOT a pre-existing platform-wallet gap. It post-dates the
2026-06-08 refresh. Cross-link: follow-up todo `1ff97ad7` (post-#3828 QA follow-ups).

- **v0.10-dev (and pre-re-pin) semantics:** `src/backend_task/shielded/nullifiers.rs:37-80`
  (OLD) called `sdk.sync_nullifiers(&unspent_nullifiers, …, NullifierSyncCheckpoint { height, … })`
  where `new_sync_height` was a **platform block height** (rs-sdk
  `nullifier_sync/types.rs:104-117` @ `9e00c8b`), and the API re-checked the full
  unspent-nullifier list with an internal full-rescan fallback.
- **PR #860 (post-re-pin):** the port reinterprets the **same persisted column**
  `last_nullifier_sync_height` as a note-tree **position**:
  `src/backend_task/shielded/nullifiers.rs:48`
  (`aligned_start = (last_nullifier_sync_height / CHUNK_SIZE) * CHUNK_SIZE`), scanned via
  `sync_shielded_notes(…, aligned_start, None)`; the cursor is re-written as
  `aligned_start + total_notes_scanned` (`:84-86`). The T-SH-02 migration copies the legacy
  value **verbatim** (`src/backend_task/migration/finish_unwire.rs:720-735`), and the test
  `tc_sh_002_sync_cursor_preserved` (`:1988`) asserts `1_234_567` is preserved — the test
  enshrines the bug. The re-pin commit message claims "the resume cursor is preserved";
  preserving it is precisely the defect.
- **User impact:** any wallet that ran nullifier sync under the old semantics carries a
  block-height-scale cursor ≫ note-tree size. The scan starts past the tree tip, returns
  zero notes, detects no spends, and re-persists the same bogus cursor — permanently.
  Spent notes stay "unspent": shielded balance overstated, spends select burned notes and
  fail at consensus. No self-heal, no surfaced error.
- **Fix direction:** version the cursor (reset to 0 / re-base when a legacy block-height
  value is detected — e.g. one-time reset during T-SH-02, or a sidecar `cursor_kind`
  column), and fix `tc_sh_002` to assert the *re-based* value.

### PROJ-029 — "Subtract fee from amount" / Max button are guaranteed-error paths for Core sends *(MEDIUM — RESOLVED 2026-06-11; `918b8e5f` + `26c13385`)*

**RESOLVED 2026-06-11** (`918b8e5f` + `26c13385`): client-side Max for Core sends implemented; unsupported subtract-fee option removed from UI. Original finding follows.

- **v0.10-dev:** subtract-fee was implemented in both send backends (RPC
  `build_multi_recipient_payment_transaction(..., subtract_fee_from_amount)`; SPV
  amount-scaling fallback on `BuilderError::InsufficientFunds`).
- **PR #860:** `src/backend_task/core/mod.rs:266-268` hard-rejects
  `subtract_fee_from_amount || override_fee.is_some()` with typed
  `TaskError::WalletPaymentOptionUnsupported` (deliberate "no silent ignore", comment
  `:255-265`). BUT the UI still offers the option: checkbox at
  `src/ui/wallets/send_screen.rs:2405-2411` and `src/ui/wallets/wallets_screen/dialogs.rs:220`
  (sent at `:1116`), and the **Max button auto-enables it**
  (`send_screen.rs:2383-2389` — "When Max is clicked for Core wallet, automatically enable
  subtract_fee"). "Send Max" from a Core wallet and any ticked-checkbox send therefore
  always fail. No funds at risk (rejected before broadcast); the lost capability
  (send-max / fee-from-amount) plus the guaranteed-dead-end affordance is the gap. Not in
  CHANGELOG Known Limitations.
- **Fix direction:** remove/disable the checkbox and the Max auto-enable for Core sends
  until upstream `send_to_addresses` supports fee deduction, and disclose the limitation.

### PROJ-030 — Shielded "Resync Notes" keeps the nullifier watermark — previously-spent notes resurrect as unspent *(MEDIUM — RESOLVED; 2026-06-10, was GAPCMP-C-3)*

**RESOLVED 2026-06-10** (`39433dac`; doc follow-up this commit). The resync handler
(`shielded_tab.rs`) now calls `delete_shielded_wallet_meta` alongside the note drop, resetting
the nullifier cursor to 0 so a resync re-derives the spent set from position 0. Regression test
`resync_sequence_resets_nullifier_cursor` pins the two-call sequence. Smythe QA: SHIP.

- **v0.10-dev:** resync also kept the checkpoint, but the old API re-checked the explicit
  unspent-nullifier set (with full-rescan fallback), so historical spends were re-flagged.
- **PR #860:** the resync handler deletes notes + the commitment tree but **not** the
  cursor (`src/ui/wallets/shielded_tab.rs:744-772` — no `delete_shielded_wallet_meta`
  call; that function is only invoked on wallet removal, `src/wallet_backend/mod.rs:806`).
  Under the new scan-from-watermark model (PROJ-028, `nullifiers.rs:48`), rebuilt notes
  come back `is_spent: false` and `check_nullifiers` scans only `[old tip, tip]`, missing
  every historical spend → balance includes previously-spent notes; spends fail.
- **Fix direction:** one call — invoke `delete_shielded_wallet_meta` (it exists and is
  tested: `src/wallet_backend/shielded.rs:255,738`) in the resync handler. Same root
  family as PROJ-028.

### PROJ-031 — Shield-from-Core silently ignores the selected source address (coin control lost; UI and MCP misleading) *(MEDIUM — RESOLVED 2026-06-11; `08c895a8` + `26c13385`)*

**RESOLVED 2026-06-11** (`08c895a8` + `26c13385`): unsupported source-address selection removed from the shield UI and MCP tool. Original finding follows.

- **v0.10-dev:** OLD `src/context/shielded.rs:613-625` threaded `source_address` into
  asset-lock UTXO selection — shielding could be restricted to one Core address.
- **PR #860:** dispatch discards it — `src/context/shielded.rs:242-252`
  (`source_address: _`, "coin selection is delegated to the upstream wallet's …
  live UTXO set"). Yet the UI still lets the user pick a specific Core address, shows
  "Available address balance" for it and validates the amount against that address
  (`src/ui/wallets/shield_screen.rs:786-812`), then sends `source_address: None`
  (`:974-985`); the MCP tool `shielded_shield_from_core` still accepts and silently drops
  the param (`src/mcp/tools/shielded.rs:33,86-108`).
- **User impact:** privacy-expectation violation in a privacy feature — the asset lock may
  link UTXOs from addresses the user explicitly tried to exclude; amount caps are computed
  against the wrong balance. Not in CHANGELOG Known Limitations.
- **Fix direction:** if per-address selection cannot be honored upstream, remove the
  selector/per-address balance display and the MCP param, and disclose; otherwise thread
  the parameter through.

### PROJ-032 — Legacy DashPay user data not migrated: payment history, nicknames/notes/hidden flags, send-address indices *(MEDIUM — CLOSED / RESOLVED-N-A)*

**CLOSED 2026-06-16 — precondition never true in any shipped release.**

v0.9.3 is the latest release. Cross-check of the v0.9.3 source tree confirms that DashPay
was a literal "Coming Soon" placeholder (`dashpay_coming_soon_screen.rs`). **Zero DashPay
tables or persistence code ever shipped in any DET release.** There is no user data to
migrate, for any user on any release. The TODO comment that anchored this finding
(`src/backend_task/migration/finish_unwire.rs`) has been removed.

- **Original concern:** `src/database/dashpay.rs` (present in PR #860's pre-unwire ancestor
  branch) was assumed to represent previously-shipped persistence. It did not — the file was
  developed on the feature branch and deleted on the same branch; it never appeared in a
  released binary.
- **`data-model-and-migration.md:58`** contradiction ("DET payment-history / avatar cache
  retained DET-side") is stale design-intent prose from the pre-release plan, not a
  description of shipped behaviour. No correction needed to docs because the design-intent
  context is already marked superseded in that file.
- **No action required.** No migration needed; no CHANGELOG disclosure needed; no follow-up.

### PROJ-033 — Dash-Qt launcher unreachable while its settings cluster survives *(MEDIUM — RESOLVED 2026-06-11; `255aa018`)*

**RESOLVED 2026-06-11** (`255aa018`): Dash-Qt launcher task and settings cluster removed along with the rest of the dead ZMQ/RPC surface. Original finding follows.

- **v0.10-dev:** two UI launch paths for `CoreTask::StartDashQT` — RPC-mode Connect button
  (OLD `network_chooser_screen.rs:633-648`) and connection-indicator click (OLD
  `top_panel.rs:168-195`).
- **PR #860:** `CoreTask::StartDashQT` still exists (`src/backend_task/core/start_dash_qt.rs`)
  but has **zero UI callers** (verified: no `StartDashQT` reference outside
  `src/backend_task/core/`). Meanwhile Settings still renders the full Dash-Qt cluster:
  "Dash Core Executable Path" Select File/Clear (`src/ui/network_chooser_screen.rs:646-736`),
  "Overwrite dash.conf" (`:744`), "Close Dash-Qt when DET exits" (`:899-933`) — three
  settings configuring a feature that can never fire.
- **Fix direction:** re-wire a launch affordance (regtest/devnet workflows) or remove the
  settings cluster + task.

### PROJ-034 — App settings, top-up history, and scheduled DPNS votes all reset/empty on upgrade (no non-wallet data migration) *(MEDIUM — OPEN; deferred-with-TODO `727e8d6a`)*

**CONFIRMED REAL per v0.9.3 cross-check (2026-06-16).** v0.9.3 persisted all three:
`settings` (network, theme, custom dash_qt_path, start screen, …), `top_up` (identity_id,
top_up_index, amount), and `scheduled_votes` (identity_id, contested_name, vote_choice,
time, executed, network). Real user data is silently lost on upgrade. **Follow-up priority:
scheduled DPNS votes (vote-window deadline risk) > app settings (UX friction) > top-up
history (audit trail).**

- **v0.10-dev / v0.9.3:** settings persisted in `data.db` `settings` (network, root screen,
  dash_qt_path, theme_mode, onboarding flag, user_mode, …; OLD `src/database/settings.rs`,
  `src/model/settings.rs:28-45`); top-up history and scheduled DPNS votes in legacy SQLite.
- **PR #860:** settings moved to k/v `det:settings:v1` in `det-app.sqlite`
  (`src/model/settings.rs:63-88`; `src/context/settings_db.rs`) with **no importer**
  (`db.get_settings` has zero callers; the migration drains only
  `LEGACY_TABLES` = wallets/secrets, `src/backend_task/migration/finish_unwire.rs`).
  Commit `e4ff9621` discloses "Existing users get default AppSettings on first launch …
  migration tool will import in a later PR" — that later PR never landed. Top-up history
  and scheduled votes likewise start empty (commit `7778eb64`: "Existing users get empty
  state for all three").
- **User impact at upgrade:** selected network resets to **Mainnet** (a testnet user
  relaunches into mainnet), theme/paths/onboarding/user-mode reset; silently dropped
  **scheduled DPNS votes can mean missed votes** for masternode voters. One-time, no fund
  loss. Disclosed in commit messages only — not in CHANGELOG (feeds DOC-001).
- **Fix direction:** add a settings/k-v importer to the cold-start migration (at minimum:
  network, theme, onboarding flag, scheduled votes), or disclose prominently. Scheduled-vote
  migration is highest priority given the vote-window deadline risk.

### PROJ-042 — Non-identity asset-lock flows bypass upstream orchestration: post-broadcast recovery gap *(MEDIUM — RESOLVED IN PR: shield-from-asset-lock false-success fixed; platform-address funding now routes wallet-owned destinations through the upstream orchestrator)*

Identity asset-lock flows route through upstream `IdentityWallet::*_with_funding`, which runs
the full resolve → `submit_with_cl_height_retry` → `consume_asset_lock` pipeline. Three
**non-identity** asset-lock flows built the asset lock upstream but then submitted the Platform
transition manually, missing the post-asset-lock / pre-final-accounting recovery upstream owns:

- **`src/backend_task/shielded/bundle.rs` — `shield_from_asset_lock`:** built the Type 18
  `ShieldFromAssetLock`, broadcast it, then **swallowed** a post-broadcast `wait_for_response`
  failure as a `warn!` + `.ok()`, falling through to `Ok(shield_amount_credits)`. A
  confirmation failure thus reported success even though the credits may not have reached the
  pool — and the locked Core funds back a single-use asset lock that resumes the same shield
  on retry.
- **`src/backend_task/wallet/fund_platform_address_from_wallet_utxos.rs`** and
  **`src/backend_task/wallet/fund_platform_address_from_asset_lock.rs`:** called the SDK
  `TopUpAddress::top_up` directly. Submit failures **do** propagate via `?` (no false
  success), but a successful top-up never marked the tracked lock `Consumed`, so it stayed
  resumable, and there was no CL-height retry on a submit-time consensus 10506.

- **Resolved in PR (shield):** the shield-from-asset-lock false-success. The post-broadcast
  confirmation failure now maps to a dedicated typed `TaskError::ShieldedConfirmationUnknown`
  (`#[source] Box<dash_sdk::Error>`, no `String` message field) that surfaces "your funds were
  sent but the confirmation could not be verified — wait, then refresh before sending again".
  Unit-tested in `bundle.rs` (`confirmation_failure_maps_to_unknown_confirmation_error`,
  `unknown_confirmation_message_is_actionable_and_jargon_free`).
- **Resolved in PR (platform addresses):** both platform-address funding flows now branch on
  **upstream pool membership** — no DET-side index window or magic constant.
  `WalletBackend::platform_address_in_pool` resolves the wallet and asks upstream's own
  `ManagedPlatformAccount::contains_platform_address` (account 0), the exact generation-based
  check the orchestrator's pre-flight runs. An in-pool destination routes through
  `WalletBackend::fund_platform_address` → `PlatformAddressWallet::fund_from_asset_lock` (`pub`
  on the public `PlatformWallet` at rev `4f432c9`), which owns the full resolve →
  `submit_with_cl_height_retry` → IS→CL fallback → `consume_asset_lock` pipeline. DET reaches
  it via the existing private `WalletBackend::resolve_wallet` + the public
  `PlatformWallet::platform()` getter, with `DetPlatformSigner` as the `Signer<PlatformAddress>`
  and `DetSigner` as the `key_wallet::signer::Signer`. The wallet-UTXO path uses
  `AssetLockFunding::FromWalletBalance` (orchestrator builds + broadcasts the lock — the manual
  `create_asset_lock_proof` + `top_up` two-step is dropped on this branch); the tracked-lock
  path uses `AssetLockFunding::FromExistingAssetLock { out_point }`. Orchestrator errors map to
  a dedicated `TaskError::PlatformAddressFundRejected` (`#[source] Box<PlatformWalletError>`,
  no `String` message field). Pool-membership semantics, the mixed-ownership routing rule, and
  error mapping are unit-tested (`upstream_pool_membership_distinguishes_in_pool_from_foreign`,
  `route_to_orchestrator` cases, `map_platform_address_fund_error_*`).
- **Resolved in PR (fee-from-wallet residual):** funding your **own in-pool** address with
  **fee-from-wallet** now also routes through the orchestrator. The branch sources the change
  recipient from the wallet's own **watched platform-payment addresses** (every one is inside
  DET's synced provider window — the `0a64be55` funds-safety invariant) and gates each through
  the same `WalletBackend::platform_address_in_pool` check the orchestrator's pre-flight runs.
  When a distinct in-pool, watched change address exists, the two-output map
  (`{destination: Some(amount_credits), change: None}` with `ReduceOutput(change_index)`)
  satisfies the orchestrator's exactly-one-`None` / all-in-pool contract, so the whole
  fee-from-wallet flow gains orchestrated recovery (CL-height retry, IS→CL fallback,
  `consume_asset_lock`). The lock is sized to `amount + estimated_fee` so the change absorbs
  exactly the fee budget — identical amount/fee semantics to the manual path. No reveal or pool
  advance happens: only already-revealed (watched + in-pool) addresses are used as change.
  Change selection is a pure, unit-tested helper (`select_in_pool_change`:
  `change_picker_returns_first_non_destination_candidate`, `change_picker_skips_the_destination`,
  `change_picker_returns_none_without_a_distinct_candidate`).
- **Two residuals on the manual fallback, stated honestly:**
  - *By design (footgun):* a **foreign / non-pool** destination — funding an address the wallet
    does not watch (advanced send, MCP/CLI) — keeps the manual `TopUpAddress` path. The
    orchestrator's pre-flight requires recipients to be in this wallet's platform-payment pool,
    and credits sent to an unwatched address are recoverable only by that key's holder.
  - *Narrow edge:* a fee-from-wallet top-up to an in-pool destination when the wallet has **no
    other** watched, in-pool platform-payment address to serve as change falls back to the manual
    path (which derives a fresh change address). This only occurs for a wallet whose single
    revealed platform-payment address is the destination itself; revealing any second receive
    address closes it on the next attempt.
  - Both manual fallbacks still propagate submit failures via `?` (no false success).

### PROJ-043 — Sibling shielded spend fns marked notes spent on an unverified post-broadcast confirmation *(LOW — RESOLVED in PR)*

PROJ-042 fixed the post-broadcast confirmation swallow only in `shield_from_asset_lock`. The
four sibling spend fns in the same file carried the identical pattern: they `warn!` + `.ok()`
the `wait_for_response` failure after a successful broadcast and returned `Ok(...)`.

- **`src/backend_task/shielded/bundle.rs`:** `shield_credits` (returned `Ok(())`),
  `shielded_transfer`, `unshield_credits`, `shielded_withdrawal` — the latter three returned
  `Ok(spent_nullifiers)`.
- **Effect (before):** the caller path took the `Ok` branch and called `mark_notes_spent`
  (`src/context/shielded.rs`), which persists each nullifier via `mark_shielded_note_spent`.
  So notes were marked spent for spends whose state transition may never have confirmed
  (`shield_credits` instead bumped the platform-address nonce on the unverified spend).
- **Why it was LOW (self-heals):** the divergence was local and temporary. The next
  nullifier/note resync (`check_nullifiers`) reconciles spent state against the chain, so a
  note wrongly hidden would resurface; no permanent note or fund loss.
- **Fix (this PR):** each fn now maps the post-broadcast `wait_for_response` failure through a
  dedicated typed error and propagates it via `?` instead of swallowing it:
  `TaskError::ShieldCreditsConfirmationUnknown`, `ShieldedTransferConfirmationUnknown`,
  `UnshieldConfirmationUnknown`, `ShieldedWithdrawalConfirmationUnknown` (each `#[source]
  Box<dash_sdk::Error>`, no `String` message field; jargon-free, actionable "wait then refresh"
  message). Because the callers commit side effects only on `Ok`, propagating the error means
  `mark_notes_spent` is skipped (transfer/unshield/withdrawal) and the platform-address nonce
  is not bumped (`shield_credits`). The notes stay unspent locally — the balance is briefly
  *overstated* rather than understated.
- **Safety reasoning:** if the broadcast actually landed on-chain, a retry rebuilds a spend of
  the same notes; consensus rejects the already-revealed nullifier (double-spend), so funds are
  never at risk, and the next `check_nullifiers` resync observes the on-chain nullifier and
  marks the notes spent — self-correcting the overstate through routine sync. This mirrors the
  `shield_from_asset_lock` precedent (PROJ-042). The change is the minimal swallow→propagate
  fix; it does not touch the resync state machine.
- **Tests:** `src/backend_task/shielded/bundle.rs` unit tests assert each mapper yields its
  typed variant and that every message is actionable (wait + refresh) and jargon-free.

### New LOW parity deltas (2026-06-10)

| ID | Title | Location (PR #860) | v0.10-dev evidence | Status / what's missing |
|----|-------|--------------------|--------------------|--------------------------|
| PROJ-035 | In-app copy directs users to the removed "Local Dash Core node" setting; phantom "Refresh" button referenced | `src/ui/wallets/wallets_screen/single_key_view.rs:13` (`SINGLE_KEY_REQUIRES_CORE`), `:128`; `src/ui/components/tools_subscreen_chooser_panel.rs:141` | OLD settings exposed the RPC-vs-SPV `CoreBackendMode` selector | **RESOLVED 2026-06-11** (`1871c59f`) — stale copy updated and dead recovery-trail controls removed. (was GAPCMP-A-04) |
| PROJ-036 | Wallet "Refresh" in Core-Only mode is a silent no-op | `src/backend_task/core/mod.rs:191-213`; mode toggle `src/ui/wallets/wallets_screen/mod.rs:77-100` | OLD `RefreshWalletInfo` ran `reconcile_spv_wallets()` / RPC re-poll | **RESOLVED 2026-06-11** (`1871c59f`) — dead Core-Only refresh mode removed. (was GAPCMP-A-07) |
| PROJ-037 | Send-dialog "Memo (optional)" field goes nowhere (pre-existing) | `src/ui/wallets/wallets_screen/dialogs.rs:224-225`; `src/backend_task/core/mod.rs:255-303` never reads `memo` | OLD also never consumed `WalletPaymentRequest.memo` in HD sends | **RESOLVED 2026-06-11** (`1871c59f`) — dead memo field removed. (was GAPCMP-A-09) |
| PROJ-038 | Failed wallet-funded identity registration leaves no visible local record; retry-adoption semantics changed | `src/backend_task/identity/register_identity.rs:23,209-231` (placeholder-id skip; only the Platform-addresses path still persists `FailedCreation`, `:394`); `src/wallet_backend/mod.rs:1910-1922` (`IdentityAlreadyExists` → generic bucket) | OLD pre-derived the real id, persisted `PendingCreation`/`FailedCreation` rows (OLD `register_identity.rs:258-295,382-423`) and silently adopted an already-registered identity on retry | **RESOLVED 2026-06-11** (`1871c59f`) — recovery trail and UI copy updated to surface the unused-asset-lock resume path. (was GAPCMP-B-1/B-2) |
| PROJ-040 | DashPay offline caches dropped — contacts/requests/profiles/avatars need network on every open | `src/ui/dashpay/contacts_list.rs:67,111-134`; `contact_requests.rs:295-297`; avatar-bytes cache dropped (`profile_screen.rs` comment) | OLD rendered instantly from `data.db` (`contacts_list.rs:113-180`, `contact_requests.rs:162-250`, avatar bytes + negative-profile caching) | **RESOLVED 2026-06-11** (`467dc807` + `dc94bba6`) — offline contact/profile reads and avatar cache implemented; cache invalidation and bounds fixed in `dc94bba6`. (was GAPCMP-C-6) |
| PROJ-041 | "Stop tracking balance" undone by "Refresh My Tokens"; watch set became identities × all-known-tokens | `src/backend_task/tokens/query_my_token_balances.rs:39-44,100-105` (re-registers `known_token_ids` for every identity), `:62-83` (unwatch) | OLD refreshed only pairs already in `identity_token_balances` (OLD `:27-44`) | OPEN — dismissed rows reappear after any refresh; rows appear for never-tracked pairs. Disclosed in code comments only. Evolution of already-resolved #5. Deferred-with-TODO (`727e8d6a`). (was GAPCMP-C-7) |
| PROJ-042 | Non-identity asset-lock flows bypass upstream orchestration: shield-from-asset-lock falsely confirmed on a post-broadcast confirmation failure; platform-address top-ups never mark the lock `Consumed` | `src/backend_task/shielded/bundle.rs` (`shield_from_asset_lock`, post-broadcast `wait_for_response`); `src/backend_task/wallet/fund_platform_address_from_wallet_utxos.rs` (`top_up`); `src/backend_task/wallet/fund_platform_address_from_asset_lock.rs` (`top_up`) | Identity asset-lock flows route through `IdentityWallet::*_with_funding`; these three never did | **RESOLVED in PR** — shield-from-asset-lock false-success fixed (typed `TaskError::ShieldedConfirmationUnknown`, `#[source]`; unit-tested). Platform-address funding now gates on **upstream pool membership** (`WalletBackend::platform_address_in_pool` → upstream `ManagedPlatformAccount::contains_platform_address`, no DET-side constant); in-pool destinations route through `WalletBackend::fund_platform_address` → `PlatformAddressWallet::fund_from_asset_lock` (full resolve → `submit_with_cl_height_retry` → IS→CL → `consume_asset_lock`); errors map to `TaskError::PlatformAddressFundRejected` (`#[source]`). Membership + mixed-ownership routing + error mapping unit-tested. **Fee-from-wallet residual now CLOSED**: funding your own in-pool address with fee-from-wallet also routes through the orchestrator, sourcing an in-pool, watched platform-payment change recipient (gated through `platform_address_in_pool`) so the two-output map satisfies the orchestrator's one-`None` / all-in-pool contract; change selection is a pure, unit-tested helper (`select_in_pool_change`). One by-design residual remains (foreign/non-pool destinations — footgun) plus a narrow edge (an in-pool destination that is the wallet's only revealed platform-payment address has no distinct change → manual fallback). Manual submit failures still propagate via `?`, so no false-success remains. |
| PROJ-043 | Four sibling shielded spend fns swallowed the same post-broadcast confirmation failure, so notes were marked spent for spends that may never confirm | `src/backend_task/shielded/bundle.rs` (`shield_credits`, `shielded_transfer`, `unshield_credits`, `shielded_withdrawal`) — all `warn!`+`.ok()` then `Ok(...)`; `mark_notes_spent` at `src/context/shielded.rs` persists via `mark_shielded_note_spent` | Same swallow pattern PROJ-042 fixed for `shield_from_asset_lock` | **RESOLVED in PR** — the four fns now map the post-broadcast `wait_for_response` failure to dedicated typed errors (`ShieldCreditsConfirmationUnknown`, `ShieldedTransferConfirmationUnknown`, `UnshieldConfirmationUnknown`, `ShieldedWithdrawalConfirmationUnknown`; `#[source] Box<dash_sdk::Error>`, no String message field) and propagate via `?`. Callers commit side effects only on `Ok`, so `mark_notes_spent` (transfer/unshield/withdrawal) and the nonce bump (`shield_credits`) are skipped on an unverified spend — balance is briefly overstated, not understated. Retry is double-spend-rejected by consensus and the next `check_nullifiers` resync reconciles the spent state, so funds are never at risk. Mirrors the PROJ-042 `shield_from_asset_lock` fix; unit-tested. |

---

## Deferred-by-design / disclosed trade-offs

Intentional scope cuts, recorded so reviewers do not mistake them for oversights. All trace
to a written decision in `docs/ai-design/2026-05-18-platform-wallet-migration/`.

| ID | Title | Location | Sev | Status | Decision ref |
|----|-------|----------|-----|--------|--------------|
| PROJ-007 | Single-key refresh + SPV-send return `SingleKeyWalletsUnsupported`; password-protected single-key wallets silently vanish post-upgrade | `src/backend_task/core/mod.rs` (`CoreTask::RefreshSingleKeyWalletInfo` / `CoreTask::SendSingleKeyWalletPayment` arms); `src/ui/wallets/import_mnemonic_screen.rs:118-126`; `src/backend_task/migration/finish_unwire.rs:120-134,377-389`; `src/wallet_backend/single_key.rs:363` | MEDIUM (bumped 2026-06-10, was LOW) | **PARTIAL 2026-06-11** (`fba925ec` + `01f2bb26` + `690d92b3` + `3a0e5909`): T1 import-consolidation + T2 data-loss-gate + T6 password-restore shipped and security-reviewed; T3/T4/T5 refresh/send PARKED on upstream `platform-wallet register_watch_only_wallet`. | Decision #7 (`single-key-mock.md`) + T-SK-03 |
| PROJ-008 | SEC-002 sign-time passphrase prompt UX | `src/wallet_backend/secret_prompt.rs`; `src/ui/components/secret_prompt_host.rs`; `src/ui/components/passphrase_modal.rs` | MEDIUM | **RESOLVED (`2272bae0..43f412cf`)** | issue #90 — per-secret JIT prompt now shipped |
| PROJ-009 | DIP-14 back-compat dropped (non-mainnet / non-account-0 legacy contact addresses not reproduced) | `src/wallet_backend/mod.rs:722-724` (`register_dashpay_contact`, "Decision #6, back-compat dropped") | MEDIUM | **RESOLVED-WONTFIX 2026-06-11** (`d504d09e`): the non-mainnet/non-account-0 legacy contact-address class never existed — account 0' is hardcoded upstream and all legacy callers hardcoded account 0; nothing stranded. PROJ-027 resolved separately. | Decision #6 (`open-questions.md`) |
| PROJ-010 | Seedless loader is READ-ONLY; nothing populated the upstream persistor after `SeedReregistrationLoader` was deleted (`e6c6c017`) → empty watch set → received funds invisible. **Regression, now fixed.** | `src/wallet_backend/loader.rs`; `src/wallet_backend/mod.rs::{load_from_persistor_seedless, register_wallet_from_seed, ensure_upstream_registered}`; `src/context/wallet_lifecycle.rs::{register_wallet, bootstrap_wallet_addresses_jit}` | HIGH | **REGRESSION — FIXED** (W1/W2 persistor writers re-introduced) | `docs/ai-design/2026-06-08-wallet-reregistration-fix/design.md` |
| PROJ-011 | `identity` `CREATE TABLE` still on fresh installs — tombstone ADR pending | `src/database/initialization.rs:845-866` | LOW | **RESOLVED 2026-06-11** (`255aa018`): legacy identity table removed. | T-DEV-02b; deferred to separate ADR (`finish-unwire/notes.md` §4) |
| PROJ-022 | `UpstreamPlatformAddresses` reserved swap-target — read methods `unimplemented!()` | `src/wallet_backend/platform_address.rs:245-307` | LOW | Open by design — **accepted-risk 2026-06-11**: by design, the `unimplemented!()` read arms are intentional until the upstream platform-address swap lands (parallels PROJ-010). Triage: `accept_risk`. | pending platform todo `e817b66a`; parallels PROJ-010 |

Notes:

- **PROJ-007** narrowed since the design docs: SEC-002 work (`6052dc72`, `48cdb8ad`) made
  single-key **import / sign / list / hydrate** genuinely real
  (`src/wallet_backend/single_key.rs`; `SingleKeyView::import_wif`). UI now imports via
  `ImportSingleKeyDialog` (`src/ui/wallets/wallets_screen/mod.rs:42,157`;
  `src/ui/wallets/import_single_key.rs`). Only balance/UTXO **refresh** and **SPV-based
  send** remain stubbed. The `single-key-mock`/`g2-mock-boundary` "fully read-only mock"
  claim has now been corrected in the design docs (see PROJ-020, RESOLVED).
  **Extended + bumped LOW→MEDIUM 2026-06-10 (was GAPCMP-A-03):** two user-facing halves of
  the T-SK-03 deferral were previously untracked. (a) Import now rejects any non-empty
  per-key password (`src/ui/wallets/import_mnemonic_screen.rs:118-126` — "Per-key passwords
  are not supported in this version"; v0.10-dev `SingleKeyWallet::from_wif(input, password,
  alias)` supported them). (b) The cold-start migration **skips** `uses_password=1`
  single-key rows (`src/backend_task/migration/finish_unwire.rs:120-134,377-389`,
  `skipped_password_protected`) and the new hydration lists only vault-persisted keys
  (`src/wallet_backend/single_key.rs:363`), so a skipped wallet **disappears from the
  wallet list entirely** post-upgrade — no in-app access or explanation. Key material is
  preserved in the untouched legacy `data.db` (no data loss). Neither caveat appears in
  CHANGELOG Known Limitations, which says single-key import "works in this release"
  (→ DOC-001). With PROJ-008's JIT prompt seam now shipped, the skipped rows could likely
  be unlocked inline on the next migration pass.
  **PARTIAL 2026-06-11** (`fba925ec` + `01f2bb26` + `690d92b3` + `3a0e5909`): T1
  import-consolidation + T2 data-loss-gate + T6 password-restore shipped and
  security-reviewed. T3/T4/T5 refresh/send PARKED on upstream `platform-wallet
  register_watch_only_wallet`.
- **PROJ-008 (RESOLVED this refresh):** the SEC-002 sign-time prompt UX that was deferred is
  now shipped by the JIT secret-access refactor (`2272bae0..43f412cf`). `secret_prompt.rs`
  defines the `SecretPrompt` async trait whose `request()` is asked per-secret, keyed by
  `SecretScope::{HdSeed { seed_hash }, SingleKey { address }}`, carrying **no plaintext** —
  only display copy and an optional `SecretPromptRetry::WrongPassphrase` re-ask reason. The
  egui host (`secret_prompt_host.rs`) and modal (`passphrase_modal.rs`) collect the
  passphrase; `RememberPolicy` (`None` default / `UntilAppClose` / `For(Duration)`) controls
  caching. `NullSecretPrompt` cleanly cancels in headless/MCP/CLI. This is exactly the
  "prompt at sign time, not an upfront session gate" UX issue #90 tracked as deferred. Moved
  out of the open deferred set.
- **PROJ-010 (REGRESSION, now FIXED):** the earlier "Resolved (PR #3692)" status was wrong.
  Swapping `SeedReregistrationLoader` for the read-only `UpstreamFromPersisted` loader
  (`e6c6c017`) deleted the **only** code that ever wrote wallets into the upstream
  `platform-wallet.sqlite` persistor (`create_wallet_from_seed_bytes` → `persister.store`).
  The seedless `load_from_persistor` only READS that persistor, so when it was empty (fresh
  install, post-reset, migrated/sidecar-only wallets) the wallet was never registered with the
  upstream SPV manager, the watch set was empty, and received Core funds stayed invisible at
  100% sync (real repro: 1.0 DASH at block 1492173 to `m/44'/1'/0'/0/0`). It also explains the
  backend-e2e "Timed out waiting for wallet to register with the upstream backend" timeout —
  `is_wallet_registered` never flipped true because nothing populated the persistor. **Fix:**
  re-introduce the persistor write at the two seed-bearing moments — `register_wallet_from_seed`
  (W1, create/import) and `ensure_upstream_registered` (W2, cold-boot reconciliation through the
  JIT chokepoint), both idempotent and routed through the account-xpub fund-routing gate, with a
  genesis birth-height floor (`Some(0)`) for imported/recovered/migrated wallets so deposits made
  before registration are still found. The seedless read path is unchanged; W1/W2 simply
  guarantee its input is populated. PROJ-008 watch-only-at-boot is preserved (no launch-time
  prompt for protected wallets). See `docs/ai-design/2026-06-08-wallet-reregistration-fix/design.md`.
- **PROJ-011** (re-verified): `legacy_detected()` (`src/database/initialization.rs:146`) gates
  `wallet` / `wallet_addresses` / `utxos` / `wallet_transactions` / `shielded_notes` behind
  `include_legacy`. The `identity` empty placeholder (`:851`) is still created
  unconditionally for legacy `database/wallet.rs` cold-start reads. `platform_address_balances`
  (`:797,933`) is still live. Documented "separate ADR" carve-out.
- **PROJ-022:** `UpstreamPlatformAddresses` (`platform_address.rs:245`) is the reserved
  swap target for reading per-address Platform funds straight from upstream. It is **NOT
  selected** — the ACTIVE impl is `KvCachedPlatformAddresses`
  (returned by `platform_addresses()`, `src/wallet_backend/mod.rs:593`). Its read methods (`get_address_info`, `all_address_info`,
  `get_sync_info`) are `unimplemented!()` pending upstream `e817b66a` (a public per-address
  balance+nonce reader + sync-cursor shape). Dead code by design; structurally identical to
  the PROJ-010 G2 loader seam. Cannot panic in any live path while the cached impl is active.
- **PROJ-009 (RESOLVED-WONTFIX 2026-06-11, `d504d09e`):** the non-mainnet/non-account-0
  legacy contact-address class never existed — account 0' is hardcoded upstream and all
  legacy callers hardcoded account 0. Nothing is stranded. The PROJ-027 general incoming-
  payment gap is resolved separately. The deferral text still stands as a design note; the
  finding is marked WONTFIX since there is nothing to migrate.
- **`FundWithUtxo` (seed item #15)** — the *removed* asset-lock funding path. Current active
  funding task is `WalletTask::FundPlatformAddressFromWalletUtxos`
  (`src/backend_task/wallet/mod.rs`), a different working path. No live broken surface.
  **Cross-ref (2026-06-10):** the removal also covered the two QR-funding UI tabs —
  `src/ui/identities/add_new_identity_screen/by_wallet_qr_code.rs` and
  `src/ui/identities/top_up_identity_screen/by_wallet_qr_code.rs` (both deleted;
  `FundingMethod::AddressWithQRCode` gone) — disclosed at `CHANGELOG.md:51` and
  `docs/user-stories.md` IDN-014 `[Removed — upstream-only funding]`. Reviewers should not
  re-flag those files. Dead `CoreItem::ReceivedAvailableUTXOTransaction` match arms remain
  in both screens (see PROJ-026). The promised in-app one-time notice for this removal was
  never shipped — see DOC-003.

### Disclosed removals — itemised (2026-06-10)

Previously implied by the RPC-mode removal but not individually auditable. All verified
absent in the working tree; none is a new gap.

| Removed surface | v0.10-dev evidence | Disclosure | Notes |
|-----------------|--------------------|------------|-------|
| `CoreTask::RecoverAssetLocks` ("Search for Unused" button) | OLD `src/backend_task/core/recover_asset_locks.rs`; OLD `wallets_screen/mod.rs:2491,2660` | `removal-inventory.md:103` ("replaced by AssetLockManager continuous tracking") | Replaced by `WalletTask::ListTrackedAssetLocks` (`src/backend_task/wallet/mod.rs:85-87`). Was already a no-op in OLD SPV mode. |
| `CoreTask::ListCoreWallets` + Core-wallet auto-detect/picker | OLD `add_new_wallet_screen.rs:567-590`, `import_mnemonic_screen.rs:470-492`, RPC `-19` recovery | `removal-inventory.md:55,101` | Pure RPC-mode surface. (MCP `core_wallets_list` is a different, surviving tool.) |
| RPC Core-backend mode (Connection Type selector, Core RPC password UI, dashmate auto-update, RPC/ZMQ status rows, indicator-click Dash-Qt launch) | OLD `network_chooser_screen.rs:262-310,410-513,672-708`; OLD `top_panel.rs:168-195` | `removal-inventory.md` §"RPC Backend Mode — Fate" (`:95-111`) | Deliberate: platform-wallet is SPV-only. CHANGELOG omission → DOC-001. Dead `dashmate_password_input` field survives unrendered (`network_chooser_screen.rs:68,281`). Launcher fallout → PROJ-033; placebo ZMQ checkbox → PROJ-012. |
| SPV peer-source expert setting ("Use local Dash Core node" for peer discovery) | OLD `network_chooser_screen.rs:1222-1269`; `db.get_use_local_spv_node()` | `removal-inventory.md:55` | Upstream owns peer discovery; devnet/regtest host config via `.env` unchanged. Record-only (was GAPCMP-D-06). |
| Proof Log screen + persistence | OLD `src/ui/tools/proof_log_screen.rs` (426 lines), `src/database/proof_log.rs`, `insert_proof_log_item` writers | `CHANGELOG.md:50`; commit `7778eb64` | Replaced by `tracing` target `"proof_log"` — history no longer survives restart. Stale doc refs → DOC-002. |
| "Total Received (DASH)" address-table column | OLD `address_table.rs` `TotalReceived` sortable column | In-code comment ("no upstream source post-migration") | CHANGELOG line missing → DOC-001. |

---

## Test gaps

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-013 | Large-stack e2e harness — SDK deep-recursion stack overflow | `tests/backend-e2e/framework/task_runner.rs:17-78,153-160` | MEDIUM | **RESOLVED (`2a9161d3`, `93a20769`)** | Harness drives every SDK task on a dedicated 32 MB-stack runtime (the only mechanism that reaches tokio threads). `#[stack_size]` investigated and rejected; `RUST_MIN_STACK` now moot. Detail below. |
| PROJ-014 | `WalletBackend::start()` start-path test coverage | `src/context/wallet_lifecycle.rs:561,583,617,649` | HIGH | **RESOLVED (`3165f98c`, `36f5a982`)** | Four offline tests now gate the start path (`start_spv_errors_when_backend_not_wired`, `start_spv_starts_after_backend_wired`, `ensure_wallet_backend_and_start_spv_wires_then_starts`, `chokepoint_wiring_failure_flips_indicator_to_error`). Full live-SPV success path remains an e2e/network gap. |
| PROJ-015 | TC-012 receive-address reuse — unverified from DET source | `src/wallet_backend/mod.rs` (`next_receive_address` → upstream) | LOW | Unverified — needs follow-up | Depends on upstream used-marking; now testable since PROJ-001 is resolved. Re-test on live network. |
| PROJ-016 | TC-066 key-not-visible-after-broadcast (flake-vs-bug) | (tracked-only, no isolated code surface) | LOW | Unverified — needs follow-up | No deterministic repro in tree. Re-classify after live run. |

**PROJ-013 — RESOLVED. Mechanism confirmed correct; `#[stack_size]` rejected.** Verified at
`tests/backend-e2e/framework/task_runner.rs`: `sdk_runtime()` (`:22`) builds a dedicated
multi-thread tokio runtime with `thread_stack_size(32 * 1024 * 1024)` (`SDK_THREAD_STACK_SIZE`,
`:17`); `run_task` (`:50`) drives every backend future through `drive_on_large_stack` (`:65`) —
the single chokepoint all e2e tasks pass through — which `block_on`s on a 32 MB blocking thread
so the deep synchronous SDK `block_on` (grovedb / drive-proof-verifier) cannot overflow the
default 8 MB stack, **regardless of `RUST_MIN_STACK`**. A deterministic, non-network smoke test
`large_stack_path_survives_deep_recursion` (`:153-160`) recurses ~12 MiB through the exact
`drive_on_large_stack` path, proving the mechanism is load-bearing without any env-var assist.

- **`#[stack_size]` (`dash-platform-macros`) investigated and rejected.** The upstream
  attribute enlarges only the single `std::thread` running the wrapped function body. The
  backend-e2e tests are async (`#[tokio_shared_rt::test(shared, flavor = "multi_thread", …)]`)
  and the SDK recurses *inside* the sync `ContextProvider::get_quorum_public_key` callback
  (`src/context_provider_spv.rs`), which bridges to async via `tokio::task::block_in_place` —
  a multi-thread-only construct. The recursion therefore lands on **tokio worker / blocking
  threads**, which `#[stack_size]` cannot reach. The shared runtime built by `tokio-shared-rt`
  carries no custom stack size, so `thread_stack_size` on a dedicated runtime is the only
  mechanism that covers those threads. The dependency was deliberately **not** added.

- **`RUST_MIN_STACK` now moot.** The harness owns the stack via the runtime, so callers no
  longer set it. `tests/backend-e2e/main.rs` (`93a20769`) and `tests/backend-e2e/README.md`
  agree on this. When CI re-enables the `#[ignore]` backend-e2e suite (run steps currently
  commented in `.github/workflows/tests.yml:85,90`), no env-level stack bump is needed —
  the runtime owns the stack for every task path (all flow through `run_task`).

Recorded test-spec gaps from `finish-unwire/notes.md` §5 (feature-flag/manual only, not
counted as new open gaps): TC-SK-010, TC-A11Y-008, TC-PERF-003.

---

## Upstream dependencies

| ID | Title | Location | Sev | Status | Blocker |
|----|-------|----------|-----|--------|---------|
| PROJ-005 | platform pin tracks unreleased dev rev (G1) | `Cargo.toml:21,31,32,35` | HIGH | OPEN | Release gate; bump to released rev before ship. Current rev `4f432c9baf10eeb051e70bc0370b1b7505b7d9c5` — the 2026-06-10 re-pin to dashpay/platform#3828 (`4247c360`); was `9e1248cb…` at prior refresh, `ddfa66ed…` / `35e4a2f6…` before, `17653ba8…` at original audit. The re-pin also introduced PROJ-028. |
| PROJ-017 | `register_identity_funding_account` absent upstream — DET carries contained exception | `src/wallet_backend/mod.rs:1407` (`provision_identity_funding_account`) / `:1489` (`ensure_identity_funding_accounts`) | LOW | **RESOLVED 2026-06-11** (`26675766`): load() comments scoped to per-index top-up; upstream-contribution path documented in code. |

---

## Doc gaps

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-018 | External user docs (dashpay/docs) not yet filed | `docs/ai-design/2026-06-03-pr860-doc-followups/external-docs-draft.md` | MEDIUM | OPEN (deferred-with-TODO, `727e8d6a`) | Draft written; must still be filed as a PR/issue against `github.com/dashpay/docs` after #860 merges. |
| PROJ-019 | ADR floor SHA placeholder unfilled | `docs/ai-design/2026-05-29-finish-unwire/notes.md:92` (`[PLACEHOLDER — fill at merge time]`) | LOW | OPEN (deferred — merge-time action only) | Cannot close pre-merge — needs this PR's squash-merge SHA recorded as the wallet-state floor. Triage decision: `defer`. |
| PROJ-020 | Design docs claimed single-key is fully read-only mock — was stale | `docs/ai-design/2026-05-18-platform-wallet-migration/single-key-mock.md:51`; `g2-mock-boundary.md:46` | LOW | **RESOLVED (`f39b085d`)** | Prose corrected: `single-key-mock.md:51` now states import/list/sign work in full via `SecretStore`; `g2-mock-boundary.md:46` records the partial capability gap accurately. |
| PROJ-021 | CHANGELOG omits single-key capability limits and DIP-14 trade-off | `CHANGELOG.md:31-46` | LOW | **RESOLVED (`f39b085d`)** | `### Known Limitations` section now states single-key send/refresh is unsupported this release and documents the DIP-14 non-mainnet/non-account-0 contact-fund re-establishment trade-off. |
| DOC-001 | CHANGELOG disclosure sweep — Removed/Known-Limitations/Fixed sections incomplete | `CHANGELOG.md:33-56` | MEDIUM | **RESOLVED 2026-06-11** (`1871c59f` + `23b81718`) — CHANGELOG Removed and Known Limitations sections updated; ZMQ and Dash-Qt launcher removals recorded in `23b81718`. |
| DOC-002 | Proof-log removal untracked in audit + stale user-story/persona refs | `docs/user-stories.md:878` (DEV-002 still `[Implemented]`); `docs/personas/platform-developer.md:27,75` | LOW | **RESOLVED 2026-06-11** (`1871c59f`) — DEV-002 user story tag flipped and persona references corrected. (was GAPCMP-B-3 + D-01) |
| DOC-003 | Promised one-time post-migration notice (invariant I3) never shipped | `docs/ai-design/2026-05-18-platform-wallet-migration/backendtask-contract.md:43,65`; `phasing.md:194` (I3); `docs/user-stories.md` IDN-014 rationale | LOW | OPEN (deferred-with-TODO, `727e8d6a`) | Three doc sites commit to an in-app one-time notice disclosing the QR-direct-fund removal; the only post-migration banner is the generic "Storage update complete — your wallet is ready." (`src/app.rs:1130-1137`). Either ship the notice text or amend I3 + the three doc sites to say "disclosed via CHANGELOG". (was GAPCMP-B-4) |

**PROJ-018 (PARTIAL).** Verified at `docs/ai-design/2026-06-03-pr860-doc-followups/external-docs-draft.md`:
a full external-docs draft now exists, targeting `dashpay/docs` →
`docs/user/network/dash-evo-tool/`, with a **Tracking** section and an explicit
`TODO: file a dashpay/docs issue linking to PR #860 … once the merge commit is confirmed`.
The draft satisfies the "write the guidance" half of the gap; the gap stays OPEN until the
PR/issue is actually filed against `dashpay/docs` post-merge.

**PROJ-019 (PARTIAL).** Verified at `docs/ai-design/2026-05-29-finish-unwire/notes.md:92`:
the placeholder is now explicit (`[PLACEHOLDER — fill at merge time]`) with a clear
instruction not to leave it in place after merge. The merge SHA cannot be filled pre-merge,
so the item stays OPEN as a merge-time action.

---

## Project conventions

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-023 | String-based error matching in DashPay add-contact UI | `src/ui/dashpay/add_contact_screen.rs` | LOW | **RESOLVED (`d852ce99`)** | `display_task_result` no longer parses error strings; classification routes through typed `classify_send_error` matching `TaskError` / `DashPayError` variants. Verified: zero `.contains(` on error/message text in the file; 6 typed-error unit tests added. |
| PROJ-025 | String-based error matching in DashPay contact-requests UI | `src/ui/dashpay/contact_requests.rs` | LOW | **RESOLVED (`39e459ff`)** | Typed classification via `display_task_error` + `classify_request_error`; routes `TaskError::DashPay(DashPayError::Missing{En,De}cryptionKey)`. Old `message.contains("ENCRYPTION key")` / `"DECRYPTION key"` sites (`:844-851`, `:983-985`) removed. Dead `Message`-arm string-matching also gone. 4 unit tests added. |
| PROJ-039 | Rust `Debug` (`{:?}`) rendered in user-facing unused-asset-lock picker; address column dropped | `src/ui/identities/add_new_identity_screen/by_using_unused_asset_lock.rs:75`; `src/ui/identities/top_up_identity_screen/by_using_unused_asset_lock.rs:61-65` | LOW | **RESOLVED 2026-06-11** (`1871c59f`) — `AssetLockStatus` mapped to user-facing copy; address column restored. (was GAPCMP-B-7) |

**PROJ-023 — RESOLVED.** Verified at `src/ui/dashpay/add_contact_screen.rs`: no `.contains(`
on error/message strings remains; `classify_send_error` (`:649`) matches typed
`TaskError::IdentityNotFound`, `TaskError::DashPay(DashPayError::{…})` variants and the
`display_task_result` path routes through it (`:616`). Six typed-error unit tests cover the
classification (`:700-761`) — they assert specific variant mapping, base58 fallback, and that
recoverable errors map through so retry is offered (not shallow "no error" checks).

**PROJ-025 — RESOLVED (`39e459ff`).** `contact_requests.rs` now implements `display_task_error`
with a pure `classify_request_error` helper that routes typed `TaskError::DashPay(DashPayError::Missing{En,De}cryptionKey)`
onto the screen-local affordance. Missing-key variants drive the "Add Encryption Key" affordance
and suppress the duplicate global banner; everything else returns `None` so the global banner
reports it. Both `message.contains("ENCRYPTION key")` / `"DECRYPTION key"` sites and the dead
`Message`-arm string-matching body are gone; `git grep` returns zero hits. Four unit tests cover
the classification. Pattern mirrors PROJ-023 exactly; zero new `TaskError` variants were required.

---

## Seed-list items found ALREADY RESOLVED (evidence)

Recorded for completeness; **not** counted in the open-gap tally.

1. **Seed #3 — eager wallet-backend init hard failure.** **Resolved.** `src/app.rs` eager init
   warns + degrades to lazy fallback — no hard "Could not access wallet data" abort.
2. **Seed #7 — TC-019 inverted error precedence.** **Resolved / moot.** The
   `CoreTask::RefreshSingleKeyWalletInfo` dispatch arm in
   `src/backend_task/core/mod.rs` returns `SingleKeyWalletsUnsupported`
   unconditionally — no seed-lookup branch left to invert. (The standalone
   stub file cited in earlier revisions was dead and has been removed.)
3. **Seed #11 — QA-004 `core_backend_mode` inert plumbing.** **Field RETAINED,
   not removed.** Correcting an earlier false claim of `rg core_backend_mode
   src` = 0 hits: the field is still live in `src/model/settings.rs` (struct
   field, default `1` = SPV, serde round-trip, tests) and is read at DB-init
   in `src/database/initialization.rs`. It is **inert/reserved** now that RPC
   backend mode is gone — the *behavioural* plumbing is dead, but the persisted
   field itself was deliberately kept (dropping it is a settings-schema change
   deferred to a later cleanup), so it is NOT "resolved by removal".
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
- **2026-06-02 — PROJ-002 resolved (removed)**: dead `add_contact` / `remove_contact` free
  functions (zero callers, orphaned from PR #464 `82399a26`, superseded by
  `DashPayTask::SendContactRequest`) deleted by a sibling commit, along with the now-orphaned
  `DashPayError::NotSupported` variant.
- **2026-06-02 — PROJ-012 re-filed (deferred-LOW → functional-MEDIUM)**: not benign dead
  plumbing — the ZMQ status sender is live (`src/app.rs:819`) but `rx_zmq_status` is never
  drained and `set_zmq_status` (`src/context/connection_status.rs:159`) has zero callers, so
  ZMQ connection-health events flow into a void. Decision #3 P4-audit deferral does not excuse
  the broken status path. Fix: wire `rx_zmq_status` → `set_zmq_status`, or remove the whole
  producer→channel→setter chain (constructed as a unit at `src/context/mod.rs:161`).
- **2026-06-03 — refresh pass (head `f39b085d`)**: build/lint/test verified clean
  (`cargo +nightly fmt --check`, `cargo clippy --all-features --all-targets -D warnings`,
  `cargo build --tests --all-features`, the 6 `add_contact_screen` typed-error tests, and the
  deterministic `large_stack_path_survives_deep_recursion` e2e smoke — all green). Dispositions:
  - **PROJ-008 resolved** (`2272bae0..43f412cf`): the deferred SEC-002 sign-time passphrase
    prompt UX (issue #90) is shipped — per-secret JIT `SecretPrompt` seam
    (`secret_prompt.rs`), egui host (`secret_prompt_host.rs`), modal (`passphrase_modal.rs`),
    keyed by `SecretScope`, with `RememberPolicy`. Moved out of the open deferred set.
  - **PROJ-013 resolved** (`2a9161d3`): test-only; 32 MB-stack `sdk_runtime` +
    `drive_on_large_stack` chokepoint + deterministic smoke test. Residual CI sub-item tracked
    (commented `#[ignore]` step in `.github/workflows/tests.yml:85,90`; no `RUST_MIN_STACK` in
    `.github/`; `.github/` edits blocked by tool policy).
  - **PROJ-020 / PROJ-021 resolved** (`f39b085d`): single-key-mock / g2-mock-boundary prose
    corrected to present state; CHANGELOG `### Known Limitations` added.
  - **PROJ-023 resolved** (`d852ce99`): add-contact UI error classification moved off string
    matching onto typed `classify_send_error`; 6 unit tests. Sibling occurrence in
    `contact_requests.rs` filed as **new PROJ-025** (LOW, pre-existing, issue #660).
  - **PROJ-018 / PROJ-019 stay OPEN** as merge-time partials; **PROJ-005** pin moved
    `35e4a2f6…` → `ddfa66ed…` (still unreleased, stays the sole open merge-blocker).
- **2026-06-03 — tally reconciliation**: recomputed the Executive severity table and category
  breakdown from the enumerable body. Now 24 total / 12 open / 12 resolved (was 23/17/6):
  PROJ-025 added (+1 total, +1 open LOW); PROJ-008 (MEDIUM), PROJ-013 (MEDIUM), PROJ-020,
  PROJ-021, PROJ-023 (LOW) flipped open→resolved.
- **2026-06-03 — PROJ-025 resolved** (`39e459ff`): contact-requests string-matching anti-pattern
  replaced by typed `display_task_error` + `classify_request_error`; routes
  `TaskError::DashPay(DashPayError::Missing{En,De}cryptionKey)`. Both keyword-sniffing sites and
  the dead `Message`-arm removed; 4 unit tests added. Pattern mirrors PROJ-023; zero new variants.
  Tally: 24 total / **11 open / 13 resolved** (LOW: open 8→7, resolved 5→6).
- **2026-06-08 — line-ref consolidation (head `954ea3f8`)**: re-pinned drifted refs against
  source, line by line. PROJ-017 → `mod.rs:1407`/`:1489` (callers `:1261,1325,1371`; tracking
  `9cdcfb25` / `a5538dc8`); PROJ-019 → `notes.md:92`; PROJ-022 active impl → `mod.rs:593`;
  PROJ-007 arm → `mod.rs:218,303` (and two by-design markers added in source, `93a20769`).
  PROJ-013 detail refreshed: `#[stack_size]` (`dash-platform-macros`) investigated and rejected
  (recursion lands on tokio threads via `block_in_place`, which the single-thread macro cannot
  reach), 32 MB-stack runtime confirmed the only load-bearing mechanism, `RUST_MIN_STACK` moot
  (`main.rs` `93a20769` + README agree); residual CI sub-item folded into the entry. No tally change.
- **2026-06-10 — v0.10-dev feature-parity pass (head `a0d5034a`)**: four domain audits
  (wallets/core, identity/DPNS/contracts, DashPay/tokens/shielded, MCP/settings/withdrawals)
  swept the full v0.10-dev feature surface; every recorded finding re-verified against live
  source. **Added 19 entries**: 3 HIGH (PROJ-026 asset-lock-QR soft-lock, PROJ-027
  incoming-DashPay-payment detection gone, PROJ-028 shielded nullifier-cursor unit-mismatch —
  a **regression introduced by the same-day #3828 re-pin** `4247c360`+`a0d5034a`, cross-linked
  to follow-up todo `1ff97ad7`); 7 MEDIUM (PROJ-029 subtract-fee/Max dead-end, PROJ-030 resync
  keeps nullifier watermark, PROJ-031 shield source-address ignored, PROJ-032 DashPay data not
  migrated [later CLOSED/N-A per v0.9.3 cross-check — DashPay never shipped], PROJ-033
  Dash-Qt launcher unreachable, PROJ-034 settings/top-up-history/scheduled-votes reset on
  upgrade [confirmed real per v0.9.3 cross-check], DOC-001 CHANGELOG sweep); 9 LOW (PROJ-035..038, PROJ-040, PROJ-041,
  PROJ-039 conventions, DOC-002 proof-log doc debt, DOC-003 I3 notice). **Corrected 4 existing
  entries**: PROJ-012 re-scoped (whole ZMQ chain dead — listener spawn gated off by
  `FeatureGate::RpcBackend=false`; placebo "Disable ZMQ" checkbox folded in), PROJ-007 extended
  + bumped LOW→MEDIUM (password-protected single-key wallets silently vanish post-upgrade;
  per-key-password import rejected), PROJ-009 flagged incomplete (zero callers of
  `register_dashpay_contact`; wider loss → PROJ-027), PROJ-005 rev advanced to `4f432c9b…`
  (#3828 re-pin, still unreleased). **Reconciled**: disclosed removals itemised
  (RecoverAssetLocks, ListCoreWallets, RPC mode, SPV peer source, Proof Log, Total Received
  column); seed #15 cross-referenced to the two deleted QR UI tab files; exec-summary
  mis-bucketing of PROJ-010 (HIGH, not LOW) fixed. Preserved-feature coverage confirmed broad
  parity everywhere else (identity/DPNS/voting/contracts/documents/tokens/MCP byte- or
  behavior-identical). Tally: 43 total / 30 open / 13 resolved.
- **2026-06-11 — triage pass (18 findings resolved/partial/accepted):** 18 findings actioned
  across the 18-commit triage program; 3 new triage decisions recorded (PROJ-016/019/022).
  - **PROJ-026 RESOLVED** (`fe01febb` + `26c13385`): asset-lock QR funding now advances.
  - **PROJ-027 RESOLVED** (`910f8833` + `dc94bba6`): incoming DashPay contact payments detected and recorded.
  - **PROJ-029 RESOLVED** (`918b8e5f` + `26c13385`): Core Max implemented; subtract-fee UI removed.
  - **PROJ-031 RESOLVED** (`08c895a8` + `26c13385`): shield source-address selector removed.
  - **PROJ-040 RESOLVED** (`467dc807` + `dc94bba6`): DashPay offline caches + avatar cache.
  - **PROJ-017 RESOLVED** (`26675766`): funding-shim load() comments scoped.
  - **PROJ-012 + PROJ-033 + PROJ-011 RESOLVED** (`255aa018` + changelog `23b81718`): ZMQ subsystem, Dash-Qt launcher, and legacy identity table removed.
  - **PROJ-035/036/037/038/039 + DOC-001/DOC-002 RESOLVED** (`1871c59f` + `23b81718`): UI copy / dead controls / recovery-trail / doc fixes.
  - **PROJ-009 RESOLVED-WONTFIX** (`d504d09e`): non-mainnet/non-account-0 legacy contact-address class never existed; nothing stranded.
  - **PROJ-007 PARTIAL** (`fba925ec` + `01f2bb26` + `690d92b3` + `3a0e5909`): T1/T2/T6 shipped; T3/T4/T5 PARKED on upstream.
  - **PROJ-034/018/015/041 + DOC-003 deferred-with-TODO** (`727e8d6a`): TODO markers placed.
  - **PROJ-032 CLOSED/N-A** (2026-06-16 cross-check): DashPay was a "Coming Soon" placeholder in v0.9.3 — zero tables or persistence ever shipped in any release; precondition for the migration was never true; TODO removed from source.
  - **PROJ-016 triage: defer** — blocked on PROJ-007 single-key send; no deterministic repro.
  - **PROJ-019 triage: defer** — merge-time action (ADR floor SHA).
  - **PROJ-022 triage: accept_risk** — by design; unimplemented!() arms intentional until upstream swap.
  Tally: 43 total / **11 open** / **32 resolved** (HIGH open 3→1, MEDIUM open 10→4, LOW open 15→6).

- **2026-06-10 — PROJ-028 + PROJ-030 resolved** (`39433dac`; doc follow-up this commit): the
  shielded nullifier-cursor unit-mismatch family. The #3828 re-pin made spend detection
  scan-based off a note-tree POSITION cursor, but `last_nullifier_sync_height` held a legacy
  platform BLOCK HEIGHT. **PROJ-028 (HIGH):** the migration carried the value verbatim, so a
  migrated wallet scanned past the tree tip → spends never flipped → balance overstated.
  `finish_unwire.rs` now resets the cursor to 0 (`SELECT … 0, 0`); `tc_sh_002` flipped to assert
  the reset; new `migrated_cursor_reset_lets_scan_flip_spent_note` covers the end-to-end fix.
  **PROJ-030 (MEDIUM):** "Resync Notes" kept the cursor, resurrecting spent notes; the resync
  handler now also calls `delete_shielded_wallet_meta`, with `resync_sequence_resets_nullifier_cursor`
  pinning it. Reset-to-0 is funds-safe: `check_nullifiers` only flips notes to spent and a from-0
  rescan is idempotent (no re-credit, no missed spend). Smythe QA: SHIP, 2 LOW non-blockers
  (SEC-001 stale doc comment fixed here; SEC-002 shallow migration test tracked separately).
  Tally: 43 total / **28 open / 15 resolved** (HIGH open 4→3, MEDIUM open 11→10).

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
  design; migration itself fully wired (`src/backend_task/migration/mod.rs`). With PROJ-008
  resolved, the JIT prompt seam that T-SK-03 depends on now exists — re-check on next migration
  pass whether these rows can be unlocked inline.
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
- `src/app.rs:1314` (`TODO(RUST-002)`) — message-text-inspection routing TODO (tracked tech-debt,
  same family as PROJ-025).
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

*Candy tally — confirmed gaps: 43 (1 CRITICAL · 7 HIGH · 15 MEDIUM · 20 LOW · 0 INFO).
Status as of 2026-06-16: 34 RESOLVED / 1 PARTIAL / 1 ACCEPTED + 9 OPEN (1 HIGH + 3 MEDIUM + 5 LOW).
RESOLVED set: PROJ-001, PROJ-002 (removed), PROJ-003, PROJ-004, PROJ-006, PROJ-008, PROJ-010,
PROJ-011, PROJ-012, PROJ-013, PROJ-014, PROJ-017, PROJ-020, PROJ-021, PROJ-023, PROJ-025,
PROJ-026, PROJ-027, PROJ-028, PROJ-029, PROJ-030, PROJ-031, PROJ-033, PROJ-035, PROJ-036,
PROJ-037, PROJ-038, PROJ-039, PROJ-040, PROJ-043, PROJ-009 (WONTFIX), DOC-001, DOC-002,
PROJ-032 (CLOSED/N-A — DashPay never persisted in any v0.9.3 release; precondition false)
+ SEC-001 follow-up.
PARTIAL: PROJ-007 (T3/T4/T5 parked on upstream). ACCEPTED: PROJ-022. OPEN deferred-with-TODO:
PROJ-034 (REAL — confirmed per v0.9.3 cross-check; priority: scheduled votes > settings > top-up),
PROJ-018, PROJ-015, PROJ-041, DOC-003. OPEN merge-blocker: PROJ-005 (release gate G1 only).
8 seed/appendix items confirmed already-resolved with evidence. 1 blocked-by-design (PROJ-024, uncounted).*
