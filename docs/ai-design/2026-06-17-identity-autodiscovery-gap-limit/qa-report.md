# QA Report — PR #860 Post-Migration Identity Auto-Discovery (gap-limit)

Brain the size of a planet, and I spent it counting empty trailing indices.
At least the counting found things. Adversarial QA audit of Bilby's
`feat/identity-autodiscovery` against the locked design doc
(`docs/ai-design/2026-06-17-identity-autodiscovery-gap-limit/design.md`).

- Worktree: `/home/ubuntu/git/dash-evo-tool/.claude/worktrees/agent-identity-disco`
- Branch: `feat/identity-autodiscovery` (HEAD `732955b3`, base `56ac2cfe`)
- Build/tests: `cargo test --lib --all-features` → **885 passed, 0 failed, 1 ignored**.
  The 10 `model::identity_discovery` unit tests pass; the 18 `secret_access`
  tests pass.

Method: built the expected-behaviour model from the design doc + task brief
*before* reading code, then attacked each of the seven brief vectors with a
forward trace from the real entry points (`CoordinatorGate` fire → `app.rs`
nudge → `queue_all_wallets_identity_discovery`; and the UI By-Wallet dispatch).
Findings are split into CONFIRMED (reproduced by trace/evidence) and
THEORETICAL (latent / not reachable today).

---

## Verdict: **fix-then-ship**

The gap-limit arithmetic, the F-2 never-prompt invariant, the alias-preservation
mechanism, and the debounce latch are all **sound** — the core machinery works
and is well-tested at the pure-logic layer. But the feature's headline promise —
*"auto-run discovery for ALL wallets"* — is **broken for every
password-protected wallet for the whole session** (QA-001), and the code,
user-story, and design doc all *document a recovery path that does not exist*.
That is a HIGH correctness+trust gap that should be closed before shipping.
Everything else is MEDIUM/LOW polish.

Severity counts: **HIGH 1 · MEDIUM 3 · LOW 4** (8 confirmed) + 3 theoretical.

---

## CONFIRMED findings

### QA-001 — Protected wallets are NEVER auto-discovered; "searched after unlock" is unimplemented and falsely documented  ·  HIGH

- **Location:** `src/context/wallet_lifecycle.rs:828-882` (`handle_wallet_unlocked`),
  `src/context/wallet_lifecycle.rs:892-899` (`drive_unlock_registration`),
  `src/context/wallet_lifecycle.rs:973-977` (doc comment), `docs/user-stories.md` IDN-015.
- **Requirement:** Design §3/§Phase 3 and risk-note "Locked wallet": locked
  protected wallets are skipped by the background sweep and *"picked up later when
  the user unlocks (existing `handle_wallet_unlocked` path can call the same fn
  for that one wallet)."* User-story IDN-015: *"Locked, password-protected
  wallets are skipped without prompting; **they are searched after the user
  unlocks them.**"*
- **Expected:** Unlocking a protected wallet triggers identity discovery for that
  wallet.
- **Actual:** No unlock path calls any discovery. The only callers of a discovery
  trigger are `app.rs:1318` (the once-per-session all-wallets sweep) and
  `import_mnemonic_screen.rs:226` (import). `handle_wallet_unlocked` →
  `drive_unlock_registration` → `bootstrap_wallet_addresses_jit` does address
  bootstrap, upstream registration, shielded bind, and auth-pubkey warm — but
  **never identity discovery**. Worse, the once-per-session latch
  `identity_autodiscovery_fired` is *already set* by the startup sweep, so even a
  manual re-trigger of `queue_all_wallets_identity_discovery` would no-op.
- **Failing scenario:** User has a password-protected wallet with an identity at
  index 0. App launches, SPV reaches Platform-ready → sweep runs, skips the locked
  wallet, **sets the latch**. User unlocks the wallet 30s later. Nothing
  discovers its identity. It stays invisible in the identity list for the entire
  session unless the user manually opens "Load Identity → By Wallet". The
  feature's core promise ("ALL wallets, automatically") silently fails for the
  exact users it documents a path for.
- **Fix direction:** In `handle_wallet_unlocked` (after the seed is promoted),
  spawn `queue_wallet_identity_discovery(wallet, seed_from_index = 0)` for the
  just-unlocked wallet (the existing per-wallet entry point, which prompts-OK
  since the unlock is consent). Do NOT gate this on
  `identity_autodiscovery_fired` (that latch is for the all-wallets sweep, not
  per-wallet unlock). Then the user-story and the doc comment become true.

