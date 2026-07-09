//! IT-MN-TAB — Masternodes root tab: Expert-Mode nav gate + live de-gating (B2),
//! empty state + card grid (B3).

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::{RootScreenType, ScreenLike};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Seed one wallet-less masternode/evonode identity into the live per-network
/// identity DB (alias = `alias`, id = `[byte; 32]`, no keys → read-only node).
fn seed_node(app_context: &Arc<AppContext>, byte: u8, alias: &str, node_type: IdentityType) {
    let pv = PlatformVersion::latest();
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), pv).expect("basic identity");
    let _ = identity.id();
    let qi = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: node_type,
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
        .expect("seed masternode insert");
}

/// TC-FR1-01…04 — the Masternodes nav entry is absent when Expert Mode is off
/// and present when it is on. Toggling `enable_developer_mode` flips the gate;
/// the nav rail re-evaluates the per-entry `FeatureGate::DeveloperMode` skip
/// each frame. Counted with `query_all_by_label` because the nav button exposes
/// both a Button and an inner Label node for the same text.
#[test]
fn nav_gated_by_expert_mode() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();

        app_context.enable_developer_mode(false);
        harness.run_steps(3);
        assert_eq!(
            harness.query_all_by_label("Masternodes").count(),
            0,
            "the Masternodes nav entry must be absent when Expert Mode is off"
        );

        app_context.enable_developer_mode(true);
        harness.run_steps(3);
        assert!(
            harness.query_all_by_label("Masternodes").count() >= 1,
            "the Masternodes nav entry must appear when Expert Mode is on"
        );
    });
}

/// TC-EDGE-05/06 (§10.11) — live de-gating: with the Masternodes tab active,
/// flipping Expert Mode off falls the active tab back to the neutral Identities
/// tab (the gated screen is never shown without its gate). Drives the guard in
/// `active_root_screen_mut` directly by selecting the tab, then revoking the
/// gate.
#[test]
fn de_gating_falls_back_to_identities() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();

        // Expert Mode on, select the Masternodes tab — it stays selected.
        app_context.enable_developer_mode(true);
        harness.state_mut().selected_main_screen = RootScreenType::RootScreenMasternodes;
        harness.run_steps(3);
        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenMasternodes,
            "the Masternodes tab stays active while Expert Mode is on"
        );

        // Flip Expert Mode off — the active tab must fall back to Identities.
        app_context.enable_developer_mode(false);
        harness.run_steps(3);
        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenIdentities,
            "de-gating must fall the active tab back to Identities"
        );
    });
}

/// Enable Expert Mode, activate the Masternodes tab, and reload its cached list
/// (the direct field-set bypasses `set_main_screen`, so drive the screen's
/// arrival refresh explicitly — the same call `set_main_screen` makes).
fn activate_masternodes_tab(
    harness: &mut egui_kittest::Harness<'static, dash_evo_tool::app::AppState>,
    app_context: &Arc<AppContext>,
) {
    app_context.enable_developer_mode(true);
    harness.state_mut().selected_main_screen = RootScreenType::RootScreenMasternodes;
    harness
        .state_mut()
        .active_root_screen_mut()
        .refresh_on_arrival();
    harness.run_steps(3);
}

/// TC-FR2-01…05 — with zero nodes loaded the Masternodes tab renders the empty
/// state with the exact canonical §7 copy: heading, body, primary CTA, and the
/// reassurance line.
#[test]
fn empty_state_renders_canonical_copy() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        activate_masternodes_tab(&mut harness, &app_context);

        assert!(
            harness.query_by_label("No masternodes loaded").is_some(),
            "empty-state heading must render (TC-FR2-02)"
        );
        assert!(
            harness
                .query_by_label(
                    "Load a masternode or evonode to vote on DPNS name contests and manage its \
                     owner and payout keys."
                )
                .is_some(),
            "empty-state body copy must render verbatim (TC-FR2-03)"
        );
        assert!(
            harness.query_by_label("Load a masternode").is_some(),
            "empty-state primary CTA must render (TC-FR2-04)"
        );
        assert!(
            harness
                .query_by_label(
                    "Have your node's ProTxHash to hand. Keys are optional — a node loads \
                     read-only without them."
                )
                .is_some(),
            "empty-state reassurance line must render verbatim (TC-FR2-05)"
        );
    });
}

/// TC-FR3-01/15, TC-FR7-01, TC-NFR6-01 — with nodes loaded the grid renders one
/// card per node (not the empty state), each card is a single accessible click
/// target labelled `Open {node}`, the status label pairs with its colour, and
/// the top-right Refresh toolbar button is present.
#[test]
fn card_grid_renders_seeded_nodes() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();

        seed_node(&app_context, 0x91, "mn-east-01", IdentityType::Masternode);
        seed_node(&app_context, 0x92, "evo-west-02", IdentityType::Evonode);
        activate_masternodes_tab(&mut harness, &app_context);

        // Empty state is gone; both node headings render (TC-FR3-01/15).
        assert!(
            harness.query_by_label("No masternodes loaded").is_none(),
            "empty state must not render once nodes are loaded (TC-FR2-07)"
        );
        assert!(
            harness.query_by_label("mn-east-01").is_some(),
            "masternode card heading must render"
        );
        assert!(
            harness.query_by_label("evo-west-02").is_some(),
            "evonode card heading must render"
        );

        // Each card is one accessible click target labelled `Open {node}`
        // (TC-NFR6-01).
        assert!(
            harness.query_by_label("Open mn-east-01").is_some(),
            "card must expose a single accessible `Open {{node}}` label"
        );

        // Status label pairs colour with text — never colour-only (TC-NFR6-03).
        assert!(
            harness.query_all_by_label("Pending Creation").count() >= 1,
            "identity-status label must render as text alongside its dot"
        );

        // Top-right Refresh toolbar button (TC-FR7-01).
        assert!(
            harness.query_all_by_label("Refresh").count() >= 1,
            "Refresh toolbar button must be present on the card list"
        );
    });
}
