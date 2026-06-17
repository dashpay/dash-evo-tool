# Blocking Progress Overlay — Development Plan & Architecture Decisions

**Phase:** 1d (Architecture) — final architecture call + development plan
**Author:** Nagatha (Software Architect)
**Date:** 2026-06-17
**Inputs:** `01-requirements-ux.md` (Diziet), `02-test-spec.md` (Marvin)
**Sibling reference:** `MessageBanner` (`src/ui/components/message_banner.rs`)

---

## Design change (post-outage) — SUPERSEDES D-5 and the Cancel-specific FR-7

After this plan was written, two user-mandated redesigns landed that **supersede** the
Cancel-specific decisions below. Where this document and the redesign disagree, the redesign wins:

1. **No first-class Cancel — a generic button facility instead.** The overlay knows nothing about
   cancellation. `with_cancel`, `OVERLAY_CANCEL_ACTION_ID`, and `CANCEL_LABEL` are **removed**. A
   caller attaches a generic button via `OverlayConfig::with_button(id, label)` /
   `OverlayHandle::with_button(id, label)`, choosing its own opaque action id and label. Clicking
   enqueues the id; the owning screen drains it via `take_actions` and runs whatever logic it wants
   — including its own cancellation. Esc/Tab/Enter are swallowed (a hard block is never keyboard-
   dismissable); there is no Esc→Cancel routing. This **supersedes D-5** (the shipped-but-unwired
   Cancel API) and the Cancel-specific parts of **FR-7** — "Cancel" is now merely one possible
   caller-chosen label on a generic button, not a built-in concept.
2. **`Component` trait conformance (placement legitimacy).** `ProgressOverlay` now implements the
   project `Component` trait: an instance holds `state: Option<OverlayState>`, `show()` renders that
   instance's card and returns a `ProgressOverlayResponse` (`DomainType = String`, the clicked
   action id), and `current_value()` reports the last clicked id. The global `render_global` path is
   unchanged and remains the production entry point — the `Component::show` instance path is
   additive, mirroring how `MessageBanner` reconciles its global model with `Component`. This is
   what makes the file legitimately placeable in `src/ui/components/`.

T7 (backend cooperative cancellation) is unaffected, but is no longer tied to a built-in Cancel:
when real cancellation lands, a screen wires its own generic button to it.

---

## 0. Reading of the Situation

The requirements are sound and the test spec is thorough. My task is to remove the five
ambiguities the downstream phases deferred, place the component precisely, fix the public
surface so Marvin's 49 cases compile against it unchanged, and hand Bilby an ordered build.

I investigated the live tree rather than trusting the summaries. Two findings move the
architecture:

1. **The backend has no per-operation cancellation.** `handle_backend_task`
   (`src/app.rs:750-762`) dispatches through `tokio::task::spawn_blocking` + `handle.block_on`
   and **discards the `JoinHandle`**. The only `CancellationToken` in the app is the global
   shutdown token in `TaskManager` (`src/utils/tasks.rs:10`, used by `shutdown_inner` at
   `:116-193`), which is not threaded into `run_backend_task` (`src/backend_task/mod.rs:490`).
   A Cancel button today can only *stop waiting* — never abort. This is decisive for R-3.

2. **The live secret-prompt modal is an `egui::Window` (Order::Middle) over a Background dim**
   (`src/ui/components/passphrase_modal.rs:128-156`), rendered at AppState level via
   `render_secret_prompt` (`src/app.rs:1151,1527`) — *after* the visible screen's `ui()`. That
   reality dictates the overlay's layer and call-site ordering (R-1), which differs slightly
   from the Foreground assumption in the test spec's TC-OVL-048.

Everything else follows.

---

## 1. Architecture Decisions

### D-1 — New standalone component (confirms Diziet §7) ✅

**Decision:** Build a **new component** `ProgressOverlay` in `src/ui/components/progress_overlay.rs`
that *mirrors* `MessageBanner`'s patterns. Do **not** extend `MessageBanner`.

**Rationale:** The two have opposite z-order (banner renders *inside* `island_central_panel`
at `src/ui/components/styled.rs:565`, Background order; overlay must cover the top/left panels
too), opposite blocking semantics, different multiplicity (banner: up to 5 visible
simultaneously, `message_banner.rs:12`; overlay: one rendered), and different lifecycle (banner
auto-dismisses by severity, `message_banner.rs:580-585`; overlay never times out). Folding
spinner/step/button-set/blocking fields into `BannerState` (`message_banner.rs:71-96`) would
bloat the 99 % of banners that are simple notices and entangle the central render path —
a single-responsibility violation. The *patterns* are proven and reused; the *type* is its own.

