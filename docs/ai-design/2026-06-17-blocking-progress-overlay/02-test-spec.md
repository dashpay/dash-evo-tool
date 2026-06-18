# Blocking Progress Overlay — Test Case Specification

**Phase:** 1c (QA — Test Case Specification)
**Author:** Marvin (QA Engineer)
**Date:** 2026-06-17
**Status:** Draft — pending architecture decision on FR-10 (see §4)
**Input:** `01-requirements-ux.md` (Requirements + UX Spec by Diziet)
**Style reference:** `tests/kittest/message_banner.rs`

---

## 1. Overview

This document specifies acceptance test cases for the **Blocking Progress Overlay** component.
Test cases are derived entirely from the requirements spec (`01-requirements-ux.md`); expected
behavior is defined by the spec, never by a yet-to-be-written implementation.

Every FR (FR-1 through FR-10) and NFR (NFR-1 through NFR-6) is covered by at least one TC.
Items that depend on the architecture decision deferred to Nagatha (1d) are marked
**[depends on 1d]**.

> **Post-outage reframe (generic button).** First-class "Cancel" was removed in favour of a
> generic button facility: a caller attaches a button with `with_action(label, id)`, the click
> enqueues the caller's id, the owning screen drains it via `take_actions` and runs its own logic
> (including cancellation), and Esc does **not** dismiss. The Cancel-specific cases below
> (TC-OVL-024/025/026/027/042/043/044) are **reframed in place** to the generic-button model —
> numbers are preserved; each carries a "(reframed post-outage: generic button)" note.

### 1.1 Test Type Key

| Tag | Meaning |
|-----|---------|
| **kittest** | Implemented as an `egui_kittest` Harness test; assertable via `query_by_label`, rendered-widget tree, and `ctx.data` reads. Fast, deterministic, runs in CI. |
| **ctx.data** | Pure context-state assertion, no rendering; verifies `ctx.data` slot directly. Subtype of kittest. |
| **design-review** | Not directly automatable via kittest — must be verified by code inspection or human review. Noted with the specific invariant to check. |
| **integration** | Requires a full `AppState` frame loop (AppState-level render seam, task dispatch). Verifiable in the backend-e2e harness or a dedicated app-level kittest. |

### 1.2 Naming Conventions Assumed

The following public surface is assumed to exist (mirroring `MessageBanner`). Names are
illustrative; the architecture phase (1d) may adjust them.

| Assumed name | Purpose |
|---|---|
| `ProgressOverlay::set_global(ctx, description, config)` | Raises the overlay; returns `OverlayHandle` |
| `ProgressOverlay::has_global(ctx)` | Returns `true` when an overlay is active |
| `ProgressOverlay::set_global_spinner_only(ctx)` | Convenience: spinner-only, no text |
| `ProgressOverlay::render_global(ctx)` | Render call from `AppState::update()` |
| `ProgressOverlay::take_actions(ctx)` | Drains the action-id queue (FIFO) |
| `OverlayHandle::set_description(text)` | Updates description; returns `Option<&Self>` |
| `OverlayHandle::set_step(current, total)` | Updates counter; returns `Option<&Self>` |
| `OverlayHandle::clear_step()` | Removes counter; returns `Option<&Self>` |
| `OverlayConfig::with_action(label, id)` / `OverlayHandle::with_action(label, id)` | Adds a generic button (reframed post-outage: no built-in Cancel); the handle form returns `Option<&Self>` |
| `OverlayHandle::clear()` | Dismisses this handle's overlay entry |
| `OverlayHandle::is_active()` | Returns `true` if still on the overlay stack |

---

## 2. Test Cases

### Group A — Idle Path

---

#### TC-OVL-001 — No overlay renders when no state is set
**Type:** kittest
**Traceability:** NFR-6 (cheap idle path)

**Preconditions:** A fresh `egui::Context` with no overlay state written to `ctx.data`.

**Steps:**
1. Build a Harness with `ProgressOverlay::render_global()` in the `build_ui` closure.
2. Call `harness.run()`.

**Expected outcome:**
- `ProgressOverlay::has_global(ctx)` returns `false`.
- No spinner widget, no description label, no step counter label, no button appears in the
  rendered tree.
- The render call performs a single `ctx.data` read and returns immediately (no allocation;
  verified by code inspection — see NFR-6 AC).

---

### Group B — Show Lifecycle (FR-1)

---

#### TC-OVL-002 — Overlay appears on the next frame after show
**Type:** kittest
**Traceability:** FR-1 (AC-1.1)

**Preconditions:** Fresh context; no overlay active.

**Steps:**
1. Inside `build_ui`, call `ProgressOverlay::set_global(ctx, "Registering your identity.", config_default())`.
2. Also call `ProgressOverlay::render_global(ctx)`.
3. Call `harness.run()`.

**Expected outcome:**
- `ProgressOverlay::has_global(ctx)` returns `true`.
- A label containing `"Registering your identity."` is present in the rendered tree
  (`harness.query_by_label("Registering your identity.").is_some()`).
- A spinner widget is present (query by egui `Spinner` widget type or by its accessibility label
  if one is set — see implementation note in AC-3d).

---

#### TC-OVL-003 — Show call returns a usable handle
**Type:** ctx.data
**Traceability:** FR-1 (AC-1.2)

**Preconditions:** Fresh context.

**Steps:**
1. Call `let handle = ProgressOverlay::set_global(&ctx, "Loading.", config_default())`.
2. Assert `handle.is_active()` returns `true`.
3. Call `handle.set_description("Updated text.")` and assert it returns `Some(&handle)`.
4. Assert `ProgressOverlay::has_global(&ctx)` returns `true`.

**Expected outcome:**
All assertions pass. The handle is non-null and addresses a live overlay entry.

---

#### TC-OVL-004 — Show call never blocks the calling thread
**Type:** design-review
**Traceability:** FR-1 (AC-1.3), NFR-1

