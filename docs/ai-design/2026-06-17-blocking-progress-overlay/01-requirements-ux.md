# Blocking Progress Overlay — Requirements + UX Specification

**Phase:** 1a (Requirements) + 1b (UX) — combined
**Author:** Diziet (Product Designer)
**Date:** 2026-06-17
**Status:** Draft for downstream phases (architecture decision belongs to Nagatha in 1d)
**Sibling component:** `MessageBanner` (`src/ui/components/message_banner.rs`)

---

> **Supersession callout (post-outage redesign + QA-wave addendum).** Parts of this spec describe
> a first-class **Cancel** control that no longer exists. The shipped design replaces it with a
> **generic button facility** (`with_button` / `with_secondary_button`, clicks delivered keyed to
> the owning screen), and adds a no-progress **watchdog** and a frame-start **`claim_input`** total
> block. Where this document and the redesign disagree, **`03-dev-plan.md`'s post-outage note,
> `04-design-addendum.md`, and the code (`src/ui/components/progress_overlay.rs`) win.** Items known
> to be superseded:
> - **FR-7 (buttons & actions), AC-7.3 / AC-7.4** — no built-in Cancel; buttons are generic and
>   styled Primary (right) / Secondary (left) in insertion order within each tier; clicks are
>   delivered to the owning screen via `OverlayHandle::take_actions` (keyed), not a Cancel action.
> - **NFR-3 AC-3b (Esc → Cancel)** — Esc never cancels; a hard block swallows Esc/Tab/Enter/Space.
>   While a secret prompt is shown above the overlay, the overlay yields the keyboard to it (SEC-004).
> - **AC-8.4 (backdrop / input)** — input is claimed at frame start (`claim_input`), including
>   `Event::Text`; a button-less block is genuinely total (QA-001).
> - **AC-10.5 (concurrent actions)** — only the topmost entry's clicks are reachable, and they are
>   keyed to that entry's owner; the app loop only sweeps orphaned ids.
> - **J-1 / J-2 / J-3 (journeys) and §6.3 / §6.4 / §6.5 (cancel UX)** — reframe any "Cancel button"
>   language to the generic-button + escalation model; the safety valve is the bounded-operation
>   contract + 30 s / 120 s honest escalation, not a dismiss/background control.

---

## 0. Executive Summary

Some operations in Dash Evo Tool are not safe to interrupt and are not meaningful to
interact *around*: broadcasting a state transition, signing, importing keys, a multi-step
identity registration, a migration step. For these, a passive banner is the wrong tool —
the user can still click into half-finished state, fire a second conflicting operation, or
simply not notice that the app is busy.

This spec defines a **full-screen blocking progress overlay**: a sibling capability to
`MessageBanner` that draws a dimming plane over the *entire* window, blocks all interaction
beneath it, and shows a "please wait" message with an **indeterminate spinner (no ETA)**, an
**optional step counter** (`Step {current} of {total}`), and **optional action buttons**
(at minimum Cancel). It is dismissed programmatically when the operation completes, or by an
optional Cancel button.

**Critical invariant (learned the hard way — PR860):** the overlay is a *visual + input*
block only. It must never synchronously wait in the egui frame loop. The real work runs on a
tokio backend task; the overlay is raised when the task is dispatched and lowered when the
`TaskResult` arrives. A synchronous wait here deadlocks rendering.

**Headline recommendation (detailed in §6):** build a **new standalone component** that
*mirrors* `MessageBanner`'s architecture (global state in egui `ctx.data`, a lifecycle
handle, an action-id queue, log-once discipline, theme tokens) — but do **not** extend
`MessageBanner` itself. The two have opposite z-order, opposite blocking semantics, and a
different render seam.

---

## 1. Personas Affected

All three personas (`docs/personas/`) hit long, uninterruptible operations; each experiences
the overlay differently.

| Persona | How they meet the overlay | What they need from it |
|---|---|---|
| **Alex — Everyday User** (low/moderate technical) | Registering a DPNS name; sending Dash; first wallet sync. Alex does not know *what* a "state transition" is and should never see that phrase. | A calm plain-language sentence ("Registering your username. This can take up to a minute."), a spinner that clearly says *working, not frozen*, and — when offered — one obvious Cancel. No jargon, no error codes. |
| **Priya — Power User** (operator) | Asset-lock → fund identity flows; credit transfers; withdrawals. Runs several operations a session and wants to know *which step* she is on. | The step counter (`Step 3 of 5`) so a multi-stage flow is legible; confidence that Cancel is safe; the operation description precise enough to trust. |
| **Jordan — Platform Developer** | Bulk identity creation, contract deploys, repeated testnet iterations. Hammers the app during sprints. | Fast, honest feedback. A spinner that doesn't pretend to know an ETA it can't compute. If something hangs, an escape hatch so a wedged test run doesn't trap the whole app. Step counter for the compound "asset lock → proof → register → fund" chain. |