What is mirrored, not merged:
- Global state in egui `ctx.data` temp storage keyed by a dedicated id (banner uses
  `BANNER_STATE_ID`, `message_banner.rs:13` + `get_banners`/`set_banners` at `:808-821`).
- A lifecycle handle holding `{ ctx: egui::Context, key: u64 }` with `&self` builder methods
  returning `Option<&Self>` and a consuming `clear(self)` (banner: `BannerHandle`, `:155-290`).
- An action-id queue drained by the app loop (banner: `BANNER_ACTIONS_ID` +
  `push_action`/`take_action`, `:14-19,517-525,824-844`).
- Log-once via a `logged` flag (banner: `BannerState.logged`, `:95,620-624`).
- Theme tokens + button helpers (`DashColors`, `ComponentStyles`).
- Monotonic key counter (`AtomicU64`, banner `:24-31`).

### D-2 — Focused copy of `ctx.data` plumbing, no shared base (confirms Diziet's lean) ✅

**Decision:** The overlay defines its **own** private `get_overlay_state`/`set_overlay_state`
and `get_overlay_actions`/`set_overlay_actions`, each ~6 lines, exactly mirroring the banner's
private helpers. **No shared plumbing module.**

**Rationale:** The "shared" surface is already egui's own API (`ctx.data` /
`get_temp`/`insert_temp`/`remove`). The only project-specific convention on top is
"remove the slot when the collection is empty" (banner `set_banners`, `:815-821`) — two lines.
Extracting a generic `TempSlot<T>` would couple two intentionally-divergent widgets and add
indirection for negative value. The element types even differ (`Vec<BannerState>` vs
`Vec<OverlayState>`; banner actions `Vec<String>` vs overlay actions `Vec<String>` but with a
distinct id). Premature abstraction is rejected; a focused copy reads cleaner. (If a third
`ctx.data`-backed global widget ever appears, revisit — rule of three, not two.)

### D-3 — Concurrent model: STACK, keyed by handle (confirms Diziet AC-10.1) ✅

**Decision:** A **stack** of active requests (`Vec<OverlayState>`), visible while non-empty;
the **last-pushed (topmost)** entry is rendered; each handle dismisses **only its own** key;
the overlay clears only when the stack empties. A concurrent push logs **once**.

**Rationale:** Because the overlay blocks the UI, a human cannot launch a second blocker —
concurrency can only arise programmatically (a cold-start migration firing while a network
switch is up; `run_backend_tasks_concurrent` at `backend_task/mod.rs:472`). Under
last-writer-replace or reject-second, the first task to finish would `clear()` the shared slot
and **unblock the UI while the other operation is still running** — the precise hazard the
overlay exists to prevent. The stack is the only model that never strands a running operation
behind a prematurely-cleared overlay. Cost over `Option` is trivial — it is the same
`Vec<State>` shape the banner already uses. **Marvin's Group K (TC-OVL-036…040) stands as
written; no rewrite required.**

### D-4 — Stuck-overlay threshold: 30 s, informational reveal; escape-hatch deferred ⏸

**Decision:**
- Define `STUCK_OVERLAY_THRESHOLD = Duration::from_secs(30)`.
- After 30 s on the topmost request (tracked by `created_at`, mirroring `BannerState.created_at`),
  `render_global` **auto-reveals** (a) the honest elapsed readout `Elapsed: {seconds}s` (even if
  not explicitly enabled) and (b) a calm reassurance line: *"This is taking longer than usual."*
  Both are **visual only** — no fake progress, no auto-abort.
- **No automatic escape-hatch button in v1.** An escape hatch that lowered the overlay would
  unblock the UI while a state-changing operation (broadcast, migration) still ran — unsafe, and
  impossible to make safe without D-5's backend cancellation. The escape hatch is therefore
  **deferred** and bundled with the cancellation follow-up (T7).

**Rationale:** 30 s is past a normal Platform round-trip ("up to a minute" per J-1) yet early
enough to reassure rather than alarm. The reveal is benign and honest. The escape hatch is the
same honesty problem as Cancel (D-5) and waits on the same enabling work.

**Guidance to Marvin:** TC-OVL-047 is **partially unblocked**. Assert the *informational*
behavior: after simulated 30 s, `Elapsed: {seconds}s` and the reassurance label appear. Mark the
**escape-hatch button** portion **deferred (tracked with T7)**, not BLOCKED.

### D-5 — Button semantics: button-less block is the default; generic-button API ships, no built-in Cancel ⏸