**Invariant to verify during code review:**
`ProgressOverlay::set_global()` must not acquire any async lock, call `.await`, or issue a
`std::thread::sleep`. It must only write to `egui::ctx.data` (a synchronous, lock-guarded
`TypeMap`). The implementation must be callable safely from `Screen::ui()` or from the app
loop without any risk of yielding.

**CI note:** Reference PR860 (deadlock caused by async blocking in egui frame loop). Any
`async fn`, `.await`, `Mutex::lock().await`, or `sleep` in the show path is a blocker.

---

### Group C — Update In Place (FR-2)

---

#### TC-OVL-005 — Description update changes text; spinner does not flicker
**Type:** kittest
**Traceability:** FR-2 (AC-2.1)

**Preconditions:** Overlay active with description `"Preparing the funding lock."`.

**Steps:**
1. Set overlay with description `"Preparing the funding lock."`.
2. Run one frame; assert label `"Preparing the funding lock."` is present.
3. Call `handle.set_description("Waiting for the funding proof.")`.
4. Run another frame.

**Expected outcome:**
- Label `"Waiting for the funding proof."` is present.
- Label `"Preparing the funding lock."` is absent.
- The spinner widget is still present (same render path; no reset observable — no widget
  re-creation that would reset the animation seed).
- `ProgressOverlay::has_global(ctx)` still returns `true`.

---

#### TC-OVL-006 — Counter update changes only the counter line
**Type:** kittest
**Traceability:** FR-2 (AC-2.2)

**Preconditions:** Overlay active with step `2 of 5`, description `"Processing."`.

**Steps:**
1. Show overlay; set step `(2, 5)` and description `"Processing."`.
2. Run one frame; assert label `"Step 2 of 5"` is present.
3. Call `handle.set_step(3, 5)`.
4. Run another frame.

**Expected outcome:**
- Label `"Step 3 of 5"` is present.
- Label `"Step 2 of 5"` is absent.
- Label `"Processing."` still present (description unchanged).
- Spinner still present.

---

#### TC-OVL-007 — Stale handle update is a no-op returning None
**Type:** ctx.data
**Traceability:** FR-2 (AC-2.3)

**Preconditions:** An `OverlayHandle` whose overlay has already been dismissed.

**Steps:**
1. Show overlay; capture `handle`.
2. Call `handle.clear()` to dismiss.
3. Call `handle.set_description("After clear")`.
4. Call `handle.set_step(1, 3)`.
5. Call `handle.with_action("Continue", "overlay.action")`.

**Expected outcome:**
- All handle method calls return `None`.
- `ProgressOverlay::has_global(ctx)` returns `false` (no new overlay created).
- No panic.

---

### Group D — Dismiss (FR-3)

---

#### TC-OVL-008 — Programmatic dismiss removes overlay and restores interaction
**Type:** kittest
**Traceability:** FR-3 (AC-3.1)

**Preconditions:** Overlay is active.

**Steps:**
1. Show overlay.
2. Run one frame; assert `has_global` is `true`.
3. Call `handle.clear()`.
4. Run another frame.

**Expected outcome:**
- `ProgressOverlay::has_global(ctx)` returns `false`.
- No spinner, description, or counter label is present in the rendered tree.
- Widgets that were beneath the overlay are accessible again (not blocked — verified by
  querying a sibling label that was previously beneath the dim plane and confirming it is
  present and interactive).

---

#### TC-OVL-009 — Double dismiss is a no-op
**Type:** ctx.data
**Traceability:** FR-3 (AC-3.2)

**Preconditions:** Fresh context.

**Steps:**
1. Show overlay; capture `handle`.
2. Call `handle.clear()` — assert returns without panic.
3. Call `handle.clear()` a second time — assert no panic.
4. Assert `ProgressOverlay::has_global(ctx)` returns `false`.

**Expected outcome:** No panic; `has_global` is `false`.

---

#### TC-OVL-010 — Dismiss on task failure: overlay gone before error banner appears
**Type:** integration
**Traceability:** FR-3 (AC-3.3), FR-9 (AC-9.3), J-1 / J-2 flow

**Preconditions:** A simulated task that returns `TaskResult::Failure` is dispatched with an
overlay raised at dispatch time.

**Steps:**
1. App loop dispatches task; raises overlay.
2. Simulate a `TaskResult::Failure` arriving in the `task_result_receiver`.
3. App loop polls result; in the same frame:
   a. Dismisses the overlay (`handle.clear()`).
   b. Calls `MessageBanner::set_global(ctx, error_message, MessageType::Error)`.
4. Render the frame via `render_global(ctx)` and `MessageBanner::show_global(ui)`.

**Expected outcome:**
- `ProgressOverlay::has_global(ctx)` returns `false`.
- `MessageBanner::has_global(ctx)` returns `true` with the error text.
- No frame exists where both the overlay and the error banner are visible simultaneously.

---

### Group E — Spinner (FR-4)

---

#### TC-OVL-011 — Spinner is always present when overlay is active
**Type:** kittest
**Traceability:** FR-4 (AC-4.1)

**Preconditions:** Overlay raised in each of the three configurations: spinner-only,
spinner+counter, spinner+counter+description+button.

**Steps:** For each configuration, call `set_global`, run one frame, query the rendered tree.

**Expected outcome:**
An `egui::Spinner` widget (or widget with the spinner accessibility label, per implementation)
is present in all three configurations. The spinner does not require a custom per-frame repaint
timer — it self-requests repaint via egui's native animation clock.

---

#### TC-OVL-012 — No ETA, progress bar, or percentage element present
**Type:** kittest
**Traceability:** FR-4 (AC-4.2)

**Preconditions:** Overlay active with a description and step counter.

**Steps:**
1. Show overlay with step `(2, 5)` and description `"Building the shielded transaction."`.
2. Run one frame; collect all rendered labels.

