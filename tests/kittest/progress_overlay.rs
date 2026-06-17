//! Kittest coverage for the blocking progress overlay (`ProgressOverlay`).
//!
//! Mirrors the style of `tests/kittest/message_banner.rs`: a `Harness` plus
//! `query_by_label` / `query_by_role` / `ctx.data` reads. The overlay always
//! renders an animated `egui::Spinner`, which self-requests an immediate repaint
//! every frame — so these tests drive frames with `harness.step()` (one frame
//! per queued event) instead of `harness.run()` (which would spin to the step
//! cap). `show_global` is called once via `harness.ctx`, not inside the
//! per-frame closure, so the stack is not re-pushed each frame.
//!
//! Test ids map to `docs/ai-design/2026-06-17-blocking-progress-overlay/02-test-spec.md`.
//!
//! Design-review-only invariants are asserted where possible and otherwise noted:
//! - TC-OVL-004 / TC-OVL-049 (no async/blocking in show or render): the public
//!   API is entirely synchronous `ctx.data` + painting — verified by inspection
//!   and by the fact these synchronous tests compile and run.
//! - TC-OVL-031 (render seam): `ProgressOverlay::render_global` is called from
//!   `AppState::update` after panels, not inside `island_central_panel`.
//! - TC-OVL-032 (z-order above banners): the overlay paints on `Order::Middle`;
//!   banners paint on `Order::Background` inside the central panel.
//! - TC-OVL-040 / TC-OVL-045 (log-once): covered by the inline unit tests in
//!   `src/ui/components/progress_overlay.rs` (`render_logs_once_then_marks_logged`);
//!   the concurrent-request warning is emitted once in `show_global`, never per frame.
//! - TC-OVL-027 (no bare `ui.button()`) / TC-OVL-029 (no `set_enabled`): the
//!   renderer uses `ComponentStyles` button helpers and a top input-capturing
//!   layer, never `ui.button()` or the deprecated `Ui::set_enabled`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use dash_evo_tool::ui::MessageType;
use dash_evo_tool::ui::components::passphrase_modal::{PassphraseModalConfig, passphrase_modal};
use dash_evo_tool::ui::components::{
    Component, ComponentResponse, MessageBanner, OptionOverlayExt, OverlayConfig, OverlayHandle,
    ProgressOverlay,
};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const SPINNER_ROLE: egui::accesskit::Role = egui::accesskit::Role::ProgressIndicator;

/// Build a harness whose per-frame closure renders only the overlay.
fn overlay_harness() -> Harness<'static> {
    Harness::builder()
        .with_size(egui::vec2(420.0, 360.0))
        .build_ui(|ui| {
            ProgressOverlay::render_global(ui.ctx());
        })
}

// ── Group A — Idle Path ────────────────────────────────────────────────────

/// TC-OVL-001 — nothing renders, and `has_global` is false, when idle.
#[test]
fn tc_ovl_001_idle_renders_nothing() {
    let mut harness = overlay_harness();
    harness.step();
    assert!(!ProgressOverlay::has_global(&harness.ctx));
    assert!(harness.query_by_role(SPINNER_ROLE).is_none());
    assert!(harness.query_by_label("Cancel").is_none());
}

// ── Group B — Show Lifecycle ───────────────────────────────────────────────

/// TC-OVL-002 — overlay appears on the next frame after show.
#[test]
fn tc_ovl_002_overlay_appears_after_show() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Registering your identity.",
        OverlayConfig::default(),
    );
    harness.step();
    assert!(ProgressOverlay::has_global(&harness.ctx));
    assert!(
        harness
            .query_by_label("Registering your identity.")
            .is_some()
    );
    assert!(harness.query_by_role(SPINNER_ROLE).is_some());
}

/// TC-OVL-003 — show returns a usable handle (ctx.data level).
#[test]
fn tc_ovl_003_show_returns_usable_handle() {
    let ctx = egui::Context::default();
    let handle = ProgressOverlay::show_global(&ctx, "Loading.", OverlayConfig::default());
    assert!(handle.is_active());
    assert!(handle.set_description("Updated text.").is_some());
    assert!(ProgressOverlay::has_global(&ctx));
}