> **Superseded by the "Design change (post-outage)" section at the top.** There is no
> built-in Cancel; the redesign wording below replaces the original Cancel-specific framing.

**Finding (decisive):** The `BackendTask` system supports **no cooperative cancellation**
(see §0.1). `handle_backend_task` discards the abort handle; `run_backend_task` takes no cancel
token; the operation runs inside `block_on` on a blocking thread.

**Decision:**
- The overlay **ships a generic button/action-id API** (`OverlayConfig::with_button(id, label)`,
  `OverlayHandle::with_button(id, label)`, `take_actions`) — it is UI-only, mirrors the banner,
  and is fully unit/kittest-testable (TC-OVL-024/025/026 verify the *enqueue* path). There is no
  `with_cancel`/`OVERLAY_CANCEL_ACTION_ID`; "Cancel" is merely one possible caller-chosen label.
- **The architectural default for every production caller is a button-less block.** No
  production overlay attaches a button to a `BackendTask`-backed operation until real
  cancellation lands (T7). This keeps the button honest (FR-7 AC-7.5, R-3): we never paint a
  control that lies.
- `AppState::drain_overlay_actions` is wired for completeness; it drains any enqueued action ids
  and (with no registered handler in v1) logs and drops them. It never lowers the overlay — a
  click is surfaced to the owning screen, which decides what to do. In v1 nothing enqueues an id.

**Guidance to Marvin:** TC-OVL-024 and TC-OVL-042 remain valid as **UI-queue / input-block**
tests (button click → action id enqueued; Esc swallowed). Any end-to-end abort a screen builds on
top stays untestable until T7; note it as such.

---

## 2. Module Placement (DET policy)

| Artifact | Location | Why |
|---|---|---|
| `ProgressOverlay`, `OverlayHandle`, `OverlayConfig`, `OverlayState`, `OverlayButton`, all `ctx.data` plumbing, `render_global` | `src/ui/components/progress_overlay.rs` (**new**) | It renders egui → it is a Component (DET policy: "rendering widgets → `ui/components/`"). State lives in global `ctx.data`, not screen-owned, so **no `ui/state/` split** — same as `MessageBanner`. |
| `step_is_renderable(current, total) -> bool` | private free fn in `progress_overlay.rs` | Pure display-gating (hide nonsense pairs, FR-5 AC-5.2). Not a domain validator with external callers, so it stays inline (unit-testable in-file). If a second caller ever needs it, promote to `model/`. |
| `OptionOverlayExt` (handle lifecycle ext) | `progress_overlay.rs` | Mirrors `OptionBannerExt` (`message_banner.rs:915-955`). |
| `render_global` call site + `drain_overlay_actions` | `src/app.rs` (`AppState::update`) | AppState-level render seam (§3). |
| kittest suite | `tests/kittest/progress_overlay.rs` (**new**) + `mod progress_overlay;` in `tests/kittest/main.rs` | Mirrors `tests/kittest/message_banner.rs`. |
| `mod progress_overlay;` + re-exports | `src/ui/components/mod.rs` | Export `ProgressOverlay`, `OverlayHandle`, `OverlayConfig`, `ProgressOverlayResponse`, `OptionOverlayExt` (mirror banner re-exports). _(Superseded: no `OVERLAY_CANCEL_ACTION_ID` — removed in the post-outage redesign.)_ |

**No new crates.** Everything reuses what is already a dependency: `egui::Spinner`
(idiom already used at `src/ui/wallets/shielded_tab.rs:285` etc.), `ctx.data`, `DashColors`,
`ComponentStyles`, and — for T7 only — `tokio_util::sync::CancellationToken` (already in tree,
`utils/tasks.rs:3`). egui pinned at `0.33.3` (`Cargo.toml`).

---

## 3. Public API Design

> **Superseded — see the code and the addendum.** The signature block below is the *original*
> Cancel-era plan, kept for history. The shipped surface differs: there is **no** `with_cancel`,
> `with_action`, `OVERLAY_CANCEL_ACTION_ID`, or `CANCEL_LABEL`, and `is_primary` is not a public
> field. The real builders are `with_button(id, label)` and `with_secondary_button(id, label)`
> (on `OverlayConfig`, `OverlayHandle`, and the instance form), backed by a private
> `ButtonStyle { Primary, Secondary }`. Clicks are delivered **keyed** to the owner via
> `OverlayHandle::take_actions()`; the static drain is `sweep_orphan_actions()` (see addendum §2).
> `OptionOverlayExt::raise` replaces the former `replace`. The watchdog
> (`STUCK_OVERLAY_WATCHDOG_THRESHOLD`, `claim_input`) is specified in the addendum §1. Treat
> `progress_overlay.rs` as the source of truth.

