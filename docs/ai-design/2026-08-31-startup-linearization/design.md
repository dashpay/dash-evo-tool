# Startup linearization: the storage-preparation gate

How DET went from three racing startup subsystems, each individually guarded,
to one explicit ordering with one guard. Companion to `docs/user-stories.md`
UX-002 (rewritten) and UX-005 (new); supersedes the deferred discussion in
`docs/ai-design/2026-06-17-blocking-progress-overlay/04-design-addendum.md`
§1 and its A-1 decision row.

## 1. The problem

Boot ran three things with no ordering between them:

- **Wallet-backend wiring** (`AppContext::ensure_wallet_backend`) — fire-and-forget,
  spawned per network context at `AppState::new`.
- **The legacy `data.db` drain** (`FinishUnwire`) — dispatched from the frame loop,
  once per network, gated on a per-frame poll of whether wiring looked done.
- **SPV chain sync** — started as soon as the user (or auto-start) asked for it,
  with no data dependency on either of the above.

Nothing sequenced them; the codebase paid for that ambiguity with five
mechanisms enforcing one invariant a single ordering would give for free:

1. A per-frame readiness poll (`dispatch_cold_start`'s `backend_ready` check)
   before the drain could dispatch.
2. A per-network dispatch guard (`dispatched: BTreeSet<Network>`) so the poll
   above only fired the drain once.
3. A process-wide `migration_run` mutex serializing the drain against
   concurrent identity deletes and the detached DAPI-node refresh.
4. A 30 s watchdog (`COLD_START_BACKEND_READY_TIMEOUT`) surfacing a stuck
   banner if wiring never finished.
5. A task-level short-circuit (`WalletBackendNotYetWired` fast-fail in
   `start_spv`) for a synchronous start that outran fire-and-forget wiring.

An exhaustive audit of every migration in the tree (DET's legacy schema
ladder, legacy settings import, the `FinishUnwire` drain, the upstream
refinery ladder V001–V014, lazy sidecar/vault upgrades) found exactly one
network-touching pass: the best-effort DAPI node refresh, which is detached,
best-effort, and retried next launch regardless. **No migration anywhere
waits on SPV.** The "do we need SPV synced for migrations?" coupling that
motivated the original guards was accidental, not load-bearing.

## 2. The decision

One `async fn`, [`AppContext::prepare_storage`](../../../src/context/wallet_lifecycle/prepare.rs),
holds one lock — `prepare_gate` (`tokio::sync::Mutex<()>`, replacing
`migration_run`) — across the whole sequence: wire the backend (which runs the
upstream schema ladder and rehydrates wallets), then drain `data.db`. Chain
sync is not part of the sequence; it starts only as a *continuation* of a
completed `prepare_storage`, inside
[`AppContext::ensure_wallet_backend_and_start_spv`](../../../src/context/wallet_lifecycle/spv.rs)
(`spv.rs:162`), which keeps its name and its role as the single chokepoint for
"start SPV" across every entry path (GUI boot, Connect, network switch,
MCP/CLI).

The `WalletBackendNotYetWired` race this replaces — a synchronous
`start_spv()` firing while fire-and-forget wiring was still in flight — closes
strictly harder than before: `start()` is now reachable only as a data
dependency inside one function, not a timing coincidence that happened to be
guarded against.

## 3. Why the gate blocks the user, not a thread

A thread-blocking gate (`Handle::current().block_on(prepare_storage(..))` at
the point boot would want to wait) is impossible here. `src/main.rs:40` runs
`runtime.block_on(start(&app_data_dir))`, and `start` calls
`eframe::run_native` (`main.rs:84`) *from inside that polled future* — so by
the time `eframe`'s own event loop begins, the main thread already sits
inside a tokio async context. Blocking that thread on another `block_on`
panics (nested-runtime).

So "linear and blocking" means: one `async fn` with real data dependencies,
blocking the *user* via the frame loop, not blocking a thread. Concretely:

- `AppState::spawn_storage_prepare` spawns `prepare_storage` and keeps the
  `oneshot::Receiver<Result<(), TaskError>>` its result arrives on.
- `StoragePrepGate` (`src/app/reconcilers.rs`) polls that channel once per
  frame, and while it is unresolved, owns the entire interaction surface: no
  root screen renders (`BootPhase::Preparing` — see §5), only the gate's own
  overlay and the storage update's own password prompt exist.
- `BootApp::Unlocking` (`src/boot.rs`) is the existing precedent for this
  shape — a spawned-task-plus-poll standing in for a blocking wait the
  runtime cannot support.

This is genuinely one explicit ordering replacing guards scattered across
four subsystems, not the elimination of concurrency — the gate is still a
spawned task the frame loop polls.

## 4. `BootPhase` and the terminal transition

```rust
enum BootPhase {
    AwaitingNetworkChoice,          // legacy-settings import failed; only the chooser renders
    Preparing { network: Network }, // gate overlay + password prompt only, no root screens
    Ready,                          // root screens exist and render normally
}
```

Root screens are constructed at the terminal transition
(`AppState::finish_boot_phase`), not eagerly at `AppState::new` — the
opposite of pre-gate behaviour, where every screen was built against
guaranteed-empty wallet/identity maps and populated later via
`change_context`/`refresh`. Deferring construction means every screen's
`refresh_on_arrival()` now runs against an already-hydrated `AppContext`.
`NetworkChooserScreen` is the one exception — it stays eager, since
`AwaitingNetworkChoice` needs somewhere to render before a network is even
chosen.

`StoragePrepGate` tracks `prepared: BTreeSet<Network>` so a network already
prepared this process (switch away, switch back) never re-raises the gate;
chain sync still starts as its continuation on the fast path, covering both
the cached-context return and a fresh `SwitchNetwork` dispatch idempotently.

### Terminal states, once `prepare_storage` resolves

| Outcome | Gate behaviour | Surface |
|---|---|---|
| `Ok(())` | Lift → `Ready` | Root screens build, chain sync starts if opted in |
| `Err(Failed)` | Block, retryable | "Try again" / "Close the app" |
| `Err(SavedDataTooOld / SavedDataTooNew)` | Block, terminal | "Close the app" only — a version-window mismatch cannot be retried into success |
| Stuck past 30 s (`STORAGE_PREP_STUCK_TIMEOUT`) | Block, still pending | Reassurance copy + "Close the app" (see §6) |

"Close the app" sends `egui::ViewportCommand::Close` — the same door
`boot.rs` and other in-app exits already use — so `on_exit` still runs vault
teardown. Nothing is written on a gate-failure path: the completion sentinel
stays unwritten and the previous version's `data.db` stays open read-only, so
the next launch retries from unchanged state.

## 5. What replaced, deleted, and kept

| Mechanism | Fate |
|---|---|
| `migration_run` mutex | **Renamed/repurposed** to the private `prepare_gate`; every consumer acquires it through `lock_prepare_gate` / `try_lock_prepare_gate`, so `rg 'lock_prepare_gate|try_lock_prepare_gate' src` is the authoritative holder inventory. Lock order remains `prepare_gate` → identity-index guard, never the reverse. |
| `dispatch_cold_start` (frame-loop readiness poll + dispatch) | **Deleted.** The drain is now an unconditional step inside `prepare_storage`, which already holds a wired backend by construction. |
| `dispatched: BTreeSet<Network>`, `backend_wait_since`, `timeout_signaled` (`MigrationReconciler` fields) | **Deleted**, not kept. (The dev plan called for keeping `dispatched` as the "Retry now" mechanism; it shipped differently — see the deviation note below.) |
| 30 s cold-start watchdog (`COLD_START_BACKEND_READY_TIMEOUT` / stuck banner) | **Moved into the gate** as `STORAGE_PREP_STUCK_TIMEOUT`, now covering the whole prepare sequence rather than only the wiring wait, and pairing with an actual exit ("Close the app") rather than a log-only banner — see §6. |
| `WalletBackendNotYetWired` fast-fail in `start_spv` | **Superseded by construction**: `start()` is reachable only as `prepare_storage`'s continuation, so the race it guarded against cannot occur. |
| `SpvBlockStep::Stand` + `dismissed` field | **Deleted** — replaced by `confirming_cancel` (two-step Cancel; see UX-002). |
| `WalletStorageNotReady` fast-fail (`backend_task/mod.rs`) | **Kept.** The headless MCP/CLI binary has no frame loop and therefore no gate to hold the user behind — it still needs a fast, typed "still running, retry" error. |

**Deviation from the dev plan.** The plan's work breakdown said to keep
`MigrationReconciler::dispatched` because it doubles as the "Retry now"
banner-action mechanism. It shipped deleted instead: with `dispatch_cold_start`
gone, there is no per-network dispatch guard left to protect, and the
post-gate "Retry now" action (`MIGRATION_RETRY_ACTION_ID`, for a `Failed`
state reached *after* the gate has already lifted — e.g. from the detached
DAPI-refresh retry path or an MCP join) needs no guard reset; it simply
re-dispatches `FinishUnwire` and clears `last_state` so the new run's banner
overwrites the stale one. The gate's own retry (`StoragePrepGate`'s "Try
again", for a *pre-gate* failure) is a fully separate path with its own state
(`PrepareFailure`) and needs `dispatched` for nothing. Net effect matches the
plan's intent (one retry path per failure surface); the mechanism differs.

## 6. The password-prompt contract — the highest-risk part

`prepare_storage`'s drain step can hit `AwaitingWalletPasswords` and await a
migrated wallet's password (`register_migrated_wallets` in `finish_unwire.rs`)
*while holding `prepare_gate`*. The only thing that can supply that password
is `MigrationReconciler::update_password_prompt`, driven from the frame loop
— the very loop the gate is blocking. **The gate blocks the loop its own
completion depends on.**

This works because the gate does not early-return from `AppState::update()`.
`BootPhase::Preparing` suppresses only the root-screen render region;
everything the frame loop already ran *after* that region for the pre-gate
overlay/prompt machinery — `render_global`, `render_secret_prompt`,
`update_banner`/`update_password_prompt`, `drain_overlay_actions` — still
runs on every `Preparing` frame, in the same order as before. The gate's own
driver (`self.boot.update(..)`, in `impl App for AppState`) is placed
*before* the screen-render decision specifically so its outcome (a
`GateEvent`) can be applied on the frame preparation finishes on, rather than
delaying screen construction by a frame.

The four-point contract this depends on is keyed on one predicate,
`AppState::has_blocking_secret_prompt` (`app.rs:2148`) — true while
`MigrationReconciler::is_prompting` reports `AwaitingWalletPasswords`:

1. The activation-frame pointer click is dropped.
2. `claim_overlay_input` (`app.rs:2152`) skips claiming input, so the user
   can type.
3. The overlay paints no dimmer, no card, no focus trap while yielding.
4. The password popup renders on `Order::Foreground`, above everything else.

`StoragePrepGate` feeds this same predicate rather than raising a parallel
surface — if it didn't, production would deadlock (the gate waiting on a
password prompt that never renders) while every test that raises a block
*directly* via `ProgressOverlay::set_global` would keep passing, since none
of them exercise the gate's own code path. `tests/kittest/migration_gate.rs`
exists specifically to close that gap: its tests raise the **gate** (a
`#[cfg(feature = "testing")]` seam mirroring `AppState::test_arm_spv_block`),
pump the real `AppState::update()` loop, and assert the password field is
present, focused, and typeable — the assertion that would go red if this
contract broke.