// ── Group C — Update In Place ──────────────────────────────────────────────

/// TC-OVL-005 — description update swaps text; spinner persists.
#[test]
fn tc_ovl_005_description_update_keeps_spinner() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Preparing the funding lock.",
        OverlayConfig::default(),
    );
    harness.step();
    assert!(
        harness
            .query_by_label("Preparing the funding lock.")
            .is_some()
    );
    assert!(harness.query_by_role(SPINNER_ROLE).is_some());

    handle.set_description("Waiting for the funding proof.");
    harness.step();
    assert!(
        harness
            .query_by_label("Waiting for the funding proof.")
            .is_some()
    );
    assert!(
        harness
            .query_by_label("Preparing the funding lock.")
            .is_none()
    );
    assert!(harness.query_by_role(SPINNER_ROLE).is_some());
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

/// TC-OVL-006 — counter update changes only the counter line.
#[test]
fn tc_ovl_006_counter_update_changes_only_counter() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Processing.",
        OverlayConfig::new().with_step(2, 5),
    );
    harness.step();
    assert!(harness.query_by_label("Step 2 of 5").is_some());

    handle.set_step(3, 5);
    harness.step();
    assert!(harness.query_by_label("Step 3 of 5").is_some());
    assert!(harness.query_by_label("Step 2 of 5").is_none());
    assert!(harness.query_by_label("Processing.").is_some());
    assert!(harness.query_by_role(SPINNER_ROLE).is_some());
}

/// TC-OVL-007 — stale handle updates are no-ops returning None (ctx.data).
#[test]
fn tc_ovl_007_stale_handle_updates_are_none() {
    let ctx = egui::Context::default();
    let handle = ProgressOverlay::show_global(&ctx, "Soon gone.", OverlayConfig::default());
    handle.clone().clear();
    assert!(handle.set_description("After clear").is_none());
    assert!(handle.set_step(1, 3).is_none());
    assert!(
        handle
            .with_button("overlay.bg", "Run in background")
            .is_none()
    );
    assert!(!ProgressOverlay::has_global(&ctx));
}

// ── Group D — Dismiss ──────────────────────────────────────────────────────

/// TC-OVL-008 — programmatic dismiss removes the overlay.
#[test]
fn tc_ovl_008_programmatic_dismiss_removes_overlay() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::show_global(&harness.ctx, "Working.", OverlayConfig::default());
    harness.step();
    assert!(ProgressOverlay::has_global(&harness.ctx));

    handle.clear();
    harness.step();
    assert!(!ProgressOverlay::has_global(&harness.ctx));
    assert!(harness.query_by_role(SPINNER_ROLE).is_none());
    assert!(harness.query_by_label("Working.").is_none());
}

/// TC-OVL-009 — double dismiss is a no-op (ctx.data).
#[test]
fn tc_ovl_009_double_dismiss_is_noop() {
    let ctx = egui::Context::default();
    let handle = ProgressOverlay::show_global(&ctx, "Once.", OverlayConfig::default());
    handle.clone().clear();
    handle.clear();
    assert!(!ProgressOverlay::has_global(&ctx));
}

/// TC-OVL-010 / TC-OVL-035 — failed task: overlay gone before the error banner.
/// Component-level simulation of the AppState hand-off (single-frame exclusivity).
#[test]
fn tc_ovl_010_dismiss_before_error_banner() {
    let ctx = egui::Context::default();
    let overlay = ProgressOverlay::show_global(&ctx, "Registering.", OverlayConfig::default());
    assert!(overlay.is_active());

    // Result arrives: lower the overlay, then show the banner — never both.
    overlay.clear();
    assert!(!ProgressOverlay::has_global(&ctx));
    MessageBanner::set_global(&ctx, "Registration failed. Try again.", MessageType::Error);

    assert!(!ProgressOverlay::has_global(&ctx));
    assert!(MessageBanner::has_global(&ctx));
}

// ── Group E — Spinner ──────────────────────────────────────────────────────