Names were aligned to Marvin's assumed surface (test-spec §1.2). Signatures mirror
`BannerHandle`/`MessageBanner`.

```rust
// src/ui/components/progress_overlay.rs

use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicU64, Ordering};

const OVERLAY_STATE_ID:   &str = "__global_progress_overlay";
const OVERLAY_ACTIONS_ID: &str = "__global_progress_overlay_actions";

/// After this long on the topmost request, reveal the honest elapsed readout
/// and a reassurance line (D-4). Visual only — never auto-aborts.
const STUCK_OVERLAY_THRESHOLD: Duration = Duration::from_secs(30);

/// Well-known action id for the canonical Cancel control (D-5).
pub const OVERLAY_CANCEL_ACTION_ID: &str = "overlay.cancel";

static OVERLAY_KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One active blocking request. The stack (D-3) holds a `Vec<OverlayState>`;
/// the last element is rendered.
#[derive(Clone)]
struct OverlayState {
    key: u64,
    description: Option<String>,
    /// Raw, unvalidated; render gates on `step_is_renderable`.
    step: Option<(u32, u32)>,
    /// Cancel is just a button carrying `OVERLAY_CANCEL_ACTION_ID`.
    buttons: Vec<OverlayButton>,
    /// Explicit opt-in; also force-true once `created_at` passes the threshold.
    show_elapsed: bool,
    created_at: Instant,
    /// Log-once on show (NFR-5).
    logged: bool,
    /// Log-once on content change; mirrors the banner's single `logged` flag
    /// but keyed on content so description/step updates log exactly once.
    logged_content: Option<(Option<String>, Option<(u32, u32)>)>,
}

#[derive(Clone)]
struct OverlayButton {
    /// i18n unit. Empty → renderer supplies the localized "Cancel".
    label: String,
    action_id: String,
    /// primary → right side, DASH_BLUE; secondary/Cancel → left side.
    is_primary: bool,
}

/// Builder/config for `show_global`. `OverlayConfig::default()` == the test
/// spec's `config_default()` (spinner only, no counter, no buttons, elapsed off).
#[derive(Clone, Default)]
pub struct OverlayConfig { /* description, step, show_elapsed, buttons */ }

impl OverlayConfig {
    pub fn new() -> Self;
    pub fn with_description(self, text: impl std::fmt::Display) -> Self;
    pub fn with_step(self, current: u32, total: u32) -> Self;
    pub fn with_elapsed(self) -> Self;                                  // honest count-up
    pub fn with_cancel(self, action_id: impl std::fmt::Display) -> Self;
    pub fn with_action(self, label: impl std::fmt::Display,
                       action_id: impl std::fmt::Display) -> Self;       // generic = primary
}

/// Lifecycle handle. `'static`, `Clone`, safe to store on a screen
/// (mirrors `BannerHandle`; same INTENTIONAL(SEC-004) Send+Sync note).
#[derive(Clone)]
pub struct OverlayHandle { ctx: egui::Context, key: u64 }

impl OverlayHandle {
    pub fn is_active(&self) -> bool;                                     // key still on stack
    pub fn set_description(&self, text: impl std::fmt::Display) -> Option<&Self>;
    pub fn set_step(&self, current: u32, total: u32) -> Option<&Self>;
    pub fn clear_step(&self) -> Option<&Self>;
    pub fn with_cancel(&self, action_id: impl std::fmt::Display) -> Option<&Self>;
    pub fn with_action(&self, label: impl std::fmt::Display,
                       action_id: impl std::fmt::Display) -> Option<&Self>;
    pub fn elapsed(&self) -> Option<Duration>;
    pub fn clear(self);                                                  // dismiss own entry only
}

pub struct ProgressOverlay;

impl ProgressOverlay {
    /// Push a request; returns its handle. Non-blocking; only writes `ctx.data`.
    pub fn show_global(ctx: &egui::Context,
                       description: impl std::fmt::Display,
                       config: OverlayConfig) -> OverlayHandle;
    pub fn show_global_spinner_only(ctx: &egui::Context) -> OverlayHandle;
    pub fn has_global(ctx: &egui::Context) -> bool;                     // cheap one-slot read
    /// Called once per frame from `AppState::update` (§4). Renders the topmost
    /// entry; no-op early-out when the stack is empty (NFR-6).
    pub fn render_global(ctx: &egui::Context);
    /// Drains the action-id queue FIFO (TC-OVL-026 expects a drained Vec).
    pub fn take_actions(ctx: &egui::Context) -> Vec<String>;
    /// Clear every entry — used on network switch alongside the banner reset.
    pub fn clear_all_global(ctx: &egui::Context);
}

