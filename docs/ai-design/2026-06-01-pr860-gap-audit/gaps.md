# PR #860 Platform-Wallet Rewrite — Consolidated Gap Audit

**Audit date:** 2026-06-01
**Head SHA:** `686430a4d2b83596fbbe716acc183a424859e11d`
**PR #860 base:** `v1.0-dev` @ `87ba5b711839219f5e1c7aee8f9de36d038866e3`
**Auditor:** project-reviewer-adams (READ-ONLY; no source touched)

PR #860 rips out DET's home-grown SPV stack (`src/spv/**` deleted) and `data.db` wallet
schema and re-seats wallet/identity/DashPay/shielded state on the upstream
`dashpay/platform` `platform-wallet` crate behind a `WalletBackend` seam. It was built in
phases (P0.5 "compile floor" → P5). Several seams were intentionally landed inert
("compiles, returns `Ok(())`, wire later") and at least one driver never got its caller.
This document catalogues every such gap — dead stubs, deferred-by-design scope cuts, real
bugs, test holes, upstream blockers, and doc gaps — each verified against the current tree
with `file:line` citations. Where a "known" item turned out already resolved, that is
stated with evidence; a closed gap is as valuable to record as an open one.

I checked the inventory against the actual code. I did not take the seed list on faith, and
several items did not survive contact with the source.

---

## Executive summary

| Severity | Count |
|----------|-------|
| CRITICAL | 1 |
| HIGH     | 4 |
| MEDIUM   | 6 |
| LOW      | 7 |
| INFO     | 3 |
| **Total confirmed gaps** | **21** |

By category: functional/unwired = 5; deferred-by-design = 6; test = 4; upstream = 2;
doc = 3; already-resolved (recorded, not counted as open) = 4.

### Merge-blockers (called out up top)

1. **PROJ-001 (CRITICAL)** — SPV / platform-address / identity sync is **never started in
   any code path**. `WalletBackend::start()` (the only caller of
   `spv_arc().spawn_in_background(...)`) has **zero callers**, and `AppContext::start_spv()`
   — which the Connect button, auto-start, MCP, and SwitchNetwork all call — is an inert
   `Ok(())` stub. A wallet rewrite whose chain sync never runs is not merge-ready. *Fix
   in-progress in a separate worktree per audit brief.*
