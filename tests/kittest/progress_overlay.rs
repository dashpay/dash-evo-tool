//! Kittest coverage for the blocking progress overlay (`ProgressOverlay`).
//!
//! Mirrors the style of `tests/kittest/message_banner.rs`: a `Harness` plus
//! `query_by_label` / `query_by_role` / `ctx.data` reads. The overlay always
//! renders an animated `egui::Spinner`, which self-requests an immediate repaint
//! every frame — so these tests drive frames with `harness.step()` (one frame
//! per queued event) instead of `harness.run()` (which would spin to the step
//! cap). `set_global` is called once via `harness.ctx`, not inside the
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
//! - TC-OVL-032 (z-order above banners): the overlay paints on `Order::Foreground`
//!   (SEC-002, above Foreground popups); banners paint on `Order::Background`
//!   inside the central panel.
//! - TC-OVL-040 / TC-OVL-045 (log-once): covered by the inline unit tests in
//!   `src/ui/components/progress_overlay.rs` (`render_logs_once_then_marks_logged`);
//!   the concurrent-request warning is emitted once in `set_global`, never per frame.
//! - TC-OVL-027 (no bare `ui.button()`) / TC-OVL-029 (no `set_enabled`): the
//!   renderer uses `ComponentStyles` button helpers and a top input-capturing
//!   layer, never `ui.button()` or the deprecated `Ui::set_enabled`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
#[cfg(feature = "testing")]
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

