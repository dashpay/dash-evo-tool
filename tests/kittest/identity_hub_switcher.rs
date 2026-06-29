//! IT-SWITCH — breadcrumb switcher / multi-identity hub integration.
//!
//! IT-SWITCH-03 (onboarding placeholders) runs on the default first-run
//! database (0 wallets, 0 identities).
//!
//! IT-SWITCH-04 and the stale-selection reconcile case seed a multi-identity
//! database directly into the live `AppContext` (the fixture Bilby's Wave-1
//! report deferred). They cover the end-to-end multi-identity path through the
//! real `AppState` frame loop: DB → `effective_view` → rendered surface, and
//! the app-scoped selection (`set_selected_identity`, the exact call the picker
//! click handler in `hub_screen` makes) driving the Picker → Home transition.
//!
//! The seeded identities are wallet-less (imported-by-id) basic identities:
//! `insert_local_qualified_identity(.., &None)`. A wallet-scoped fixture (a
//! loaded HD `Wallet` in `AppContext::wallets` with matching `wallet_hash`)
//! is what IT-SWITCH-01/02 (the wallet dropdown + wallet-scoped identity list)
//! additionally require; that is still out of reach here (see the QA report).

use crate::support::with_isolated_data_dir;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::RootScreenType;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The hub tab bar's `Activity` tab label only renders in the Home view, never
/// in the Picker or Onboarding surfaces — a reliable "we are on Home" marker.
const HOME_ONLY_MARKER: &str = "Activity";
/// The Picker grid heading (verbatim, picker.rs / design-spec §B.14).
const PICKER_HEADING: &str = "Pick an identity";

/// Mount the full `AppState` on the Identities hub. Returns the harness so the
/// caller can seed the DB through the live context and step the frame loop.
fn mount_hub() -> Harness<'static, dash_evo_tool::app::AppState> {
    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        let mut app = dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false);
        app.show_welcome_screen = false;
        app.welcome_screen = None;
        app.selected_main_screen = RootScreenType::RootScreenIdentityHub;
        app
    });
    harness.set_size(egui::vec2(1280.0, 800.0));
    harness.run_steps(5);
    harness
}

/// Seed one wallet-less basic identity (alias = `alias`, id = `[byte; 32]`)
/// into the live per-network identity DB, and return its `Identifier`.
fn seed_identity(app_context: &Arc<AppContext>, byte: u8, alias: &str) -> Identifier {
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
        .expect("seed identity insert");
    id
}

/// IT-SWITCH-03 — onboarding (0 wallets, 0 identities): the breadcrumb shows
/// `(no wallet yet)` and `(no identity yet)` placeholders, and the onboarding
/// CTAs still render beneath (switcher coexists on every landing).
#[test]
fn it_switch_03_onboarding_shows_placeholder_segments() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            let mut app = dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false);
            app.show_welcome_screen = false;
            app.welcome_screen = None;
            app.selected_main_screen = RootScreenType::RootScreenIdentityHub;
            app
        });
        harness.set_size(egui::vec2(1280.0, 800.0));
        harness.run_steps(10);

        assert!(
            harness.query_by_label("(no wallet yet)").is_some(),
            "breadcrumb must show the no-wallet placeholder on onboarding"
        );
        assert!(
            harness.query_by_label("(no identity yet)").is_some(),
            "breadcrumb must show the no-identity placeholder on onboarding"
        );
        // The onboarding CTA still renders beneath the switcher.
        assert!(
            harness.query_by_label("Create my first identity").is_some(),
            "onboarding CTA must coexist with the breadcrumb switcher"
        );
    });
}

/// IT-SWITCH-04 — with two identities and no explicit selection the hub lands
/// on the Picker; setting the app-scoped selection (the picker click effect)
/// transitions the hub to that identity's Home. Exercises the real
/// DB → `effective_view` → surface path and `resolve_selected_identity`.
#[test]
fn it_switch_04_picker_then_selection_drives_home() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_hub();
        let app_context = harness.state().current_app_context().clone();

        let alpha = seed_identity(&app_context, 0xA1, "Switch Alpha");
        let _beta = seed_identity(&app_context, 0xB2, "Switch Beta");
        harness.run_steps(5);

        // Two identities, none chosen → Picker, listing both seeded identities.
        assert!(
            harness.query_by_label(PICKER_HEADING).is_some(),
            "two identities with no selection must land on the picker"
        );
        assert!(
            harness.query_by_label("Switch Alpha").is_some()
                && harness.query_by_label("Switch Beta").is_some(),
            "the picker must list both seeded identities by alias"
        );
        assert!(
            harness.query_by_label(HOME_ONLY_MARKER).is_none(),
            "the Home tab bar must NOT render while the picker is showing"
        );

        // Select Alpha — exactly what `hub_screen`'s picker-click handler does.
        app_context.set_selected_identity(Some(alpha));
        harness.run_steps(5);

        assert!(
            harness.query_by_label(PICKER_HEADING).is_none(),
            "selecting an identity must leave the picker"
        );
        assert!(
            harness.query_by_label(HOME_ONLY_MARKER).is_some(),
            "selecting an identity must land on its Home (tab bar present)"
        );
        // The app-scoped read every operate-as site uses now resolves to Alpha.
        assert_eq!(
            app_context
                .resolve_selected_identity()
                .map(|qi| qi.identity.id()),
            Some(alpha),
            "resolve_selected_identity must return the explicitly selected identity"
        );
    });
}

/// Stale-selection reconcile: a selected id that is not among the loaded
/// identities must NOT render a phantom Home — the hub falls back to the
/// Picker (guards the R4 / stale-id concern end-to-end, complementing the
/// `model::selected_identity::keep_if_loaded` unit test).
#[test]
fn it_switch_stale_selection_falls_back_to_picker() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_hub();
        let app_context = harness.state().current_app_context().clone();

        seed_identity(&app_context, 0xC3, "Reconcile A");
        seed_identity(&app_context, 0xD4, "Reconcile B");

        // Point the selection at an identity that was never loaded.
        app_context.set_selected_identity(Some(Identifier::from([0xEE; 32])));
        harness.run_steps(5);

        assert!(
            harness.query_by_label(PICKER_HEADING).is_some(),
            "a stale (unloaded) selection must not be treated as explicit; the hub stays on the picker"
        );
        assert!(
            harness.query_by_label(HOME_ONLY_MARKER).is_none(),
            "a stale selection must NOT render a phantom Home"
        );
        // The fallback read still yields a real, loaded identity (selected→first).
        assert!(
            app_context.resolve_selected_identity().is_some(),
            "resolve_selected_identity must fall back to a loaded identity, never the stale id"
        );
        assert_ne!(
            app_context
                .resolve_selected_identity()
                .map(|qi| qi.identity.id()),
            Some(Identifier::from([0xEE; 32])),
            "the stale id must never be resolved as the active identity"
        );
    });
}