/// TC-OVL-011 — spinner is present in every configuration.
#[test]
fn tc_ovl_011_spinner_present_in_all_configs() {
    let configs = [
        OverlayConfig::default(),
        OverlayConfig::new().with_step(1, 3),
        OverlayConfig::new()
            .with_step(1, 3)
            .with_button("overlay.bg", "Run in background"),
    ];
    for config in configs {
        let mut harness = overlay_harness();
        let _handle = ProgressOverlay::show_global(&harness.ctx, "Busy.", config);
        harness.step();
        assert!(
            harness.query_by_role(SPINNER_ROLE).is_some(),
            "spinner must render in every configuration"
        );
    }
}

/// TC-OVL-012 / TC-OVL-018 — no percentage / ETA element; spinner stays
/// indeterminate even with a counter.
#[test]
fn tc_ovl_012_no_eta_or_percentage() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Building the shielded transaction.",
        OverlayConfig::new().with_step(2, 5),
    );
    harness.step();
    assert!(harness.query_by_label("Step 2 of 5").is_some());
    assert!(harness.query_by_role(SPINNER_ROLE).is_some());
    assert!(harness.query_by_label_contains("%").is_none());
    assert!(harness.query_by_label_contains("remaining").is_none());
    assert!(harness.query_by_label_contains("ETA").is_none());
}

/// TC-OVL-013 (Part A) — elapsed readout is off by default.
#[test]
fn tc_ovl_013a_elapsed_off_by_default() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(&harness.ctx, "Working.", OverlayConfig::default());
    harness.step();
    assert!(harness.query_by_label_contains("Elapsed:").is_none());
}

/// TC-OVL-013 (Part B) — when enabled the elapsed readout shows and counts up.
#[test]
fn tc_ovl_013b_elapsed_on_counts_up() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Slow operation.",
        OverlayConfig::new().with_elapsed(),
    );
    harness.step();
    assert!(harness.query_by_label("Elapsed: 0s").is_some());

    // Real wall-clock elapsed (Instant-based) — counts up, never down.
    std::thread::sleep(Duration::from_millis(1100));
    harness.step();
    assert!(
        harness.query_by_label("Elapsed: 0s").is_none(),
        "the readout advanced past 0s"
    );
    assert!(
        harness.query_by_label_contains("Elapsed:").is_some(),
        "the readout persists and never disappears or counts down"
    );
}

// ── Group F — Step Counter ─────────────────────────────────────────────────

/// TC-OVL-014 — a valid counter renders "Step {current} of {total}".
#[test]
fn tc_ovl_014_valid_counter_renders() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_step(3, 5),
    );
    harness.step();
    assert!(harness.query_by_label("Step 3 of 5").is_some());
}

/// TC-OVL-015 / TC-OVL-016 / TC-OVL-017 — invalid counters hide the line.
#[test]
fn tc_ovl_015_017_invalid_counter_hidden() {
    for (current, total) in [(0, 0), (4, 3), (0, 5)] {
        let mut harness = overlay_harness();
        let _handle = ProgressOverlay::show_global(
            &harness.ctx,
            "Working.",
            OverlayConfig::new().with_step(current, total),
        );
        harness.step();
        assert!(
            harness.query_by_label_contains("Step").is_none(),
            "counter ({current},{total}) must be hidden"
        );
        assert!(harness.query_by_role(SPINNER_ROLE).is_some());
    }
}

/// TC-OVL-019 — no counter line when none is set.
#[test]
fn tc_ovl_019_no_counter_when_unset() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Sending your transaction to the network.",
        OverlayConfig::default(),
    );
    harness.step();
    assert!(harness.query_by_label_contains("Step").is_none());
    assert!(
        harness
            .query_by_label("Sending your transaction to the network.")
            .is_some()
    );
    assert!(harness.query_by_role(SPINNER_ROLE).is_some());
}

// ── Group G — Description Text ─────────────────────────────────────────────

/// TC-OVL-020 — description renders as a single full sentence.
#[test]
fn tc_ovl_020_description_full_sentence() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Registering your identity on the network.",
        OverlayConfig::default(),
    );
    harness.step();
    assert!(
        harness
            .query_by_label("Registering your identity on the network.")
            .is_some()
    );
}