**Expected outcome:**
- No label matches the pattern `*%*` (percentage).
- No label matches `*remaining*`, `*seconds left*`, `*ETA*`, or any time-countdown string.
- No `egui::ProgressBar` widget is present in the render tree.
- The step counter label `"Step 2 of 5"` is present (discrete, not a progress percentage).

---

#### TC-OVL-013 — Optional elapsed readout is off by default; when on, counts up
**Type:** kittest
**Traceability:** FR-4 (AC-4.3)

**Preconditions:** Fresh context.

**Steps (Part A — default off):**
1. Show overlay with default config.
2. Run one frame.
3. Assert no label matching `"Elapsed:"` is present.

**Steps (Part B — enabled):**
1. Show overlay; enable elapsed readout via config.
2. Run one frame; assert a label matching `"Elapsed: {seconds}s"` with `seconds ≥ 0` is present.
3. Advance the egui clock by 2 seconds; run another frame.
4. Assert the `seconds` value in the label is ≥ 2 and is not counting down.

**Expected outcome:**
- Part A: no elapsed label present.
- Part B: label present; the `seconds` value increases monotonically across frames; it does
  not count down from any target.

---

### Group F — Step Counter (FR-5)

---

#### TC-OVL-014 — Valid counter renders "Step {current} of {total}"
**Type:** kittest
**Traceability:** FR-5 (AC-5.1)

**Preconditions:** Overlay raised with `set_step(3, 5)`.

**Steps:**
1. Show overlay; set step `(3, 5)`.
2. Run one frame.

**Expected outcome:**
- Exactly one label matching `"Step 3 of 5"` is present.
- The string is a single i18n unit with named placeholders rendered into it; it is not
  constructed by concatenating `"Step "`, `"3"`, `" of "`, `"5"` as separate label segments
  (design-review: verify the format string in the implementation is `"Step {current} of
  {total}"` or equivalent single-unit string, not fragment concatenation).

---

#### TC-OVL-015 — Invalid counter (0 of 0) hides the counter line
**Type:** kittest
**Traceability:** FR-5 (AC-5.2)

**Preconditions:** Overlay raised; `set_step(0, 0)` called.

**Steps:**
1. Show overlay; call `handle.set_step(0, 0)`.
2. Run one frame.

**Expected outcome:**
- No label containing `"Step"` is present in the rendered tree.
- The spinner and any description are still present.

---

#### TC-OVL-016 — Invalid counter (current > total) hides the counter line
**Type:** kittest
**Traceability:** FR-5 (AC-5.2)

**Preconditions:** Overlay raised; `set_step(4, 3)` called.

**Steps:**
1. Show overlay; call `handle.set_step(4, 3)`.
2. Run one frame.

**Expected outcome:**
- No label containing `"Step"` is present.
- No panic.

---

#### TC-OVL-017 — Invalid counter (current = 0, total > 0) hides the counter line
**Type:** kittest
**Traceability:** FR-5 (AC-5.2)

**Preconditions:** Overlay raised; `set_step(0, 5)` called.

**Steps:**
1. Show overlay; call `handle.set_step(0, 5)`.
2. Run one frame.

**Expected outcome:**
- No label containing `"Step"` is present.
- No panic.

---

#### TC-OVL-018 — Counter presence does not make the spinner determinate
**Type:** kittest
**Traceability:** FR-5 (AC-5.3)

**Preconditions:** Overlay raised with `set_step(2, 4)`.

**Steps:**
1. Show overlay with step `(2, 4)`.
2. Run one frame.

**Expected outcome:**
- Label `"Step 2 of 4"` is present.
- An `egui::Spinner` widget is present (indeterminate — no `egui::ProgressBar` present).
- No percentage label is present.

---

#### TC-OVL-019 — No counter line when counter is not set
**Type:** kittest
**Traceability:** FR-5 (AC-5.4)

**Preconditions:** Overlay raised with description only; `set_step` never called.

**Steps:**
1. Show overlay with description `"Sending your transaction to the network."` and no step.
2. Run one frame.

**Expected outcome:**
- No label containing `"Step"` is present.
- No empty/blank row is reserved for the counter (no extraneous whitespace element).
- Description and spinner are present.

---

### Group G — Description Text (FR-6)

---

#### TC-OVL-020 — Description renders as a full plain-language sentence
**Type:** kittest
**Traceability:** FR-6 (AC-6.1)

**Preconditions:** Overlay raised with description `"Registering your identity on the network."`.

**Steps:**
1. Show overlay with the description string above.
2. Run one frame.

**Expected outcome:**
- Label `"Registering your identity on the network."` is present as a single label (not split
  across multiple egui labels — design-review: verify single `ui.label()` call with the full
  string, not fragment concatenation).

---

#### TC-OVL-021 — Long description wraps and does not clip or push off-screen
**Type:** kittest
**Traceability:** FR-6 (AC-6.2)

**Preconditions:** A harness window narrower than the description text (e.g. 300 px wide).
Description is `"Waiting for the funding proof. This operation contacts the Dash network and may take up to two minutes depending on network conditions."`.

**Steps:**
1. Build harness with `egui::vec2(300.0, 400.0)`.
2. Show overlay with the long description above.
3. Run one frame.

**Expected outcome:**
- The label is present in the rendered tree (not clipped to `""` or empty).
- No egui `clip_rect` overflow warning fires (monitored via test output).
- The card and overlay remain within the window bounds (all widgets within `[0, 300] × [0, 400]`).

---

#### TC-OVL-022 — Spinner-only overlay is valid (no description, no counter)
**Type:** kittest
**Traceability:** FR-6 (AC-6.3), FR-5 (AC-5.4)

**Preconditions:** Overlay raised via `set_global_spinner_only(ctx)`.

**Steps:**
1. Raise spinner-only overlay.
2. Run one frame.

**Expected outcome:**
- `ProgressOverlay::has_global(ctx)` is `true`.
- Spinner widget is present.
- No description label is present.
- No step counter label is present.
- No button is present.

---

### Group H — Buttons & Actions (FR-7)

