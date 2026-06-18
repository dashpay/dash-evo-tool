# Blocking Progress Overlay — Design Addendum (QA wave resolutions)

**Phase:** 1d (Architecture) — addendum to `03-dev-plan.md`
**Author:** Nagatha (Software Architect)
**Date:** 2026-06-17
**Resolves:** SEC-003, Diziet F-5 (Decision 1 · Safety-Valve); Diziet F-2, SEC-007
(Decision 2 · Action-Dispatch). Touches QA-001 (button-less input leak) as a dependency of
Decision 1.
**Status:** Decided. Bilby builds directly from §1 and §2. One sub-question flagged for the user
(§1, "For the user to weigh in").
**Supersedes:** the open portions of D-4 (escape-hatch) and D-5 (`drain_overlay_actions` policy)
in `03-dev-plan.md`. Where this addendum and the plan disagree, this addendum wins.

---

## 0. Reading of the two problems

Both findings share one root cause: **the overlay's lifecycle is owned entirely by callers, but
the caller wiring was never finished.** A button-less block trusts the owning operation to clear
its handle; a button trusts the owning screen to receive its click. QA proved both trusts are
currently un-backed — a hang traps the UI forever (SEC-003/F-5), and a click is drained globally
and dropped before any screen sees it (F-2). I am not adding a new owner; I am making the existing
ownership contract real, and adding the minimum machinery so a *violation* is loud rather than
silent.

A second truth shapes Decision 1. The operations that use a button-less block — broadcast,
signing, key import, migration — are exactly the ones it is **unsafe to background**. So any
renderer-level "dismiss / continue in background" valve would reintroduce the precise hazard the
overlay exists to prevent, for exactly the population of overlays it would apply to. The escape
from a trap therefore cannot be "let the user act mid-op"; it must be "guarantee the block always
lowers through the normal path, and make a stuck block impossible by construction."

---

## 1. Decision 1 — Stuck/hang safety-valve (SEC-003, Diziet F-5)

> **Post-decision update (SPV-sync adopter, F-SPV-1).** The "ship NO dismiss/background button in
> v1" call below was scoped to the **unsafe-to-interrupt** operations (broadcast, signing, migration)
> whose safety rests on C1 + C2 *boundedness*. The user has since decided to make the **startup/Connect
> SPV sync** the overlay's first adopter (Task 9 / PR #863) and to **ship an always-visible
> "Continue in the background" escape** for it. That does not contradict this decision: SPV sync is
> **read-only and safe to background** (clicking the escape strands nothing — unlike a broadcast/
> migration), and it is **unbounded** (no peers ⇒ no terminal signal), so its C2 "never trap the
> user" guarantee is met by the **always-on escape**, not by boundedness. A future dev must NOT
> "restore the original docs" by removing that button — it is the load-bearing safety valve for this
> adopter. See UX-002 (`docs/user-stories.md`), `01-requirements-ux.md` §5, and
> `AppState::update_spv_overlay`.

### Decision

**Keep the block total. Ship NO renderer-level dismiss/background button in v1.** The safety valve
is a *layered guarantee that the block always lowers through the normal path*, not an escape that
unblocks the UI while an unsafe operation is still running. Three layers:

1. **Caller contract (the real fix), two clauses — both enforced by review:**
   - **C1 — Clear on every terminal path.** A screen that raises a global overlay stores
     `op_overlay: Option<OverlayHandle>` and MUST call `take_and_clear()` on **both** the success
     and the error branch of `display_task_result`. (The dev-plan's FR-9 hand-off already says
     this; C1 makes it a hard contract, not a convention.)
   - **C2 — Bounded operation.** A button-less block may only cover an operation that is
     **guaranteed to terminate** — every network/IO wait inside its `BackendTask` path has a
     timeout that surfaces as a `TaskError`. This is the clause that makes "trap forever"
     impossible *without* fake cancellation: a bounded op always produces a `TaskResult`, which
     always triggers C1, which always lowers the block.

