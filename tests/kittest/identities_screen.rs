use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::{RootScreenType, ScreenLike};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Test that the identities screen can be rendered
#[test]
fn test_identities_screen_renders() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(1024.0, 768.0));
        harness.run_steps(10);
    });
}

/// Test that the app renders correctly at minimum size
#[test]
fn test_minimum_window_size() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        // Test with a small window size
        harness.set_size(egui::vec2(400.0, 300.0));
        harness.run_steps(10);
    });
}

/// Test that the app handles resize gracefully
#[test]
fn test_window_resize() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        // Start small
        harness.set_size(egui::vec2(640.0, 480.0));
        harness.run_steps(5);

        // Resize larger
        harness.set_size(egui::vec2(1280.0, 720.0));
        harness.run_steps(5);

        // Resize smaller again
        harness.set_size(egui::vec2(800.0, 600.0));
        harness.run_steps(5);
    });
}

/// Test multiple frame batches
#[test]
fn test_frame_batch_processing() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(150).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(1024.0, 768.0));

        // Process frames in batches
        for batch in 0..10 {
            harness.run_steps(10);
            // Just ensure we can run multiple batches without error
            let _ = batch;
        }
    });
}

/// Seed one keyless user identity into the live per-network identity DB so the
/// list renders a row with its action buttons.
fn seed_user_identity(app_context: &Arc<AppContext>, byte: u8, alias: &str) {
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), PlatformVersion::latest())
            .expect("basic identity");
    let qualified_identity = QualifiedIdentity {
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
        status: IdentityStatus::Active,
        network: app_context.network(),
    };
    app_context
        .insert_local_qualified_identity(&qualified_identity, &None)
        .expect("seed user identity");
}

/// The list's Remove confirmation is destructive and irreversible, so it must
/// carry the same specific verbs as the Identity Hub's unload confirmation —
/// generic Yes/No labels are forbidden for destructive actions.
#[test]
fn remove_confirmation_uses_specific_verbs() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_user_identity(&app_context, 0xB1, "list-remove-verbs");
        harness
            .state_mut()
            .active_root_screen_mut()
            .refresh_on_arrival();
        harness.run_steps(3);

        harness.get_by_label("Remove").click();
        harness.run_steps(3);

        assert!(
            harness.query_by_label("Permanently unload").is_some(),
            "the confirm button must name the action it performs"
        );
        assert!(
            harness.query_by_label("Keep identity").is_some(),
            "the cancel button must name the outcome of cancelling"
        );
        assert!(
            harness.query_by_label("Yes").is_none(),
            "a destructive confirmation must not offer a generic Yes"
        );
        assert!(
            harness.query_by_label("No").is_none(),
            "a destructive confirmation must not offer a generic No"
        );
    });
}