/// Build a harness whose per-frame closure mirrors `AppState::update`: claim
/// input at frame start (the sole keyboard/text block), then render the overlay.
fn overlay_harness() -> Harness<'static> {
    Harness::builder()
        .with_size(egui::vec2(420.0, 360.0))
        .build_ui(|ui| {
            ProgressOverlay::claim_input(ui.ctx());
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
    let _handle = ProgressOverlay::set_global(
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
    let handle = ProgressOverlay::set_global(&ctx, "Loading.", OverlayConfig::default());
    assert!(handle.is_active());
    assert!(handle.set_description("Updated text.").is_some());
    assert!(ProgressOverlay::has_global(&ctx));
}

// ── Group C — Update In Place ──────────────────────────────────────────────

/// TC-OVL-005 — description update swaps text; spinner persists.
#[test]
fn tc_ovl_005_description_update_keeps_spinner() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::set_global(
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
    let handle = ProgressOverlay::set_global(
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
    let handle = ProgressOverlay::set_global(&ctx, "Soon gone.", OverlayConfig::default());
    handle.clone().clear();
    assert!(handle.set_description("After clear").is_none());
    assert!(handle.set_step(1, 3).is_none());
    assert!(
        handle
            .with_action("Run in background", "overlay.bg")
            .is_none()
    );
    assert!(!ProgressOverlay::has_global(&ctx));
}

// ── Group D — Dismiss ──────────────────────────────────────────────────────

/// TC-OVL-008 — programmatic dismiss removes the overlay.
#[test]
fn tc_ovl_008_programmatic_dismiss_removes_overlay() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::set_global(&harness.ctx, "Working.", OverlayConfig::default());
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
    let handle = ProgressOverlay::set_global(&ctx, "Once.", OverlayConfig::default());
    handle.clone().clear();
    handle.clear();
    assert!(!ProgressOverlay::has_global(&ctx));
}

/// TC-OVL-010 / TC-OVL-035 — failed task: overlay gone before the error banner.
/// Component-level simulation of the AppState hand-off (single-frame exclusivity).
#[test]
fn tc_ovl_010_dismiss_before_error_banner() {
    let ctx = egui::Context::default();
    let overlay = ProgressOverlay::set_global(&ctx, "Registering.", OverlayConfig::default());
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
            .with_action("Run in background", "overlay.bg"),
    ];
    for config in configs {
        let mut harness = overlay_harness();
        let _handle = ProgressOverlay::set_global(&harness.ctx, "Busy.", config);
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
    let _handle = ProgressOverlay::set_global(
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
    let _handle = ProgressOverlay::set_global(&harness.ctx, "Working.", OverlayConfig::default());
    harness.step();
    assert!(harness.query_by_label_contains("Elapsed:").is_none());
}

/// TC-OVL-013 (Part B) — when enabled the elapsed readout shows and counts up.
/// Uses the deterministic clock seam (`backdate`) instead of a wall-clock sleep,
/// mirroring `tc_ovl_047b_threshold_reveals_via_clock_seam`. QA-008: assert the
/// readout advanced to a concrete 2s, not merely past 0s.
#[cfg(feature = "testing")]
#[test]
fn tc_ovl_013b_elapsed_on_counts_up() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::set_global(
        &harness.ctx,
        "Slow operation.",
        OverlayConfig::new().with_elapsed(),
    );
    harness.step();
    assert!(harness.query_by_label("Elapsed: 0s").is_some());

    // Shift this entry's clock 2s into the past via the test seam, then re-render:
    // the readout counts up to 2s deterministically, with zero wall-clock waiting.
    handle.backdate(Duration::from_secs(2));
    harness.step();
    assert!(
        harness.query_by_label("Elapsed: 2s").is_some(),
        "the readout advanced to 2s via the clock seam"
    );
    assert!(
        harness.query_by_label("Elapsed: 0s").is_none()
            && harness.query_by_label("Elapsed: 1s").is_none(),
        "the readout counts up, never down, and never stalls at 0s"
    );
}

// ── Group F — Step Counter ─────────────────────────────────────────────────

/// TC-OVL-014 — a valid counter renders "Step {current} of {total}".
#[test]
fn tc_ovl_014_valid_counter_renders() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::set_global(
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
        let _handle = ProgressOverlay::set_global(
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
    let _handle = ProgressOverlay::set_global(
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
    let _handle = ProgressOverlay::set_global(
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
    let _handle = ProgressOverlay::set_global(&harness.ctx, long, OverlayConfig::default());
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
    // QA-008: also bound it vertically inside the 400px-tall window.
    assert!(
        rect.min.y >= -1.0 && rect.max.y <= 401.0,
        "description stays within the window vertically: {rect:?}"
    );
}

/// TC-OVL-022 — spinner-only overlay is valid with no text, counter, or button.
#[test]
fn tc_ovl_022_spinner_only_valid() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::set_global_spinner_only(&harness.ctx);
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
    let handle = ProgressOverlay::set_global(&harness.ctx, "Hard block.", OverlayConfig::default());
    harness.step();
    assert!(harness.query_by_label("Cancel").is_none());
    assert!(handle.take_actions().is_empty());
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

/// TC-OVL-024 — clicking a generic button enqueues its caller-chosen action id;
/// the overlay persists. "Cancel" here is just a label the caller picked, not a
/// built-in concept — the facility is fully generic.
#[test]
fn tc_ovl_024_button_click_enqueues_action() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::set_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_action("Cancel", "overlay.cancel"),
    );
    // The centered card (anchored CENTER_CENTER) needs a few frames to cache its
    // size before it stops moving; settle before clicking so the click lands.
    harness.step();
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Cancel").is_some());

    harness.get_by_label("Cancel").click();
    harness.step();
    // A-3: the owning handle drains its own click.
    assert_eq!(handle.take_actions(), vec!["overlay.cancel".to_string()]);
    // The click does not auto-dismiss — only the app loop lowers it.
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

/// TC-OVL-025 — a generic button click enqueues its action id.
#[test]
fn tc_ovl_025_generic_button_click_enqueues_action() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::set_global(
        &harness.ctx,
        "Background-able.",
        OverlayConfig::new().with_action("Run in background", "overlay.run_in_bg"),
    );
    harness.step();
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Run in background").is_some());

    harness.get_by_label("Run in background").click();
    harness.step();
    assert_eq!(handle.take_actions(), vec!["overlay.run_in_bg".to_string()]);
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

/// TC-OVL-026 — the owning handle drains its own clicks FIFO then empties (A-3).
#[test]
fn tc_ovl_026_action_queue_drains_fifo() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::set_global(
        &harness.ctx,
        "Two buttons.",
        OverlayConfig::new()
            .with_action("Primary", "primary")
            .with_secondary_action("Secondary", "secondary"),
    );
    // Settle the centered card before clicking (anchored CENTER_CENTER moves for
    // a couple of frames until its size is cached).
    harness.step();
    harness.step();
    harness.step();

    harness.get_by_label("Primary").click();
    harness.step();
    harness.get_by_label("Secondary").click();
    harness.step();

    assert_eq!(
        handle.take_actions(),
        vec!["primary".to_string(), "secondary".to_string()]
    );
    assert!(handle.take_actions().is_empty());
}

/// TC-OVL-027 — F-3/F-4/F-7 layout: the primary action hugs the RIGHT edge and a
/// secondary button sits to its LEFT (mirrors `ConfirmationDialog`). The renderer
/// uses `ComponentStyles` button helpers, never a bare `ui.button()`.
#[test]
fn tc_ovl_027_secondary_left_primary_right() {
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::set_global(
        &harness.ctx,
        "Two buttons.",
        OverlayConfig::new()
            .with_action("Primary action", "primary")
            .with_secondary_action("Secondary action", "secondary"),
    );
    harness.step();

    let second_x = harness.get_by_label("Secondary action").rect().center().x;
    let first_x = harness.get_by_label("Primary action").rect().center().x;
    assert!(
        second_x < first_x,
        "the secondary button must sit to the left of the primary action"
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
    let handle = ProgressOverlay::set_global_spinner_only(&harness.ctx);
    harness.step();

    harness.get_by_label("Increment").click();
    harness.step();
    assert_eq!(
        counter.get(),
        0,
        "widget beneath the overlay must not receive the click"
    );
    assert!(handle.take_actions().is_empty());
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
            ProgressOverlay::claim_input(ui.ctx());
            let mut buffer = text_ui.borrow_mut();
            ui.text_edit_singleline(&mut *buffer);
            ProgressOverlay::render_global(ui.ctx());
        });
    let _handle = ProgressOverlay::set_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_action("Cancel", "overlay.cancel"),
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
    let handle = ProgressOverlay::set_global_spinner_only(&harness.ctx);
    harness.step();
    assert!(ProgressOverlay::has_global(&harness.ctx));

    let corner = egui::pos2(10.0, 10.0);
    harness.drag_at(corner);
    harness.drop_at(corner);
    harness.step();
    assert!(ProgressOverlay::has_global(&harness.ctx));
    assert!(handle.take_actions().is_empty());
}

// ── Group J — Coexistence with MessageBanner ───────────────────────────────

/// TC-OVL-032 / TC-OVL-033 — banners persist in ctx.data while the overlay is
/// up and survive its dismissal; both can be active at once (overlay on top).
#[test]
fn tc_ovl_032_033_banner_persists_under_overlay() {
    let ctx = egui::Context::default();
    MessageBanner::set_global(&ctx, "Banner A", MessageType::Error);
    MessageBanner::set_global(&ctx, "Banner B", MessageType::Warning);

    let overlay = ProgressOverlay::set_global(&ctx, "Blocking.", OverlayConfig::default());
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
    let overlay = ProgressOverlay::set_global(&ctx, "Registering.", OverlayConfig::default());

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
    let a = ProgressOverlay::set_global(&harness.ctx, "Operation A.", OverlayConfig::default());
    let b = ProgressOverlay::set_global(&harness.ctx, "Operation B.", OverlayConfig::default());
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
    let a = ProgressOverlay::set_global(&ctx, "Operation A.", OverlayConfig::default());
    let b = ProgressOverlay::set_global(&ctx, "Operation B.", OverlayConfig::default());

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
    let a = ProgressOverlay::set_global(
        &harness.ctx,
        "Operation A.",
        OverlayConfig::new().with_action("Cancel", "cancel_a"),
    );
    let b = ProgressOverlay::set_global(
        &harness.ctx,
        "Operation B.",
        OverlayConfig::new().with_action("Cancel", "cancel_b"),
    );
    // Settle the centered card before clicking (anchored CENTER_CENTER moves for
    // a couple of frames until its size is cached).
    harness.step();
    harness.step();
    harness.step();

    harness.get_by_label("Cancel").click();
    harness.step();
    // Only the topmost (B) renders, so the click is keyed to B: B drains it, A
    // never sees it (A-3 cross-owner isolation).
    assert_eq!(b.take_actions(), vec!["cancel_b".to_string()]);
    assert!(
        a.take_actions().is_empty(),
        "the lower request's handle drains nothing"
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
            ProgressOverlay::claim_input(ui.ctx());
            let mut buffer = text_ui.borrow_mut();
            ui.text_edit_singleline(&mut *buffer);
            ProgressOverlay::render_global(ui.ctx());
        });
    let _handle = ProgressOverlay::set_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_action("Cancel", "overlay.cancel"),
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
    let handle = ProgressOverlay::set_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_action("Cancel", "overlay.cancel"),
    );
    harness.step();

    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(
        handle.take_actions().is_empty(),
        "Esc must never trigger a button action"
    );
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

/// TC-OVL-043 — Esc is swallowed when the overlay has no button.
#[test]
fn tc_ovl_043_esc_swallowed_without_button() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::set_global(&harness.ctx, "Hard block.", OverlayConfig::default());
    harness.step();

    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(
        ProgressOverlay::has_global(&harness.ctx),
        "Esc must not dismiss a hard block"
    );
    assert!(handle.take_actions().is_empty());
}

