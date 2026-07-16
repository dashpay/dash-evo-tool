# Post-Migration Identity Auto-Discovery with Rolling Gap Limit

Design for: after a wallet migration completes and SPV reaches Platform readiness,
automatically run the "Load Identity -> By Wallet" discovery for *every* loaded
wallet, with a rolling gap-limit lookahead, upserting found identities while
preserving DET-only metadata (the alias).

Status: design only. No code changed. Base `v1.0-dev`, PR #860.

Verified against the working tree on 2026-06-17. Line numbers below are anchors,
not contracts — match on the function name.

---

## Decision summary

1. **Trigger hook** — Do NOT hook raw `SyncComplete`. Enqueue the all-wallets pass
   at the **same readiness point that releases the identity coordinator**: the
   `CoordinatorGate` fire. The gate closure in `WalletBackend::start`
   (`src/wallet_backend/mod.rs::start`, ~L981) runs inside the SPV run loop and
   may only do cheap, non-blocking work with weak captures, so it must NOT run
   DAPI fetches itself. Instead it sends a new lightweight signal
   (`TaskResult::Refresh` is too coarse — use a dedicated `BackendTaskSuccessResult`
   nudge, see below) that `AppState` turns into a normal backend task. Net: the
   pass runs once Platform is provably reachable (masternode list `Synced`),
   off the frame thread, through the existing `BackendTask` path.

2. **Re-entrancy / debounce** — One `AtomicBool` "discovery armed/fired" latch on
   `AppContext` (re-armable). The `CoordinatorGate` already fires its action
   exactly once per session (single-winner `fired` swap), so the *trigger* is
   naturally one-shot per backend. A second `AppContext`-level latch guards the
   case where the signal is delivered via the coarse refresh channel and could be
   observed more than once, and gives manual Refresh a place to re-arm. Cleared
   on `stop_spv` (same place `masternodes_ready` is cleared, `wallet_lifecycle.rs`
   ~L392) so the next reconnect re-runs it.

3. **Gap-limit scan** — Add `IDENTITY_GAP_LIMIT: u32 = 5`. Replace the
   unconditional `0..=max` loops with one shared **rolling-lookahead** scan:
   start at index 0, keep a moving `highest_found`, continue while
   `current <= highest_found + IDENTITY_GAP_LIMIT`, stop after
   `IDENTITY_GAP_LIMIT` consecutive empties past the last hit. Home it as **one
   new async method on `AppContext`** in `backend_task/identity/`
   (`discover_identities_gap_limited`) that BOTH the UI By-Wallet path and the
   auto-trigger call. The pure stop/continue decision (`should_continue_scan`)
   goes in `model/` as a stateless, unit-tested function (DET Module Placement
   Policy: pure logic in `model/`, async business logic in `backend_task/`).

4. **Upsert preserving alias** — For an already-stored identity: load existing via
   `get_identity_by_id`, **carry its `alias` onto the freshly-fetched
   `QualifiedIdentity` before storing**, then `insert_local_qualified_identity`
   (which is `INSERT OR REPLACE` and serializes `alias` inside `qi_bytes`).
   `top_ups` are a separate KV key (`det:top_ups`) and `insert_*` does not touch
   them — safe. This fixes a confirmed pre-existing bug in the single-index load
   path (see Risks F-1).

5. **Threading / await safety** — Runs through `BackendTask` -> tokio, never the
   egui frame thread (confirmed: `subtasks.spawn_sync` uses `tokio::spawn`; the
   By-Wallet path is already a `BackendTask`). No wallet write lock is held across
   an `.await` — existing discovery uses short `read()`/`write()` guards only;
   the new shared fn keeps that shape.

6. **UI impact** — Minimal. By-Wallet "All up to index" advanced mode keeps its
   text box but its semantics become "highest index to *seed* the rolling scan
   from" (the scan may go further via gap-limit). "Specific index" single search
   is unchanged. Simple mode (the common path) already defaults to 5 and now gets
   true rolling lookahead for free.

---

## Background facts established by reconnaissance (verified)