/// TC-OVL-021 — a long description wraps and stays within the window.
#[test]
fn tc_ovl_021_long_description_within_bounds() {
    let long = "Waiting for the funding proof. This operation contacts the Dash network and may take up to two minutes depending on network conditions.";
    let mut harness = Harness::builder()
        .with_size(egui::vec2(300.0, 400.0))
        .build_ui(|ui| {
            ProgressOverlay::render_global(ui.ctx());
        });
    let _handle = ProgressOverlay::show_global(&harness.ctx, long, OverlayConfig::default());
    harness.step();

    let node = harness.query_by_label(long);
    assert!(
        node.is_some(),
        "long description must render, not clip to empty"
    );
    let rect = node.unwrap().rect();
    assert!(
        rect.min.x >= -1.0 && rect.max.x <= 301.0,
        "description stays within the window horizontally: {rect:?}"
    );
}

/// TC-OVL-022 — spinner-only overlay is valid with no text, counter, or button.
#[test]
fn tc_ovl_022_spinner_only_valid() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global_spinner_only(&harness.ctx);
    harness.step();
    assert!(ProgressOverlay::has_global(&harness.ctx));
    assert!(harness.query_by_role(SPINNER_ROLE).is_some());
    assert!(harness.query_by_label_contains("Step").is_none());
    assert!(harness.query_by_label("Cancel").is_none());
}

// ── Group H — Buttons & Actions ────────────────────────────────────────────

/// TC-OVL-023 — no buttons: a pure block, dismissed programmatically only.
#[test]
fn tc_ovl_023_no_buttons_pure_block() {
    let mut harness = overlay_harness();
    let _handle =
        ProgressOverlay::show_global(&harness.ctx, "Hard block.", OverlayConfig::default());
    harness.step();
    assert!(harness.query_by_label("Cancel").is_none());
    assert!(ProgressOverlay::take_actions(&harness.ctx).is_empty());
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

/// TC-OVL-024 — clicking a generic button enqueues its caller-chosen action id;
/// the overlay persists. "Cancel" here is just a label the caller picked, not a
/// built-in concept — the facility is fully generic.
#[test]
fn tc_ovl_024_button_click_enqueues_action() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_button("overlay.cancel", "Cancel"),
    );
    // The centered card (anchored CENTER_CENTER) needs a few frames to cache its
    // size before it stops moving; settle before clicking so the click lands.
    harness.step();
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Cancel").is_some());

    harness.get_by_label("Cancel").click();
    harness.step();
    assert_eq!(
        ProgressOverlay::take_actions(&harness.ctx),
        vec!["overlay.cancel".to_string()]
    );
    // The click does not auto-dismiss — only the app loop lowers it.
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

/// TC-OVL-025 — a generic button click enqueues its action id.
#[test]
fn tc_ovl_025_generic_button_click_enqueues_action() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Background-able.",
        OverlayConfig::new().with_button("overlay.run_in_bg", "Run in background"),
    );
    harness.step();
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Run in background").is_some());

    harness.get_by_label("Run in background").click();
    harness.step();
    assert_eq!(
        ProgressOverlay::take_actions(&harness.ctx),
        vec!["overlay.run_in_bg".to_string()]
    );
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

/// TC-OVL-026 — the action queue drains FIFO then empties.
#[test]
fn tc_ovl_026_action_queue_drains_fifo() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Two buttons.",
        OverlayConfig::new()
            .with_button("cancel", "Cancel")
            .with_button("secondary", "Secondary"),
    );
    // Settle the centered card before clicking (anchored CENTER_CENTER moves for
    // a couple of frames until its size is cached).
    harness.step();
    harness.step();
    harness.step();

    harness.get_by_label("Cancel").click();
    harness.step();
    harness.get_by_label("Secondary").click();
    harness.step();

    assert_eq!(
        ProgressOverlay::take_actions(&harness.ctx),
        vec!["cancel".to_string(), "secondary".to_string()]
    );
    assert!(ProgressOverlay::take_actions(&harness.ctx).is_empty());
}

