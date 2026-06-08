# PR #860 Platform-Wallet Rewrite — Consolidated Gap Audit

**Audit date:** 2026-06-01 — **Refreshed:** 2026-06-08
**Head SHA (refresh):** `954ea3f8`
**Prior refresh head:** `39e459ff`
**Original audit head:** `686430a4d2b83596fbbe716acc183a424859e11d`
**PR #860 base:** `v1.0-dev` @ `87ba5b711839219f5e1c7aee8f9de36d038866e3`
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

---

## Executive summary

| Severity | Open | Resolved | Total |
|----------|------|----------|-------|
| CRITICAL | 0    | 1        | 1 |
| HIGH     | 1    | 2        | 3 |
| MEDIUM   | 3    | 4        | 7 |
| LOW      | 7    | 6        | 13 |
| INFO     | 0    | 0        | 0 |
| **Total** | **11** | **13** | **24** |

Open by category: upstream/release-gate = 2 (PROJ-005, PROJ-017); functional/unwired = 1
(PROJ-012); deferred-by-design = 4 (PROJ-007, PROJ-009, PROJ-011, PROJ-022); test = 2
(PROJ-015, PROJ-016); doc = 2 (PROJ-018, PROJ-019). Sum = 11 = total open. Resolved this
refresh: PROJ-025 (LOW, typed classification mirroring PROJ-023; zero new variants; 4 tests).

### Merge-blocker verdict (called out up top)

**No CRITICAL merge-blocker remains open.** The one functional release gate stands:

1. **PROJ-001 (CRITICAL)** — **RESOLVED on-branch (`36f5a982`).** SPV / platform-address /
   identity sync is now started across all four caller paths. See PROJ-001 section + Resolution log.