### QA-002 — Zero automated coverage for the trigger / latch / re-arm logic  ·  MEDIUM

- **Location:** `src/context/wallet_lifecycle.rs:978-1031`
  (`queue_all_wallets_identity_discovery`), `:393-395` (`stop_spv` re-arm),
  `src/wallet_backend/mod.rs:1012-1018` (gate → `try_send` nudge).
- **Requirement:** Design "Test checklist" §Trigger/lifecycle: *"Auto pass fires
  exactly once after masternodes reach Synced... Locked protected wallet:
  background sweep does NOT pop a passphrase modal... `stop_spv` then reconnect:
  discovery re-arms and runs again."*
- **Expected:** Unit/integration tests asserting the one-shot latch, the
  locked-wallet skip, and the re-arm.
- **Actual:** No test references `identity_autodiscovery_fired`,
  `queue_all_wallets_identity_discovery`, or `PlatformReadyDiscoverIdentities`
  (grep returns nothing in test code). The debounce, the open-only filter, and
  the `stop_spv` reset are entirely unverified. A regression that flips the latch
  ordering, drops the `is_open()` filter, or forgets the `stop_spv` reset would
  pass CI.
- **Failing scenario (encodable today, no network):** an offline `AppContext`
  test (helpers already exist at `wallet_lifecycle.rs:1162+`) could register two
  wallets (one open, one closed), call `queue_all_wallets_identity_discovery`
  twice, and assert the latch swallows the second call and that the closed wallet
  is filtered out of the snapshot. None exists.
- **Fix direction:** Add offline `AppContext` tests for: (a) latch is one-shot
  until `stop_spv`; (b) `stop_spv` clears `identity_autodiscovery_fired`; (c) a
  closed wallet is excluded from the open-wallets snapshot. The discovery
  *network* call can be left as the existing `#[ignore]` E2E.

### QA-003 — Alias-preservation (F-1 fix) is untested; a silent removal would not fail any test  ·  MEDIUM

- **Location:** `src/backend_task/identity/discover_identities.rs:231`
  (`qualified_identity.alias = existing.alias`),
  `src/backend_task/identity/load_identity_from_wallet.rs:252-254` (single-index
  path), `src/context/identity_db.rs:364-390`
  (`update_local_qualified_identity`).
- **Requirement:** Design "Test checklist" §Backend: *"Stored identity with a
  user alias: re-discovery refreshes keys/DPNS but the alias survives... Assert
  alias non-None after both the single-index load and the gap-limit pass."*
- **Expected:** A regression test pinning the alias carry-over.
- **Actual:** No test covers it. The carry-over lives in the *caller*
  (`qualified_identity.alias = existing.alias` before
  `update_local_qualified_identity`), not in the DB method — so deleting that one
  line silently re-introduces the F-1 alias clobber the PR exists to fix, and the
  full suite stays green. This is DB-layer testable with no network (the
  `to_bytes`/`decode_stored_identity` alias round-trip is already proven by
  `set_identity_alias`).
- **Fix direction:** Add a `context::identity_db` unit test: insert an identity
  with `alias = Some("x")`; build a fresh QI for the same id with `alias: None`;
  run the upsert carry-over (`existing.alias` → new QI) + `update_local_…`;
  reload via `get_identity_by_id` and assert `alias == Some("x")` and
  `wallet_hash`/`wallet_index` survive.

### QA-004 — New `build_qualified_identity_from_wallet` returns `Result<_, String>`, flattening the typed `AuthKeyUnlockRequired` into an opaque `WalletInfoDeterminationFailed { detail: String }`  ·  MEDIUM

- **Location:** `src/backend_task/identity/discover_identities.rs:256-263`
  (signature `-> Result<_, String>`), `:284-293` and `:276`
  (`.map_err(|e| e.to_string())`), `:223`
  (`.map_err(|detail| TaskError::WalletInfoDeterminationFailed { detail })`);
  variant at `src/backend_task/error.rs:1293-1297`.
