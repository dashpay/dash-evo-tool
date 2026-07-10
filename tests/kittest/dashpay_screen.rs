//! Kittest coverage for `DashPayScreen` typed-error routing.
//!
//! F95 regression: the embedded `ContactRequests` classifies a
//! missing-encryption-key failure and surfaces an inline "Add Encryption
//! Key" affordance, but only if `DashPayScreen` forwards `display_task_error`
//! to the active subscreen. Before the fix `DashPayScreen` used the
//! `ScreenLike` default (always `false`), so the classification never ran.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::backend_task::dashpay::errors::DashPayError;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::dashpay::add_contact_screen::AddContactScreen;
use dash_evo_tool::ui::dashpay::contact_requests::ContactRequests;
use dash_evo_tool::ui::dashpay::contacts_list::ContactsList;
use dash_evo_tool::ui::dashpay::profile_screen::ProfileScreen;
use dash_evo_tool::ui::dashpay::profile_search::ProfileSearchScreen;
use dash_evo_tool::ui::dashpay::qr_code_generator::QRCodeGeneratorScreen;
use dash_evo_tool::ui::dashpay::qr_scanner::QRScannerScreen;
use dash_evo_tool::ui::dashpay::send_payment::PaymentHistory;
use dash_evo_tool::ui::dashpay::{DashPayScreen, DashPaySubscreen};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use std::collections::BTreeMap;
use std::sync::Arc;

/// On the Contacts subscreen, a `MissingEncryptionKey` error must route to
/// the embedded `ContactRequests`, which claims it (returns `true`) and
/// arms its inline recovery affordance. An unrelated error must NOT be
/// claimed, so it falls through to the global banner.
#[test]
fn missing_encryption_key_error_routes_to_embedded_contact_requests() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("create AppState")
                .with_animations(false)
        });
        harness.run_steps(3);

        let app_context = harness.state().current_app_context().clone();
        let mut screen = DashPayScreen::new(&app_context, DashPaySubscreen::Contacts);

        let handled =
            screen.display_task_error(&TaskError::DashPay(DashPayError::MissingEncryptionKey));
        assert!(
            handled,
            "Contacts subscreen must claim the missing-encryption-key error so the inline \
             recovery affordance can render"
        );

        let unrelated = screen.display_task_error(&TaskError::DocumentNotFound);
        assert!(
            !unrelated,
            "an unrelated error must fall through to the global banner, not be swallowed"
        );
    });
}

// ── W3 B3 — DashPay screens seed from app-scoped identity ───────────────────

/// Seed a wallet-less identity (mirrors identity_hub_switcher fixture).
fn seed_dp_identity(app_context: &Arc<AppContext>, byte: u8, alias: &str) -> Identifier {
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
        .expect("seed dashpay identity");
    id
}

/// Build a harness and return the live context (wallet backend wired).
fn build_ctx() -> (
    Harness<'static, dash_evo_tool::app::AppState>,
    Arc<AppContext>,
) {
    let mut h = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("AppState builds")
            .with_animations(false)
    });
    h.run_steps(5);
    let ctx = h.state().current_app_context().clone();
    (h, ctx)
}

/// `ContactsList::new()` must default to the app-scoped identity, not identities[0].
///
/// This is the B3 seeding test: verifies that `ContactsList` opens on the
/// identity the user last operated as, not always the first DB row.
///
/// Write-back (picker change → `resolve_selected_identity()` moves) is covered
/// at the component level by
/// `identity_selector::tests::syncing_global_writes_selection_to_app_context`
/// which tests `sync_to_global()` directly without a screen harness.
#[test]
fn contacts_list_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        let _first = seed_dp_identity(&ctx, 0xA1, "CL Alpha");
        let second = seed_dp_identity(&ctx, 0xB2, "CL Beta");

        ctx.set_selected_identity(Some(second));

        let screen = ContactsList::new(ctx.clone());

        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(second),
            "ContactsList must default to the app-scoped selected identity"
        );
    });
}

/// `ContactRequests::new()` must default to the app-scoped identity.
#[test]
fn contact_requests_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        let _first = seed_dp_identity(&ctx, 0xC3, "CR Alpha");
        let second = seed_dp_identity(&ctx, 0xD4, "CR Beta");

        ctx.set_selected_identity(Some(second));

        let screen = ContactRequests::new(ctx.clone());

        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(second),
            "ContactRequests must default to the app-scoped selected identity"
        );
    });
}

/// `PaymentHistory::new()` must default to the app-scoped identity.
#[test]
fn payment_history_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        let _first = seed_dp_identity(&ctx, 0xE5, "PH Alpha");
        let second = seed_dp_identity(&ctx, 0xF6, "PH Beta");

        ctx.set_selected_identity(Some(second));

        let screen = PaymentHistory::new(ctx.clone());

        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(second),
            "PaymentHistory must default to the app-scoped selected identity"
        );
    });
}

/// `ProfileScreen::new()` must default to the app-scoped identity.
#[test]
fn profile_screen_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        let _first = seed_dp_identity(&ctx, 0x11, "PS Alpha");
        let second = seed_dp_identity(&ctx, 0x22, "PS Beta");

        ctx.set_selected_identity(Some(second));

        let screen = ProfileScreen::new(ctx.clone());

        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(second),
            "ProfileScreen must default to the app-scoped selected identity"
        );
    });
}