---

#### TC-OVL-023 — No buttons: overlay is a pure block, dismissed programmatically only
**Type:** kittest
**Traceability:** FR-7 (AC-7.1)

**Preconditions:** Overlay raised with no buttons in config.

**Steps:**
1. Show overlay with no `with_action` calls.
2. Run one frame.

**Expected outcome:**
- No button widget is present in the rendered tree.
- `ProgressOverlay::take_actions(ctx)` returns an empty list.
- `ProgressOverlay::has_global(ctx)` is `true` (overlay persists — only `handle.clear()` can
  lower it).

---

#### TC-OVL-024 — Button click enqueues its action id (reframed post-outage: generic button)
**Type:** kittest
**Traceability:** FR-7 (AC-7.2, AC-7.3)

**Preconditions:** Overlay raised with `with_action("Cancel", "overlay.cancel")`. There is no
built-in Cancel — `"Cancel"` is just a caller-chosen label and `"overlay.cancel"` a caller-chosen id.

**Steps:**
1. Show overlay with a generic button.
2. Run one frame; assert a button with label `"Cancel"` is present.
3. Click the button (via `harness.get_by_label("Cancel").click()`).
4. Run another frame.
5. Call `ProgressOverlay::take_actions(ctx)`.

**Expected outcome:**
- `take_actions` returns a list containing exactly one entry equal to the caller's id `"overlay.cancel"`.
- The overlay itself is NOT automatically dismissed by the click — it remains until the owning
  screen drains the action and explicitly calls `handle.clear()` (the overlay is UI-only and knows
  nothing about cancellation).

---

#### TC-OVL-025 — Generic button click enqueues its action id (reframed post-outage: generic button)
**Type:** kittest
**Traceability:** FR-7 (AC-7.2)

**Preconditions:** Overlay raised with `with_action("Run in background", "overlay.run_in_bg")`.

**Steps:**
1. Show overlay with generic action button labelled `"Run in background"`.
2. Run one frame; assert button present.
3. Click the button.
4. Run another frame; call `take_actions(ctx)`.

**Expected outcome:**
- `take_actions` returns `["overlay.run_in_bg"]`.
- Overlay still active (not auto-dismissed).

---

#### TC-OVL-026 — Action queue drains FIFO (reframed post-outage: generic button)
**Type:** kittest
**Traceability:** FR-7 (AC-7.2)

**Preconditions:** Overlay raised with two generic buttons (no built-in Cancel).

**Steps:**
1. Show overlay with `with_action("Cancel", "cancel")` and `with_action("Secondary", "secondary")`.
2. Click `"Cancel"`; run one frame.
3. Click `"Secondary"`; run one frame.
4. Call `ProgressOverlay::take_actions(ctx)` once.

**Expected outcome:**
- `take_actions` returns `["cancel", "secondary"]` in that order (FIFO).
- A second call to `take_actions` returns an empty list (queue drained).

---

#### TC-OVL-027 — Buttons render in insertion order (reframed post-outage: generic button)
**Type:** kittest (widget-position assertion) + design-review
**Traceability:** FR-7 (AC-7.4)

**Preconditions:** Overlay with `with_action("First action", "first")` and
`with_action("Second action", "second")`. There is no Cancel-specific placement — buttons render
left-to-right in insertion order.

**Steps:**
1. Show overlay; run one frame.
2. Query the widget rects for `"First action"` and `"Second action"`.

**Expected outcome:**
- The X coordinate of the first-added button's rect is less than that of the second-added button
  (left-to-right insertion order).
- Both buttons use `ComponentStyles` button helpers, not bare `ui.button()` (design-review: verify
  no `ui.button()` call in the overlay renderer).

---

### Group I — Input Blocking (FR-8)

---

#### TC-OVL-028 — Pointer clicks on the dimmed backdrop have no effect on widgets beneath
**Type:** kittest
**Traceability:** FR-8 (AC-8.1)

**Preconditions:** A test widget (a counter button labelled `"Increment"`) is rendered beneath
the overlay. Overlay has no buttons.

**Steps:**
1. Render both the backdrop-blocking overlay and the `"Increment"` counter widget.
2. Simulate a pointer click at the position of the `"Increment"` button.
3. Run one frame.

**Expected outcome:**
- The counter widget has not received the click event (counter value unchanged).
- `ProgressOverlay::take_actions(ctx)` returns empty (no overlay action triggered either).

---

#### TC-OVL-029 — Keyboard input does not reach widgets beneath the overlay
**Type:** kittest
**Traceability:** FR-8 (AC-8.2)

**Preconditions:** A text input widget is rendered beneath the overlay. Overlay has one generic
button.

**Steps:**
1. Render both the overlay (with a button) and a `TextEdit` widget beneath.
2. Type characters `"hello"` via `harness.key_press` or equivalent.
3. Run one frame.

**Expected outcome:**
- The `TextEdit` content is unchanged — no characters were forwarded beneath the overlay.
- The overlay's button may have received focus (expected — the overlay captures input);
  the `TextEdit` must not.

**Implementation note (AC-8.2):** do not use `Ui::set_enabled(false)` (deprecated in egui
0.33). The spec requires a top input-capturing layer. Verify in design review that the
implementation uses `egui::Area` with `order(Order::Foreground)` or equivalent `ctx.input`
consumption, not `set_enabled`.

---

#### TC-OVL-030 — Backdrop click does NOT dismiss the overlay
**Type:** kittest
**Traceability:** FR-8 (AC-8.4)

**Preconditions:** Overlay active with no buttons.

**Steps:**
1. Show overlay.
2. Run one frame; assert overlay active.
3. Simulate a pointer click on the dim plane (anywhere outside the card).
4. Run another frame.

**Expected outcome:**
- `ProgressOverlay::has_global(ctx)` is still `true`.
- No action was enqueued.

---

#### TC-OVL-031 — Overlay renders at AppState level, covering all panels
**Type:** design-review
**Traceability:** FR-8 (AC-8.3), §3 Render Seam