/// TC-OVL-044 — neither Enter nor Space activates a focused button (QA-002): a
/// hard block is never keyboard-activatable.
#[test]
fn tc_ovl_044_enter_and_space_do_not_activate_button() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::set_global(
        &harness.ctx,
        "Working.",
        OverlayConfig::new().with_action("Cancel", "overlay.cancel"),
    );
    harness.step();
    harness.step();

    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.key_press(egui::Key::Space);
    harness.step();
    assert!(
        handle.take_actions().is_empty(),
        "neither Enter nor Space may trigger the focused button's action"
    );
    assert!(ProgressOverlay::has_global(&harness.ctx));
}

// ── Group M — Non-Functional ───────────────────────────────────────────────

/// TC-OVL-046 — switching theme mid-overlay re-renders without panic.
#[test]
fn tc_ovl_046_theme_switch_mid_overlay() {
    let mut harness = overlay_harness();
    harness.ctx.set_visuals(egui::Visuals::dark());
    let _handle = ProgressOverlay::set_global(
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
    let _handle = ProgressOverlay::set_global(&harness.ctx, "Signing.", OverlayConfig::default());
    harness.step();
    harness.step();

    assert!(ProgressOverlay::has_global(&harness.ctx));
    assert!(
        harness
            .query_by_label("Enter your passphrase to continue.")
            .is_some(),
        "the secret prompt renders above the overlay and remains visible"
    );
    // RQ-1: the prompt is INTERACTIVE above the overlay, not merely visible — its
    // submit control renders and its input holds keyboard focus, so the overlay's
    // Foreground dim/sink does not capture the prompt's interaction.
    assert!(
        harness.query_by_label("Unlock").is_some(),
        "the prompt's submit button renders interactively above the overlay"
    );
    assert!(
        harness.ctx.memory(|m| m.focused()).is_some(),
        "the prompt input holds keyboard focus above the overlay"
    );
}

/// TC-OVL-047 (informational portion) — below the threshold a default overlay
/// shows no elapsed readout and no reassurance line. The past-threshold reveals
/// (soft 30 s line, the 120 s watchdog line replacing it, and the Elapsed
/// force-reveal) are exercised by `tc_ovl_047b_threshold_reveals_via_clock_seam`
/// using the test clock seam; the threshold predicates by the inline
/// `stuck_reveal_*` / `watchdog_tripped_*` unit tests. Per addendum §1 there is NO
/// escape-hatch button by design — a button-less block stays total and the safety
/// valve is the bounded-op contract + honest escalation — so that portion of
/// TC-OVL-047 is closed as "won't build" for v1, not deferred to T7.
#[test]
fn tc_ovl_047_stuck_threshold_is_informational_only() {
    // Below the threshold a default overlay shows no elapsed readout and no
    // reassurance line — the reveal is purely time-driven and benign.
    let mut harness = overlay_harness();
    let _handle = ProgressOverlay::set_global(&harness.ctx, "Working.", OverlayConfig::default());
    harness.step();
    assert!(harness.query_by_label_contains("Elapsed:").is_none());
    assert!(
        harness
            .query_by_label_contains("This is taking longer than usual.")
            .is_none()
    );
}

/// TC-OVL-047b (RQ-2) — using the test clock seam, render PAST the thresholds and
/// assert the addendum's escalation: the soft 30 s line + Elapsed force-reveal,
/// then the 120 s watchdog line REPLACING the soft line (never stacked).
#[cfg(feature = "testing")]
#[test]
fn tc_ovl_047b_threshold_reveals_via_clock_seam() {
    let mut harness = overlay_harness();
    let handle = ProgressOverlay::set_global(&harness.ctx, "Working.", OverlayConfig::default());
    harness.step();
    assert!(harness.query_by_label_contains("Elapsed:").is_none());
    assert!(
        harness
            .query_by_label("This is taking longer than usual.")
            .is_none()
    );

    // Past 30 s (still making progress recently): soft line + Elapsed force-reveal,
    // no watchdog yet.
    handle.backdate(Duration::from_secs(31));
    harness.step();
    assert!(
        harness.query_by_label_contains("Elapsed:").is_some(),
        "Elapsed is force-revealed once past 30 s, even though with_elapsed was off"
    );
    assert!(
        harness
            .query_by_label("This is taking longer than usual.")
            .is_some(),
        "the soft reassurance line appears past 30 s"
    );
    assert!(
        harness
            .query_by_label_contains("much longer than expected")
            .is_none(),
        "the watchdog line has not tripped yet"
    );

    // Past 120 s with no progress: the watchdog line REPLACES the soft line.
    handle.backdate(Duration::from_secs(120));
    harness.step();
    assert!(
        harness
            .query_by_label_contains("much longer than expected")
            .is_some(),
        "the 120 s no-progress watchdog line appears"
    );
    assert!(
        harness
            .query_by_label("This is taking longer than usual.")
            .is_none(),
        "the watchdog line replaces the soft line, never stacks with it"
    );
    assert!(
        harness.query_by_label_contains("Elapsed:").is_some(),
        "the Elapsed readout persists through the watchdog escalation"
    );
}

/// TC-OVL-045 (ctx.data portion) — `OptionOverlayExt::raise` swaps the entry
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
            .with_action("Run in background", "overlay.bg"),
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
    // reads the click from the response, not via the handle/sweeper.
    assert!(ProgressOverlay::sweep_orphan_actions(&harness.ctx).is_empty());
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
    let _handle = ProgressOverlay::set_global_spinner_only(&harness.ctx);
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

// SEC-002 (additive hardening): `claim_input` also strips edit keys
// (Backspace/Delete/Home/End/PageUp/PageDown) and clipboard events
// (Copy/Cut/Paste) at frame start, so a focused field beneath a block is neither
// edited nor pasted into. This locks the new classes via event survival (fails
// if the strip is removed) plus the field-beneath behavioral contract.
#[test]
fn qa_buttonless_overlay_strips_edit_and_clipboard_events() {
    let text = Rc::new(RefCell::new(String::new()));
    let text_ui = Rc::clone(&text);
    let survived = Rc::new(Cell::new(false));
    let survived_ui = Rc::clone(&survived);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(420.0, 360.0))
        .build_ui(move |ui| {
            ProgressOverlay::claim_input(ui.ctx());
            let leaked = ui.ctx().input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Paste(_)
                            | egui::Event::Key {
                                key: egui::Key::Backspace | egui::Key::Delete,
                                pressed: true,
                                ..
                            }
                    )
                })
            });
            survived_ui.set(leaked);
            let mut buffer = text_ui.borrow_mut();
            ui.text_edit_singleline(&mut *buffer);
            ProgressOverlay::render_global(ui.ctx());
        });

    // Focus the field and type real content while idle (claim_input is a no-op
    // with no overlay), so egui holds a live cursor at the end of "keep".
    harness.step();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
    harness.step();
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("keep".to_string()));
    harness.step();
    assert_eq!(
        text.borrow().as_str(),
        "keep",
        "typing must reach the focused field while no overlay is up"
    );

    // Raise a button-less block over the focused, populated field.
    let _handle = ProgressOverlay::set_global_spinner_only(&harness.ctx);
    harness.step();

    // Inject edit + clipboard events; claim_input must strip them all.
    for key in [egui::Key::Backspace, egui::Key::Delete] {
        harness.input_mut().events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness
        .input_mut()
        .events
        .push(egui::Event::Paste("INJECT".to_string()));
    harness.step();

    assert!(
        !survived.get(),
        "claim_input must strip Backspace/Delete/Paste while a block is up"
    );
    assert_eq!(
        text.borrow().as_str(),
        "keep",
        "edit/clipboard events reached a field beneath a button-less overlay: {:?}",
        text.borrow()
    );
}