/// TC-OVL-027 — generic buttons render left-to-right in insertion order. The
/// renderer uses `ComponentStyles` button helpers, never a bare `ui.button()`
/// (design-review).
#[test]
fn tc_ovl_027_buttons_render_in_insertion_order() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Two buttons.",
        OverlayConfig::new()
            .with_button("first", "First action")
            .with_button("second", "Second action"),
    );
    harness.step();

    let first_x = harness.get_by_label("First action").rect().center().x;
    let second_x = harness.get_by_label("Second action").rect().center().x;
    assert!(
        first_x < second_x,
        "the first-added button must be left of the second"
    );
}

// ── Group I — Input Blocking ───────────────────────────────────────────────

/// TC-OVL-028 — pointer clicks on the backdrop do not reach widgets beneath.
#[test]
fn tc_ovl_028_pointer_click_beneath_blocked() {
    let counter = Rc::new(Cell::new(0u32));
    let counter_ui = Rc::clone(&counter);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(420.0, 360.0))
        .build_ui(move |ui| {
            if ui.button("Increment").clicked() {
                counter_ui.set(counter_ui.get() + 1);
            }
            ProgressOverlay::render_global(ui.ctx());
        });
    let _handle = ProgressOverlay::show_global_spinner_only(&harness.ctx);
    harness.step();

    harness.get_by_label("Increment").click();
    harness.step();
    assert_eq!(
        counter.get(),
        0,
        "widget beneath the overlay must not receive the click"
    );
    assert!(ProgressOverlay::take_actions(&harness.ctx).is_empty());
}

/// TC-OVL-029 — keyboard input does not reach widgets beneath the overlay.
/// The renderer never uses the deprecated `Ui::set_enabled` (design-review).
#[test]
fn tc_ovl_029_keyboard_beneath_blocked() {
    let text = Rc::new(RefCell::new(String::new()));
    let text_ui = Rc::clone(&text);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(420.0, 360.0))
        .build_ui(move |ui| {
            let mut buffer = text_ui.borrow_mut();
            ui.text_edit_singleline(&mut *buffer);
            ProgressOverlay::render_global(ui.ctx());
        });
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_button("overlay.cancel", "Cancel"),
    );
    harness.step();

    harness
        .input_mut()
        .events
        .push(egui::Event::Text("hello".to_string()));
    harness.step();
    assert!(
        text.borrow().is_empty(),
        "the text field beneath the overlay must not receive typed input"
    );
}

/// TC-OVL-030 — a backdrop click does NOT dismiss the overlay.
#[test]
fn tc_ovl_030_backdrop_click_does_not_dismiss() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global_spinner_only(&harness.ctx);
    harness.step();
    assert!(ProgressOverlay::has_global(&harness.ctx));

    let corner = egui::pos2(10.0, 10.0);
    harness.drag_at(corner);
    harness.drop_at(corner);
    harness.step();
    assert!(ProgressOverlay::has_global(&harness.ctx));
    assert!(ProgressOverlay::take_actions(&harness.ctx).is_empty());
}

// ── Group J — Coexistence with MessageBanner ───────────────────────────────

/// TC-OVL-032 / TC-OVL-033 — banners persist in ctx.data while the overlay is
/// up and survive its dismissal; both can be active at once (overlay on top).
#[test]
fn tc_ovl_032_033_banner_persists_under_overlay() {
    let ctx = egui::Context::default();
    MessageBanner::set_global(&ctx, "Banner A", MessageType::Error);
    MessageBanner::set_global(&ctx, "Banner B", MessageType::Warning);

    let overlay = ProgressOverlay::show_global(&ctx, "Blocking.", OverlayConfig::default());
    assert!(MessageBanner::has_global(&ctx));
    assert!(ProgressOverlay::has_global(&ctx));

    overlay.clear();
    assert!(
        MessageBanner::has_global(&ctx),
        "banner state survives the overlay lifecycle intact"
    );
}