2. **PROJ-005 (HIGH)** — release gate G1: the `dash-sdk` / `platform-wallet` pin (`Cargo.toml`)
   tracks an **unreleased** platform dev rev. Project policy (Decision #1) classifies this as
   a release-hardening blocker, not a start blocker — but it gates *merge-to-ship*. **This is
   the sole remaining merge-blocker.** The pin moved again since the prior refresh — now
   `rev = ddfa66ed…` (was `35e4a2f6…`, originally `17653ba8…`) — but is still a dev rev, not a
   released tag.

Everything else is fixable post-merge or is a disclosed scope cut.

---

## Merge-blocking gaps

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-001 | SPV sync never driven — dead `start()`, inert `start_spv()` | `src/context/wallet_lifecycle.rs:103,130`; `src/wallet_backend/mod.rs:462-479` | CRITICAL | **RESOLVED (`36f5a982`)** | See Resolution log 2026-06-01 |
| PROJ-005 | Pin tracks unreleased platform rev (G1) | `Cargo.toml:21,31,32,35` (`rev = ddfa66ed37…`) | HIGH | OPEN | Pin must move to a released platform rev before shipping. Still a dev rev. |

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

### PROJ-012 — ZMQ connection-health events flow into a void *(MEDIUM — OPEN)*

The ZMQ status producer is **live** but its consumer side is entirely unwired. Three confirmed
facts:
- **Live sender:** `src/app.rs:819` clones `ctx.sx_zmq_status` into the ZMQ listener, which
  pushes `Connected` / `Disconnected` connection events into the channel.
- **Unread receiver:** `rx_zmq_status` (`src/context/mod.rs:76`) is stored on `AppContext`
  (`:305`) but is **never drained** — no `recv` / `try_recv` anywhere in the tree.
- **Zero-caller setter:** the canonical `ConnectionStatus::set_zmq_status`
  (`src/context/connection_status.rs:159`) — the only path that would feed those events into
  the single-source-of-truth status — has **zero callers**.

Net effect: ZMQ connection-health events are produced and then discarded; the status indicator
never reflects ZMQ state. The channel pair is constructed as a unit at `src/context/mod.rs:161`
(`let (sx_zmq_status, rx_zmq_status) = …`), so it cannot be trimmed piecemeal. The binary fix
is to either **wire** the chain — drain `rx_zmq_status` and forward each event to
`set_zmq_status` — **or remove** the whole producer → channel → setter chain.

This was previously mis-scoped as deferred-by-design (Decision #3 P4 audit), which masked the
wiring gap. The Decision #3 deferral still stands as written, but it does **not** excuse a
broken status path: events leaving the producer must reach the status, or the producer should
not exist.

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
| PROJ-007 | Single-key refresh + SPV-send return `SingleKeyWalletsUnsupported` | `src/backend_task/core/refresh_single_key_wallet_info.rs:16`; `src/backend_task/core/send_single_key_wallet_payment.rs:19`; `src/backend_task/core/mod.rs:218,303` | LOW | Open by design | Decision #7 (`single-key-mock.md`) |
| PROJ-008 | SEC-002 sign-time passphrase prompt UX | `src/wallet_backend/secret_prompt.rs`; `src/ui/components/secret_prompt_host.rs`; `src/ui/components/passphrase_modal.rs` | MEDIUM | **RESOLVED (`2272bae0..43f412cf`)** | issue #90 — per-secret JIT prompt now shipped |
| PROJ-009 | DIP-14 back-compat dropped (non-mainnet / non-account-0 legacy contact addresses not reproduced) | `src/wallet_backend/mod.rs:722-724` (`register_dashpay_contact`, "Decision #6, back-compat dropped") | MEDIUM | Open by design | Decision #6 (`open-questions.md`) |
| PROJ-010 | `UpstreamFromPersisted` seedless watch-only loader implemented; `SeedReregistrationLoader` removed | `src/wallet_backend/loader.rs`; `src/wallet_backend/mod.rs::load_from_persistor_seedless` | LOW | Resolved (PR #3692 `ddfa66ed`) | `docs/ai-design/2026-06-02-rehydration-rewire/design.md` |
| PROJ-011 | `identity` `CREATE TABLE` still on fresh installs — tombstone ADR pending | `src/database/initialization.rs:845-866` | LOW | Open by design | T-DEV-02b; deferred to separate ADR (`finish-unwire/notes.md` §4) |
| PROJ-022 | `UpstreamPlatformAddresses` reserved swap-target — read methods `unimplemented!()` | `src/wallet_backend/platform_address.rs:245-307` | LOW | Open by design | pending platform todo `e817b66a`; parallels PROJ-010 |

Notes:

- **PROJ-007** narrowed since the design docs: SEC-002 work (`6052dc72`, `48cdb8ad`) made
  single-key **import / sign / list / hydrate** genuinely real
  (`src/wallet_backend/single_key.rs`; `SingleKeyView::import_wif`). UI now imports via
  `ImportSingleKeyDialog` (`src/ui/wallets/wallets_screen/mod.rs:42,157`;
  `src/ui/wallets/import_single_key.rs`). Only balance/UTXO **refresh** and **SPV-based
  send** remain stubbed. The `single-key-mock`/`g2-mock-boundary` "fully read-only mock"
  claim has now been corrected in the design docs (see PROJ-020, RESOLVED).
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
- **`FundWithUtxo` (seed item #15)** — the *removed* asset-lock funding path. Current active
  funding task is `WalletTask::FundPlatformAddressFromWalletUtxos`
  (`src/backend_task/wallet/mod.rs`), a different working path. No live broken surface.

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
| PROJ-005 | platform pin tracks unreleased dev rev (G1) | `Cargo.toml:21,31,32,35` | HIGH | OPEN | Release gate; bump to released rev before ship. Current rev `ddfa66ed373beaebdae9a5d919f896af43cbcd33` (was `35e4a2f6…` at prior refresh, `17653ba8…` at original audit). |
| PROJ-017 | `register_identity_funding_account` absent upstream — DET carries contained exception | `src/wallet_backend/mod.rs:1407` (`provision_identity_funding_account`) / `:1489` (`ensure_identity_funding_accounts`) | LOW | OPEN (tracked, live) | `rs-platform-wallet` has no funding-account registrar sibling to `register_contact_account`. Verified live — called from register/topup (`mod.rs:1261,1325,1371`). Upstream-contribution `9cdcfb25`; persister-load recurrence `a5538dc8`. |

---

## Doc gaps

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-018 | External user docs (dashpay/docs) not yet filed | `docs/ai-design/2026-06-03-pr860-doc-followups/external-docs-draft.md` | MEDIUM | OPEN (tracked) | Draft written; must still be filed as a PR/issue against `github.com/dashpay/docs` after #860 merges. |
| PROJ-019 | ADR floor SHA placeholder unfilled | `docs/ai-design/2026-05-29-finish-unwire/notes.md:92` (`[PLACEHOLDER — fill at merge time]`) | LOW | OPEN | Cannot close pre-merge — needs this PR's squash-merge SHA recorded as the wallet-state floor. |
| PROJ-020 | Design docs claimed single-key is fully read-only mock — was stale | `docs/ai-design/2026-05-18-platform-wallet-migration/single-key-mock.md:51`; `g2-mock-boundary.md:46` | LOW | **RESOLVED (`f39b085d`)** | Prose corrected: `single-key-mock.md:51` now states import/list/sign work in full via `SecretStore`; `g2-mock-boundary.md:46` records the partial capability gap accurately. |
| PROJ-021 | CHANGELOG omits single-key capability limits and DIP-14 trade-off | `CHANGELOG.md:31-46` | LOW | **RESOLVED (`f39b085d`)** | `### Known Limitations` section now states single-key send/refresh is unsupported this release and documents the DIP-14 non-mainnet/non-account-0 contact-fund re-establishment trade-off. |

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

*Candy tally — confirmed gaps: 24 (1 CRITICAL · 3 HIGH · 7 MEDIUM · 13 LOW · 0 INFO).
Status: 13 RESOLVED (PROJ-001, PROJ-002 (removed), PROJ-003, PROJ-004, PROJ-006, PROJ-008,
PROJ-013, PROJ-014, PROJ-020, PROJ-021, PROJ-023, PROJ-025) + SEC-001 follow-up; 11 OPEN; of
those, 4 deferred-by-design and 1 blocked-by-design (PROJ-024). 8 seed/appendix items confirmed
already-resolved with evidence. PROJ-005 remains the sole open merge-blocker.*