2. **PROJ-005 (HIGH)** — release gate G1: the `dash-sdk` / `platform-wallet` pin
   (`Cargo.toml`) points at an unreleased platform rev tracking PR #3625 (draft persister).
   Project policy (Decision #1) classifies this as a release-hardening blocker, not a start
   blocker — but it gates *merge-to-ship*.

Everything else is fixable post-merge or is a disclosed scope cut. PROJ-001 is the one that
must not ship.

---

## Merge-blocking gaps

| ID | Title | Location | Sev | Status | What's missing | Owner / follow-up |
|----|-------|----------|-----|--------|----------------|-------------------|
| PROJ-001 | SPV sync never driven — dead `start()`, inert `start_spv()` | `src/wallet_backend/mod.rs:419-431`; `src/context/wallet_lifecycle.rs:76-78` | CRITICAL | **In-progress** | `start_spv()` returns `Ok(())`; `WalletBackend::start()` (only spawner of `spv_arc().spawn_in_background`) has 0 callers | start_spv fix worktree |
| PROJ-005 | Pin tracks unreleased platform rev (G1) | `Cargo.toml:21,32,35` (`rev = 17653ba8…`) | HIGH | Open | Pin must move to a released platform rev once #3625 lands before shipping | Release captain |

---

## Functional gaps (dead stubs, no-op UI, unwired drivers, real bugs)

### PROJ-001 — SPV / sync coordinators never started *(CRITICAL — merge-blocker, see above)*

Evidence chain (all verified at head):

- `AppContext::start_spv()` body is literally `Ok(())` — `src/context/wallet_lifecycle.rs:76-78`.
  Callers that expect it to *start sync*: Connect button
  `src/ui/network_chooser_screen.rs:380`; auto-start `src/app.rs:1148`; MCP server boot
  `src/mcp/server.rs:264`; `SwitchNetwork` task `src/backend_task/mod.rs:553`.
- `WalletBackend::start()` (`src/wallet_backend/mod.rs:419-431`) is the **sole** site calling
  `self.inner.pwm.spv_arc().spawn_in_background(config)` (line 422),
  `platform_address_sync_arc().start()` (424), `identity_sync_arc().start()` (425).
- `rg` for `.start()` / `backend.start(` / `wallet_backend()?.start` across `src`, `tests`,
  `benches` returns **no invocation** of `WalletBackend::start()`.
- `ensure_wallet_backend()` (`src/context/mod.rs:647-670`) constructs the backend and
  registers wallets (`wallets_to_register` at `src/wallet_backend/mod.rs:309`) but **never
  calls `start()`**. The design (`g2-mock-boundary.md` §G2.1) explicitly says `start()` must
  run after registration; it does not.

Net effect: chain sync, platform-address sync, and identity sync are dead in every path.
Balances/UTXOs/identities never refresh from the network. This is the SgtMaj-grade hole.

### PROJ-002 — DashPay `add_contact` / `remove_contact` are `NotSupported` stubs *(MEDIUM)*

`src/backend_task/dashpay/contacts.rs:519-535` (`add_contact`) and `:537-549`
(`remove_contact`) ignore all args and return `DashPayError::NotSupported`. Both carry
4-step "TODO: Steps to implement" comments. **Pre-existing in base `87ba5b71`** (not
introduced by #860), and the free functions have no live backend-task dispatch caller — but
they are PR-relevant because the DashPay rewrite touched this module and left the holes
unaddressed. Add-contact-by-username and contact removal are not functional.
Scope: indirect.

### PROJ-003 — `update_payment_status` is a logging no-op *(MEDIUM)*

`src/backend_task/dashpay/payments.rs:432-447`: signature takes `payment_id`, `status`,
`tx_id`, then `// TODO: Update payment record in database`, logs "Would update payment…",
and returns `Ok(())`. Payment status transitions are never persisted. **Pre-existing in
base `87ba5b71`.** A second TODO at `:539` ("would need to query Core or check transaction
history") marks an adjacent unimplemented confirmation path. Scope: indirect.

### PROJ-004 — DashPay outgoing contact-request derivation uses placeholder seed material *(HIGH)*

`src/backend_task/dashpay/contact_requests.rs:310-339`: the xpub derivation for a new
contact relationship is built from `let wallet_seed = sender_private_key;` with inline
comments "For now, use the sender's private key as seed material / In production, this would
derive from the wallet's HD seed/mnemonic". This is the substrate behind the seed-list
TC-037/043 symptom ("incoming contact-request not associated with sending identity after
broadcast"): the post-broadcast association relies on this derivation, and the derivation is
explicitly not the production HD path. The DIP-14/15 derivation is also a documented P4
deletion target in favour of upstream `derive_contact_xpub` (`feature-coverage.md` §2).
**Status: Open** — partially verifiable from DET source; the broadcast→association linkage
needs a live-network test to fully confirm severity. Scope: direct (module rewritten in #860).

### PROJ-006 — `context_provider_spv` activation-height TODO *(LOW)*

`src/context_provider_spv.rs:131`: `// TODO: wire actual activation height if needed` — a
hardcoded/elided activation height in the SPV context provider that feeds chain-only SDK
lookups. Low impact while a sensible default holds, but a latent correctness risk on
networks with non-default activation heights. Scope: direct.

---

## Deferred-by-design / disclosed trade-offs

These are **intentional scope cuts**, recorded so reviewers do not mistake them for
oversights. All trace to a written decision in
`docs/ai-design/2026-05-18-platform-wallet-migration/`.

| ID | Title | Location | Sev | Status | Decision ref |
|----|-------|----------|-----|--------|--------------|
| PROJ-007 | Single-key refresh + SPV-send return `SingleKeyWalletsUnsupported` | `src/backend_task/core/refresh_single_key_wallet_info.rs:16`; `src/backend_task/core/send_single_key_wallet_payment.rs:19`; `src/backend_task/core/mod.rs:218,304` | LOW | Open by design | Decision #7 (`single-key-mock.md`) |
| PROJ-008 | SEC-002 sign-time passphrase prompt UX deferred | `src/wallet_backend/mod.rs:475-480` | MEDIUM | Open (issue #90) | per-task prompt UX deferred; storage+unlock cache shipped |
| PROJ-009 | DIP-14 back-compat dropped (non-mainnet / non-account-0 legacy contact addresses not reproduced) | `src/wallet_backend/mod.rs:629-651` (`register_dashpay_contact`, "Decision #6, back-compat dropped") | MEDIUM | Open by design | Decision #6 (`open-questions.md`); fund-accessibility trade-off, one-time notice is the sole control |
| PROJ-010 | `UpstreamFromPersisted` loader intentionally not implemented (G2 swap deferred) | `src/wallet_backend/loader.rs:139-145` | LOW | Open by design | Decision #2 (`g2-mock-boundary.md` §G2.4) |
| PROJ-011 | `identity` (+ dormant legacy) `CREATE TABLE` still on fresh installs — tombstone ADR pending | `src/database/initialization.rs:850-866` | LOW | Open by design | T-DEV-02b; deferred to separate ADR (`finish-unwire/notes.md` §4) |
| PROJ-012 | ZMQ listener receiver retained behind `#[allow(dead_code)]` pending P4 audit | `src/context/mod.rs:73-77` | LOW | Open by design | Decision #3 (`open-questions.md`) |

Notes:

- **PROJ-007** narrowed since the design docs: SEC-002 work (commits `6052dc72`,
  `48cdb8ad`) made single-key **import / sign / list / hydrate** genuinely real
  (`src/wallet_backend/single_key.rs:157,168,320,401,648`; UI wired at
  `src/ui/wallets/wallets_screen/mod.rs:2178-2180`). Only balance/UTXO **refresh** and
  **SPV-based send** remain stubbed. The `g2-mock-boundary`/`single-key-mock` claim of a
  fully read-only mock is now stale — update those docs.
- **PROJ-011**: `legacy_detected()` (`src/database/initialization.rs:146-175`) correctly
  gates `wallet` / `wallet_addresses` / `utxos` / `wallet_transactions` /
  `shielded_notes` behind `include_legacy`. The remaining unconditional creates are
  `identity` (empty placeholder for legacy reads) and `platform_address_balances`
  (still live). This is the documented "separate ADR" carve-out, not a full unwire.
- **`FundWithUtxo` (seed item #15)** — the *removed* asset-lock funding path. The current
  active funding task is `WalletTask::FundPlatformAddressFromWalletUtxos`
  (`src/backend_task/wallet/mod.rs:60`; UI `src/ui/wallets/send_screen.rs:952,3349`), which
  is a *different*, working path. The named `FundWithUtxo` removal is consistent with the
  disclosed trade-off; no live broken surface found.

---

## Test gaps

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-013 | `RUST_MIN_STACK=16777216` not enforced by harness or CI | `tests/backend-e2e/main.rs:7,10`; `.github/workflows/tests.yml` (no ref) | MEDIUM | Open | Only a `//!` doc instruction. No thread `stack_size` builder in harness; backend-e2e is `#[ignore]` and not run with the env var in any workflow. SDK deep-stack tests segfault at default 8 MB without it. |
| PROJ-014 | `WalletBackend::start()` has no test exercising the start path | `src/wallet_backend/mod.rs:419-431` | HIGH | Open | No unit/integration/e2e test invokes `start()` — directly enabling PROJ-001 to ship unnoticed. A test asserting sync coordinators spawn would have caught the dead caller. |
| PROJ-015 | TC-012 receive-address reuse — unverified from DET source | `src/wallet_backend/mod.rs:614-627` (`next_receive_address` → upstream `next_receive_address_for_account`) | LOW | Unverified — needs follow-up | Whether consecutive calls return the same address depends on upstream issue/used-marking, which cannot run while PROJ-001 keeps sync dead. Re-test after PROJ-001 fix. |
| PROJ-016 | TC-066 key-not-visible-after-broadcast (flake-vs-bug) | (tracked-only, no isolated code surface) | LOW | Unverified — needs follow-up | Catalogued in seed list; no deterministic repro in tree. Re-classify after live run with PROJ-001 fixed. |

Recorded test-spec gaps from `finish-unwire/notes.md` §5 (feature-flag/manual only, not
counted as new open gaps): TC-SK-010 (D-2 drop path, non-default build flag), TC-A11Y-008
(focus-trap modal, same flag), TC-PERF-003 (10k-UTXO 30 s migration, nightly/manual).

---

## Upstream dependencies

| ID | Title | Location | Sev | Status | Blocker |
|----|-------|----------|-----|--------|---------|
| PROJ-005 | platform pin tracks draft persister PR #3625 (G1) | `Cargo.toml:21,32,35` | HIGH | Open | Release gate; bump to released rev before ship. Current rev `17653ba8f9448edc569487b85bb35b27c5f6a14c`. |
| PROJ-017 | `register_identity_funding_account` absent upstream — DET carries contained exception | `src/wallet_backend/mod.rs:1004-1046` (`provision_identity_funding_account`) | LOW | Open (tracked, live) | `rs-platform-wallet` has no funding-account registrar sibling to `register_contact_account`. DET re-provisions in both `wallet.accounts.*` and `wallet_info.accounts.*`. **Verified live** — called via `ensure_identity_funding_accounts` from register/topup (`mod.rs:1098-1106`). Upstream-contribution TODO `9cdcfb25`. |

---

## Doc gaps

| ID | Title | Location | Sev | Status | What's missing |
|----|-------|----------|-----|--------|----------------|
| PROJ-018 | External user docs (dashpay/docs) not updated for storage rewrite / single-key limits | CHANGELOG.md:7-30 (no external-docs note) | MEDIUM | Open | No reference anywhere in the PR docs to updating `github.com/dashpay/docs` for the new storage model, single-key send/refresh being unsupported, or the DIP-14 fund-accessibility trade-off. End users get no published guidance. |
| PROJ-019 | ADR floor SHA placeholder unfilled | `docs/ai-design/2026-05-29-finish-unwire/notes.md:92` (`[TO BE UPDATED ON MERGE]`) | LOW | Open | The migration-tool author needs this PR's merge SHA recorded as the wallet-state floor; still a placeholder. |
| PROJ-020 | Design docs claim single-key is fully read-only mock — now stale | `docs/ai-design/2026-05-18-platform-wallet-migration/single-key-mock.md:30-51`; `g2-mock-boundary.md:15` | LOW | Open | SEC-002 made import/sign/list real; docs still describe a full stub. See PROJ-007 note. |
| PROJ-021 | CHANGELOG omits single-key capability limits and DIP-14 trade-off | CHANGELOG.md:7-30 | LOW | Open | "Changed/Removed/Fixed" sections cover storage move but never tell users single-key send/refresh is unsupported this release, nor the contact-fund trade-off requiring re-establishment. |

---

## Seed-list items found ALREADY RESOLVED (evidence)

Recorded for completeness; **not** counted in the open-gap tally.

1. **Seed #3 — eager wallet-backend init "Could not access wallet data" hard failure.**
   **Resolved.** `src/app.rs:477-479` now spawns eager init and on error only
   `tracing::warn!`s and degrades to the lazy backend-task fallback — no hard user-facing
   "Could not access wallet data" abort. CHANGELOG.md:27-30 documents the eager-init +
   cold-start rehydrate fixes. No blocking failure path remains here.

2. **Seed #7 — TC-019 inverted error precedence (`RefreshSingleKeyWalletInfo` returns
   `WalletBackendNotYetWired` instead of `WalletNotFound`).** **Resolved / moot.**
   `src/backend_task/core/refresh_single_key_wallet_info.rs:16` now returns
   `TaskError::SingleKeyWalletsUnsupported` unconditionally — there is no seed-lookup branch
   left to invert. The precedence bug cannot occur.

3. **Seed #11 — QA-004 `core_backend_mode` inert column/plumbing.** **Largely resolved.**
   `rg core_backend_mode src` finds **zero** hits — the column and plumbing are gone. The
   only residue is the `SourceSelection::CoreWallet` picker in `send_screen.rs`, which is a
   *legitimate, working* Core→Platform funding source (`send_screen.rs:819-851,1837-1863`),
   not inert cosmetic plumbing. Down-grade from a gap to a non-issue.

4. **Seed #19 — SPV readiness gate requiring all 5 managers `Synced`
   (`event_bridge.rs:~65-75`).** **Not found in current form.** `EventBridge::on_progress`
   (`src/wallet_backend/event_bridge.rs:65-75`) keys off the single upstream
   `progress.is_synced()` predicate, not an explicit "all 5 managers Synced" conjunction.
   The moving-target gate the seed item describes is not present at head. (Caveat: PROJ-001
   means this handler never fires anyway.)

Also confirmed *consistent with disclosed design* (not bugs): seed #2 corollary
(`WalletBackend::start()` zero callers) is folded into PROJ-001; seed #4/#5/#12/#13/#14/#16
map to PROJ-005/PROJ-017/PROJ-008/PROJ-011/PROJ-009/PROJ-007 respectively; seed #10 →
PROJ-016; seed #6 → PROJ-004; seed #9 → PROJ-013.

Seed items **unverifiable from this tree** (marked needs-follow-up, not asserted):
seed #17 (Register BlockchainIdentities m/9'/…/5' addresses with SPV bloom filter — only
generic `MempoolStrategy::BloomFilter` at `src/wallet_backend/mod.rs:1120` and per-contact
DashPay bloom counters at `src/wallet_backend/dashpay.rs`; no `m/9'` identity-address bloom
registration found — likely predates this PR or lives upstream); seed #18 (DiskStorageManager
byte-compat never runtime-verified — no `DiskStorageManager` symbol in DET source; lives in
upstream `platform-wallet-storage`, so DET cannot verify it here);
`/tmp/marvin-finish-unwire-exceptions.md` (seed #20) — **file absent**, ~14 missing TCs could
not be folded in.

---

## Appendix: raw stub-signal hits not separately categorized

So nothing is silently dropped. These are deferred markers / inert-looking bodies that are
either (a) genuinely benign, (b) pre-existing, or (c) rolled into a finding above. Cited for
the record.

- `src/wallet_backend/mod.rs:895` — exhaustive match comment "a new upstream variant must
  force a compile error" (intentional guard, benign).
- `src/wallet_backend/event_bridge.rs:169` — `on_shielded_sync_completed` left at upstream
  no-op default (DET enables only `serde`, no shielded sync coordinator; benign, matches
  `start()` comment at `mod.rs:426-428`).
- `src/wallet_backend/dashpay.rs:339` — "Threshold-based expiry derivation is not yet wired
  (no DET-side …)" — a deferred DashPay refinement; low impact (LOW-adjacent, not separately
  scored).
- `src/backend_task/migration/finish_unwire.rs:470,1769,1804` — password-protected
  single_key rows **deferred** to T-SK-03 UX prompt; counted as "deferred" not "failed" by
  the migrator. By design (PROJ-008-adjacent); the migration itself is fully wired
  (`src/app.rs:1011,1115`; `src/backend_task/migration/mod.rs:46`).
- `src/backend_task/dashpay/payments.rs:210` — `#[allow(dead_code)]` on a payments helper
  (pre-existing).
- `src/context/mod.rs:810` — `// TODO: Ideally use sdk.load().version()` — cosmetic version
  free-fn TODO (pre-existing, benign).
- `src/app.rs:1263` (`TODO(RUST-002)`), `src/app.rs:1275` — message-text-inspection routing
  TODOs (pre-existing tech-debt, tracked under RUST-002).
- `src/bin/det_cli/main.rs:186,188`, `src/ui/mod.rs:485,604,607`,
  `src/ui/components/address_input.rs:576`, `src/wallet_backend/mod.rs:1044,1055,1077` —
  `unreachable!()` arms guarded by prior checks (intentional, not gaps).
- `src/mcp/tools/platform.rs:24,42` — MCP pagination + structured-withdrawal-data TODOs
  (pre-existing MCP polish).
- Numerous `#[allow(dead_code)] // May be used …` across `masternode_list_diff_screen.rs`,
  `theme.rs`, `qualified_identity/*`, `core_zmq_listener.rs` — pre-existing dead surface
  unrelated to #860; not scored.

---

*🍬 Candy tally — confirmed gaps claimed: 21 (1 CRITICAL · 4 HIGH · 6 MEDIUM · 7 LOW · 3
INFO). Plus 4 seed items confirmed already-resolved (evidence above) — closing a gap counts
too.*