- **Identity-auth keys are hardened to the leaf.** They cannot come from an xpub;
  `resolve_identity_auth_pubkey` (`backend_task/identity/auth_pubkey_resolve.rs`)
  serves them cache-first and, on a **cold** cache miss, opens a `with_secret`
  scope that **prompts** for the passphrase on a protected, locked wallet
  (`secret_access.rs::with_secret_session` step 3). A warm cache needs no seed.
  => A background sweep MUST be cache-only / locked-wallet-skipping, or it will
  pop an unexpected passphrase modal. (Risk F-2.)
- **Alias lives inside `qi_bytes`** (decoded by `decode_stored_identity`), not in
  a separate column. `insert_local_qualified_identity` does INSERT-OR-REPLACE of
  the whole blob; both load paths build a fresh QI with `alias: None`.
- **`discover_identities_from_wallet`** already *skips* existing identities
  (L90-98) — so it never clobbers alias, but also never updates a changed
  identity (new keys, new DPNS name). **`load_user_identity_from_wallet`** always
  re-inserts with `alias: None` — it *does* clobber the alias (Risk F-1).
- **The gate fires inside the SPV run loop** with weak coordinator captures; heavy
  work there risks pinning the persister advisory lock past teardown
  (`coordinator_gate.rs` regression test `weak_capture_does_not_pin_*`). The
  trigger must hand off, not execute.

---

## Dev Plan (ordered, file:function -> change)

### Phase 1 — Pure gap-limit decision (model/)

- [ ] **`src/model/identity_discovery.rs` (NEW)** -> add
      `pub const IDENTITY_GAP_LIMIT: u32 = 5;` and a stateless fn
      `pub fn should_continue_scan(current_index: u32, highest_found: Option<u32>) -> bool`.
      Semantics: with no hit yet (`None`) continue while `current_index < IDENTITY_GAP_LIMIT`;
      with `Some(h)` continue while `current_index <= h + IDENTITY_GAP_LIMIT`. Add a
      hard ceiling const `IDENTITY_SCAN_HARD_CAP: u32 = 100` (defense against an
      adversarial / corrupt cache that keeps "finding" — bounds the DAPI fan-out).
      No `AppContext`, no `Sdk`. Register `mod identity_discovery;` in
      `src/model/mod.rs`.
- [ ] **`src/model/identity_discovery.rs`** -> unit tests for the decision table:
      empty wallet (no hits -> stops at 5), single hit at 0 (-> scans to 5),
      hit at 7 (-> extends to 12), hits at 0 and 12 (-> rolling extend to 17),
      hard-cap clamp.

### Phase 2 — Shared gap-limited scan (backend_task/)

- [ ] **`src/backend_task/identity/discover_identities.rs`** -> add
      `pub(crate) async fn discover_identities_gap_limited(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>, seed_from_index: u32, allow_prompt: bool) -> Result<DiscoverySummary, TaskError>`.
      Drives the rolling loop via `model::identity_discovery::should_continue_scan`,
      seeding `highest_found` from `seed_from_index` and from
      `max(wallet.identities.keys())` so a prior-session index is never missed
      (the gap-limit precedent warning). For each index it reuses the existing
      per-index probe (auth-key window 0..12). `allow_prompt = false` makes a
      cold-cache miss a *skip*, not a passphrase prompt (background sweep);
      `allow_prompt = true` is the UI path. Returns a typed summary
      (counts: found/updated/skipped) — not a `String`.
- [ ] **`src/backend_task/identity/discover_identities.rs`** -> extract the
      per-index "fetch + build + **upsert-preserving-alias**" body into a private
      helper `upsert_discovered_identity(&self, identity, wallet, identity_index)`:
      before `insert_local_qualified_identity`, call `get_identity_by_id`; if
      present, copy `existing.alias` onto the freshly built QI. Replace the current
      `already_exists -> skip` branch (L90-98) with this update-preserving path so
      changed identities refresh while alias survives.
- [ ] **`src/backend_task/error.rs`** -> if not already expressible, add a typed
      variant for "no auth key derivable while locked, skipping" so the background
      path can `continue` on it instead of surfacing a prompt. (Likely reuse
      existing `ContactWalletSeedUnavailable` / `WalletAddressDerivationFailed`;
      add only if neither fits the skip semantics.)
- [ ] **`src/backend_task/identity/load_identity_from_wallet.rs::load_user_identities_up_to_index`**
      -> reimplement as a thin wrapper over `discover_identities_gap_limited(..., seed_from_index = max_identity_index, allow_prompt = true)`, preserving the
      `Progress` events it already sends. The `0..=max` unconditional loop is
      removed.