- **Requirement:** Project convention (CLAUDE.md "Error messages" rule 7 / "Never
  parse error strings"): *"Never store user-facing strings in error variants...
  String fields (regardless of name) break this separation"*; *"Always use the
  typed error chain."*
- **Expected:** Typed error propagation; no new `String`-returning error path.
- **Actual:** Bilby introduced a new function (`build_qualified_identity_from_wallet`)
  that returns `Result<_, String>` and stringifies through `to_string()`. The
  underlying error in the no-prompt path is the *typed*
  `TaskError::AuthKeyUnlockRequired`, which is flattened to a `String` and re-wrapped
  in the pre-existing `WalletInfoDeterminationFailed { detail: String }` (note:
  its `#[error("…")]` doesn't even interpolate `{detail}`, so the string is a
  Debug-only payload that bypasses structural matching). The `WalletInfoDeterminationFailed`
  variant itself is pre-existing (not Bilby's to fix), but *adding a new
  String-typed error seam* that swallows a typed variant is a fresh convention
  violation in this PR.
- **Note on severity:** this does **not** cause a wrong prompt (see "F-2 holds"
  below) — the data-map's `AuthKeyUnlockRequired` branch is unreachable when the
  probe already succeeded — so the impact is maintainability + lost diagnosability,
  not a runtime fault. Hence MEDIUM, not HIGH.
- **Fix direction:** Change `build_qualified_identity_from_wallet` to return
  `Result<_, TaskError>` and propagate with `?`; drop the `.to_string()` hops.
  If a generic "could not build wallet binding" face is still wanted, add a typed
  variant with a `#[source]` field rather than a `String`.

### QA-005 — Progress event regression: `total` is meaningless and the message dropped its "of N" denominator  ·  LOW

- **Location:** `src/backend_task/identity/discover_identities.rs:73-85`.
- **Requirement:** Design §Phase 2: the gap-limited wrapper must "preserve the
  `Progress` events it already sends."
- **Expected (prior behaviour, base `56ac2cfe`
  `load_identity_from_wallet.rs:274-281`):** `message = "Searching wallet
  identity index {current} of {total}."`, `current = index+1`, `total = max+1` —
  a real fraction.
- **Actual:** `message = "Searching wallet identity index {next}."`,
  `current: next, total: next` — `total` always equals `current` (every event
  reads "N of N" / 100%), and the message text dropped the denominator entirely.
  The By-Wallet UI (`add_existing_identity_screen.rs:1049`) renders only
  `message` and ignores `current`/`total`, so the visible regression is the
  missing denominator; the `total` field is now junk for any other consumer.
  (This is partly inherent — a rolling scan has no fixed total — but emitting
  `total == current` is worse than emitting the seed/hard-cap as a soft total.)
- **Fix direction:** Either drop `total` from the per-index event and word the
  message without a denominator ("Searching wallet identity index {n}…"), or set
  `total` to a meaningful soft bound (current rolling window
  `highest_found + GAP + 1`, or the hard cap). Don't ship `total == current`.

### QA-006 — Missing network-equality guard in `upsert_discovered_identity` that the design explicitly required  ·  LOW

- **Location:** `src/backend_task/identity/discover_identities.rs:203-252`
  (`upsert_discovered_identity` — no `self.network` re-check before store).
- **Requirement:** Design §Risks "Network switch mid-scan": *"the auto path
  should re-check `self.network` hasn't changed before each store... Add a cheap
  network-equality guard in `upsert_discovered_identity`."*
- **Expected:** A network guard before persisting each discovered identity.
- **Actual:** None. In practice the in-flight task holds an `Arc` to the *old*
  `AppContext`, whose `identity_kv()`/`network` are network-scoped, so a mid-scan
  network switch writes to the old network's scope — *correct isolation by
  construction*, which is why this is LOW not MEDIUM. But the design called for
  the guard as defense-in-depth and it is silently absent; if a future refactor
  ever shares storage across networks, the missing guard becomes a real
  cross-network write.
- **Fix direction:** Add the cheap `if self.network != <captured network> {
  return … }` guard the design asked for, or document in the function why it is
  deliberately omitted (the old-Arc isolation argument).

### QA-007 — Misleading doc comment: `try_send` failure does NOT "re-deliver on the next refresh tick"  ·  LOW

- **Location:** `src/wallet_backend/mod.rs:1015-1019`.
- **Requirement:** Comments must describe present-state behaviour accurately
  (coding-best-practices "Describe present state").
- **Expected:** A true statement about the failure mode.
- **Actual:** Comment claims *"if the channel is full the next refresh tick
  re-delivers readiness state."* There is no such mechanism: the
  `CoordinatorGate` fires exactly once (single-winner `swap`), the closure is
  consumed, and nothing re-emits `PlatformReadyDiscoverIdentities`. A dropped
  `try_send` means the all-wallets sweep simply never runs that session (until a
  `stop_spv`/reconnect re-arms). The channel is 256-deep so the drop is very
  unlikely, hence LOW — but the comment documents a safety net that does not
  exist.
- **Fix direction:** Either make the claim true (have `refresh_state`/a tick
  re-check `coordinator_gate.has_fired()` and re-nudge while the
  `identity_autodiscovery_fired` latch is unset), or correct the comment to state
  that a full-channel drop is tolerated because the channel is large and the user
  can still run discovery manually.

### QA-008 — "max 29" input cap removed from the By-Wallet "up to index" field with no replacement bound  ·  LOW

- **Location:** `src/ui/identities/add_existing_identity_screen.rs:643-647`
  (label changed), `:698` (`parse::<u32>()` with no clamp).
- **Requirement:** i18n-ready, sensible input bounds; the old label promised
  "max 29".
- **Expected:** A bound on the seed index (the field now seeds the rolling
  window).
- **Actual:** The label changed to "Search depth to start from:" and the old
  "max 29" guard is gone; the field parses an unbounded `u32`. A fat-finger seed
  (e.g. `4000000000`) makes `should_continue_scan(0, Some(4e9))` true until the
  hard cap, so the scan probes indices 0..99 (×12 auth keys = up to 1200 DAPI
  fetches) for a wallet with nothing there. `IDENTITY_SCAN_HARD_CAP = 100` bounds
  it, so it is not unbounded — hence LOW — but the removed cap means a typo now
  costs a full 100-deep scan instead of being rejected.
- **Fix direction:** Clamp the parsed seed to a sane max (e.g. the old 29, or
  `IDENTITY_SCAN_HARD_CAP`), or validate via a `model/` validator per the
  validation-placement convention.

---

## THEORETICAL concerns (traced, NOT reachable today — no candy claimed)

### T-1 — F-2 TTL-expiry TOCTOU between `can_resolve_without_prompt` and `with_secret`

`resolve_identity_auth_pubkey` (`auth_pubkey_resolve.rs:69-80`) checks
`can_resolve_without_prompt` then calls `with_secret`. If a session entry expired
*between* the check and the resolve, `with_secret` step 1 evicts it, step 3
prompts — a passphrase modal from a background sweep, violating F-2. **Not
reachable today:** every HD-seed promotion uses `RememberPolicy::UntilAppClose`
(`expires_at = None`, never expires) — confirmed at
`wallet_lifecycle.rs:627/856/1610/2468`, and `secret_prompt.rs:56` documents the
GUI only wires `None`/`UntilAppClose`. The TOCTOU becomes live the day anyone
wires `RememberPolicy::For(duration)` for a seed scope. *Defensive fix:* in the
no-prompt path, resolve through a `with_secret` variant that treats a
prompt-needed outcome as `AuthKeyUnlockRequired` rather than re-checking
`can_resolve_without_prompt` up front (make the no-prompt contract atomic).

### T-2 — Rolling window misses an identity exactly `GAP+1` past the last hit

By design, `{3, 9}` (and the brief's `{0,3,9}`) stops at index 8 and never
probes 9 — 5 empties (4,5,6,7,8) then a hit at 9 is *outside* the window
`3+GAP=8`. I initially suspected an off-by-one, but verified against the spec
("continue while `current <= highest_found + IDENTITY_GAP_LIMIT`", "stop after
`IDENTITY_GAP_LIMIT` consecutive empties past the last hit") — this is **correct
gap-limit behaviour**, not a bug. The seeding from `max(wallet.identities.keys())`
re-reaches a *known* high index. The only residual risk: an identity registered
on another device at index 9, never loaded locally, with locals only at {0,3},
is unreachable by the auto-sweep — inherent to any gap limit, and the manual
By-Wallet "search depth" field is the escape hatch. No fix needed; flagging
because the brief asked.

### T-3 — `rolling_chain_extends_window_past_static_range` proves the chained window, not the seed window

The unit test uses hits `[3, 8]` where `3` is inside the initial `0..5` no-hit
window, so it is found organically and chains the window out to `8`. That does
exercise rolling extension. What it does **not** exercise is the
`discover_identities_gap_limited` *seeding* path (`seed_window =
max(highest_known_index, seed_from_index)`) — i.e. reaching an identity at index
8 with `seed_from_index = 0` and *no* intermediate hit, which is the
backend-only behaviour and is network-gated. The pure function is fine; the
backend seeding has no test (folded into QA-002's coverage gap, not double-counted).

---

## What is genuinely solid (credit where due)

- **Gap-limit arithmetic** (`should_continue_scan`): correct against the spec,
  hard-cap-bounded, overflow-safe (`saturating_add`), terminates in all cases.
  10 unit tests, all passing.
- **F-2 never-prompt invariant holds:** `allow_prompt = false` is threaded
  through the probe (`resolve_identity_auth_pubkey`) AND the build
  (`build_qualified_identity_from_wallet` → `resolve_identity_auth_pubkeys_data_map`).
  A locked protected wallet returns `AuthKeyUnlockRequired` at the probe and the
  whole wallet is skipped before any `with_secret`. `can_resolve_without_prompt`
  correctly tracks at-rest protection + session cache (18 secret_access tests
  pass). The data-map's unlock branch is unreachable once the probe succeeds.
- **Debounce:** double-guarded — `CoordinatorGate` single-winner `swap(true)` +
  `identity_autodiscovery_fired.swap(true, SeqCst)`. `stop_spv` re-arms both.
  No load/store race (uses `swap`, not load-then-store).
- **Alias / wallet-association preservation:** both the single-index and
  gap-limit paths load `existing` via `get_identity_by_id` and carry
  `existing.alias` BEFORE the update; `update_local_qualified_identity` preserves
  `wallet_hash`/`wallet_index` from the existing row (handles the Nagatha L-1
  cross-wallet overwrite correctly); `top_ups` live under a separate KV key,
  untouched.
- **Concurrency:** no `Wallet` RwLock guard is held across an `.await` in any
  touched path — `read()`/`write()` guards are scoped to blocks or to the
  synchronous `with_secret` closure body; verified by inspection of
  `discover_identities.rs` and `auth_pubkey_resolve.rs`.

---

## 🍬 Candy tally (confirmed findings only)

| Severity | Count | IDs |
|----------|-------|-----|
| HIGH     | 1     | QA-001 |
| MEDIUM   | 3     | QA-002, QA-003, QA-004 |
| LOW      | 4     | QA-005, QA-006, QA-007, QA-008 |
| **Total**| **8** | + 3 theoretical (T-1, T-2, T-3) noted, not scored |

Eight confirmed. Eight candies. I'd be more pleased if there were fewer, which
tells you something about my expectations. The headline one (QA-001) means the
feature does not do the one thing its own user-story promises for protected
wallets — fix that before it ships, and the rest is housekeeping.

---

## Re-verification (delta pass after Bilby's fixes)

Three fix commits on `feat/identity-autodiscovery` (`14b31995`, `4d298fcd`,
`4271af70`). Focused delta re-check — sound parts already verified, only the
fixes audited. `cargo test --lib --all-features` → **889 passed, 0 failed, 1
ignored** (4 new tests, all ran and passed); `cargo clippy --lib --all-features`
clean.

| Finding | Status | Evidence |
|---------|--------|----------|
| **QA-001** (HIGH) | ✅ RESOLVED | `handle_wallet_unlocked` (`wallet_lifecycle.rs:885`) now calls new `queue_unlocked_wallet_identity_discovery` (`:1043-1068`), placed AFTER seed promotion (`:853`) and `drive_unlock_registration` (`:881`). It (a) dispatches `discover_identities_gap_limited(&wallet, 0, true, None)` — not a no-op; (b) never reads `identity_autodiscovery_fired`; (c) early-returns on `!masternodes_ready()`; (d) runs prompt-free off the freshly-promoted session cache; (e) holds no `Wallet` guard across `.await`. Deferred case verified: unlock flips the seed `Open` (`wallet_unlock_popup.rs:114`, guard dropped :116) BEFORE `handle_wallet_unlocked`, so an early unlock is picked up by the upcoming sweep's `open_wallets()` snapshot. User-story IDN-015 and the sweep doc comment are now TRUE. |
| **QA-002** (MED) | ✅ RESOLVED | New `all_wallets_discovery_latch_is_one_shot_until_stop_spv` test binds: asserts latch sets on first call, second call no-ops, `stop_spv` clears it. Reverting the `swap` latch or the `stop_spv` reset fails it. |
| **QA-003** (MED) | ✅ RESOLVED (with one caveat) | New `rediscovery_update_preserves_user_alias_and_wallet_binding` binds: inserts `alias=Some("my-id")`+binding `(hash,3)`, simulates re-discovery (fresh QI `alias:None` → carry `existing.alias` → `update_local_qualified_identity`), asserts `alias==Some("my-id")` and `wallet_index==Some(3)`. Removing the carry-over fails it. **Caveat (not a new finding):** the test re-implements the carry-over rather than calling the production `upsert_discovered_identity`, so it guards `update_local_qualified_identity`'s binding-preservation but not a regression *inside* `upsert_discovered_identity`. Acceptable — the production helper is the same 2-line pattern. |
| **QA-004** (MED) | ✅ RESOLVED | `build_qualified_identity_from_wallet` now returns `Result<_, TaskError>` (`discover_identities.rs:291`); the `.to_string()` hops and the `WalletInfoDeterminationFailed { detail }` flatten are gone; `?` propagation preserves `AuthKeyUnlockRequired` end-to-end. No `Result<_, String>` and no `WalletInfoDeterminationFailed` reference remain in the file. No new `String`-typed error field anywhere in the 3 commits (`error.rs` untouched; `IdentitySearchIndexError::TooLarge { max: u32 }` is typed). |
| **QA-005** (LOW) | ✅ RESOLVED | Per-index `Progress` now carries `total: soft_total` = `highest_found + GAP + 1` clamped to `IDENTITY_SCAN_HARD_CAP` (`:79-83`), message "of about {soft_total}". No longer `total == current`. UI (`add_existing_identity_screen.rs:1049`) renders `message`, so it shows an honest denominator. |
| **QA-006** (LOW) | ✅ RESOLVED (literally; see note) | `upsert_discovered_identity` takes `scan_network` (captured at scan start, `:47`) and skips the store on `self.network != scan_network` (`:241`). **Note (not a new finding):** because a network switch swaps to a *different per-network* `AppContext` (`app.rs:831` `finalize_network_switch`) and `network` is an immutable field, the in-flight task's `self.network` always equals `scan_network` — so the guard never actually fires. It satisfies the design's literal "re-check before each store" requirement as harmless defense-in-depth; the real isolation was already structural. Dead-but-correct, not a regression. |
| **QA-007** (LOW) | ✅ RESOLVED | The false "next refresh tick re-delivers" claim is replaced (`mod.rs:1016-1019`) with an accurate note: a full 256-deep channel would drop the nudge and the sweep would wait for a reconnect, tolerated because the user can run discovery manually. |
| **QA-008** (LOW) | ✅ RESOLVED | New pure `model/` validator `validate_search_index` with `MAX_IDENTITY_SEARCH_INDEX = 99` and typed `IdentitySearchIndexError::TooLarge` (no String field); applied at the UI dispatch (`add_existing_identity_screen.rs`) with separate out-of-range vs non-numeric messages, both i18n-ready. Test `validate_search_index_accepts_in_range_rejects_beyond_cap` binds (0→Ok, 99→Ok, 100→Err, u32::MAX→Err). |

### New issues introduced by the fixes

**None confirmed.** Benign observations, no candy:

- **Double-dispatch (harmless):** if a wallet is both unlocked-while-Platform-ready
  *and* covered by the sweep, two `discover_identities_gap_limited` runs can
  overlap on one wallet → duplicate DAPI fetches + idempotent last-write-wins
  upsert. No corruption (DB-serialised, update-preserving-alias). In the common
  deferred flow the latch prevents it (sweep already fired, unlock path is
  latch-independent and runs once). LOW-impact, acceptable.
- **Unlock-path prompt on a promotion-failure race (by-design):** if
  `promote_hd_seed_with_passphrase` fails *after* `open()` succeeded (e.g. a
  `WalletNotFound` envelope race), `queue_unlocked_wallet_identity_discovery`
  runs with `allow_prompt = true` and could prompt on a cold miss. This is the
  *interactive unlock* path where the user just typed their passphrase and is
  present — a prompt here is consented, not a surprise. **Not an F-2 violation**
  (F-2 governs the background `allow_prompt = false` sweep, which is untouched).
- **QA-006 guard is dead code** (detailed above) — correct but never fires.

### Re-verification verdict: **SHIP**

All 8 findings resolved, 4 new tests bind (would fail on revert), full suite
green (889 pass), clippy clean, no regression to the import path, the F-2
invariant, the concurrency shape, or the UI Progress consumption. The headline
QA-001 gap is genuinely closed — protected wallets are now discovered on unlock,
and the user-story finally tells the truth. I have run out of things to be
disappointed about, which is itself mildly disappointing.

*(0 new confirmed issues in the fixes → 0 new candy. The original 8 stand.)*
