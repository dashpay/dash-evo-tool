//! Kittest coverage for the just-in-time secret prompt modal.
//!
//! Drives the shared [`passphrase_modal`] chrome directly (the same body
//! `EguiSecretPromptHost` renders) to assert the GUI surface the
//! remember-policy mapping depends on:
//!
//! - the scope body label, hint, and inline retry error render;
//! - the "Keep this wallet unlocked until I close the app." checkbox renders
//!   and toggles (unchecked maps to `RememberPolicy::None`, checked to
//!   `UntilAppClose` — the mapping itself is unit-tested in
//!   `secret_prompt_host`).
//!
//! NOTE: the kittest suite has pre-existing `DivergentVersion` failures
//! unrelated to this module.

use std::cell::Cell;
use std::rc::Rc;

use dash_evo_tool::ui::components::ProgressOverlay;
use dash_evo_tool::ui::components::passphrase_modal::{
    KEEP_UNLOCKED_LABEL, PassphraseModalConfig, passphrase_modal,
};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// The modal renders the scope body, the hint, the retry error, and the
/// remember checkbox.
#[test]
fn modal_renders_body_hint_error_and_remember_checkbox() {
    let mut remember = false;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(move |ui| {
            let ctx = ui.ctx().clone();
            let config = PassphraseModalConfig {
                state_id: egui::Id::new("test_prompt_body"),
                window_title: "Unlock to continue",
                body: "My Wallet",
                hint: Some("granny's birthday"),
                error: Some("That passphrase is not correct. Try again."),
                submit_label: "Unlock",
                secondary_action_label: None,
                input_placeholder: "Enter passphrase",
                remember_label: None,
                cancellable: true,
            };
            passphrase_modal(&ctx, &config, |ui| {
                ui.checkbox(&mut remember, KEEP_UNLOCKED_LABEL);
            });
        });
    harness.run();

    assert!(
        harness.query_by_label("My Wallet").is_some(),
        "scope body label should render"
    );
    assert!(
        harness
            .query_by_label_contains("granny's birthday")
            .is_some(),
        "password hint should render"
    );
    assert!(
        harness
            .query_by_label_contains("That passphrase is not correct")
            .is_some(),
        "inline retry error should render"
    );
    assert!(
        harness.query_by_label(KEEP_UNLOCKED_LABEL).is_some(),
        "remember-until-close checkbox should render"
    );
    assert!(
        harness.query_by_label("Unlock").is_some(),
        "submit button should render"
    );
    assert!(
        harness.query_by_label("Cancel").is_some(),
        "cancel button should render"
    );
}

/// Clicking the remember checkbox flips the bound flag — the source the
/// host maps to `RememberPolicy::UntilAppClose`.
#[test]
fn remember_checkbox_toggles() {
    use std::cell::Cell;
    use std::rc::Rc;

    let remember = Rc::new(Cell::new(false));

    let remember_for_ui = Rc::clone(&remember);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(move |ui| {
            let ctx = ui.ctx().clone();
            let config = PassphraseModalConfig {
                state_id: egui::Id::new("test_prompt_remember"),
                window_title: "Unlock to continue",
                body: "My Wallet",
                hint: None,
                error: None,
                submit_label: "Unlock",
                secondary_action_label: None,
                input_placeholder: "Enter passphrase",
                remember_label: None,
                cancellable: true,
            };
            let mut local = remember_for_ui.get();
            passphrase_modal(&ctx, &config, |ui| {
                ui.checkbox(&mut local, KEEP_UNLOCKED_LABEL);
            });
            remember_for_ui.set(local);
        });
    harness.run();

    assert!(!remember.get(), "checkbox starts unchecked (default None)");
    harness.get_by_label(KEEP_UNLOCKED_LABEL).click();
    harness.run();
    assert!(
        remember.get(),
        "clicking the checkbox flips it on (maps to UntilAppClose)"
    );
}

/// A *cancellable* prompt owns the interaction surface too.
///
/// The blocking progress overlay yields for **any** passphrase prompt
/// (`AppState::has_blocking_secret_prompt`), painting no dimmer and no pointer
/// sink. The prompt must therefore supply the barrier itself — otherwise the
/// ordinary just-in-time unlock (which is cancellable) leaves the app beneath a
/// supposedly-blocking overlay fully clickable.
#[test]
fn cancellable_passphrase_modal_blocks_clicks_beneath_a_yielding_overlay() {
    let counter = Rc::new(Cell::new(0u32));
    let counter_ui = Rc::clone(&counter);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(move |ui| {
            if ui.button("Increment").clicked() {
                counter_ui.set(counter_ui.get() + 1);
            }
            // Frame order mirrors `AppState::update`: the overlay yields to the
            // prompt, then the prompt renders on top.
            ProgressOverlay::render_global(ui.ctx(), true);
            let config = PassphraseModalConfig {
                state_id: egui::Id::new("test_jit_prompt_sink"),
                window_title: "Unlock to continue",
                body: "My Wallet",
                hint: None,
                error: None,
                submit_label: "Unlock",
                secondary_action_label: None,
                input_placeholder: "Enter passphrase",
                remember_label: None,
                cancellable: true,
            };
            passphrase_modal(ui.ctx(), &config, |_| {});
        });
    let _overlay = ProgressOverlay::set_global_spinner_only(&harness.ctx);
    harness.step();

    harness.get_by_label("Increment").click();
    harness.step();

    assert_eq!(
        counter.get(),
        0,
        "a control beneath a cancellable passphrase prompt must not receive the click",
    );
}

/// Owning the interaction surface must not cost the prompt its dismissal: the
/// pointer sink absorbs clicks for the app beneath, while the modal's own
/// controls stay live.
#[test]
fn cancellable_passphrase_modal_still_dismisses_from_its_own_controls() {
    use dash_evo_tool::ui::components::passphrase_modal::PassphraseModalOutcome;

    let cancelled = Rc::new(Cell::new(false));
    let cancelled_ui = Rc::clone(&cancelled);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(move |ui| {
            let config = PassphraseModalConfig {
                state_id: egui::Id::new("test_jit_prompt_cancel"),
                window_title: "Unlock to continue",
                body: "My Wallet",
                hint: None,
                error: None,
                submit_label: "Unlock",
                secondary_action_label: None,
                input_placeholder: "Enter passphrase",
                remember_label: None,
                cancellable: true,
            };
            if passphrase_modal(ui.ctx(), &config, |_| {}) == PassphraseModalOutcome::Cancel {
                cancelled_ui.set(true);
            }
        });
    harness.step();

    harness.get_by_label("Cancel").click();
    harness.step();

    assert!(
        cancelled.get(),
        "Cancel must still dismiss a prompt that installs its own input sink",
    );
}

#[test]
fn blocking_passphrase_modal_has_no_dismiss_control() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(|ui| {
            let config = PassphraseModalConfig {
                state_id: egui::Id::new("test_storage_update_prompt"),
                window_title: "Continue the storage update",
                body: "Enter the password for \"Savings\" to update this wallet now.",
                hint: None,
                error: None,
                submit_label: "Continue",
                secondary_action_label: Some("Skip this wallet"),
                input_placeholder: "Enter your password.",
                remember_label: None,
                cancellable: false,
            };
            passphrase_modal(ui.ctx(), &config, |_| {});
        });
    harness.run();

    assert!(harness.query_by_label("Cancel").is_none());
    assert!(
        harness
            .query_by_label_contains("Enter the password for \"Savings\"")
            .is_some()
    );
    assert!(harness.query_by_label("Skip this wallet").is_some());
}