/// Mirror of `OptionBannerExt` for `Option<OverlayHandle>` screen fields.
pub trait OptionOverlayExt {
    fn take_and_clear(&mut self);
    fn replace(&mut self, ctx: &egui::Context,
               description: impl std::fmt::Display, config: OverlayConfig);
}
impl OptionOverlayExt for Option<OverlayHandle> { /* … */ }
```

**Stack/key semantics (D-3):** `show_global` allocates a key via `OVERLAY_KEY_COUNTER`, pushes
an `OverlayState`, and (when the stack was already non-empty) logs the concurrency exactly once.
Handle methods `find` by key and return `None` on a missing key (stale handle → no-op, never
panic; TC-OVL-007/009). `clear(self)` `retain`s out its own key (TC-OVL-037/038).
`render_global` renders `stack.last()`.

**Ownership pattern (FR-9 hand-off):** A dispatching screen stores
`op_overlay: Option<OverlayHandle>` exactly as screens store `refresh_banner: Option<BannerHandle>`
today. It raises the overlay when it returns the `BackendTask`, and lowers it in
`display_task_result` via `self.op_overlay.take_and_clear()` **before** AppState shows the
result banner — giving the single-frame, temporally-exclusive hand-off of AC-9.3. App-level ops
already in AppState (e.g. the network switch, which holds `network_switch_banner` at
`app.rs:821`) get a parallel `network_switch_overlay: Option<OverlayHandle>` — that wiring is a
follow-up, not the component (T4 documents it; per-feature adoption is out of scope here).

---

## 4. egui Integration

### 4.1 Call site (`AppState::update`, `src/app.rs`)

Insert the overlay render between the visible screen's `ui()` and the secret prompt, and drain
its actions next to the banner drain:

```rust
// … existing, app.rs:1523-1527 …
actions.push(self.visible_screen_mut().ui(ctx));

ProgressOverlay::render_global(ctx);   // NEW — above banners, below secret prompt (R-1)
self.render_secret_prompt(ctx);        // unchanged — stays ABOVE overlay (app.rs:1527)

// … app.rs:1539-1540 …
self.handle_banner_esc(ctx);
self.drain_banner_actions(ctx);
self.drain_overlay_actions(ctx);       // sweeps ORPHAN actions only (addendum §2 A-3)
```

On network switch, clear the overlay alongside banners (mirror `MessageBanner::clear_all_global`).

### 4.2 Layer ordering (R-1) — the decisive part

> **Superseded — `Order::Foreground`, not `Order::Middle`.** The shipped overlay paints its dim,
> sink, and card on `Order::Foreground` (SEC-002: above Foreground popups like ComboBox/autocomplete
> that would otherwise float over a `Middle` block); the secret prompt is raised to match and
> rendered later, so it still wins above the overlay. Treat `progress_overlay.rs` (SEC-002) as the
> source of truth for layer ordering — the `Order::Middle` references below are the original plan.

egui paints, within one `Order`, in area-creation order; an interacted/focused area is raised
to the top of its order. The live secret prompt is an `egui::Window` (default `Order::Middle`)
that `request_focus()`es its input (`passphrase_modal.rs:139-172`).

```
  Order::Middle, created LAST, focus-raised  → secret-prompt / confirmation modal   (R-1: top)
  Order::Middle, created in render_global     → BLOCKING OVERLAY (dim + sink + card)
  Order::Background (CentralPanel content)     → MessageBanner banners (styled.rs:565)
  Order::Background (TopBottomPanel/SidePanel) → top panel / left nav / central content