// ── Cross-finding reconciliations (lead brief) ──────────────────────────────

/// Reconciliation #1 (SEC-004 / Diziet F-1) — while a secret prompt is active the
/// app gates `claim_input` OFF, so only `render_global` runs over the overlay. It
/// must NOT strip keyboard events, or it would eat the prompt's Enter/Esc/Tab.
/// (TC-OVL-048 separately proves the prompt renders interactively above the
/// overlay; this proves the overlay does not swallow its keyboard.)
#[test]
fn reconciliation_render_global_keeps_keyboard_for_prompt() {
    let saw_keys = Rc::new(Cell::new(false));
    let saw_keys_ui = Rc::clone(&saw_keys);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(420.0, 360.0))
        .build_ui(move |ui| {
            // No claim_input — mirrors AppState gating it off while a prompt is up.
            ProgressOverlay::render_global(ui.ctx());
            let survived = ui.ctx().input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::Enter | egui::Key::Escape | egui::Key::Tab,
                            pressed: true,
                            ..
                        }
                    )
                })
            });
            saw_keys_ui.set(survived);
        });
    let _handle = ProgressOverlay::set_global_spinner_only(&harness.ctx);
    harness.step();
    assert!(!saw_keys.get(), "no keys injected yet");

    // Inject the prompt's navigation/confirm keys; after render_global they must
    // still be present (the overlay must not consume them).
    for key in [egui::Key::Enter, egui::Key::Escape, egui::Key::Tab] {
        harness.input_mut().events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.step();
    assert!(
        saw_keys.get(),
        "render_global must not swallow Enter/Esc/Tab while an overlay is up — a \
         secret prompt above it needs them (SEC-004/F-1)"
    );
}

