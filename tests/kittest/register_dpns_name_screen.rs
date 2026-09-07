//! Kittest coverage for the Bucket A overlay adoption on the DPNS registration
//! screen (`RegisterDpnsNameScreen`).
//!
//! Proves the canonical contract the remaining transaction screens will copy:
//! dispatching the registration raises the global blocking overlay, and every
//! terminal result — success (`display_task_result`) or error
//! (`display_message`) — tears it down (the no-hard-lock guarantee).
//!
//! The screen needs an `Arc<AppContext>`, so each test borrows one from a
//! throwaway `AppState` built in an isolated data dir. The overlay raise/teardown
//! runs against an independent `egui::Context` the AppState never renders, so the
//! app's own SPV block can't perturb the `has_global` assertions.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::app::AppState;
use dash_evo_tool::backend_task::{BackendTaskSuccessResult, FeeResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::dpns::DpnsRegistrationOutcome;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::MessageType;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::components::ProgressOverlay;
use dash_evo_tool::ui::identity::register_dpns_name_screen::{
    RegisterDpnsNameScreen, RegisterDpnsNameSource,
};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Build a `RegisterDpnsNameScreen` over a fresh, isolated `AppContext`.
fn screen_with_context() -> RegisterDpnsNameScreen {
    let app_state = AppState::new(egui::Context::default()).expect("AppState builds");
    let app_context = app_state.current_app_context().clone();
    RegisterDpnsNameScreen::new(&app_context, RegisterDpnsNameSource::Dpns)
}

/// Dispatching the registration raises the global blocking overlay.
#[test]
fn dpns_dispatch_raises_blocking_overlay() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut screen = screen_with_context();
        let ctx = egui::Context::default();

        assert!(!ProgressOverlay::has_global(&ctx));
        screen.raise_progress_overlay_for_test(&ctx);
        assert!(
            ProgressOverlay::has_global(&ctx),
            "dispatching the registration must raise the blocking overlay"
        );
    });
}

/// A successful registration result tears the overlay down (success terminal path).
#[test]
fn dpns_success_result_clears_overlay() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut screen = screen_with_context();
        let ctx = egui::Context::default();

        screen.raise_progress_overlay_for_test(&ctx);
        assert!(ProgressOverlay::has_global(&ctx));

        screen.display_task_result(BackendTaskSuccessResult::RegisteredDpnsName {
            outcome: DpnsRegistrationOutcome::Registered,
            fee_result: FeeResult::new(0, 0),
        });
        assert!(
            !ProgressOverlay::has_global(&ctx),
            "a successful result must tear down the blocking overlay"
        );
    });
}

fn assert_registration_outcome_copy(outcome: DpnsRegistrationOutcome, expected: &str, wrong: &str) {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut screen = screen_with_context();
        screen.display_task_result(BackendTaskSuccessResult::RegisteredDpnsName {
            outcome,
            fee_result: FeeResult::new(0, 0),
        });

        let mut harness = Harness::builder().build_ui(move |ui| {
            screen.ui(ui);
        });
        harness.run();

        assert!(harness.query_by_label(expected).is_some());
        assert!(harness.query_by_label("DPNS Name Registered!").is_none());
        assert!(harness.query_by_label(wrong).is_none());
    });
}

#[test]
fn dpns_registered_outcome_renders_finalized_copy() {
    assert_registration_outcome_copy(
        DpnsRegistrationOutcome::Registered,
        "Your username is registered. You can use it now.",
        "Your username request was submitted. Other people can also request this name, so the community will vote on who receives it. Check the Pending label on your identity for updates.",
    );
}

#[test]
fn dpns_pending_outcome_renders_voting_copy() {
    assert_registration_outcome_copy(
        DpnsRegistrationOutcome::PendingCommunityVote,
        "Your username request was submitted. Other people can also request this name, so the community will vote on who receives it. Check the Pending label on your identity for updates.",
        "Your username is registered. You can use it now.",
    );
}

// ── W2 B2 — app-scoped seeding ───────────────────────────────────────────────

/// Seed a wallet-less identity into the live context (mirrors identity_hub_switcher).
fn seed_identity_for_dpns(app_context: &Arc<AppContext>, byte: u8, alias: &str) -> Identifier {
    let pv = PlatformVersion::latest();
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), pv).expect("basic identity");
    let id = identity.id();
    let qi = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some(alias.to_string()),
        private_keys: KeyStorage::default(),
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::PendingCreation,
        network: app_context.network(),
    };
    app_context
        .insert_local_qualified_identity(&qi, &None)
        .expect("seed dpns identity");
    id
}

/// W2 B2: `RegisterDpnsNameScreen` must default to the app-scoped selected identity,
/// not necessarily the first loaded identity.
#[test]
fn dpns_registration_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("AppState builds")
                .with_animations(false)
        });
        let app_context = crate::support::wait_for_wallet_backend(&mut harness);

        let _first = seed_identity_for_dpns(&app_context, 0x11, "DPNS Alpha");
        let second = seed_identity_for_dpns(&app_context, 0x22, "DPNS Beta");

        app_context.set_selected_identity(Some(second));

        let screen = RegisterDpnsNameScreen::new(&app_context, RegisterDpnsNameSource::Dpns);

        assert_eq!(
            screen
                .selected_qualified_identity
                .as_ref()
                .map(|qi| qi.identity.id()),
            Some(second),
            "RegisterDpnsNameScreen must default to the app-scoped selected identity"
        );
    });
}

/// An error message tears the overlay down (error terminal path):
/// failed registration can never leave the window hard-locked.
#[test]
fn dpns_error_message_clears_overlay() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut screen = screen_with_context();
        let ctx = egui::Context::default();

        screen.raise_progress_overlay_for_test(&ctx);
        assert!(ProgressOverlay::has_global(&ctx));

        screen.display_message("Registration failed. Try again.", MessageType::Error);
        assert!(
            !ProgressOverlay::has_global(&ctx),
            "an error result must tear down the blocking overlay"
        );
    });
}