2. **Informational escalation (renderer-level, honest, no escape) — two thresholds:**
   - **Soft, 30 s total elapsed** (`STUCK_OVERLAY_THRESHOLD`, unchanged): force-reveal the honest
     `Elapsed: {seconds}s` readout and the reassurance line *"This is taking longer than usual."*
     Visual only. (Existing behaviour; retained verbatim.)
   - **Watchdog, 120 s with no progress** (`STUCK_OVERLAY_WATCHDOG_THRESHOLD`, new): the
     reassurance line escalates to *"This is taking much longer than expected. The operation is
     still running — please keep the app open."* "No progress" is measured from a new
     `last_progress_at: Instant`, reset on real progress — either a shown `(description, step)`
     change **or** an advance of the hidden `progress_token` (a liveness signal the owner feeds from
     an advancing underlying operation, e.g. a climbing SPV height while the shown "Step N of 5"
     stays constant for minutes). The `progress_token` is **never rendered**, and its reset is
     intentionally decoupled from the once-per-shown-content-change log (NFR-5): a per-frame token
     advance resets the clock but emits no log. So a legitimately-advancing flow — multi-step (J-1,
     four ~minute steps) **or** a single slow-but-advancing phase — **never** trips it, while a
     genuinely wedged step does.

3. **Developer watchdog (the leak detector for C1/C2 violations):** when the watchdog threshold is
   crossed, fire a **one-shot** `tracing::error!` (guarded by a `watchdog_logged` flag, logged
   once, never per frame) naming the over-long overlay: an overlay alive this long without progress
   is almost always a leaked handle (C1) or an un-bounded op (C2) — i.e. a bug. **No `debug_assert`
   / panic** — a time-based assert is flaky (a slow test or a legitimately slow op would panic the
   process). The log is the signal; CI and review are the gate.

**Safety side — the block must be *genuinely* total (resolves QA-001).** Because nothing lowers the
block mid-op, the block must actually capture all input even when it has no buttons. Today
`render_global` filters Tab/Enter/Esc *after* the panels beneath already ran this frame, and never
filters `Event::Text` at all — so a button-less block leaks typed characters into a focused field
beneath (the J-2/J-4 case; `qa_buttonless_overlay_blocks_typing_into_focused_field_beneath`,
currently `#[ignore]`). Fix: add `ProgressOverlay::claim_input(ctx)`, called **near the top of
`AppState::update`** (after the shutdown guard at `app.rs:1264`, **before** the visible screen's
`ui()` at `app.rs:1543`), gated on `has_global(ctx)`:
   - **Release beneath focus on raise** so a focused text field stops drawing a caret and stops
     consuming text (move focus off any beneath widget; the existing focus-lock pattern in
     `render_buttons` at `progress_overlay.rs:690` is the buttoned-case analogue).
   - **Strip `Event::Text` and the navigation/confirm keys (Tab/Enter/Esc, arrows) from
     `i.events` at frame start**, so widgets beneath never observe them. Doing it *before* the
     screen runs is the whole point — the current end-of-frame filter in `render_global` is one
     frame too late. Keep the in-`render_global` filter as a belt-and-suspenders second pass, or
     remove it once `claim_input` is verified; do not rely on it alone.
   - **Button-less (all v1 ops): total keyboard + text claim.** When a caller later attaches
     buttons (post-T7), `claim_input` still strips text and beneath-navigation, and the overlay's
     own button area re-grants only its buttons' navigation via the existing
     `set_focus_lock_filter`.
   - **QA-002 refinement — one opt-in keyboard escape (mechanism superseded by SEC-001/SEC-002 —
     `progress_overlay.rs` is the source of truth).** A hard block is never keyboard-activatable
     (Enter/Space stripped every frame), so a focused button cannot be triggered by keyboard. The
     one exception is a block that opts in via `OverlayConfig::with_keyboard_escape(action_id)`: it
     designates a single action as a keyboard-reachable escape, for **unbounded** blocks that would
     otherwise strand a keyboard-only / assistive-tech user. `claim_input` activates it **at frame
     start, before the beneath `ui()` runs**: a press of Enter/Space enqueues the designated action
     directly (the same queue a click feeds), and the key is then stripped like every other one.
     The activation needs no focus (SEC-001 — the earlier "keep Enter/Space only while the escape is
     *confirmed focused*" scheme re-requested the button's focus every frame and ran before the
     secret-prompt render, stealing focus from a passphrase modal above the block, so it was
     removed) and the key never survives to a widget beneath, focus-dependent or not (SEC-002).
     Focus on the escape is now purely visual and is suppressed while a secret prompt is up. The
     reference adopter is the unbounded SPV-sync block (`update_spv_overlay`), whose "Continue in
     the background" escape is so designated. Every OTHER hard block stays fully keyboard-blocked
     (`TC-OVL-044` guards the general rule; `TC-OVL-051/052/053` cover the opt-in escape;
     `sec001_*` / `sec002_*` cover the focus-steal and beneath-leak fixes).