/// Reconciliation #3 (QA-003) — the instance `Component::show` must NOT seize the
/// host screen's focus or install the global focus-lock. A host text field stays
/// focused after the inline overlay renders, proving the trap is global-only.
#[test]
fn reconciliation_instance_show_leaves_host_focus_navigable() {
    let overlay = Rc::new(RefCell::new(
        ProgressOverlay::new()
            .with_description("Inline overlay.")
            .with_action("Act", "inline.act"),
    ));
    let overlay_ui = Rc::clone(&overlay);
    let text = Rc::new(RefCell::new(String::new()));
    let text_ui = Rc::clone(&text);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(420.0, 360.0))
        .build_ui(move |ui| {
            let mut buffer = text_ui.borrow_mut();
            ui.text_edit_singleline(&mut *buffer);
            overlay_ui.borrow_mut().show(ui);
        });
    harness.step();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
    harness.step();

    // The inline overlay rendered (trap_focus = false), so the host field keeps
    // focus — an instance widget never wedges the host screen's navigation.
    assert!(
        harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .is_focused(),
        "the instance overlay must leave the host screen's focus alone (QA-003)"
    );
}

// ── RQ-1: AppState-level secret-prompt gate ─────────────────────────────────

/// RQ-1 (security) — drives the REAL `AppState::update` loop: a passphrase prompt
/// active above a button-less blocking overlay stays focusable AND typeable,
/// because `AppState::claim_overlay_input` suppresses the overlay's frame-start
/// `claim_input` while a secret prompt is active (SEC-004/F-1). Deleting that gate
/// makes `claim_input` (button-less → `stop_text_input`) steal the prompt's focus
/// and strip its keystrokes — which BOTH assertions below detect.
#[cfg(feature = "testing")]
#[test]
fn rq1_appstate_secret_prompt_gate_keeps_prompt_typeable_over_overlay() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            let mut app = dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false);
            // A secret prompt is active (renders above the overlay, needs keyboard).
            app.test_set_secret_prompt_active(true);
            app
        });
        harness.set_size(egui::vec2(800.0, 600.0));

        // Raise a button-less blocking overlay beneath the active prompt.
        ProgressOverlay::set_global_spinner_only(&harness.ctx);
        harness.run_steps(5);

        // The prompt renders above the overlay...
        assert!(
            harness.query_by_label_contains("Test prompt").is_some(),
            "the secret prompt renders above the overlay"
        );
        // ...and KEEPS keyboard focus: deleting the gate lets the overlay's
        // claim_input (stop_text_input, button-less) clear it → focused() == None.
        assert!(
            harness.ctx.memory(|m| m.focused()).is_some(),
            "the prompt input keeps keyboard focus over the overlay — removing the \
             app.rs secret-prompt gate lets the overlay's claim_input steal it"
        );

        // ...and ACCEPTS typed text: type a passphrase and submit with Enter; the
        // prompt resolves and closes. With the gate deleted the keystrokes are
        // stripped, so the prompt would stay open and this would fail.
        harness
            .input_mut()
            .events
            .push(egui::Event::Text("pw".to_string()));
        harness.run_steps(2);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(5);
        assert!(
            harness.query_by_label_contains("Test prompt").is_none(),
            "the prompt accepted the typed passphrase and submitted on Enter (it \
             would stay open if the overlay had stripped its keyboard)"
        );
    });
}