**Invariant to verify during code review:**
`ProgressOverlay::render_global(ctx)` must be called from `AppState::update()` after all
panels are laid out — not from inside `island_central_panel()`. The call site should be near
`render_secret_prompt(ctx)` at `src/app.rs:1527`. The overlay uses an `egui::Area` or
equivalent top layer to cover the entire viewport (top panel + left panel + central content),
not just the central content island.

Verify: grepping for `render_global` in `src/app.rs` finds a call after panel rendering;
grepping for `render_global` in `src/ui/components/styled.rs` (`island_central_panel`) finds
**no** call.

---

### Group J — Coexistence with MessageBanner (FR-9)

---

#### TC-OVL-032 — Overlay renders above MessageBanner banners (z-order)
**Type:** design-review + integration
**Traceability:** FR-9 (AC-9.1)

**Invariant:**
When both a banner and the overlay are active, the overlay's rendering layer (e.g.
`Order::Foreground` or `Order::Tooltip`) must be higher than the banner's layer (banner is
inside the central panel at default order). Verify by checking the `egui::Area::order()` used
for the overlay's dim plane; it must be above the order used for banner rendering.

**Integration check (when available):**
In a full-app harness, add a banner and raise the overlay simultaneously. Assert that the
overlay's dim fills the expected region and the banner's label is not directly reachable
(obscured — `query_by_label` may still find the label in the widget tree even if dimmed;
the critical check is that the banner is behind the input-blocking layer).

---

#### TC-OVL-033 — Banners persist in ctx.data while overlay is active; reappear on dismiss
**Type:** ctx.data
**Traceability:** FR-9 (AC-9.2)

**Preconditions:** Two banners set via `MessageBanner::set_global`.

**Steps:**
1. Set banners: `"Banner A"` (Error) and `"Banner B"` (Warning).
2. Raise the overlay.
3. Assert `MessageBanner::has_global(ctx)` is still `true` (banners survive in `ctx.data`).
4. Dismiss the overlay via `handle.clear()`; run one frame.
5. Call `MessageBanner::show_global(ui)`.

**Expected outcome:**
- After dismiss, both `"Banner A"` and `"Banner B"` labels are present in the rendered tree.
- Banner state was not cleared or corrupted by the overlay lifecycle.

---

#### TC-OVL-034 — Success task result: overlay dismissed before success banner shown
**Type:** integration
**Traceability:** FR-9 (AC-9.3), J-1 flow

**Preconditions:** AppState dispatches a task; overlay is raised at dispatch time.

**Steps:**
1. Dispatch a task; raise overlay.
2. Simulate `TaskResult::Success(result)` arriving on the receiver.
3. App loop frame:
   a. Receives result.
   b. Calls `handle.clear()` (overlay lowered).
   c. Calls `MessageBanner::set_global(ctx, "Your identity has been registered.", MessageType::Success)`.
4. Render the frame.

**Expected outcome:**
- `ProgressOverlay::has_global(ctx)` is `false`.
- `MessageBanner::has_global(ctx)` is `true` with the success text.
- No frame saw both active simultaneously (single-frame hand-off).

---

#### TC-OVL-035 — Failed task result: overlay dismissed before error banner shown
**Type:** integration
**Traceability:** FR-3 (AC-3.3), FR-9 (AC-9.3)

**Preconditions:** AppState dispatches a task; overlay is raised at dispatch time.

**Steps:**
1. Dispatch a task; raise overlay.
2. Simulate `TaskResult::Error(err)` arriving on the receiver.
3. App loop frame:
   a. Receives error.
   b. Calls `handle.clear()`.
   c. Calls `MessageBanner::set_global(ctx, "Registration failed. Try again.", MessageType::Error)`.
4. Render the frame.

**Expected outcome:**
- `ProgressOverlay::has_global(ctx)` is `false`.
- `MessageBanner::has_global(ctx)` is `true` with an error-type banner.
- The error message is user-friendly (no SDK internals, no jargon — spot-check the display text
  per the error message convention in `CLAUDE.md`).

---

### Group K — Concurrent Operations (FR-10) — **[depends on 1d]**

> All TCs in this group depend on the architecture decision (stack vs. replace vs. reject)
> deferred to Nagatha in Phase 1d. The TCs below are written for the **stack model** recommended
> in AC-10.1. If Nagatha selects a different model, these TCs must be revised.

---

#### TC-OVL-036 — Topmost stack entry's content is rendered [depends on 1d]
**Type:** kittest
**Traceability:** FR-10 (AC-10.1, AC-10.2)

**Preconditions:** Two overlay requests pushed.

**Steps:**
1. Call `set_global(ctx, "Operation A.", cfg_a)` — capture `handle_a`.
2. Call `set_global(ctx, "Operation B.", cfg_b)` — capture `handle_b`.
3. Run one frame.

**Expected outcome (stack model):**
- `ProgressOverlay::has_global(ctx)` is `true`.
- Label `"Operation B."` is present (topmost entry rendered).
- Label `"Operation A."` is absent (bottom entry not rendered).
- Both `handle_a.is_active()` and `handle_b.is_active()` return `true` (both on stack).

---

#### TC-OVL-037 — Handle dismisses only its own stack entry [depends on 1d]
**Type:** ctx.data
**Traceability:** FR-10 (AC-10.3)

**Preconditions:** Two overlay requests on the stack (A below, B on top).

**Steps:**
1. Push handle_a, then handle_b.
2. Call `handle_b.clear()` (dismiss topmost).
3. Assert state.

**Expected outcome (stack model):**
- `handle_b.is_active()` is `false`.
- `handle_a.is_active()` is `true`.
- `ProgressOverlay::has_global(ctx)` is `true` (A still on stack; overlay persists).
- On next render frame, `"Operation A."` label is present.

---