### Rationale

- **Why no background/dismiss valve.** It is unsafe for exactly the overlays it would cover, and
  it cannot be made safe without either (a) real cooperative cancellation (does not exist — T7,
  confirmed in dev-plan §0.1) or (b) per-operation in-flight guards (do not exist). The directive
  forbids fake cancellation; a valve that lowers the block while a broadcast/migration runs is
  fake safety. The honest move is to remove the *possibility* of a hang, not to paper a dismiss
  button over it.
- **Why the trap, weighed against the alternative, is the lesser harm — and why C2 dissolves it.**
  Trapping until force-quit is *recoverable* (restart; a bounded op will have completed or failed
  cleanly). Letting the user fire a conflicting second op (double broadcast, interrupted
  migration) is potentially *unrecoverable* — fund loss or corrupted state. Given that asymmetry,
  the block must stay total. C2 then removes the only path to a real trap: if every blocked op
  terminates, the block always lowers on its own. The 30 s/120 s reveals keep the *waiting* user
  informed without ever offering an unsafe exit.
- **Why measure the watchdog on no-progress, not total elapsed.** A correct multi-step flow can
  legitimately run several minutes; keying the watchdog (and its escalated copy) to time-since-
  last-progress makes it fire only on a true stall, eliminating false dev-error logs and false
  "much longer than expected" copy during healthy long flows.
- **Why QA-001 belongs here.** "Keep the block total" is only sound if the block is *actually*
  total. The button-less keyboard/text leak is the one place it currently is not; fixing it is a
  precondition of this decision, not a separate nicety.

### Implementation spec

**File: `src/ui/components/progress_overlay.rs`**

- Add constant:
  ```rust
  /// After this long *without progress* on the topmost request, escalate the
  /// reassurance copy and fire the one-shot developer watchdog. A leaked handle
  /// (C1) or an un-bounded op (C2) is the usual cause — both are bugs.
  const STUCK_OVERLAY_WATCHDOG_THRESHOLD: Duration = Duration::from_secs(120);
  ```
- `OverlayState`: add `last_progress_at: Instant` (init `Instant::now()` in `OverlayState::new`)
  and `watchdog_logged: bool` (init `false`), plus a hidden `progress_token: Option<u64>` and its
  `last_progress_token` shadow. In `log_overlay_state`, reset `last_progress_at = Instant::now()`
  when **either** the shown `(description, step)` changes **or** the `progress_token` advances — but
  emit the content-update log **only** on a shown change (a token advance is a hidden liveness
  signal, not a user-visible update — NFR-5). The token is never rendered.
- New helpers, mirroring `stuck_reveal`:
  ```rust
  fn watchdog_tripped(last_progress: Instant) -> bool {
      last_progress.elapsed() >= STUCK_OVERLAY_WATCHDOG_THRESHOLD
  }
  ```
- `render_card`: when `watchdog_tripped`, render the escalated line
  (`STUCK_WATCHDOG_REASSURANCE`) **instead of** the soft `STUCK_REASSURANCE` (do not stack both).
  The soft 30 s elapsed-reveal logic is unchanged.