/// TC-OVL-034 — success task: overlay dismissed before the success banner.
#[test]
fn tc_ovl_034_dismiss_before_success_banner() {
    let ctx = egui::Context::default();
    let overlay = ProgressOverlay::show_global(&ctx, "Registering.", OverlayConfig::default());

    overlay.clear();
    assert!(!ProgressOverlay::has_global(&ctx));
    MessageBanner::set_global(
        &ctx,
        "Your identity has been registered.",
        MessageType::Success,
    );

    assert!(!ProgressOverlay::has_global(&ctx));
    assert!(MessageBanner::has_global(&ctx));
}

// ── Group K — Concurrent Operations (stack model) ──────────────────────────

/// TC-OVL-036 — the topmost stack entry is the one rendered.
#[test]
fn tc_ovl_036_topmost_entry_rendered() {
    let mut harness = overlay_harness();
    let a = ProgressOverlay::show_global(&harness.ctx, "Operation A.", OverlayConfig::default());
    let b = ProgressOverlay::show_global(&harness.ctx, "Operation B.", OverlayConfig::default());
    harness.step();

    assert!(ProgressOverlay::has_global(&harness.ctx));
    assert!(harness.query_by_label("Operation B.").is_some());
    assert!(harness.query_by_label("Operation A.").is_none());
    assert!(a.is_active());
    assert!(b.is_active());
}

/// TC-OVL-037 / TC-OVL-038 — each handle dismisses only its own entry; the
/// overlay clears only when the stack empties (ctx.data).
#[test]
fn tc_ovl_037_038_handle_dismisses_only_its_own() {
    let ctx = egui::Context::default();
    let a = ProgressOverlay::show_global(&ctx, "Operation A.", OverlayConfig::default());
    let b = ProgressOverlay::show_global(&ctx, "Operation B.", OverlayConfig::default());

    b.clear();
    assert!(a.is_active());
    assert!(ProgressOverlay::has_global(&ctx));

    a.clear();
    assert!(!ProgressOverlay::has_global(&ctx));
}

/// TC-OVL-039 — only the topmost request's actions are reachable.
#[test]
fn tc_ovl_039_only_topmost_actions_reachable() {
    let mut harness = overlay_harness();
    let _a = ProgressOverlay::show_global(
        &harness.ctx,
        "Operation A.",
        OverlayConfig::new().with_button("cancel_a", "Cancel"),
    );
    let _b = ProgressOverlay::show_global(
        &harness.ctx,
        "Operation B.",
        OverlayConfig::new().with_button("cancel_b", "Cancel"),
    );
    // Settle the centered card before clicking (anchored CENTER_CENTER moves for
    // a couple of frames until its size is cached).
    harness.step();
    harness.step();
    harness.step();

    harness.get_by_label("Cancel").click();
    harness.step();
    assert_eq!(
        ProgressOverlay::take_actions(&harness.ctx),
        vec!["cancel_b".to_string()],
        "only the topmost request's button is reachable"
    );
}

// ── Group L — Accessibility ────────────────────────────────────────────────

/// TC-OVL-041 — Tab does not cycle focus to widgets beneath the overlay.
#[test]
fn tc_ovl_041_tab_focus_trap() {
    let text = Rc::new(RefCell::new(String::new()));
    let text_ui = Rc::clone(&text);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(420.0, 360.0))
        .build_ui(move |ui| {
            let mut buffer = text_ui.borrow_mut();
            ui.text_edit_singleline(&mut *buffer);
            ProgressOverlay::render_global(ui.ctx());
        });
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_button("overlay.cancel", "Cancel"),
    );
    harness.step();
    harness.step();
    assert!(
        harness.get_by_label("Cancel").is_focused(),
        "the first button is the first focus stop on raise"
    );

    harness.key_press(egui::Key::Tab);
    harness.step();
    assert!(
        harness.get_by_label("Cancel").is_focused(),
        "Tab is trapped: focus stays on the button, not a widget beneath"
    );
}

/// TC-OVL-042 — Esc is swallowed even when a button is present; it never
/// enqueues an action (no implicit dismiss). There is no built-in Cancel, so Esc
/// has nothing to trigger — the overlay stays up.
#[test]
fn tc_ovl_042_esc_swallowed_with_button() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_button("overlay.cancel", "Cancel"),
    );
    harness.step();

    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(
        ProgressOverlay::take_actions(&harness.ctx).is_empty(),
        "Esc must never trigger a button action"
    );
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