The escape from the prompt is per-wallet "Skip this wallet"
(`MigrationWalletUnlockResult::Skipped` → `MigrationStatus::skip_wallet`);
`AwaitingWalletPasswords` filters skipped wallets from future rounds, so
skipping every locked wallet still terminates the wait. There is no "no
escape" claim here — "no background escape" means no *backgrounding*
escape, not no way out of an individual password prompt.

## 7. The vault-cleanup sweep hazard

`AppContext::resume_pending_vault_cleanups` (`src/context/identity_db.rs`) is
the *only* recovery path for vault keys an interrupted identity removal
orphaned — once an identity leaves the Global index, no UI path can reach it
again, so a manifest left behind by a removal that crashed between
`index_remove_identity` and the vault-key delete can only be finished by this
boot-time sweep. It normally runs from
`bootstrap_loaded_wallets` (`wallet_lifecycle/bootstrap.rs:708`), inside
wiring, guarded by a `prepare_gate.try_lock()` that is meant to defer to a
concurrent storage migration and retry next boot.

Holding `prepare_gate` across the *entire* `prepare_storage` sequence turned
that guard into a permanent skip: `bootstrap_loaded_wallets` now always runs
*while* `prepare_storage` already holds the gate it `try_lock`s, so the sweep
loses every time, on every boot, forever — with no user-visible signal that
it never ran. This was not a hypothetical found in review; it is a direct
consequence of the gate's own locking, silently disabling a recovery path
that existed for exactly this kind of interrupted-operation cleanup.

