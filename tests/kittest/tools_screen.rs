//! W5 kittest — grovestark (GroveSTARK screen) READ-only EdDSA-guarded seeding.
//!
//! B5 migration: `GroveSTARKScreen::new()` seeds `selected_identity` from the
//! app-scoped selection **only** if the identity is in the EdDSA-filtered list
//! (READ-only R4: no syncing_global).
//!
//! Negative guard: basic identities (no EdDSA keys) must NOT be seeded, even if
//! they are the app-scoped selection. `selected_identity` stays `None`.
//!
//! Positive guard (seed when EdDSA keys present): deferred — requires a QI fixture
//! with an EDDSA_25519_HASH160 key loaded in `private_keys`.
//!
//! # TODO(WalletFixture / EdDSA-key fixture)
//! Add positive seed assertion once an identity fixture with a loaded EdDSA key
//! (EDDSA_25519_HASH160, AUTH or TRANSFER purpose) exists.
//!
//! # Note — `create_asset_lock_screen` (READ-only R1)
//! `CreateAssetLockScreen` uses `IdentitySelector::with_app_default()`, whose
//! membership guard is already covered by `IdentitySelector` unit tests
//! (lines 347 and 360 in `src/ui/components/identity_selector.rs`). No additional
//! kittest is added here to avoid duplicating component-level coverage.
//!
//! # Note — `send_screen` (READ-only R1)
//! `SendScreen` seeds `selected_identity` at render-time from the wallet-membership
//! list. Testing requires a HD wallet fixture (TI-1 gap); deferred.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::tools::grovestark_screen::GroveSTARKScreen;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use std::collections::BTreeMap;
use std::sync::Arc;

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

fn seed_basic_identity(app_context: &Arc<AppContext>, byte: u8, alias: &str) -> Identifier {
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
        .expect("seed identity");
    id
}

/// W5 B5 (R4 negative guard): when the app-scoped identity has NO EdDSA keys,
/// `GroveSTARKScreen` must NOT seed it into `selected_identity` — the EdDSA-only
/// filter rejects it and falls back to `None` (no EdDSA identities at all).
#[test]
fn grovestark_does_not_seed_non_eddsa_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        // Both identities are basic (ECDSA, no EdDSA keys).
        let _first = seed_basic_identity(&ctx, 0xA1, "GS Alpha");
        let second = seed_basic_identity(&ctx, 0xB2, "GS Beta");

        // Point the global selection at the second identity (ECDSA only).
        ctx.set_selected_identity(Some(second));

        let screen = GroveSTARKScreen::new(&ctx);

        assert_eq!(
            screen.selected_identity,
            None,
            "GroveSTARKScreen must not seed an identity that has no EdDSA keys (R4 guard)"
        );
    });
}
