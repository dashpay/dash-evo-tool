//! Kittest coverage for the just-in-time secret prompt modal.
//!
//! Drives both the real `AppState::update` loop and the shared
//! [`passphrase_modal`] chrome directly to assert the activation wiring and the
//! GUI surface the remember-policy mapping depends on:
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
    KEEP_UNLOCKED_LABEL, PassphraseModalConfig, drop_activation_frame_pointer_click,
    passphrase_modal,
};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

#[cfg(feature = "testing")]
use dash_evo_tool::context::migration_status::MigrationState;
#[cfg(feature = "testing")]
use dash_evo_tool::model::secret::Secret;
#[cfg(feature = "testing")]
use dash_evo_tool::model::wallet::Wallet;
#[cfg(feature = "testing")]
use dash_evo_tool::model::wallet::birth_height::WalletOrigin;
#[cfg(feature = "testing")]
use dash_sdk::dpp::dashcore::Network;

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

/// The **transition frame** — the first frame a prompt becomes active — is not
/// protected by the sink, so the app must drop this frame's pending click itself.
///
/// egui computes each frame's click interaction at `begin_pass` from the
/// *previous* frame's widget geometry (`viewport.prev_pass.widgets`, see
/// `Context::begin_pass`) and the *previous* frame's modal layer
/// (`Focus::top_modal_layer`, published only in `Focus::end_pass`). On the frame
/// a prompt first renders, the control beneath still existed last frame with no
/// sink above it and no modal layer recorded, so egui completes the click on it
/// *before* `modal_chrome` installs the sink / calls `set_modal_layer` later in
/// the same frame — reordering the render cannot help.
///
/// The fix drops this frame's pending pointer click as the prompt is promoted,
/// before the screen beneath runs (`AppState::update` calls
/// [`drop_activation_frame_pointer_click`] on the prompt-activation rising edge,
/// which this closure mirrors). A widget only reports a click while a `Released`
/// event is still in `input.pointer`; clearing it strands the leaked click.
///
/// The sibling test below primes the sink a full frame before the press, so it
/// only ever exercises frame N+1 and later. This test presses the underlying
/// button while no prompt exists, activates the prompt, then releases on the very
/// frame the prompt first renders — the transition frame the sink cannot cover.
#[test]
fn transition_frame_click_leaks_through_a_newly_activated_prompt() {
    let counter = Rc::new(Cell::new(0u32));
    let counter_ui = Rc::clone(&counter);
    let show_prompt = Rc::new(Cell::new(false));
    let show_prompt_ui = Rc::clone(&show_prompt);
    let was_active = Rc::new(Cell::new(false));
    let was_active_ui = Rc::clone(&was_active);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(move |ui| {
            // Mirror `AppState::update`: promote the prompt at frame start, and on
            // the frame it first becomes active drop this frame's pending click
            // before the screen beneath (the button) runs.
            let active = show_prompt_ui.get();
            if active && !was_active_ui.get() {
                drop_activation_frame_pointer_click(ui.ctx());
            }
            was_active_ui.set(active);

            if ui.button("Increment").clicked() {
                counter_ui.set(counter_ui.get() + 1);
            }
            if active {
                // Same frame order as `AppState::update`: the overlay yields to
                // the prompt (paints no sink of its own), the prompt renders on top.
                ProgressOverlay::render_global(ui.ctx(), true);
                let config = PassphraseModalConfig {
                    state_id: egui::Id::new("test_transition_prompt_sink"),
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
            }
        });

    // Frame N-1: no prompt. Only the button exists; its geometry is recorded.
    harness.step();
    let button_center = harness.get_by_label("Increment").rect().center();

    // The pointer presses the button while no prompt exists — the press resolves
    // to the button (no sink in the prior frame's geometry).
    harness.hover_at(button_center);
    harness.event(egui::Event::PointerButton {
        pos: button_center,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    // The prompt becomes active exactly as the click completes: the release lands
    // on the transition frame, resolved against the prior (still sink-less) frame.
    show_prompt.set(true);
    harness.event(egui::Event::PointerButton {
        pos: button_center,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert_eq!(
        counter.get(),
        0,
        "a control beneath a prompt must not receive a click on the frame the \
         prompt first becomes active — the app drops this frame's pending click \
         before the screen renders",
    );
}

/// The migration password prompt has the same transition-frame exposure as a
/// just-in-time unlock: it renders *after* the screen (via `update_banner`), so
/// its first frame's click resolves against the prior, prompt-less frame before
/// the sink exists. It is non-cancellable and shows a "Skip this wallet"
/// secondary action instead of Cancel — a different modal chrome — yet the same
/// [`drop_activation_frame_pointer_click`] on the activation rising edge must
/// strand the leaked click.
#[test]
fn transition_frame_click_leaks_through_a_newly_activated_migration_prompt() {
    let counter = Rc::new(Cell::new(0u32));
    let counter_ui = Rc::clone(&counter);
    let show_prompt = Rc::new(Cell::new(false));
    let show_prompt_ui = Rc::clone(&show_prompt);
    let was_active = Rc::new(Cell::new(false));
    let was_active_ui = Rc::clone(&was_active);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(move |ui| {
            let active = show_prompt_ui.get();
            if active && !was_active_ui.get() {
                drop_activation_frame_pointer_click(ui.ctx());
            }
            was_active_ui.set(active);

            if ui.button("Increment").clicked() {
                counter_ui.set(counter_ui.get() + 1);
            }
            if active {
                ProgressOverlay::render_global(ui.ctx(), true);
                let config = PassphraseModalConfig {
                    state_id: egui::Id::new("test_transition_migration_prompt_sink"),
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
            }
        });

    harness.step();
    let button_center = harness.get_by_label("Increment").rect().center();

    harness.hover_at(button_center);
    harness.event(egui::Event::PointerButton {
        pos: button_center,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    show_prompt.set(true);
    harness.event(egui::Event::PointerButton {
        pos: button_center,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert_eq!(
        counter.get(),
        0,
        "a control beneath the migration password prompt must not receive a click \
         on the frame the prompt first becomes active",
    );
}

/// The real `AppState::update` rising-edge wiring drops a click completed on
/// the frame a just-in-time passphrase prompt first becomes active.
#[cfg(feature = "testing")]
#[test]
fn appstate_jit_prompt_activation_drops_transition_frame_click() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });
        harness.set_size(egui::vec2(1024.0, 768.0));
        harness.run_steps(5);

        let card_center = harness.get_by_label("Just Explore").rect().center();
        harness.hover_at(card_center);
        harness.event(egui::Event::PointerButton {
            pos: card_center,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();

        harness.state_mut().test_set_secret_prompt_active(true);
        harness.event(egui::Event::PointerButton {
            pos: card_center,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();

        assert!(
            harness.state().show_welcome_screen,
            "the real update loop must drop the onboarding click completed on the JIT prompt's activation frame",
        );
        assert!(
            harness.query_by_label_contains("Test prompt").is_some(),
            "the JIT prompt must render on the transition frame",
        );
    });
}

/// The same real `AppState::update` rising edge covers the migration wallet
/// password path, which activates from `MigrationStatus` rather than the JIT host.
#[cfg(feature = "testing")]
#[test]
fn appstate_migration_prompt_activation_drops_transition_frame_click() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let seed_hash = Rc::new(Cell::new([0; 32]));
        let seed_hash_for_app = Rc::clone(&seed_hash);
        let mut harness = Harness::builder()
            .with_max_steps(100)
            .build_eframe(move |ctx| {
                let app = dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                    .expect("Failed to create AppState")
                    .with_animations(false);

                let password = Secret::new("correct password");
                let seed = [0xA7; 64];
                let wallet = Wallet::new_from_seed(
                    seed,
                    Network::Testnet,
                    Some("Savings".to_string()),
                    Some(&password),
                )
                .expect("build protected wallet");
                let (seed_hash, wallet) = app
                    .current_app_context()
                    .register_wallet(wallet, &seed, WalletOrigin::Imported)
                    .expect("register protected wallet fixture");
                wallet.write().expect("wallet lock").wallet_seed.close();
                seed_hash_for_app.set(seed_hash);
                app
            });
        harness.set_size(egui::vec2(1024.0, 768.0));
        let app_context = crate::support::wait_for_wallet_backend(&mut harness);
        harness.run_steps(5);

        let card_center = harness.get_by_label("Just Explore").rect().center();
        harness.hover_at(card_center);
        harness.event(egui::Event::PointerButton {
            pos: card_center,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();

        app_context
            .migration_status()
            .set_state(MigrationState::AwaitingWalletPasswords {
                wallets: vec![seed_hash.get()],
            });
        harness.event(egui::Event::PointerButton {
            pos: card_center,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();

        assert!(
            harness.state().show_welcome_screen,
            "the real update loop must drop the onboarding click completed on the migration prompt's activation frame",
        );
        assert!(
            harness
                .query_by_label("Enter the password for \"Savings\" to update this wallet now.")
                .is_some(),
            "the migration password prompt must render on the transition frame",
        );
    });
}

/// Control for `transition_frame_click_leaks_through_a_newly_activated_prompt`:
/// the identical manual press/release sequence, but the prompt is activated one
/// frame *earlier* so the sink already existed in the frame before the press.
/// The click is absorbed (counter stays 0), proving the leak is specific to the
/// transition frame — not an artifact of the injected pointer events.
#[test]
fn primed_prompt_blocks_the_same_injected_click_sequence() {
    let counter = Rc::new(Cell::new(0u32));
    let counter_ui = Rc::clone(&counter);
    let show_prompt = Rc::new(Cell::new(false));
    let show_prompt_ui = Rc::clone(&show_prompt);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(move |ui| {
            if ui.button("Increment").clicked() {
                counter_ui.set(counter_ui.get() + 1);
            }
            if show_prompt_ui.get() {
                ProgressOverlay::render_global(ui.ctx(), true);
                let config = PassphraseModalConfig {
                    state_id: egui::Id::new("test_primed_prompt_sink"),
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
            }
        });

    harness.step();
    let button_center = harness.get_by_label("Increment").rect().center();

    // Prompt is already active a full frame before the press: the sink is in the
    // prior frame's geometry when the press resolves.
    show_prompt.set(true);
    harness.step();

    harness.hover_at(button_center);
    harness.event(egui::Event::PointerButton {
        pos: button_center,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: button_center,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert_eq!(
        counter.get(),
        0,
        "a control beneath an already-primed prompt must not receive the click",
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

/// NEW-005 regression: the password field inside the passphrase modal can take
/// keyboard focus and receive typed input, while a widget behind the modal
/// receives none of it.
///
/// `modal_chrome` blocks background input by registering the **window's own**
/// layer as egui's modal layer. A prior version registered a separate
/// full-screen "sink" `Area` instead; because that sink was a different layer
/// than the window, the window did not resolve at/above the modal layer and the
/// password `TextEdit` was silently denied keyboard focus — typing did nothing
/// (release-blocking). This test pins both halves at once: the modal layer is
/// the window's own layer (not a sink), the field holds focus and receives the
/// typed text (surfaced through `Submit`), and the background stays blocked.
#[test]
fn passphrase_modal_password_field_focuses_and_blocks_background() {
    use std::cell::RefCell;

    use dash_evo_tool::ui::components::passphrase_modal::PassphraseModalOutcome;

    let outcome = Rc::new(RefCell::new(PassphraseModalOutcome::Pending));
    let outcome_ui = Rc::clone(&outcome);
    let background_text = Rc::new(RefCell::new(String::new()));
    let background_ui = Rc::clone(&background_text);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui(move |ui| {
            // A widget behind the modal. It must never receive the typed text.
            {
                let mut bg = background_ui.borrow_mut();
                ui.add(egui::TextEdit::singleline(&mut *bg).id(egui::Id::new("background_field")));
            }

            let config = PassphraseModalConfig {
                state_id: egui::Id::new("test_focus_blocks_bg"),
                window_title: "Unlock Wallet",
                body: "My Wallet",
                hint: None,
                error: None,
                submit_label: "Unlock",
                secondary_action_label: None,
                input_placeholder: "Enter password",
                remember_label: None,
                cancellable: true,
            };
            *outcome_ui.borrow_mut() = passphrase_modal(ui.ctx(), &config, |_| {});
        });

    // Frame 1 opens the modal and focuses the field; frame 2 lets the modal layer
    // (registered at the end of frame 1) take effect.
    harness.step();
    harness.step();

    // The modal layer is the window's OWN Foreground layer, not a separate sink.
    let sink_id = egui::Id::new("passphrase_modal_overlay")
        .with("Unlock Wallet")
        .with("input_sink");
    let modal_layer = harness
        .ctx
        .memory(|m| m.top_modal_layer())
        .expect("the blocking modal registers a modal layer");
    assert_eq!(
        modal_layer.order,
        egui::Order::Foreground,
        "the modal layer is the Foreground window",
    );
    assert_ne!(
        modal_layer.id, sink_id,
        "the modal layer must be the window's own layer, not a separate input sink \
         — registering the sink is what denied the password field focus (NEW-005)",
    );

    // Below-modal layers get neither pointer nor keyboard input.
    assert!(
        !harness
            .ctx
            .memory(|m| m.is_above_modal_layer(egui::LayerId::background())),
        "the background layer is blocked below the modal",
    );

    // The password field holds keyboard focus (the regression left it unfocusable).
    assert!(
        harness.ctx.memory(|m| m.focused()).is_some(),
        "the password field can hold keyboard focus",
    );

    // Typed text reaches the focused password field, not the background.
    harness.event(egui::Event::Text("secret".to_string()));
    harness.step();

    harness.get_by_label("Unlock").click();
    harness.step();

    assert_eq!(
        *background_text.borrow(),
        "",
        "a widget behind the modal must not receive the typed text",
    );
    match &*outcome.borrow() {
        PassphraseModalOutcome::Submit(text) => assert_eq!(
            text.as_str(),
            "secret",
            "the password field received the typed keyboard input",
        ),
        other => panic!("expected Submit carrying the typed password, got {other:?}"),
    }
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