The fix re-drives the sweep explicitly, under the gate `prepare_storage`
already holds, via a `_gate: &PrepareGateGuard<'_>` parameter that exists as
compile-time proof of "callable only under the gate", not documentation. It
has to run *before* the gate releases, not after: the drain's own detached
DAPI-node refresh is queued behind the same `prepare_gate` and would very
likely win a post-release `try_lock` race, reproducing the identical skip on
a different code path.

**The trap, restated:** a mutex introduced purely to sequence startup silently
disabled a recovery path two layers below it, with no relationship in the type
system, a call graph, or a test to flag the interaction — `prepare_gate` and
`resume_pending_vault_cleanups`'s own guard are the same lock, so widening one
caller's hold on it silently starves every other caller that only ever
`try_lock`s. Nothing short of tracing every `try_lock` on a mutex before
widening any one caller's hold on it would have caught this by inspection.

**Not success-only.** Running the sweep only after the drain *succeeds*
reintroduces the same permanent-skip failure by a different route: a drain
that fails *deterministically* — a version-window mismatch
(`SavedDataTooOld`/`SavedDataTooNew`) — would postpone the sweep forever on
every future launch of this install, exactly like the bug being fixed. The
sweep therefore runs on **both** the drain's success and failure paths, with
one deliberate exception: a **wiring** failure still skips it, because the
sweep reads through the same backend k/v store that wiring just failed to
open — it could only bail on its own "not ready" branch, so running it there
would be a duplicate no-op, not a broader net. The call also stopped
tolerating a non-terminal `MigrationStatus` silently: by the point it runs,
the drain has published a terminal state either way, so an in-progress
status means an unpublished step would make the sweep skip unnoticed — the
exact failure mode this fix closes — and it now logs a warning instead of
passing quietly.