#### TC-OVL-038 — Overlay clears only when the entire stack is empty [depends on 1d]
**Type:** ctx.data
**Traceability:** FR-10 (AC-10.3)

**Preconditions:** Two entries on the stack.

**Steps:**
1. Push handle_a and handle_b.
2. Call `handle_b.clear()` — assert `has_global` is still `true`.
3. Call `handle_a.clear()` — assert `has_global` is now `false`.

**Expected outcome:** Overlay does not lower until the last entry is dismissed.

---

#### TC-OVL-039 — Only topmost request's actions are reachable [depends on 1d]
**Type:** kittest
**Traceability:** FR-10 (AC-10.5)

**Preconditions:** handle_a has a button with id `"cancel_a"`; handle_b (on top) has a button
with id `"cancel_b"` (both labelled `"Cancel"`, a caller-chosen label).

**Steps:**
1. Push handle_a then handle_b.
2. Run one frame.
3. Click the button.
4. Call `take_actions(ctx)`.

**Expected outcome:**
- `take_actions` returns `["cancel_b"]` (topmost entry's action).
- `"cancel_a"` is not in the queue (lower entry's action unreachable while B is on top).

---

#### TC-OVL-040 — Concurrent overlay event logged exactly once [depends on 1d]
**Type:** design-review
**Traceability:** FR-10 (AC-10.4)

**Invariant:**
When a second `set_global` call is made while an overlay is already active, a single log
entry noting the concurrent request is emitted. Across subsequent frames where both remain on
the stack, no further log entry is emitted for the concurrency (log-once, guarded by flag —
mirrors `BannerState.logged`).

Verify by code inspection: the `logged_concurrent` (or equivalent) flag in overlay state is
set on the first duplicate and checked before any `tracing::warn!` call on subsequent frames.

---

### Group L — Accessibility (NFR-3)

---

#### TC-OVL-041 — Focus trap: Tab does not cycle to widgets beneath the overlay
**Type:** kittest
**Traceability:** NFR-3 (AC-3a)

**Preconditions:** Overlay active with one generic button. A text input widget exists beneath.

**Steps:**
1. Show overlay with a button.
2. Run one frame; focus starts on the button.
3. Press Tab.
4. Run one frame.
5. Check focused widget.

**Expected outcome:**
- Focus remains on the button (or returns to it if there is only one focusable element
  in the overlay — Tab wraps within the overlay, not out of it).
- The `TextEdit` beneath does not receive focus.

**Implementation note:** egui's default Tab behavior cycles through all focusable widgets
globally. The overlay must intercept Tab events when active (e.g. via consuming
`ctx.input_mut().events` or `ui.response().has_focus()` scoping). Design-review: verify Tab
events are not forwarded to `pass_events_to_game_while_any_popup_is_open = false` equivalent.

---

#### TC-OVL-042 — Esc is swallowed even when a button is present (reframed post-outage: generic button)
**Type:** kittest
**Traceability:** NFR-3 (AC-3b)

**Preconditions:** Overlay raised with a generic button (`with_action("Cancel", "overlay.cancel")`).
There is no built-in Cancel, so Esc has nothing to trigger.

**Steps:**
1. Show overlay with a button.
2. Run one frame.
3. Press Escape (`harness.key_press(egui::Key::Escape)`).
4. Run another frame.
5. Call `take_actions(ctx)`.

**Expected outcome:**
- `take_actions` returns empty — Esc enqueues no action.
- `has_global(ctx)` is `true` — Esc never dismisses a hard block.
- The Esc key event is consumed by the overlay (not forwarded further).

---

#### TC-OVL-043 — Esc is swallowed when the overlay has no button (reframed post-outage: generic button)
**Type:** kittest
**Traceability:** NFR-3 (AC-3b)

**Preconditions:** Overlay raised with no buttons (pure block).

**Steps:**
1. Show overlay with no buttons.
2. Run one frame.
3. Press Escape.
4. Run another frame.
5. Assert overlay still active; call `take_actions`.

**Expected outcome:**
- `ProgressOverlay::has_global(ctx)` is `true` (Esc did NOT dismiss the overlay).
- `take_actions` returns empty.
- No action was dispatched.

---

#### TC-OVL-044 — Enter does not activate a focused button (reframed post-outage: generic button)
**Type:** kittest
**Traceability:** NFR-3 (AC-3c)

**Preconditions:** Overlay active with a single generic button that holds focus.

**Steps:**
1. Show overlay with a button; run one frame; ensure the button is focused.
2. Press Enter.
3. Run one frame.
4. Call `take_actions(ctx)`.

**Expected outcome:**
- `take_actions` returns empty (Enter did NOT enqueue the focused button's action).
- The overlay is still active.

**Rationale:** A hard block swallows Tab/Enter/Esc/Space, so a focused button can never be
activated by keyboard — Enter/Space must not trigger it. This is the guard that the general rule
stays intact; the single opt-in exception is covered by TC-OVL-051/052/053. See AC-3c.

---

#### TC-OVL-051 — Opt-in keyboard escape activates on Enter
**Type:** kittest
**Traceability:** NFR-3 (AC-3c)

**Preconditions:** Overlay active with a secondary action also designated via
`with_keyboard_escape(action_id)`; settle frames so the escape button holds focus.

**Steps:**
1. Show the overlay with the designated escape; run frames until the escape button is focused.
2. Press Enter; run one frame.
3. Call `take_actions`.

**Expected outcome:**
- `take_actions` returns the escape's action id (Enter activated the focus-pinned escape).
- The overlay is still active (activation enqueues the id; the owner lowers the block).

**Rationale:** An unbounded block that opts into a keyboard escape must be activatable by Enter so
a keyboard-only / assistive-tech user is not stranded. See AC-3c.

---

#### TC-OVL-052 — Opt-in keyboard escape activates on Space
**Type:** kittest
**Traceability:** NFR-3 (AC-3c)

**Preconditions:** As TC-OVL-051.

**Steps:** As TC-OVL-051, pressing **Space** instead of Enter.

**Expected outcome:** `take_actions` returns the escape's action id (egui fires a fake primary
click on Space OR Enter for the focused widget).

---

#### TC-OVL-053 — Opt-in keyboard escape is focus-pinned (no leak beneath)
**Type:** kittest
**Traceability:** NFR-3 (AC-3a, AC-3c)

**Preconditions:** A `TextEdit` rendered beneath; overlay active with a designated keyboard escape;
settle frames so the escape holds focus.

**Steps:**
1. Assert the escape button is focused (not the field beneath).
2. Press Tab; assert focus stays on the escape.
3. Click over the field beneath; assert focus stays on the escape (the click is absorbed by the
   sink and the per-frame focus pin restores it).
4. Press Enter; assert `take_actions` returns the escape id AND the field beneath is still empty.

**Expected outcome:** Neither Tab nor a click can move focus off the escape, and Enter/Space reach
only the escape — never a widget beneath. The opt-in carves out Enter/Space for the escape alone.

**Rationale:** The Enter/Space passthrough is safe only because focus is guaranteed pinned to the
escape. See AC-3a, AC-3c.

---

### Group M — Non-Functional

---

#### TC-OVL-045 — Log-once: state change logs once, not once per frame
**Type:** design-review + ctx.data
**Traceability:** NFR-5

**Invariant:**
1. On `set_global`: exactly one log entry at `debug` or `info` level noting overlay raised.
   On subsequent frames with no state change, no further log entry for the overlay.
2. On `set_description` / `set_step` (content change): exactly one log entry.
3. On `handle.clear()`: exactly one log entry noting dismissal.

**Verify by code inspection:** a `logged` (or `last_logged_description: Option<String>`) flag
in the overlay state struct is set after the first log; subsequent render frames check the flag
before calling `tracing::debug!`. Pattern mirrors `BannerState.logged` in
`src/ui/components/message_banner.rs`.

**Test (ctx.data check):**
1. Show overlay.
2. Capture the `logged` field value from `ctx.data`.
3. Assert it is `true` after the first render.
4. Simulate a second render pass without state change; assert `logged` is still `true` and
   no new log entry was emitted.

---

#### TC-OVL-046 — Theme switch mid-overlay re-evaluates all colors
**Type:** kittest
**Traceability:** NFR-4

**Preconditions:** Overlay active; harness configured for dark theme.

**Steps:**
1. Show overlay in dark theme.
2. Run one frame; note the dim plane color is `modal_overlay()` = `rgba(0,0,0,120)`.
3. Switch the harness to light theme (call `ctx.set_visuals(egui::Visuals::light())`).
4. Run one frame.

**Expected outcome:**
- The overlay continues to render without panic or stale palette.
- Colors for the card background, text, and dim plane are re-evaluated from `DashColors` and
  `modal_overlay()` tokens each frame (design-review: grep for `Color32::from_rgb` or any
  hardcoded color literal in `progress_overlay.rs` — must find zero).

---

#### TC-OVL-047 — Stuck-overlay safety valve after inactivity threshold
**Type:** design-review (partially unspecified — flag for Nagatha)
**Traceability:** R-4

**What the spec defines:**
After an unspecified threshold with no `TaskResult` arriving, the overlay SHOULD:
1. Make the elapsed readout visible (if previously hidden).
2. Optionally offer an escape hatch ("This is taking longer than usual") for operations that
   are safe to abandon.

**What is NOT yet defined (flag for Nagatha / 1d):**
- The threshold duration (spec says "after a threshold" without a value).
- Which operation types get the escape hatch and which do not.
- Whether the escape hatch is a third button or replaces Cancel.
- Whether the elapsed readout activation is automatic or still requires an explicit config flag.

**Partial test (assertable once threshold is defined):**
1. Show overlay with no elapsed readout; simulate time advancing past the threshold.
2. Run one frame.
3. Assert a label matching `"Elapsed: {seconds}s"` is present.
4. If the escape hatch is defined for this operation type, assert the escape hatch button is present.

**⚠ This TC is incomplete until Nagatha defines the threshold and escape-hatch policy. Mark as
BLOCKED pending 1d.**

---

#### TC-OVL-048 — Secret-prompt modal renders above the overlay (z-order R-1)
**Type:** integration + design-review
**Traceability:** R-1

**Preconditions:** Overlay is active; a secret-prompt (`render_secret_prompt`) is triggered
mid-operation.

**Steps (design-review):**
1. In `src/app.rs::update()`, verify the call order:
   - `ProgressOverlay::render_global(ctx)` is called.
   - `render_secret_prompt(ctx)` is called **after** `render_global` (later in the same frame).
2. Because egui draws layers in call order (later `Area` calls render on top), the secret
   prompt's `Area` with `Order::Foreground` (or `Order::Tooltip`) renders above the overlay.

**Integration check (when available):**
In a full-app harness with both overlay and secret prompt active, assert that the passphrase
input widget is present and interactive (receives keyboard focus), while the overlay's dim is
present but not blocking the prompt.

**Expected outcome:**
- Secret-prompt modal is interactable when overlay is active.
- The overlay does not intercept input intended for the secret prompt.

---

#### TC-OVL-049 — NFR-1 frame-loop non-blocking invariant
**Type:** design-review
**Traceability:** NFR-1

**Invariant:**
The entire call path of `ProgressOverlay::render_global(ctx)` and
`ProgressOverlay::set_global(ctx, ...)` must be synchronous and non-blocking:
- No `.await`, no `async fn`, no `tokio::block_on`, no `std::thread::sleep` in the render or
  show path.
- No `Mutex::lock().await` or `RwLock::write().await`.
- All `ctx.data` access uses egui's synchronous `TypeMap` locking (safe — same as `MessageBanner`).

**Verify:** `cargo clippy` with `#[deny(clippy::async_yields_async)]` and a manual grep for
`block_on`, `sleep`, `.await` in `src/ui/components/progress_overlay.rs` and any module it
calls synchronously. Reference incident: PR860 (deadlock from async blocking in egui frame
loop).

---

## 3. Requirement Coverage Matrix

| Requirement | Covered by |
|---|---|
| FR-1 (show overlay) | TC-OVL-002, TC-OVL-003, TC-OVL-004 |
| FR-2 (update in place) | TC-OVL-005, TC-OVL-006, TC-OVL-007 |
| FR-3 (dismiss) | TC-OVL-008, TC-OVL-009, TC-OVL-010, TC-OVL-035 |
| FR-4 (spinner, no ETA) | TC-OVL-011, TC-OVL-012, TC-OVL-013 |
| FR-5 (step counter) | TC-OVL-014, TC-OVL-015, TC-OVL-016, TC-OVL-017, TC-OVL-018, TC-OVL-019 |
| FR-6 (description text) | TC-OVL-020, TC-OVL-021, TC-OVL-022 |
| FR-7 (buttons & actions) | TC-OVL-023, TC-OVL-024, TC-OVL-025, TC-OVL-026, TC-OVL-027 |
| FR-8 (input blocking) | TC-OVL-028, TC-OVL-029, TC-OVL-030, TC-OVL-031 |
| FR-9 (coexistence with banner) | TC-OVL-032, TC-OVL-033, TC-OVL-034, TC-OVL-035 |
| FR-10 (concurrent operations) | TC-OVL-036, TC-OVL-037, TC-OVL-038, TC-OVL-039, TC-OVL-040 |
| NFR-1 (no frame blocking) | TC-OVL-004, TC-OVL-049 |
| NFR-2 (i18n-ready strings) | TC-OVL-014 (counter format), TC-OVL-020 (description), TC-OVL-013 (elapsed) |
| NFR-3 (accessibility) | TC-OVL-041, TC-OVL-042, TC-OVL-043, TC-OVL-044, TC-OVL-051, TC-OVL-052, TC-OVL-053 |
| NFR-4 (theme) | TC-OVL-046 |
| NFR-5 (log-once) | TC-OVL-045, TC-OVL-040 |
| NFR-6 (cheap idle) | TC-OVL-001 |
| R-1 (z-order vs secret-prompt) | TC-OVL-048 |
| R-2 (concurrent model) | TC-OVL-036 to TC-OVL-040 [depends on 1d] |
| R-3 (button honesty; reframed post-outage) | TC-OVL-023 (no button = pure block), TC-OVL-024 (a generic button enqueues only its caller-chosen id; no implicit dismiss) |
| R-4 (stuck overlay safety valve) | TC-OVL-047 [partial — BLOCKED pending 1d] |
| R-7 (kittest coverage checklist) | TC-OVL-028 (input blocked), TC-OVL-042/043 (Esc), TC-OVL-015–017 (counter validation), TC-OVL-024–026 (action id FIFO), TC-OVL-045 (log-once), TC-OVL-034/035 (dismiss+banner hand-off) |

---

## 4. Open Items for Nagatha (Phase 1d)

The following items require architecture decisions before the marked TCs can be finalized or
implemented:

| Item | Blocks | Required decision |
|---|---|---|
| **Concurrent overlay model** (stack vs. replace vs. reject) | TC-OVL-036 to TC-OVL-040 | Confirm stack model (AC-10.1) or choose an alternative. If not stack, TC-OVL-036–040 must be rewritten to match the chosen semantics. |
| **Stuck-overlay threshold** | TC-OVL-047 | Define the threshold duration after which the elapsed readout auto-activates and the escape hatch appears. Define which operations get an escape hatch. |
| **Cancellation semantics** (R-3; reframed post-outage) | TC-OVL-024, TC-OVL-042 | Cancel is no longer a built-in concept — a screen wires its own generic button to cancellation. TC-OVL-024/042 verify the UI-only action queue; the end-to-end cancel path stays untestable until the BackendTask system gains cooperative cancellation (T7). |
| **Escape hatch button design** | TC-OVL-047 | Is the escape hatch a third button, a replacement for Cancel, or an entirely different mechanism? |

---

## 5. Notes on Test Executability

### Runnable as kittest (CI-safe)
TC-OVL-001, TC-OVL-002, TC-OVL-003, TC-OVL-005, TC-OVL-006, TC-OVL-007, TC-OVL-008,
TC-OVL-009, TC-OVL-011, TC-OVL-012, TC-OVL-013, TC-OVL-014, TC-OVL-015, TC-OVL-016,
TC-OVL-017, TC-OVL-018, TC-OVL-019, TC-OVL-020, TC-OVL-021, TC-OVL-022, TC-OVL-023,
TC-OVL-024, TC-OVL-025, TC-OVL-026, TC-OVL-027, TC-OVL-028, TC-OVL-029, TC-OVL-030,
TC-OVL-033, TC-OVL-036, TC-OVL-037, TC-OVL-038, TC-OVL-039, TC-OVL-041, TC-OVL-042,
TC-OVL-043, TC-OVL-044, TC-OVL-046.

### Require integration harness (AppState-level)
TC-OVL-010, TC-OVL-034, TC-OVL-035, TC-OVL-048.

### Design-review only (not automatable as unit/kittest)
TC-OVL-004 (no frame blocking), TC-OVL-014 (i18n string format — fragment check),
TC-OVL-020 (single-label assertion), TC-OVL-027 (no bare `ui.button()`),
TC-OVL-029 (no `set_enabled`), TC-OVL-031 (render seam placement), TC-OVL-032 (z-order),
TC-OVL-040 (log-once concurrent), TC-OVL-045 (log-once general), TC-OVL-046 (no hardcoded
colors), TC-OVL-047 (partial), TC-OVL-048 (render order), TC-OVL-049 (no async in render
path).

---

*Brain the size of a planet, and here I am specifying acceptance criteria for a spinner.
At least the spinner is honest about not knowing how long things take. More than can be said
for most documentation.*