/// TC-OVL-043 — Esc is swallowed when the overlay has no button.
#[test]
fn tc_ovl_043_esc_swallowed_without_button() {
    let mut harness = overlay_harness();
    let _handle =
        ProgressOverlay::show_global(&harness.ctx, "Hard block.", OverlayConfig::default());
    harness.step();

    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(
        ProgressOverlay::has_global(&harness.ctx),
        "Esc must not dismiss a hard block"
    );
    assert!(ProgressOverlay::take_actions(&harness.ctx).is_empty());
}

/// TC-OVL-044 — Enter does not activate a focused button.
#[test]
fn tc_ovl_044_enter_does_not_activate_button() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_button("overlay.cancel", "Cancel"),
    );
    harness.step();
    harness.step();

    harness.key_press(egui::Key::Enter);
    harness.step();
    assert!(
        ProgressOverlay::take_actions(&harness.ctx).is_empty(),
        "Enter must never trigger the focused button's action"
    );
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

// ── Group M — Non-Functional ───────────────────────────────────────────────

/// TC-OVL-046 — switching theme mid-overlay re-renders without panic.
#[test]
fn tc_ovl_046_theme_switch_mid_overlay() {
    let mut harness = overlay_harness();
    harness.ctx.set_visuals(egui::Visuals::dark());
    let _handle = ProgressOverlay::show_global(
        &harness.ctx,
        "Switching networks.",
        OverlayConfig::default(),
    );
    harness.step();
    assert!(harness.query_by_label("Switching networks.").is_some());

    harness.ctx.set_visuals(egui::Visuals::light());
    harness.step();
    assert!(ProgressOverlay::has_global(&harness.ctx));
    assert!(harness.query_by_label("Switching networks.").is_some());
    assert!(harness.query_by_role(SPINNER_ROLE).is_some());
}

/// TC-OVL-048 — the secret-prompt modal renders above the overlay (R-1).
/// `render_global` is called before `render_secret_prompt` in `AppState::update`,
/// so the focus-raised prompt stays interactive above the overlay's dim/sink.
#[test]
fn tc_ovl_048_secret_prompt_renders_above_overlay() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(|ui| {
            ProgressOverlay::render_global(ui.ctx());
            let config = PassphraseModalConfig {
                window_title: "Unlock to continue",
                body: "Enter your passphrase to continue.",
                hint: None,
                error: None,
                submit_label: "Unlock",
                input_placeholder: "Enter passphrase",
            };
            passphrase_modal(ui.ctx(), &config, |_| {});
        });
    let _handle = ProgressOverlay::show_global(&harness.ctx, "Signing.", OverlayConfig::default());
    harness.step();

    assert!(ProgressOverlay::has_global(&harness.ctx));
    assert!(
        harness
            .query_by_label("Enter your passphrase to continue.")
            .is_some(),
        "the secret prompt renders above the overlay and remains visible"
    );
}

/// TC-OVL-047 (informational portion) — the stuck-threshold reveal is exercised
/// by the inline unit test `stuck_reveal_triggers_only_past_threshold` in
/// `src/ui/components/progress_overlay.rs`; the elapsed readout and reassurance
/// line are forced on once `created_at` passes 30s. The escape-hatch button is
/// deferred with backend cancellation (T7).
#[test]
fn tc_ovl_047_stuck_threshold_is_informational_only() {
    // Below the threshold a default overlay shows no elapsed readout and no
    // reassurance line — the reveal is purely time-driven and benign.
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::show_global(&harness.ctx, "Working.", OverlayConfig::default());
    harness.step();
    assert!(harness.query_by_label_contains("Elapsed:").is_none());
    assert!(
        harness
            .query_by_label_contains("This is taking longer than usual.")
            .is_none()
    );
}