/// `QRCodeGeneratorScreen::new()` must default to the app-scoped identity.
#[test]
fn qr_generator_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        let _first = seed_dp_identity(&ctx, 0x33, "QRG Alpha");
        let second = seed_dp_identity(&ctx, 0x44, "QRG Beta");

        ctx.set_selected_identity(Some(second));

        let screen = QRCodeGeneratorScreen::new(ctx.clone());

        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(second),
            "QRCodeGeneratorScreen must default to the app-scoped selected identity"
        );
    });
}

/// `QRScannerScreen::new()` must default to the app-scoped identity.
#[test]
fn qr_scanner_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        let _first = seed_dp_identity(&ctx, 0x55, "QRS Alpha");
        let second = seed_dp_identity(&ctx, 0x66, "QRS Beta");

        ctx.set_selected_identity(Some(second));

        let screen = QRScannerScreen::new(ctx.clone());

        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(second),
            "QRScannerScreen must default to the app-scoped selected identity"
        );
    });
}

/// `AddContactScreen::new()` must default to the app-scoped identity (sender).
#[test]
fn add_contact_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        let _first = seed_dp_identity(&ctx, 0x77, "AC Alpha");
        let second = seed_dp_identity(&ctx, 0x88, "AC Beta");

        ctx.set_selected_identity(Some(second));

        let screen = AddContactScreen::new(ctx.clone());

        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(second),
            "AddContactScreen must default to the app-scoped selected identity"
        );
    });
}

/// Seed a wallet-less masternode identity into the local DB.
fn seed_dp_masternode(app_context: &Arc<AppContext>, byte: u8, alias: &str) -> Identifier {
    let pv = PlatformVersion::latest();
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), pv).expect("basic identity");
    let id = identity.id();
    let qi = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::Masternode,
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
        .expect("seed dashpay masternode");
    id
}

/// FR-6 boundary through a DashPay screen: a masternode/evonode must
/// never become selectable in a DashPay identity selector, and so can never be
/// written to the app-global identity via the selector's `syncing_global`
/// write-back. The DashPay selectors source their list from the User-filtered
/// accessor. With ONLY a masternode stored, a DashPay screen must seed no
/// identity (the masternode is not eligible) — before the fix it fell back to
/// the masternode as `identities.first()`, leaking it into the sync path.
#[test]
fn masternode_never_selectable_in_dashpay_screens() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        // Only a masternode is stored — no User identity.
        let mn_id = seed_dp_masternode(&ctx, 0x4D, "mn-dashpay-leak");

        // It IS in the unfiltered store (proving the screens filter, not that
        // the DB is empty).
        assert_eq!(
            ctx.load_local_qualified_identities().expect("load").len(),
            1,
            "the masternode must be present in the unfiltered store"
        );
        assert!(
            ctx.selected_identity_id().is_none(),
            "no app-global identity should be selected initially"
        );

        // Every DashPay screen that carries a `syncing_global` selector must
        // seed NO identity from a masternode-only store.
        assert!(
            ContactsList::new(ctx.clone()).selected_identity.is_none(),
            "ContactsList must not seed a masternode as the selected identity"
        );
        assert!(
            ContactRequests::new(ctx.clone())
                .selected_identity
                .is_none(),
            "ContactRequests must not seed a masternode as the selected identity"
        );
        assert!(
            PaymentHistory::new(ctx.clone()).selected_identity.is_none(),
            "PaymentHistory must not seed a masternode as the selected identity"
        );
        assert!(
            ProfileScreen::new(ctx.clone()).selected_identity.is_none(),
            "ProfileScreen must not seed a masternode as the selected identity"
        );
        assert!(
            AddContactScreen::new(ctx.clone())
                .selected_identity
                .is_none(),
            "AddContactScreen must not seed a masternode as the selected identity"
        );
        assert!(
            QRScannerScreen::new(ctx.clone())
                .selected_identity
                .is_none(),
            "QRScannerScreen must not seed a masternode as the selected identity"
        );
        assert!(
            QRCodeGeneratorScreen::new(ctx.clone())
                .selected_identity
                .is_none(),
            "QRCodeGeneratorScreen must not seed a masternode as the selected identity"
        );

        // ProfileSearchScreen holds no selected identity — it only ever reads
        // identities through the User-filtered accessor (as its profile-viewing
        // context), so a masternode can never surface there. Assert that data
        // source excludes the masternode.
        let _ = ProfileSearchScreen::new(ctx.clone());
        assert!(
            ctx.load_local_user_identities()
                .expect("user identities")
                .is_empty(),
            "the User-filtered accessor ProfileSearchScreen reads must exclude the masternode"
        );

        // The FR-6 invariant: no masternode ever reached the app-global identity.
        assert!(
            ctx.selected_identity_id().is_none(),
            "a masternode must never be written to the app-global identity via DashPay"
        );
        assert_ne!(
            ctx.selected_identity_id(),
            Some(mn_id),
            "the app-global identity must never be the masternode"
        );
    });
}

/// Non-Contacts subscreens have no classifier embedded, so they must not
/// claim the error — it belongs to the global banner there.
#[test]
fn profile_subscreen_does_not_claim_dashpay_errors() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("create AppState")
                .with_animations(false)
        });
        harness.run_steps(3);

        let app_context = harness.state().current_app_context().clone();
        let mut screen = DashPayScreen::new(&app_context, DashPaySubscreen::Profile);

        let handled =
            screen.display_task_error(&TaskError::DashPay(DashPayError::MissingEncryptionKey));
        assert!(
            !handled,
            "Profile subscreen has no embedded classifier and must defer to the global banner"
        );
    });
}