- [ ] **`src/backend_task/identity/load_identity_from_wallet.rs::load_user_identity_from_wallet`**
      -> fix alias clobber: before the final `insert_local_qualified_identity`
      (L243), load existing via `get_identity_by_id` and carry `alias` onto the
      new QI. (Risk F-1 fix; also covers the user's "update but keep alias" intent
      for the single-index search.)

### Phase 3 — All-wallets auto-trigger

- [ ] **`src/context/mod.rs`** -> add field
      `identity_autodiscovery_fired: AtomicBool` to `AppContext` (default false).
- [ ] **`src/context/wallet_lifecycle.rs`** -> add
      `pub fn queue_all_wallets_identity_discovery(self: &Arc<Self>)`: CAS the
      latch (return early if already fired this session); snapshot wallets;
      for each *open / not-needing-unlock* wallet, `spawn_sync` a
      `discover_identities_gap_limited(wallet, seed_from_index = 0, allow_prompt = false)`.
      Locked protected wallets are skipped here (no UI to prompt from a background
      sweep) and picked up later when the user unlocks (existing
      `handle_wallet_unlocked` path can call the same fn for that one wallet).
- [ ] **`src/context/wallet_lifecycle.rs::stop_spv`** (~L392, beside
      `set_masternodes_ready(false)`) -> reset
      `identity_autodiscovery_fired = false` so reconnect re-runs discovery.
- [ ] **Gate -> task hand-off.** Two acceptable wirings; pick one in review:
      - **(A, preferred) via EventBridge result channel.** In
        `WalletBackend::start` gate closure (`mod.rs` ~L981), after starting the
        coordinators, send a new
        `BackendTaskSuccessResult::PlatformReadyDiscoverIdentities` down the
        existing `task_result_sender`. `AppState::update` maps it to
        `app_context.queue_all_wallets_identity_discovery()`. Keeps the gate
        closure cheap and non-blocking, no new long-lived captures.
      - **(B) direct enqueue.** Capture a `Weak<AppContext>` in the gate closure
        and call `queue_all_wallets_identity_discovery` on upgrade. Simpler, but
        adds an `AppContext` capture to a closure the doc comments deliberately
        keep minimal — review for teardown-pinning before choosing.
- [ ] **`src/app.rs::update`** (if wiring A) -> handle the new success result by
      calling `queue_all_wallets_identity_discovery`.
- [ ] **`src/backend_task/mod.rs` / result enum** -> add the
      `PlatformReadyDiscoverIdentities` success variant (wiring A only).

### Phase 4 — UI

