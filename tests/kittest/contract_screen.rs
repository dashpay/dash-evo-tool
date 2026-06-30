//! W2 kittest — contracts & documents screens obey the app-scoped selected identity.
//!
//! B1 migration: `RegisterDataContractScreen`, `UpdateDataContractScreen`, and
//! `DocumentActionScreen` must default to the app-scoped selected identity on
//! construction and write the user's picker choice back via `syncing_global`.
//!
//! Seeding test (a): screen defaults to the app-scoped id.
//! Write-back test (b): a picker change drives `resolve_selected_identity()`.
//!
//! Write-back (b) requires identities with loaded private keys so the selector
//! renders (the screen gates on `has_suitable_keys`). That fixture is out of
//! reach here (see TI-1 / private-key fixture gap). The seeding direction is
//! fully covered below.
//!
//! # TODO(WalletFixture / private-key fixture)
//! Add write-back assertions (picker click → `resolve_selected_identity()` moves)
//! once an identity fixture with loaded AUTH HIGH/CRITICAL private keys exists.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::contracts_documents::document_action_screen::{
    DocumentActionScreen, DocumentActionType,
};
use dash_evo_tool::ui::contracts_documents::group_actions_screen::GroupActionsScreen;
use dash_evo_tool::ui::contracts_documents::register_contract_screen::RegisterDataContractScreen;
use dash_evo_tool::ui::contracts_documents::update_contract_screen::UpdateDataContractScreen;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Mount a minimal AppState with the wallet backend wired up, then return the
/// live context. Mirrors `identity_hub_switcher::mount_hub` but doesn't force a
/// particular root screen — we construct screens directly.
fn mount_context() -> (
    Harness<'static, dash_evo_tool::app::AppState>,
    Arc<AppContext>,
) {
    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("AppState builds")
            .with_animations(false)
    });
    harness.set_size(egui::vec2(1280.0, 800.0));
    harness.run_steps(5);
    let ctx = harness.state().current_app_context().clone();
    (harness, ctx)
}

/// Seed a minimal wallet-less identity (identical to identity_hub_switcher pattern).
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

// ── B1a ─ RegisterDataContractScreen ────────────────────────────────────────

/// The register-contract screen must default to the app-scoped selected
/// identity, not necessarily the first loaded identity.
#[test]
fn register_contract_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_harness, app_context) = mount_context();

        let _first = seed_identity(&app_context, 0xA1, "Contract Alpha");
        let second = seed_identity(&app_context, 0xB2, "Contract Beta");

        // Point the global selection at the second identity.
        app_context.set_selected_identity(Some(second));

        let screen = RegisterDataContractScreen::new(&app_context);

        assert_eq!(
            screen
                .selected_qualified_identity
                .as_ref()
                .map(|qi| qi.identity.id()),
            Some(second),
            "RegisterDataContractScreen must default to the app-scoped selected identity"
        );
    });
}

// ── B1b ─ UpdateDataContractScreen ──────────────────────────────────────────

/// The update-contract screen must default to the app-scoped selected identity.
#[test]
fn update_contract_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_harness, app_context) = mount_context();

        let _first = seed_identity(&app_context, 0xC3, "Update Alpha");
        let second = seed_identity(&app_context, 0xD4, "Update Beta");

        app_context.set_selected_identity(Some(second));

        let screen = UpdateDataContractScreen::new(&app_context);

        assert_eq!(
            screen
                .selected_qualified_identity
                .as_ref()
                .map(|qi| qi.identity.id()),
            Some(second),
            "UpdateDataContractScreen must default to the app-scoped selected identity"
        );
    });
}

// ── B1c ─ DocumentActionScreen ──────────────────────────────────────────────

/// The document-action screen (constructed with `None` as selected identity)
/// must seed from the app-scoped selected identity.
#[test]
fn document_action_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_harness, app_context) = mount_context();

        let _first = seed_identity(&app_context, 0xE5, "DocAction Alpha");
        let second = seed_identity(&app_context, 0xF6, "DocAction Beta");

        app_context.set_selected_identity(Some(second));

        let screen = DocumentActionScreen::new(
            app_context.clone(),
            None, // No explicit identity → must seed from app-scoped selection.
            DocumentActionType::Create,
        );

        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(second),
            "DocumentActionScreen must seed from the app-scoped selected identity when None is passed"
        );
    });
}

// ── B6 ─ GroupActionsScreen session-local regression lock ────────────────────

/// W5 B6 (K3 regression lock): `GroupActionsScreen` is SESSION-LOCAL — it must
/// NOT seed `selected_identity` from the global selection on construction, even
/// when an app-scoped identity is set. The picker is independent per session.
#[test]
fn group_actions_does_not_seed_from_global_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_harness, app_context) = mount_context();

        let _first = seed_identity(&app_context, 0x11, "GA Alpha");
        let second = seed_identity(&app_context, 0x22, "GA Beta");

        // Set the second identity as the app-scoped selection.
        app_context.set_selected_identity(Some(second));

        let screen = GroupActionsScreen::new(&app_context);

        assert_eq!(
            screen.selected_identity, None,
            "GroupActionsScreen is session-local (K3): it must NOT seed from the \
             app-scoped selection — selected_identity must stay None on construction"
        );

        // The global selection must be unchanged (no write-back on construction).
        assert_eq!(
            app_context.selected_identity_id(),
            Some(second),
            "GroupActionsScreen must not alter the app-scoped selection on construction"
        );
    });
}