- `render_global`: after computing `stuck`, compute `watchdog` from `top.last_progress_at`; if
  `watchdog && !top.watchdog_logged`, `tracing::error!(key = top.key, "Blocking overlay has shown
  no progress for over 2 minutes — likely a leaked handle or an un-bounded operation")` and set
  `top.watchdog_logged = true`. Keep the existing `request_repaint_after(1s)` when
  `show_elapsed || watchdog` so the escalation actually appears.
- New `pub fn claim_input(ctx: &egui::Context)`: early-out when `!has_global(ctx)`; otherwise
  release beneath focus and strip `Event::Text` + Tab/Enter/Esc/arrow key events from
  `i.events`. Document that it must run before the panels each frame.

**File: `src/app.rs`**

- In `update()`, immediately after the shutdown guard (`:1264`) and before the screen `ui()`
  (`:1543`):
  ```rust
  ProgressOverlay::claim_input(ctx); // total input block at frame start (button-less safe)
  ```
- Document the C1/C2 caller contract at the `op_overlay` convention site (T4 docs) and add a
  one-line review rule to §8 risks: *"A button-less global block may only cover a bounded
  operation (every wait times out to a TaskError)."*

**i18n strings (complete sentences, named placeholders — NFR-2):**

| Const | Text |
|---|---|
| `STUCK_REASSURANCE` (existing) | `This is taking longer than usual.` |
| `STUCK_WATCHDOG_REASSURANCE` (new) | `This is taking much longer than expected. The operation is still running — please keep the app open.` |
| `Elapsed: {seconds}s` (existing) | unchanged |

### For the user to weigh in

The only genuinely contested point: **do we ever want a manual backgrounding escape as
belt-and-suspenders, in case C2 is violated somewhere we missed?** I have decided **no** for v1,
because the safe version of it requires per-operation in-flight guards (so a backgrounded op
cannot be duplicated), which is net-new work properly scoped alongside T7 (cooperative
cancellation). My recommendation is to invest in C2 + the watchdog now and revisit a *safe*
backgrounding valve only if the watchdog log ever fires in practice. If the user values guaranteed
availability over the conflicting-op risk more highly than I have weighted it, that is their call
to make — flagging it rather than burying it.

### Test obligations

- **Un-ignore** `qa_buttonless_overlay_blocks_typing_into_focused_field_beneath` (QA-001); it must
  pass once `claim_input` lands. This is the acceptance test for "the block is genuinely total."
- TC-OVL-047 (informational portion) stays green; **add** an inline unit test
  `watchdog_tripped_only_past_threshold` mirroring `stuck_reveal_triggers_only_past_threshold`.
- New inline tests: a shown content update via `set_step`/`set_description` resets
  `last_progress_at`; and a hidden `progress_token` advance ALSO resets it (without emitting a
  content-update log), while an unchanged token leaves the clock alone so a true stall still trips
  the watchdog.
- New inline test: `watchdog_logged` flips once and stays set (no per-frame log spam — NFR-5).
- kittest: button-less overlay with an injected long elapse renders `STUCK_WATCHDOG_REASSURANCE`,
  not the soft line, and still exposes no dismiss control.