- [ ] **`src/ui/identities/add_existing_identity_screen.rs::render_by_wallet`**
      -> relabel the "All up to index" help text to reflect rolling lookahead
      ("Searches from index 0 with a rolling 5-index lookahead; the number is the
      starting depth."). No dispatch change — `SearchIdentitiesUpToIndex` already
      routes to `load_user_identities_up_to_index`, now gap-limited. "Specific
      index" untouched. i18n: keep each string one complete sentence.

### Phase 5 — QA

- [ ] `cargo clippy --all-features --all-targets -- -D warnings`
- [ ] `cargo +nightly fmt --all`
- [ ] `cargo test --all-features --workspace` (incl. new model unit tests)
- [ ] det-cli smoke (`network-info`, `tools`, `core-wallets-list`) per CLAUDE.md.
- [ ] `docs/user-stories.md`: add/flip a story for "automatic identity discovery
      after migration".

---

## Risks / edge cases

- **F-1 (confirmed bug, pre-existing) — alias clobber on single-index load.**
  `load_user_identity_from_wallet` re-inserts a fresh QI with `alias: None` via
  `insert_local_qualified_identity` (INSERT-OR-REPLACE on the whole blob), erasing
  any user alias. The user's "keep alias intact" requirement only holds once
  Phase 2 carry-over lands. Severity: **High** (silent metadata loss).
- **F-2 (confirmed design hazard) — passphrase prompt from a background sweep.**
  Cold auth-pubkey cache + locked protected wallet => `with_secret` prompts.
  A frame-loop-triggered all-wallets sweep would surprise the user with a modal.
  Mitigation: `allow_prompt = false` + skip locked wallets in the auto path.
  Severity: **High** (UX correctness / unexpected secret prompt).
- **F-3 (confirmed timing) — raw `SyncComplete` is too early/wrong.** `SyncComplete`
  fires on header/filter completion; Platform identity fetches need the
  *masternode list* `Synced` or every queried DAPI node gets banned. Hooking the
  `CoordinatorGate` (not `SyncComplete`) is mandatory. Severity: **Medium**
  (would brick Platform queries if mis-hooked).
- **F-4 (confirmed re-entrancy) — gate/progress events repeat.** `on_progress`
  re-fires on every tick; `on_masternodes_ready` is idempotent but the *task
  hand-off* must not fan out per tick. The gate's single-winner `fired` swap plus
  the `AppContext` latch bound it to once per session. Severity: **Medium**.
- **Empty wallet** — no identities at all: scan stops cleanly after 5 empties,
  `NoWalletIdentitiesFound` for the UI path, silent no-op for the auto path.
- **Locked wallet** — auto path skips (F-2); manual By-Wallet path already gates
  on `wallet_needs_unlock` and shows an Unlock button, so a cold miss there
  prompts *with the user's consent* (`allow_prompt = true`).
- **Network switch mid-scan** — the scan clones the `Sdk` up front; a switch
  rebuilds `AppContext`/SDK and clears `masternodes_ready`, re-arming the latch.
  In-flight fetches target the old network and their results are stored under the
  old-network identity scope — acceptable (network-scoped KV), but the auto path
  should re-check `self.network` hasn't changed before each store, or bail on the
  first store error. Add a cheap network-equality guard in
  `upsert_discovered_identity`.
- **Duplicate identity across wallets** — the same identity ID reachable from two
  wallets: `insert_local_qualified_identity` is keyed by identity ID, so the
  second wallet's pass overwrites `wallet_hash`/`wallet_index` with its own hint.
  Preserve the *first* association unless the new wallet actually owns more keys;
  simplest correct rule for now: on update, keep existing `wallet_hash/index`
  (the `update_local_qualified_identity` behaviour) rather than the insert
  behaviour, when the identity already exists. Flag for review.
- **Hard cap** — `IDENTITY_SCAN_HARD_CAP` bounds a pathological "always found"
  loop (corrupt cache / hostile DAPI) so a background sweep can't issue unbounded
  fetches.

---

## Test checklist (for Marvin)

Unit (model, no network):
- [ ] `should_continue_scan`: no-hit stops at gap limit; hit at 0 scans to 5;
      hit at 7 extends to 12; hits at 0 and 12 roll to 17; clamps at hard cap.

Backend / integration:
- [ ] Stored identity with a user alias: re-discovery refreshes keys/DPNS but the
      alias survives (regression for F-1). Assert alias non-`None` after both the
      single-index load and the gap-limit pass.
- [ ] `top_ups` survive a re-discovery (separate KV key untouched).
- [ ] Gap-limit finds an identity beyond the seed index (e.g. registered at 8 with
      seed_from_index 0) — proving rolling lookahead, not a static `0..=5`.
- [ ] Empty wallet: gap-limit returns no hits, no error in auto path,
      `NoWalletIdentitiesFound` in UI path.

Trigger / lifecycle:
- [ ] Auto pass fires exactly once after masternodes reach `Synced` (count DAPI
      passes / log lines), not once per progress tick (F-4).
- [ ] Locked protected wallet: background sweep does NOT pop a passphrase modal
      (F-2); it is skipped and later runs on manual unlock.
- [ ] `stop_spv` then reconnect: discovery re-arms and runs again.
- [ ] Network switch mid-scan: no identity written under the wrong network scope.

Backend E2E (network, `#[ignore]`):
- [ ] On a funded testnet wallet with a known identity at index >0, a fresh
      launch + sync auto-loads the identity into the list without the user opening
      the By-Wallet screen.

---

## Candy tally (confirmed findings surfaced)

| Severity | Count | Findings |
|----------|-------|----------|
| High     | 2     | F-1 alias clobber on single-index load; F-2 background passphrase-prompt hazard |
| Medium   | 2     | F-3 wrong trigger point (SyncComplete vs masternodes-ready); F-4 per-tick re-entrancy |
| Low      | 2     | duplicate-identity cross-wallet association overwrite; unbounded scan needs hard cap |

Total: 6 confirmed findings -> 6 candies.