**Cross-persona truth:** an indeterminate spinner with *no fake progress bar* is the honest
choice — we frequently cannot know how long a Platform round-trip takes. A counterfeit
percentage erodes trust faster than an honest "this is working." Validated first against Alex
(least technical): if Alex understands "the app is busy and will tell me when it's done," the
others are covered.

---

## 2. Functional Requirements

Each FR is written so it can be acceptance-tested. "Overlay" = the blocking progress overlay.

### FR-1 — Show the overlay
A caller can raise the overlay with a single call that returns a lifecycle **handle**
(mirroring `BannerHandle`). The call accepts at minimum a description string and a config
(spinner always on; counter, buttons optional).
**AC-1.1** After a show call, the overlay is visible on the next frame, centered, over the whole window.
**AC-1.2** The call returns a handle usable to update or dismiss the overlay later.
**AC-1.3** The show call is safe to issue from a screen's `ui()` return path or from the app loop; it never blocks the calling thread.

### FR-2 — Replace / update overlay content
The handle can update the description, the step counter, and the button set **in place**
without tearing the overlay down or restarting the spinner.
**AC-2.1** Updating the description changes the visible text on the next frame; the spinner does not flicker or reset.
**AC-2.2** Updating the counter from `2 of 5` to `3 of 5` changes only the counter line.
**AC-2.3** A stale handle (overlay already dismissed) is a no-op returning `None` — never a panic.

### FR-3 — Hide the overlay
The overlay is dismissed (a) programmatically via the handle, or (b) by the app loop when
the owning operation's `TaskResult` arrives.
**AC-3.1** On dismissal the overlay disappears next frame and interaction beneath is fully restored.
**AC-3.2** Dismissing an already-dismissed overlay is a no-op.
**AC-3.3** When the overlay is dismissed because a task failed, the resulting error is shown via `MessageBanner` *after* the overlay is gone (clean hand-off — see FR-9).

### FR-4 — Indeterminate spinner, no ETA
The overlay always shows an animated indeterminate spinner. It **must not** render any
time-derived percentage, progress bar, or ETA.
**AC-4.1** The spinner animates continuously while visible (`egui::Spinner`, which self-requests repaint — no custom per-frame timer).
**AC-4.2** No element expresses "X% complete" or "N seconds remaining" derived from elapsed time.
**AC-4.3** An *optional* honest elapsed-time readout (`Elapsed: {seconds}s`, counting up, never counting down) MAY be shown for reassurance on very long waits — this is not an ETA and is off by default.

### FR-5 — Optional step counter (determinate, discrete)
For multi-step operations, the overlay MAY show a discrete step counter.
**AC-5.1** When a counter is set, the overlay shows a single i18n-ready line: `Step {current} of {total}` (named placeholders, no fragment concatenation).
**AC-5.2** `current` and `total` are positive integers; `current ≤ total`. An invalid pair (e.g. `0 of 0`, `4 of 3`) hides the counter rather than rendering nonsense.
**AC-5.3** The counter is independent of the spinner — the spinner stays indeterminate even when a counter is present (a step counter is *not* a progress percentage).
**AC-5.4** A counter is optional: spinner-only overlays render no counter line and reserve no empty space for it.