- Escape-hatch button portion of TC-OVL-047 is **closed as "won't build" for v1** (was "deferred
  to T7"): there is no renderer escape by design. Note it as a deliberate non-feature.

---

## 2. Decision 2 — Action-dispatch contract (Diziet F-2, SEC-007)

### Decision

**The caller receives its own clicks through its own handle. Actions are scoped by the owning
overlay entry's key; the global drain becomes a true orphan-sweeper that can never pre-empt a
live owner.** Concretely:

1. **Actions are keyed.** The action queue stores `(key, action_id)`, not bare `action_id`. A
   click in `render_global` enqueues the **topmost entry's key** alongside the id.
2. **The owning caller drains via its handle.** New
   `OverlayHandle::take_actions(&self) -> Vec<String>` returns (FIFO) and removes only the action
   ids whose key matches this handle, **leaving other entries' actions untouched**. The owning
   screen calls it at the **top of its own `ui()`** each frame and matches its own ids:
   ```rust
   if let Some(h) = &self.op_overlay {
       for action_id in h.take_actions() {
           // caller-owned logic, e.g. the screen's own cancellation
       }
   }
   ```
   This is the literal implementation of the directive "the caller RECEIVES click events" — no
   central registry, caller owns semantics. The instance `Component` path already does the
   equivalent by surfacing the click through `ProgressOverlayResponse` (unchanged).
3. **`OverlayHandle::clear(self)` also purges its key's pending actions**, so a normal dismiss
   leaves no stray id behind to be swept and logged.
4. **The global drain is demoted to an orphan-sweeper.** Rename the static
   `ProgressOverlay::take_actions(ctx)` → **`sweep_orphan_actions(ctx) -> Vec<String>`**: it
   drains and returns only actions whose key is **no longer on the stack** (owner already cleared
   or dropped its handle without draining). `AppState::drain_overlay_actions` calls it and logs
   each truly-orphaned id (`warn!`, "overlay action received for an overlay that is no longer
   active — dropping"). Because it only ever takes dead-owner ids, it **cannot** race or pre-empt
   the screen that owns a live overlay, regardless of call order — so it may stay at its current
   position (`app.rs:1567`).
5. **App-level owners use the same mechanism.** When `AppState` itself raises an overlay (e.g. a
   future network-switch block with a button), it holds the `OverlayHandle` and drains it with
   `take_actions()` exactly like a screen — `AppState` is just another caller. There is one
   dispatch mechanism, not a screen-path and an app-path. (This is *more* consistent than copying
   the banner's central-registry `drain_banner_actions`; it honours "caller owns logic" uniformly.)
6. **Network-switch hygiene (SEC-007).** `clear_all_global(ctx)` MUST clear the action queue too —
   not just the state stack. Today it clears only `OVERLAY_STATE_ID`, so a click queued just
   before a network switch survives into the new context and could be mis-dispatched. Add a clear
   of `OVERLAY_ACTIONS_ID` (remove the slot) inside `clear_all_global`.

### Rationale

- **Keying, not a registry.** Scoping actions by the owning entry's key is the minimum needed to
  let two parties drain one queue without stealing each other's ids — it is data scoping, not a
  handler registry, and it is what makes "caller owns logic" *safe* under the single-global-queue
  model the banner established. With a bare-string drain-all queue and two drainers, whoever runs
  first swallows everything; ordering tricks cannot fix that without re-enqueue hacks. Keying
  dissolves the race by construction.
- **Why the handle is the delivery channel.** The handle is already the caller's one reference to
  its overlay (it updates content and clears through it). Delivering clicks through the same
  handle is the most cohesive surface and needs no new plumbing in screens beyond the `op_overlay`
  field they already hold for the FR-9 hand-off.
- **Why the global drain survives at all.** A screen can be popped between a click (frame N) and
  its next `ui()` (frame N+1); its handle is dropped without draining. Those ids would otherwise
  accumulate in `ctx.data` forever. The orphan-sweeper reclaims exactly those and logs them as the
  anomaly they are — observability without interference.
- **Action-id convention.** Follow the live banner convention `MIGRATION_RETRY_ACTION_ID =
  "migration:retry:finish_unwire"` — colon-namespaced `domain:object:action`, declared as a
  `pub const &str` near the owning screen (e.g. `shielded:build:cancel`). The dev-plan's earlier
  `overlay.cancel`/dot form is superseded; align to colons. (`OVERLAY_CANCEL_ACTION_ID` was already
  removed in the post-outage redesign, so there is no built-in id to reconcile.)

### Implementation spec

**File: `src/ui/components/progress_overlay.rs`**

- New private type:
  ```rust
  #[derive(Clone)]
  struct OverlayAction { key: u64, action_id: String }
  ```
  Queue type changes `Vec<String>` → `Vec<OverlayAction>` in `get/set_overlay_actions`.
- `push_overlay_action(ctx, key: u64, action_id: &str)` — `render_global` passes `top.key`; the
  instance `Component` path does not enqueue (it returns via the response, unchanged).
- `OverlayHandle::take_actions(&self) -> Vec<String>`: read queue, partition by `key == self.key`,
  write back the non-matching remainder, return matching ids in order.
- `OverlayHandle::clear(self)`: after `retain`-ing the state stack, also `retain` the action queue
  to drop `key == self.key` entries.
- Rename static `take_actions(ctx)` → `sweep_orphan_actions(ctx) -> Vec<String>`: returns ids whose
  `key` is absent from the current state stack; writes back the rest.
- `clear_all_global(ctx)`: clear `OVERLAY_STATE_ID` (existing) **and** `OVERLAY_ACTIONS_ID` (new).

**File: `src/app.rs`**

- `drain_overlay_actions`: replace the blanket `take_actions` loop with
  `for id in ProgressOverlay::sweep_orphan_actions(ctx) { warn!(... "no longer active — dropping") }`.
  Keep it at `:1567`.
- Document at the `op_overlay` convention site (T4 docs): *screens drain their own clicks at the
  top of `ui()` via `OverlayHandle::take_actions()` and match their own colon-namespaced ids; the
  app loop only sweeps orphans.*

### Test obligations

- **Reframe** TC-OVL-024/025/026 to the handle-scoped API: click enqueues this handle's id;
  `handle.take_actions()` returns it FIFO then empties; an unrelated handle's `take_actions()`
  returns empty (no cross-owner theft).
- **Update** inline `take_actions_drains_fifo_then_empties` → exercise `OverlayHandle::take_actions`
  (per-key FIFO) plus `sweep_orphan_actions` (dead-owner ids only).
- New inline test: two stacked overlays A (bottom) and B (top); a click enqueues against B's key;
  `A.take_actions()` is empty, `B.take_actions()` returns the id.
- New inline test (SEC-007): enqueue an action, then `clear_all_global`; the action queue is empty.
- New inline test: a handle dropped without draining leaves its id only reachable via
  `sweep_orphan_actions`; `clear()` instead leaves nothing for the sweeper.
- kittest: a screen-style harness drains its handle and observes its own id; `drain_overlay_actions`
  logs nothing for a live owner.

---

## 3. Decision summary

| # | Decision | Status |
|---|---|---|
| A-1 | No renderer dismiss/background valve; safety = caller contract (C1 clear-on-terminal, C2 bounded-op) + 30 s soft / 120 s-no-progress escalation + one-shot dev watchdog | Decided (one sub-point flagged for user) |
| A-2 | Button-less block must be genuinely total: `claim_input` at frame start releases beneath focus + strips text/nav keys (resolves QA-001) | Decided |
| A-3 | Clicks delivered to the caller via keyed `OverlayHandle::take_actions`; global drain demoted to `sweep_orphan_actions` | Decided |
| A-4 | `clear_all_global` clears the action queue too (SEC-007) | Decided |

---

## 4. Candy tally

| Severity | Count | Items |
|---|---|---|
| **High** | 3 | A-1 hang/trap resolution without fake cancellation (SEC-003/F-5); A-2 button-less total-input block (QA-001); A-3 finished receive-side dispatch (F-2) |
| **Medium** | 2 | C2 bounded-operation contract (makes "trap forever" impossible by construction); A-4 action-queue network-switch hygiene (SEC-007) |
| **Low** | 1 | No-progress watchdog metric + one-shot dev-error (leak detector for C1/C2; no flaky time-based assert) |

**Total: 6 findings.** Six candies — and not one of them required teaching the spinner to lie about
how long it will take. The overlay keeps its dignity: it blocks honestly, it reports honestly, and
when something is truly wedged it says so plainly rather than offering a door that opens onto a
cliff.