// ── Task 9: SPV-sync blocking overlay (startup + Connect) ────────────────────

/// Task 9 / F-SPV-A — the SPV-sync block is SCOPED to a user-initiated (armed)
/// episode: an armed Connecting/Syncing blocks; an UN-armed (ambient) reconnect or
/// per-block Synced→Syncing flip does NOT block; an armed episode disarms on a
/// terminal state and stays disarmed; the escape lowers it without re-raising; and
/// only a fresh armed episode re-blocks. Jargon-free copy (F-SPV-B). Drives the
/// REAL `AppState::update_spv_overlay` against a forced connection state.
#[cfg(feature = "testing")]
#[test]
fn task9_spv_overlay_armed_scope_disarm_and_escape() {
    use dash_evo_tool::context::connection_status::OverallConnectionState;
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui(|ui| {
                ProgressOverlay::render_global(ui.ctx());
            });
        let mut app = dash_evo_tool::app::AppState::new(harness.ctx.clone())
            .expect("Failed to create AppState");
        // Separate Arc clone so we can force connection state without borrowing app.
        let app_context = app.current_app_context().clone();
        let set_state = |s| app_context.connection_status().set_overall_state(s);

        // F-SPV-A regression guard: an UN-armed Connecting must NOT block (ambient).
        set_state(OverallConnectionState::Connecting);
        app.test_drive_spv_overlay(&harness.ctx);
        assert!(
            !ProgressOverlay::has_global(&harness.ctx),
            "an un-armed (ambient) connecting sync must NOT hard-block the user"
        );

        // Arm a user-initiated episode (startup / Connect) → Connecting blocks.
        app.test_arm_spv_block();
        app.test_drive_spv_overlay(&harness.ctx);
        assert!(
            ProgressOverlay::has_global(&harness.ctx),
            "an armed connecting sync raises the block"
        );
        harness.step();
        harness.step();
        harness.step();
        assert!(
            harness
                .query_by_label("Continue in the background")
                .is_some(),
            "the secondary 'Continue in the background' escape button renders"
        );
        // F-SPV-B: no blockchain jargon leaks into the description.
        assert!(
            harness.query_by_label_contains("Headers:").is_none()
                && harness.query_by_label_contains("Masternodes:").is_none(),
            "the block description must be jargon-free"
        );

        // C1 / F-SPV-A: Synced → lowers AND DISARMS.
        set_state(OverallConnectionState::Synced);
        app.test_drive_spv_overlay(&harness.ctx);
        assert!(
            !ProgressOverlay::has_global(&harness.ctx),
            "the block lowers when synced"
        );

        // After disarm, ambient Connecting/Syncing (reconnect, per-block catch-up)
        // must NOT re-block — the core F-SPV-A fix.
        set_state(OverallConnectionState::Syncing);
        app.test_drive_spv_overlay(&harness.ctx);
        assert!(
            !ProgressOverlay::has_global(&harness.ctx),
            "ambient syncing after the episode disarmed must NOT re-block"
        );

        // C2 escape: arm a fresh episode, block, click escape → lowers and stays
        // down for the rest of THIS episode even though sync is still in progress.
        app.test_arm_spv_block();
        set_state(OverallConnectionState::Connecting);
        app.test_drive_spv_overlay(&harness.ctx);
        harness.step();
        harness.step();
        harness.step();
        assert!(ProgressOverlay::has_global(&harness.ctx));
        harness.get_by_label("Continue in the background").click();
        harness.step();
        app.test_drive_spv_overlay(&harness.ctx);
        assert!(
            !ProgressOverlay::has_global(&harness.ctx),
            "the escape lowers the block (user never trapped)"
        );
        app.test_drive_spv_overlay(&harness.ctx);
        assert!(
            !ProgressOverlay::has_global(&harness.ctx),
            "the block is not re-raised within the dismissed episode"
        );

        // Only a fresh ARMED episode re-blocks; an ambient one still does not.
        set_state(OverallConnectionState::Disconnected);
        app.test_drive_spv_overlay(&harness.ctx);
        set_state(OverallConnectionState::Connecting);
        app.test_drive_spv_overlay(&harness.ctx);
        assert!(
            !ProgressOverlay::has_global(&harness.ctx),
            "ambient connecting (no fresh arm) must NOT block"
        );
        app.test_arm_spv_block();
        app.test_drive_spv_overlay(&harness.ctx);
        assert!(
            ProgressOverlay::has_global(&harness.ctx),
            "a fresh armed episode re-blocks"
        );
    });
}