### FR-6 — Description text
The overlay MAY show a description of the operation in progress.
**AC-6.1** The description is a complete, plain-language sentence (i18n unit; see NFR-2).
**AC-6.2** Long descriptions wrap; they do not clip or force the window off-screen (scroll within the overlay card if needed).
**AC-6.3** A description is optional; spinner-only is valid (purely informational block with no text is permitted but discouraged for Alex's sake).

### FR-7 — Optional action buttons (Cancel + generic actions)
The overlay MAY show zero or more action buttons. Cancel is the canonical one but the
mechanism is generic.
**AC-7.1** Buttons are optional. With none, the overlay is a pure block dismissed only programmatically.
**AC-7.2** Each button carries a label (i18n unit) and an opaque **action id**. Clicking pushes the action id into an overlay-action queue that the app loop drains and dispatches — exactly mirroring `BannerHandle::with_action` / `MessageBanner::take_action`. The overlay never calls backend code directly (UI-only seam; see NFR-1 and §6).
**AC-7.3** A Cancel button uses a well-known action id; the app loop maps it to the operation's cancellation path.
**AC-7.4** Buttons follow project button order (Confirm/primary RIGHT, Cancel LEFT) and use `StyledButton`/`ComponentStyles`, never bare `ui.button()`.
**AC-7.5** Cancel SHOULD be offered only when the operation is genuinely cancelable (see Risk R-3). When it cannot truly cancel, do not show a button that lies.

### FR-8 — Block all interaction beneath
While visible, the overlay blocks every interactive element beneath it — central content,
left navigation panel, and top panel — and consumes keyboard input not directed at the
overlay's own controls.
**AC-8.1** Pointer clicks/drags on any region outside the overlay's own buttons have no effect on the UI beneath.
**AC-8.2** Keyboard input (Tab, Enter, typing) does not reach widgets beneath the overlay. (Do not rely on `Ui::set_enabled()` — deprecated in egui 0.33; use a top input-capturing layer instead.)
**AC-8.3** The block covers the *entire* window, including top and left panels — therefore the overlay renders at the `AppState` level, not inside `island_central_panel()` (which only wraps central content). See §3.
**AC-8.4** Clicking the dimmed backdrop does **not** dismiss the overlay (unlike a passphrase modal) — a blocking progress overlay is not click-outside-to-cancel; dismissal is programmatic or via an explicit Cancel button only.

### FR-9 — Coexistence with MessageBanner (z-order + hand-off)
**AC-9.1** The overlay renders **above** all `MessageBanner` banners (banners live inside the island content area at a background layer; the overlay sits on a top layer). The overlay wins z-order.
**AC-9.2** Banners already on screen when the overlay appears are covered/dimmed; because banner state persists in `ctx.data`, they reappear intact when the overlay is dismissed.
**AC-9.3** For a single operation, overlay and result-banner are **temporally exclusive**: the overlay is up *while running*; on completion the overlay is dismissed and then the success/error banner is shown. AppState owns this hand-off so screens don't double-report.

### FR-10 — Multiple operations requesting the overlay
There is a **single global overlay slot**, but requests are tracked so concurrent owners
behave predictably.
**AC-10.1** The overlay holds a small **stack of active requests** (each keyed by its handle). The overlay is visible while the stack is non-empty.
**AC-10.2** The **most-recently-shown** request is the one rendered (its description, counter, buttons). Earlier requests remain on the stack but are not rendered.
**AC-10.3** Each handle dismisses **only its own** entry. A stale/earlier handle dismissing does not lower an overlay still owned by a later request. The overlay fully clears only when the stack empties.
**AC-10.4** Rationale and caveat: because the overlay *blocks the UI*, a human cannot launch a second blocking operation — concurrent requests can only come from background/programmatic tasks and are a design smell. The stack model degrades gracefully (it never strands a running operation behind a prematurely-cleared overlay) but callers SHOULD avoid stacking blockers. A single "concurrent overlay" event is logged once, not per frame.
**AC-10.5** Only the topmost request's Cancel/actions are reachable; lower requests' actions become reachable when the top is dismissed.

> **Decision deferred to Nagatha (1d):** stack (AC-10.1, recommended) vs. simple
> last-writer-replace vs. reject-second. The stack is the only model that never unblocks the
> UI while an operation is still running; the simpler models are acceptable if product
> guarantees no concurrent blockers.

---

## 3. Render Seam (important — differs from MessageBanner)

`MessageBanner::show_global()` is called *inside* `island_central_panel()`
(`src/ui/components/styled.rs:565`), i.e. within the central content island — it therefore
**cannot** cover the top panel or left navigation panel.

The blocking overlay must cover **everything**. So it follows the **`AppState`-level render
pattern** already used by the just-in-time secret prompt
(`render_secret_prompt(ctx)`, `src/app.rs:1527`), which draws over all panels after they are
laid out. The overlay's *state* still lives globally in egui `ctx.data` (mirroring banners),
but its *render call* belongs at the end of `AppState::update()`, on a top input-capturing
layer — not in `island_central_panel()`.

Layering target (top to bottom):

```
  ┌─ secret-prompt / confirmation modals  (must stay ABOVE overlay — see R-1)
  ├─ BLOCKING PROGRESS OVERLAY            (top input-capturing layer + dim plane)
  ├─ MessageBanner banners                (inside island content area)
  └─ Top panel / Left panel / Central content
```

---

## 4. Non-Functional Requirements

### NFR-1 — Never block the frame/render thread
The overlay is a visual + input block **only**. The owning operation runs on a tokio backend
task via the existing `BackendTask`/`TaskResult` channel; the overlay is raised at dispatch
and lowered when the result is polled in `AppState::update()`.
**AC:** No code path holds a lock across `.await` to keep the overlay up; no synchronous
sleep/wait in the frame loop. (Reference incident: PR860 — async blocking in the egui frame
loop deadlocked the UI.)

### NFR-2 — i18n-ready strings
All overlay text is i18n-ready: complete sentences, named placeholders, no fragment
concatenation, no grammar assembled from pieces.
**AC:** Counter is one unit `Step {current} of {total}`; elapsed is `Elapsed: {seconds}s`;
descriptions are full sentences. No positional `format!` of sentence fragments. Matches the
project's i18n convention (Rust `{name}` specifiers today, Fluent `{ $name }` later).

### NFR-3 — Accessibility (within egui's constraints)
**AC-3a (focus trap):** while the overlay is up, keyboard focus is confined to its own
controls; Tab does not cycle into widgets beneath.
**AC-3b (Esc):** Esc triggers Cancel **only when the overlay is cancelable** (a Cancel action
is present). With no Cancel, Esc is swallowed (does nothing) — it must not dismiss a
non-cancelable block.
**AC-3c (Enter):** Enter does not auto-confirm anything destructive; if a single primary
action exists, Enter MAY activate it, but Cancel is never the Enter default.
**AC-3d (not color-only):** "busy" is signalled by the moving spinner + text, not color alone.
**AC-3e (contrast):** text/buttons meet WCAG 2.1 AA (4.5:1 text, 3:1 UI) on the dimmed plane in both themes.
**AC-3f (known limitation):** egui has no screen-reader annotation support (per
`docs/ux-design-patterns.md` §10). Documented as a constraint; the moving spinner + visible
sentence are the available affordances. No false promise of SR announcements.

### NFR-4 — Light + dark theme
**AC:** All colors via `DashColors` (dim plane via `modal_overlay()` = `rgba(0,0,0,120)`,
re-evaluated each frame so a theme switch mid-overlay is correct). Card uses
`surface`/`window_fill`, text via `text_primary`/`text_secondary`, spinner via `DASH_BLUE`,
buttons via `ComponentStyles`. Zero hardcoded `Color32`.

### NFR-5 — No per-frame log spam
**AC:** State changes log **once** — on show, on each counter/description change, on dismiss —
guarded by a `logged`/last-logged flag in the stored state (mirroring `BannerState.logged`).
Rendering at ~60fps must not emit ~60 logs/sec. State lives in `ctx.data`, **not** a per-frame
reconstructed instance (avoids the "fresh instance each frame resets state + spams logs"
trap).

### NFR-6 — Cheap render
**AC:** When no overlay is active, the render call is an early-out reading one `ctx.data` slot
(mirroring `show_global`'s empty check). No allocation on the idle path.

---

## 5. User Journeys

> **Usage rule (when to block vs. when to banner):** use the overlay only when continued
> interaction would be *unsafe or meaningless*. Long *background* work the user can safely
> ignore (e.g. ambient SPV sync, identity discovery sweeps) stays a non-blocking
> `MessageBanner::with_elapsed()` progress banner. Blocking the whole UI for ambient sync
> would punish Priya and Jordan, who legitimately work while syncing.

### J-1 — Identity registration (multi-step, counter + Cancel) — Priya / Jordan
1. User confirms "Register identity."
2. Overlay appears: `Step 1 of 4` · "Preparing the funding lock." · Cancel.
3. Backend advances: handle updates to `Step 2 of 4` "Waiting for the funding proof.", then
   `Step 3 of 4` "Registering your identity.", then `Step 4 of 4` "Funding your identity."
4. Spinner stays indeterminate throughout (each step's duration is unknown).
5. On the final `TaskResult::Success`, AppState dismisses the overlay, then shows a success
   banner. On failure, overlay is dismissed, then an error banner appears (FR-9).

### J-2 — Broadcasting / signing a transaction (spinner + description, often no Cancel) — all
1. User confirms a send / state-transition broadcast.
2. Overlay: spinner + "Sending your transaction to the network." No Cancel (a broadcast in
   flight cannot be safely recalled — R-3).
3. Overlay lowers on result; banner reports outcome.

### J-3 — Multi-step shielded operation (spinner + counter + description + Cancel) — Jordan/Priya
1. User starts a shield/unshield.
2. Overlay: `Step 2 of 3` · "Building the shielded transaction." · Cancel (note generation /
   proving can be long and is locally cancelable before broadcast).
3. If the user cancels before broadcast, the Cancel action id is dispatched, the backend
   aborts the local build, the overlay lowers, and an info banner notes "Operation canceled."

### J-4 — Migration / key import (informational hard block, no buttons) — all
1. A one-shot migration step or sensitive key import runs.
2. Overlay: spinner + "Updating your wallet data. Please keep the app open." No buttons
   (interrupting mid-migration risks inconsistent state).
3. Overlay lowers only when the step completes. (This is exactly the open question raised in
   `docs/ai-design/2026-05-28-migration-tool/notes.md` — "Spinner with progress, modal
   blocker, banner?" — this overlay is the answer for the blocking case.)

### J-5 — Network switch (brief block) — Jordan
1. User switches Testnet ⇄ Devnet.
2. Brief overlay: spinner + "Switching networks." prevents interacting with stale
   network state during the swap; lowers when the new context is ready.

---

## 6. Interaction Patterns & Wireframes (ASCII)

The dim plane (`modal_overlay()`) covers the whole window; a centered card holds the content.
The card uses the dialog idiom (rounded corners, shadow, `surface`/`window_fill`).

### 6.1 Spinner-only (pure block)

```
┌───────────────────────────────────────────────────────────────┐
│░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░│
│░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░│
│░░░░░░░░░░░░░░░░░░░░┌───────────────────────┐░░░░░░░░░░░░░░░░░░░░│
│░░░░░░░░░░░░░░░░░░░░│                       │░░░░░░░░░░░░░░░░░░░░│
│░░░░░░░░░░░░░░░░░░░░│          (◠)          │░░░░░░░░░░░░░░░░░░░░│
│░░░░░░░░░░░░░░░░░░░░│        spinner        │░░░░░░░░░░░░░░░░░░░░│
│░░░░░░░░░░░░░░░░░░░░│                       │░░░░░░░░░░░░░░░░░░░░│
│░░░░░░░░░░░░░░░░░░░░└───────────────────────┘░░░░░░░░░░░░░░░░░░░░│
│░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░│
└───────────────────────────────────────────────────────────────┘
   ░ = dimmed, input-blocked backdrop (entire window incl. panels)
```

### 6.2 Spinner + step counter

```
░░░░░░░░░░░░┌─────────────────────────────────┐░░░░░░░░░░░░
░░░░░░░░░░░░│              (◠)                 │░░░░░░░░░░░░
░░░░░░░░░░░░│            spinner               │░░░░░░░░░░░░
░░░░░░░░░░░░│                                  │░░░░░░░░░░░░
░░░░░░░░░░░░│           Step 3 of 5            │░░░░░░░░░░░░
░░░░░░░░░░░░└─────────────────────────────────┘░░░░░░░░░░░░
```

### 6.3 Spinner + counter + description + Cancel (full)

```
░░░░░░░┌───────────────────────────────────────────────┐░░░░░░░
░░░░░░░│                    (◠)                          │░░░░░░░
░░░░░░░│                  spinner                        │░░░░░░░
░░░░░░░│                                                 │░░░░░░░
░░░░░░░│                Step 2 of 4                      │░░░░░░░
░░░░░░░│                                                 │░░░░░░░
░░░░░░░│   Waiting for the funding proof. This can       │░░░░░░░
░░░░░░░│   take up to a minute.                          │░░░░░░░
░░░░░░░│                                                 │░░░░░░░
░░░░░░░│                                  [   Cancel   ] │░░░░░░░
░░░░░░░└───────────────────────────────────────────────┘░░░░░░░
```

### 6.4 Two generic actions (Cancel left, primary right) + optional elapsed

```
░░░░░░░┌───────────────────────────────────────────────┐░░░░░░░
░░░░░░░│                    (◠)  spinner                 │░░░░░░░
░░░░░░░│                                                 │░░░░░░░
░░░░░░░│   Building the shielded transaction.            │░░░░░░░
░░░░░░░│   Elapsed: 23s                                  │░░░░░░░  ← honest, counts UP, optional
░░░░░░░│                                                 │░░░░░░░
░░░░░░░│  [ Cancel ]                    [ Run in background ] │░░░░░░░
░░░░░░░└───────────────────────────────────────────────┘░░░░░░░
```

### 6.5 Interaction states

| State | Behavior |
|---|---|
| Visible, no buttons | Pure block. Esc/Enter swallowed. Backdrop click ignored. Dismissed only programmatically. |
| Visible, cancelable | Esc → Cancel. Cancel button focusable and is the first focus stop. Backdrop click ignored. |
| Button hover/focus | Standard `StyledButton` hover (pointing-hand) + focus ring (`BORDER_WIDTH_THICK`, ≥3:1). |
| Counter update | Only the counter line changes; spinner uninterrupted. |
| Theme switch mid-overlay | Colors re-evaluate next frame; no stale palette. |
| Window resized very small | Card shrinks to a min width; long description scrolls within the card; never pushed off-screen. |
| Operation hangs (no result) | After a threshold, surface optional elapsed readout and (if safe) an escape hatch — see R-4. |

---

## 7. UX Recommendation — Extend `MessageBanner` vs. New Standalone Component

**Recommendation: a NEW standalone component that mirrors `MessageBanner`'s architecture, not
an extension of `MessageBanner`.** (Final architecture call is Nagatha's in 1d; this is the
UX/maintainability advisory.)

### Why not extend `MessageBanner`
| Dimension | MessageBanner | Blocking overlay | Verdict |
|---|---|---|---|
| Blocking | Never blocks; user works around it | Blocks the entire UI | Opposite semantics |
| Multiplicity | Up to 5 stacked, all visible | One rendered at a time | Different model |
| Z-order / seam | Inside `island_central_panel()` content area | `AppState`-level top layer over all panels | Different render seam (§3) |
| Lifecycle | Severity-based auto-dismiss | Dismissed by task result or Cancel; never auto-times-out | Different lifecycle |
| Anatomy | Icon + text + dismiss + optional details/suggestion/action | Spinner + description + counter + optional buttons, centered card | Different anatomy |

Folding all of this into `BannerState` would bloat every banner with
spinner/step/button-set/blocking fields that are meaningless for the 99% of banners that are
simple notices, and would entangle the central render path. It violates single-responsibility
and would make the well-understood banner harder to reason about.

### What to reuse (mirror, don't merge)
The overlay should **copy the proven *patterns***, keeping its own type:
1. **Global state in egui `ctx.data` temp storage**, keyed by a dedicated id (e.g.
   `__global_progress_overlay`) — same mechanism as `BANNER_STATE_ID`.
2. **A lifecycle `OverlayHandle`** mirroring `BannerHandle` (`set_description`, `set_step`,
   `with_button` / `with_secondary_button`, `clear`), all returning `Option` and no-op on a
   dismissed overlay. _(Superseded: no `with_cancel` — buttons are generic; see the dev-plan
   post-outage note and the code.)_
3. **An action-id queue** drained by the app loop. As shipped (addendum §2) the queue is **keyed**
   per overlay entry: a click enqueues against the owner's key, the owner drains its own ids via
   `OverlayHandle::take_actions`, and the app loop only `sweep_orphan_actions` — keeps the overlay
   UI-only; backend dispatch stays in `AppState`. This is the i18n-clean, `ctx.data`-friendly
   equivalent of "callbacks":
   storing closures in temp storage is awkward (not `Clone`); an opaque action id is the
   established seam.
4. **Log-once discipline** via a `logged` flag (NFR-5).
5. **Theme tokens + button helpers** (`DashColors`, `ComponentStyles`, `StyledButton`).

### Placement
Per the DET module-placement policy, it renders egui → it is a **component** in
`src/ui/components/` (e.g. `progress_overlay.rs`), with its `show_global(ctx)` invoked from
`AppState::update()` near `render_secret_prompt`. Non-rendering helpers stay out of it.

A thin shared helper for the `ctx.data` get/set/clear plumbing *could* be factored out and
used by both components, but only if it reads cleanly — the volume of shared code is small,
and premature abstraction here would couple two intentionally-different widgets. Lean toward a
focused copy over a forced shared base; defer to Nagatha.

---

## 8. Open Questions & Risks

- **R-1 — Z-order vs. secret-prompt / confirmation modals.** If an operation behind the
  overlay needs a passphrase mid-flight, the secret-prompt modal (`render_secret_prompt`) must
  render **above** the overlay and stay interactive — otherwise the operation wedges. Per the
  sign-time prompt design (gate-on-error + auto-retry, not mid-flight), the common case avoids
  this, but the layer ordering must be explicit: secret prompt / confirmation dialog > overlay.
  Decide and test.
- **R-2 — Concurrent blocking operations (FR-10).** Confirm the stack model vs.
  last-writer-replace vs. reject-second. Stack is safest (never unblocks while a task runs);
  simpler models need a product guarantee that blockers don't overlap.
- **R-3 — Does Cancel actually cancel?** The honesty of the Cancel button depends on the
  `BackendTask` system supporting cooperative cancellation (cancel tokens / abortable tasks).
  If a task cannot truly be aborted, Cancel can only *stop waiting* while the work continues —
  which is misleading and unsafe (e.g. a broadcast). **Verify backend cancellation support
  downstream.** Until then, show Cancel only for operations that are genuinely cancelable
  (local pre-broadcast work); broadcasts/migrations should be button-less blocks.
- **R-4 — Stuck overlay / no TaskResult.** With an indeterminate spinner and no ETA, a hung
  task could trap the user forever. Need a safety valve: after a threshold, reveal the optional
  elapsed readout and — only where safe — an escape hatch ("This is taking longer than usual"
  + a way out). Define the threshold and which operations get an escape hatch.
- **R-5 — Should the top-panel connection/network indicator remain readable?** A full dim
  hides connection status. Decide whether the overlay should leave the connection indicator
  legible (e.g. lighter dim on the top strip) so a user waiting on a network op can see the
  network dropped. Leaning: keep the block total for simplicity, surface connection loss via
  the post-dismissal banner; revisit if it confuses users.
- **R-6 — Accessibility ceiling.** egui exposes no screen-reader annotations
  (`ux-design-patterns.md` §10). The moving spinner + visible sentence are the only
  affordances; a non-sighted user gets no announced "busy." Documented limitation — flag if a
  future AccessKit pass can announce overlay open/close.
- **R-7 — Tests.** kittest coverage should assert: input beneath is blocked, Esc cancels only
  when cancelable, counter validation hides nonsense pairs, action id is enqueued on click and
  drained FIFO, log-once, and dismiss-on-task-result hand-off to banner. Mirror the existing
  `tests/kittest/message_banner.rs` style.

---

## 9. Requirements Quality Checklist

- ✅ Every persona has at least one journey (J-1…J-5 cover Alex, Priya, Jordan).
- ✅ Every FR has testable acceptance criteria.
- ✅ ≥3 real-life scenarios per major workflow (5 journeys, multiple step states).
- ✅ Edge/failure cases addressed (stale handle, hang, theme switch, tiny window, concurrent owners, failed task).
- ✅ Priorities/decisions flagged for the downstream owner (Nagatha) with rationale.
- ✅ Assumptions explicit (cancellation support, single-slot semantics, render seam).

---

## 10. Notes for Downstream Phases

- **Worktree/merge:** the requested first-action `git merge --ff-only 0484bcb6…` was a no-op —
  local HEAD already sits at `0484bcb6` per `git status`. No shell was available in this design
  session; all file/line references were read directly from the live working tree, so no
  "re-verify after merge" caveat is needed for the citations.
- **Commit:** this design session had no shell access, so the doc could not be `git add`/
  committed automatically. The team lead should commit it:
  `git add -A && git commit -m "docs(overlay): requirements + UX spec for blocking progress overlay"`.
- **Key source references (live tree):** `src/ui/components/message_banner.rs` (sibling
  pattern), `src/ui/components/styled.rs:539-571` (`island_central_panel` render seam),
  `src/app.rs:1527` (`render_secret_prompt` — the AppState-level modal render analog),
  `src/ui/components/secret_prompt_host.rs` (global-modal-via-AppState pattern),
  `src/ui/components/passphrase_modal.rs:128-156` (full-screen dim + centered window),
  `src/ui/theme.rs:430` (`modal_overlay()`), `docs/ux-design-patterns.md` §8/§10/§11.
```
