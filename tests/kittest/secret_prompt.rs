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

use dash_evo_tool::ui::components::passphrase_modal::{PassphraseModalConfig, passphrase_modal};
use dash_evo_tool::ui::components::password_input::PasswordInput;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const REMEMBER_LABEL: &str = "Keep this wallet unlocked until I close the app.";

/// The modal renders the scope body, the hint, the retry error, and the
/// remember checkbox.
#[test]
fn modal_renders_body_hint_error_and_remember_checkbox() {
    let mut password_input = PasswordInput::new();
    let mut focus_requested = false;
    let mut remember = false;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(move |ui| {
            let ctx = ui.ctx().clone();
            let config = PassphraseModalConfig {
                window_title: "Unlock to continue",
                body: "My Wallet",
                hint: Some("granny's birthday"),
                error: Some("That passphrase is not correct. Try again."),
                submit_label: "Unlock",
            };
            passphrase_modal(
                &ctx,
                &config,
                &mut password_input,
                &mut focus_requested,
                |ui| {
                    ui.checkbox(&mut remember, REMEMBER_LABEL);
                },
            );
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
        harness.query_by_label(REMEMBER_LABEL).is_some(),
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
    let mut password_input = PasswordInput::new();
    let mut focus_requested = false;

    let remember_for_ui = Rc::clone(&remember);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(move |ui| {
            let ctx = ui.ctx().clone();
            let config = PassphraseModalConfig {
                window_title: "Unlock to continue",
                body: "My Wallet",
                hint: None,
                error: None,
                submit_label: "Unlock",
            };
            let mut local = remember_for_ui.get();
            passphrase_modal(
                &ctx,
                &config,
                &mut password_input,
                &mut focus_requested,
                |ui| {
                    ui.checkbox(&mut local, REMEMBER_LABEL);
                },
            );
            remember_for_ui.set(local);
        });
    harness.run();

    assert!(!remember.get(), "checkbox starts unchecked (default None)");
    harness.get_by_label(REMEMBER_LABEL).click();
    harness.run();
    assert!(
        remember.get(),
        "clicking the checkbox flips it on (maps to UntilAppClose)"
    );
}