/// Item A (one-frame interactive gap) — driving the REAL `AppState::update` loop,
/// an armed SPV episode RAISES **and PAINTS** the block on the SAME frame it is
/// first observed, because `update_spv_overlay` now runs before `claim_input`, the
/// screen `ui()`, and `render_global`. Under the pre-fix order the block was raised
/// only after `render_global`, so it painted a frame late — leaving that frame
/// fully interactive. This test would need two `step()`s under the old order: the
/// single-frame paint assertion is the regression guard for the gap.
#[cfg(feature = "testing")]
#[test]
fn item_a_armed_episode_blocks_and_paints_same_frame() {
    use dash_evo_tool::context::connection_status::OverallConnectionState;
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            let mut app = dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false);
            // Arm a user-initiated episode and force Connecting, exactly as the
            // Connect button / boot auto-start do — but BEFORE the first frame runs.
            let app_context = app.current_app_context().clone();
            app_context
                .connection_status()
                .set_overall_state(OverallConnectionState::Connecting);
            app.test_arm_spv_block();
            app
        });
        harness.set_size(egui::vec2(800.0, 600.0));

        // Exactly ONE frame. `update_spv_overlay` runs at frame start (before the
        // input claim, the screen, and `render_global`), so the block is both raised
        // and painted this same frame.
        harness.step();
        assert!(
            ProgressOverlay::has_global(&harness.ctx),
            "the armed episode raises the block"
        );
        assert!(
            harness
                .query_by_label("Continue in the background")
                .is_some(),
            "the block is PAINTED on the same frame it is armed+observed — under the \
             pre-fix order (raise after render_global) it would only paint next frame, \
             leaving this frame interactive (the one-frame gap)"
        );
    });
}