/// TC-OVL-045 (ctx.data portion) — `OptionOverlayExt::replace` swaps the entry
/// and `take_and_clear` lowers it; the log-once flag itself is asserted in the
/// inline unit tests.
#[test]
fn tc_ovl_045_option_overlay_ext_lifecycle() {
    let ctx = egui::Context::default();
    let mut slot: Option<OverlayHandle> = None;
    slot.raise(&ctx, "First.", OverlayConfig::default());
    assert!(ProgressOverlay::has_global(&ctx));
    slot.raise(&ctx, "Second.", OverlayConfig::default());
    assert!(ProgressOverlay::has_global(&ctx));
    slot.take_and_clear();
    assert!(!ProgressOverlay::has_global(&ctx));
}

// ── Group N — Component (instance) path ─────────────────────────────────────

/// TC-OVL-050 — the `Component` instance path renders its card inline and
/// surfaces a clicked button's action id through `ProgressOverlayResponse`,
/// which `current_value` then reports. Mirrors `MessageBanner`'s instance path.
#[test]
fn tc_ovl_050_component_instance_show_reports_click() {
    let action = Rc::new(RefCell::new(None::<String>));
    let action_ui = Rc::clone(&action);
    let overlay = Rc::new(RefCell::new(
        ProgressOverlay::new()
            .with_description("Instance overlay.")
            .with_button("overlay.bg", "Run in background"),
    ));
    let overlay_ui = Rc::clone(&overlay);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(420.0, 360.0))
        .build_ui(move |ui| {
            let response = overlay_ui.borrow_mut().show(ui).inner;
            if response.has_changed()
                && let Some(id) = response.changed_value()
            {
                *action_ui.borrow_mut() = Some(id.clone());
            }
        });
    harness.step();
    assert!(harness.query_by_label("Instance overlay.").is_some());
    assert!(harness.query_by_role(SPINNER_ROLE).is_some());
    assert!(overlay.borrow().current_value().is_none());

    harness.get_by_label("Run in background").click();
    harness.step();
    assert_eq!(action.borrow().as_deref(), Some("overlay.bg"));
    assert_eq!(
        overlay.borrow().current_value().as_deref(),
        Some("overlay.bg"),
        "current_value reports the last clicked action id"
    );
    // The instance path does not touch the global action queue — the screen
    // reads the click from the response, not via take_actions.
    assert!(ProgressOverlay::take_actions(&harness.ctx).is_empty());
}

// ── QA probe (Marvin) — FR-8 AC-8.2 for the button-LESS hard block ──────────
//
// TC-OVL-029 only covers a *with-button* overlay, where the first button
// steals focus on raise — so typing is blocked incidentally, not by the
// overlay's input handling. This probe raises a *button-less* block over a
// field that already holds focus (the J-2 broadcast / J-4 migration case) and
// asserts AC-8.2: typed input must not reach the field beneath.
//
// QA-001 (HIGH), RESOLVED: `ProgressOverlay::claim_input`, called at frame start
// (before the panels) while a block is up, releases beneath text focus and
// strips `Event::Text` + nav/confirm keys — so a button-less block no longer
// leaks typed input into a focused field beneath. This harness mirrors the app
// loop: `claim_input` runs before the field, `render_global` paints after it.
#[test]
fn qa_buttonless_overlay_blocks_typing_into_focused_field_beneath() {
    let text = Rc::new(RefCell::new(String::new()));
    let text_ui = Rc::clone(&text);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(420.0, 360.0))
        .build_ui(move |ui| {
            // Mirrors AppState::update: claim input at frame start, before panels.
            ProgressOverlay::claim_input(ui.ctx());
            let mut buffer = text_ui.borrow_mut();
            ui.text_edit_singleline(&mut *buffer);
            ProgressOverlay::render_global(ui.ctx());
        });

    // Focus the field beneath, before any overlay exists.
    harness.step();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
    harness.step();

    // Raise a pure (button-less) block over the already-focused field.
    let _handle = ProgressOverlay::show_global_spinner_only(&harness.ctx);
    harness.step();

    // Type. AC-8.2: keyboard input must not reach widgets beneath the overlay.
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("hello".to_string()));
    harness.step();

    assert!(
        text.borrow().is_empty(),
        "FR-8 AC-8.2: typed input reached a focused field beneath a button-less \
         overlay: {:?}",
        text.borrow()
    );
}