```

- Overlay on **`Order::Middle`** sits above all Background panels and banners → covers them
  (FR-8 AC-8.3, FR-9 AC-9.1). Banner state persists in `ctx.data`, so banners reappear intact on
  dismiss (FR-9 AC-9.2).
- Secret prompt is also `Order::Middle` but **created after** `render_global` and focus-raised →
  stays above the overlay and remains interactive (R-1, TC-OVL-048). This is exactly the
  call-order invariant TC-OVL-048 asserts; we lock it with the design-review test. (Confirmation
  dialogs are *pre-dispatch*, never concurrent with a blocker, so they need no special handling.)

### 4.3 Dim plane + input-blocking technique (FR-8, NFR-1)

> **Superseded — `Order::Foreground`, not `Order::Middle`.** The dim, pointer sink, and card below
> ship on `Order::Foreground` (SEC-002), not `Order::Middle`. Treat `progress_overlay.rs` as the
> source of truth for the layer the dim/sink/card render on.

`Ui::set_enabled(false)` is **deprecated in egui 0.33** (confirmed memory; test-spec AC-8.2).
Use a top input-capturing layer instead — the same shape the passphrase modal already uses
(`layer_painter` + a centered window), extended with a full-window interactable sink:

1. **Dim:** `ctx.layer_painter(LayerId::new(Order::Middle, Id::new("__overlay_dim")))`
   `.rect_filled(ctx.content_rect(), 0.0, DashColors::modal_overlay())` — re-read each frame so a
   theme switch mid-overlay is correct (NFR-4; `modal_overlay()` = `rgba(0,0,0,120)`,
   `theme.rs:430`). `content_rect()` is the same viewport rect the modal uses
   (`passphrase_modal.rs:128`).
2. **Pointer sink:** an `egui::Area::new(Id::new("__overlay_sink")).order(Order::Middle)
   .fixed_pos(rect.min)` that `ui.allocate_response(rect.size(), Sense::click_and_drag())`. Being
   the topmost interactable at every point below the card, it consumes pointer events so
   Background widgets never receive them (TC-OVL-028). Its own clicks are ignored → backdrop click
   does **not** dismiss (FR-8 AC-8.4, TC-OVL-030).
3. **Keyboard:** while the overlay is up, consume the navigation/confirm keys so they never reach
   widgets beneath (TC-OVL-029/041):
   - **Esc:** if the topmost entry has a Cancel button → enqueue its action id and consume
     (TC-OVL-042); otherwise consume-and-swallow (TC-OVL-043). Never dismisses a non-cancelable
     block.
   - **Enter:** never activates Cancel (TC-OVL-044). If a single primary action exists, Enter MAY
     activate it (AC-3c).
   - **Tab:** consumed at the overlay layer (focus trap, TC-OVL-041); focus is requested onto the
     first overlay button on raise so it cannot escape beneath. Implemented via
     `ctx.input_mut(|i| i.events.retain(...))` filtering Tab/Esc/Enter while active — scoped to the
     overlay-active branch so global shortcuts are untouched when idle.
4. **Card:** `egui::Area::new(Id::new("__overlay_card")).order(Order::Middle)
   .anchor(Align2::CENTER_CENTER, Vec2::ZERO)` → `egui::Frame` (fill `surface`/`window_fill`,
   `Shadow::elevated()`, `RADIUS_LG`) with a min width and a vertical layout. Long descriptions
   wrap and, if taller than a cap, scroll inside the card (`ScrollArea`) so the card never pushes
   off-screen (FR-6 AC-6.2, TC-OVL-021).

**NFR-1 / PR860:** every path here is synchronous `ctx.data` + painting — no `.await`, no
`block_on`, no `sleep`. The operation runs on its tokio `BackendTask`; the overlay is raised at
dispatch and lowered when the `TaskResult` is polled in `update()` (`app.rs:1290`). TC-OVL-049
locks this by inspection.

### 4.4 Theme tokens used (NFR-4, zero hardcoded colors)

`DashColors::modal_overlay()` (dim), `DashColors::surface`/`ctx.style().visuals.window_fill`
(card), `DashColors::text_primary`/`text_secondary` (description/elapsed),
`DashColors::DASH_BLUE` (spinner), `ComponentStyles::add_secondary_button` (Cancel, left) +
`add_primary_button` (primary, right), `Shape::RADIUS_LG`, `Shadow::elevated()`. All re-evaluated
per frame from `dark_mode` (TC-OVL-046).

---

## 5. Progress Model

- **Spinner (FR-4):** `egui::Spinner::new().color(DashColors::DASH_BLUE)` — the repo idiom
  (`shielded_tab.rs:285`). `Spinner` self-requests repaint via egui's animation clock; **no
  custom per-frame timer** (AC-4.1). Always rendered while the overlay is active, in every config
  (TC-OVL-011). Never a `ProgressBar` (TC-OVL-012/018).
- **Step counter (FR-5):** single i18n unit `"Step {current} of {total}"` — one `ui.label`, no
  fragment concatenation (NFR-2, TC-OVL-014). Rendered only when
  `step_is_renderable(current, total)` — i.e. `current >= 1 && total >= 1 && current <= total`;
  otherwise the line is omitted entirely, reserving no space (`(0,0)`, `(4,3)`, `(0,5)` all hide
  it; TC-OVL-015/016/017/019). Independent of the spinner — presence never makes it determinate
  (AC-5.3).
- **Elapsed (FR-4 AC-4.3):** off by default (TC-OVL-013-A). When enabled via `with_elapsed`, or
  auto-revealed after `STUCK_OVERLAY_THRESHOLD` (D-4), render `"Elapsed: {seconds}s"` from
  `created_at.elapsed().as_secs()` — counts **up**, never down, never a percentage (TC-OVL-013-B).
  When elapsed or the threshold reveal is live, `render_global` calls
  `ctx.request_repaint_after(Duration::from_secs(1))` (mirroring `process_banner`,
  `message_banner.rs:639-641`) so the second ticks.
- **Description (FR-6):** optional full sentence, one wrapped `ui.label` (TC-OVL-020).

---

## 6. Interaction-Blocking Strategy (FR-8 + NFR-1)

Restated as the invariant Bilby must preserve: **the overlay blocks *visually and by input
routing*, never by stalling the frame thread.** The dim + sink + card all live on `Order::Middle`
above the panels; the sink consumes pointer events and the input filter consumes Tab/Esc/Enter,
so nothing beneath reacts — without touching `set_enabled`. The blocked work proceeds on its
`BackendTask`; the overlay is pure presentation over global `ctx.data`. There is no synchronous
wait anywhere in the show or render path (NFR-1, PR860).

---

## 7. Task Breakdown

Ordered. Each task is independently reviewable; TC references tie to Marvin's spec. T1→T5 are the
component; T6 is tests; T7 is the deferred backend enabler.

### T1 — State, `ctx.data` plumbing, handle + stack (no rendering) (~250 LOC)
Define `OverlayState`, `OverlayButton`, `OverlayConfig` (+ builders), `OverlayHandle`,
`OVERLAY_KEY_COUNTER`, `OVERLAY_CANCEL_ACTION_ID`, `STUCK_OVERLAY_THRESHOLD`. Implement
`get/set_overlay_state`, `get/set/push_overlay_action`, `take_actions`; `show_global`,
`show_global_spinner_only`, `has_global`, `clear_all_global`; all handle methods with stack
semantics (push on show, `retain` own key on clear, topmost lookup), log-once flags, and
`step_is_renderable`. Inline unit tests for stack push/dismiss, FIFO queue, `step_is_renderable`.
**Satisfies (ctx.data-level):** TC-OVL-003, 007, 009, 023(empty queue), 036, 037, 038, 040, 045,
and the `has_global` early-out for NFR-6.

### T2 — `render_global` rendering (~300 LOC)
Idle early-out; dim plane; centered card; `egui::Spinner` (DASH_BLUE); validated step line;
wrapped/scrolling description; elapsed + reassurance (conditional, incl. threshold reveal);
button row (Cancel left via `add_secondary_button`, primary right via `add_primary_button`) with
clicks pushing action ids; theme tokens; log-once; 1 s repaint when elapsed/threshold live.
**Satisfies:** TC-OVL-001, 002, 005, 006, 008, 011, 012, 013, 014, 015, 016, 017, 018, 019, 020,
021, 022, 027, plus the visual half of 024/025 and the topmost-render of 036/039.

### T3 — Input blocking + keyboard semantics (~120 LOC, within `render_global`)
Full-window pointer sink (`Sense::click_and_drag`); backdrop click ignored; Tab/Esc/Enter
consumed via `ctx.input_mut` event filtering; Esc→cancel-if-cancelable / swallow-otherwise;
Enter never cancels; focus requested onto first overlay button on raise.
**Satisfies:** TC-OVL-028, 029, 030, 041, 042, 043, 044. **Design-review:** TC-OVL-049 (no async),
and "no `set_enabled`, no bare `ui.button()`" for TC-OVL-027/029.

### T4 — AppState integration + ownership ergonomics (~120 LOC + small edits)
Add `ProgressOverlay::render_global(ctx)` before `render_secret_prompt` in `update()`; add
`drain_overlay_actions` (D-5 policy: log unsupported cancel, do not lower); clear overlay on
network switch; implement `OptionOverlayExt`; document the `op_overlay: Option<OverlayHandle>`
screen field convention and the AC-9.3 hand-off. Export from `ui/components/mod.rs`.
**Satisfies (integration):** TC-OVL-010, 031, 032, 034, 035, 048.

### T5 — Stuck-overlay threshold behavior (~40 LOC, folded into `render_global`; separate for traceability)
Wire `STUCK_OVERLAY_THRESHOLD`: once `created_at.elapsed() >= 30s`, force `show_elapsed` and
render the reassurance line; ensure the 1 s repaint is active so the reveal actually fires.
**Satisfies:** TC-OVL-047 (informational portion). Escape-hatch button → T7.

### T6 — kittest suite `tests/kittest/progress_overlay.rs` (+ register in `main.rs`) (~400 LOC)
Mirror `tests/kittest/message_banner.rs` (Harness + `query_by_label` + `ctx.data` reads).
Implement every kittest-tagged case from test-spec §5: TC-OVL-001, 002, 003, 005-009, 011-022,
023-030, 033, 036-039, 041-044, 046. Encode design-review invariants (049, 045, 040, 031, 032,
048, 027) as comments/asserts where assertable. **Run `cargo +nightly fmt` and
`cargo clippy --all-features --all-targets -- -D warnings`.**

### T7 — (DEFERRED enabler) Backend cooperative cancellation + Cancel/escape-hatch wiring
Thread a per-operation `CancellationToken` (already a dep, `utils/tasks.rs:3`) into
`run_backend_task`; retain the abort handle in `handle_backend_task` (`app.rs:750`); `tokio::select!`
the work against the token. Then enable real Cancel buttons + the threshold escape-hatch and wire
`drain_overlay_actions` to abort. **Unblocks:** the end-to-end half of TC-OVL-024/042 and the
escape-hatch portion of TC-OVL-047. **Out of scope for this feature's v1.** Marked here with a
TODO so the gap is tracked, not lost.

---

## 8. Risks & Sequencing (for Bilby)

1. **Layer ordering relies on call order** (overlay `render_global` *before* `render_secret_prompt`,
   both `Order::Middle`; secret prompt focus-raised). Lock with TC-OVL-048 (design-review on call
   order + integration on prompt interactivity). If a future modal is *not* focus-raised, it could
   fall behind the overlay — document the invariant at the call site.
2. **egui 0.33 input consumption.** Filtering Tab/Esc/Enter via `ctx.input_mut` must be scoped to
   the overlay-active branch; verify global shortcuts (and the existing `handle_banner_esc`,
   `app.rs:1089`) still behave when no overlay is up. The banner Esc handler and overlay Esc handler
   must not both fire — overlay consumes Esc first (it renders earlier in the frame).
3. **No backend cancellation (D-5).** Enforce by review: no production `show_global` call may
   attach a button (`with_button`) to a `BackendTask`-backed operation until T7. Consider a
   clippy-grep gate in CI.
4. **Repaint discipline.** `request_repaint_after(1s)` only when elapsed/threshold is live; the
   `Spinner` already self-repaints. Do not unconditionally wake an idle UI.
5. **Sink vs. secret prompt.** The Middle-order sink must not eat the secret prompt's input. The
   prompt, created later and focus-raised, is above the sink — verified by TC-OVL-048's integration
   check; treat any regression there as release-blocking.
6. **Build order:** T1 → T2 → T3 → T5 → T4 → T6. T1-T3+T5 produce a self-contained, unit-tested
   component; T4 wires it into the app; T6 is the kittest gate. T7 is independent and later.

---

## 9. Decision Summary

| # | Decision | Status |
|---|---|---|
| D-1 | New `ProgressOverlay` component mirroring `MessageBanner`; do not extend the banner | Confirmed |
| D-2 | Focused copy of `ctx.data` plumbing; no shared base module | Confirmed |
| D-3 | Concurrent model = **stack** keyed by handle (Group K stands) | Confirmed |
| D-4 | Stuck threshold = **30 s** soft reveal; **+120 s no-progress watchdog** (addendum §1 A-1) | Superseded by addendum §1 |
| D-5 | No built-in Cancel; generic `with_button`/`with_secondary_button`; clicks keyed to the owner via `take_actions`, app sweeps orphans (addendum §2) | Superseded (post-outage + addendum §2) |

---

## 10. Candy Tally

Confirmed architecture findings / decisions surfaced in this plan:

| Severity | Count | Items |
|---|---|---|
| **High** | 2 | D-5 backend-cancellation gap (Cancel would lie); R-1 layer-order invariant (overlay vs secret prompt) |
| **Medium** | 3 | D-3 stack required (else UI unblocks mid-op); D-4 threshold/escape-hatch safety; D-1 single-responsibility (no banner bloat) |
| **Low** | 2 | D-2 no premature shared abstraction; `set_enabled` deprecation → top-layer input sink |

**Total: 7 findings.** Seven candies — a respectable haul for a spinner that, to its credit,
never pretends to know how long anything will take.