/// Item B (watchdog liveness) — using the `backdate` clock seam: a phase whose
/// elapsed exceeds the 120 s no-progress watchdog but whose hidden `progress_token`
/// ADVANCES must NOT trip the watchdog (a slow-but-advancing phase is real
/// progress); the same elapsed WITHOUT an advancing token MUST still trip it (a
/// genuine stall). The shown copy never changes, so the existing `(description,
/// step)` change-detection alone could not tell the two apart.
#[cfg(feature = "testing")]
#[test]
fn item_b_advancing_progress_token_resets_watchdog() {
    let mut harness = overlay_harness();
    // Raise with a seed token; render once to log "shown" and record the token.
    let handle = ProgressOverlay::set_global(
        &harness.ctx,
        "Syncing with the Dash network.",
        OverlayConfig::new().with_progress_token(100),
    );
    harness.step();
    assert!(
        harness
            .query_by_label_contains("much longer than expected")
            .is_none(),
        "no watchdog at the start"
    );

    // Age past the 120 s watchdog, but ADVANCE the hidden token before the next
    // render — the shown description/step is untouched.
    handle.backdate(Duration::from_secs(200));
    handle.set_progress_token(200);
    harness.step();
    assert!(
        harness
            .query_by_label_contains("much longer than expected")
            .is_none(),
        "an advancing progress token resets the watchdog — no false-stall escalation \
         even though the shown copy is unchanged"
    );

    // Age past the watchdog again WITHOUT advancing the token: a genuine stall must
    // still trip it.
    handle.backdate(Duration::from_secs(200));
    harness.step();
    assert!(
        harness
            .query_by_label_contains("much longer than expected")
            .is_some(),
        "a real stall (token unchanged) still trips the watchdog"
    );
}