**Why that is safe even for a drain that failed part-way through, not just
one that failed before writing anything.** The sweep's irreversible delete
acts on one piece of evidence: the identity is **off the roster**. The
question is whether a partially applied import — the drain died after adding
some rows but before finishing — could ever put back a roster entry for an
identity the sweep is about to act on, and so falsify that evidence. It
cannot, for a reason narrower than "the drain never deletes":

- The drain's import path can only *add* roster entries; nothing in it
  removes one. So a partial drain leaves the roster more populated than it
  started, never less — absence can only predate the drain, never be created
  by a failure inside it.
- More specifically, the import path cannot even **resurrect** the one
  roster entry that would matter: `write_local_qualified_identity_locked`
  declines any write for an identity that is off the roster and carries an
  unload marker (`identity_db.rs:1118-1131`). `delete_local_qualified_identity`
  persists the vault-cleanup manifest (`identity_db.rs:1695`), then writes
  that same Global-scope unload marker (`mark_identity_unloaded`,
  `identity_db.rs:1700`) — both *before* the roster delisting that follows
  — so every identity a pending manifest belongs to already carries the
  marker that blocks its own reimport by the time the manifest exists.
  `purge_identity_scope` only ever touches `DetScope::Identity(id)`
  (`identity_db.rs:620`), so no step the drain runs can clear a Global-scope
  marker it does not touch.
- The sweep's absence check therefore does not depend on the drain being
  correct or complete — it depends on the drain being structurally unable to
  write in the one direction (resurrecting a specifically-manifested
  identity) that would matter, regardless of where in its own sequence it
  stopped.

That property is the tripwire for the next maintainer: it holds only as long
as nothing makes the drain (or any future import path) remove a roster entry,
or write an identity record without first consulting the unload marker the
way `write_local_qualified_identity_locked` does. A change that adds either
capability invalidates this analysis and reopens the hazard, silently, the
same way the gate's own locking silently disabled the sweep in the first
place.

The one path that *does* clear the whole roster —
`delete_all_local_qualified_identities_in_devnet` (`identity_db.rs:2199`) —
does not threaten this either: it hard-returns unless `network == Network::Devnet`
(`:2202-2204`), is reachable only from an explicit user action on the network
chooser (`network_chooser_screen.rs:1384,1561`, via `SystemTask::WipePlatformData`),
and purges each identity's vault keys itself before removing it — so it
leaves no orphan behind for the sweep to find.

## 8. Risks and limitations

- **Network switching through the gate had essentially no dedicated test
  coverage when this shipped** — the kittest suite (`migration_gate.rs`,
  `startup.rs`) exercises the gate's happy path and its password-prompt
  contract, but not a switch-triggered re-raise. Review found a real wedge in
  exactly that gap. The lesson worth keeping: for a state machine like this
  one, the untested surface is the escapes (retry, close, network switch),
  not the happy path — that is where to look first in any future change here.

## 9. Related, not covered here

- The Cancel/confirm replacement for SPV sync's "Continue in the background"
  button is a related but separate UX change to the *chain-sync* overlay
  (`SpvBlockReconciler`, not the gate) — see `docs/user-stories.md` UX-002
  and the superseded discussion in
  `2026-06-17-blocking-progress-overlay/04-design-addendum.md` §1.
- `docs/user-stories.md` UX-005 covers the gate's own user-facing contract
  and acceptance criteria.
