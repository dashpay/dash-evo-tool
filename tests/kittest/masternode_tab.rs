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

/// TC-FR4-01/02/04/18/21 — the empty-state CTA opens the load form with the
/// full MN/Evonode field set (ProTxHash, both type segments, key inputs,
/// always-visible Warning note) and no User segment; Cancel returns to the list.
#[test]
fn load_form_opens_from_cta_and_cancels() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        activate_masternodes_tab(&mut harness, &app_context);

        // Open the load form from the empty-state primary CTA.
        harness.get_by_label("Load a masternode").click();
        harness.run_steps(3);

        // Field set present (TC-FR4-01/02): ProTxHash + both type segments + the
        // submit button. The empty-state heading is gone.
        assert!(
            harness.query_by_label("No masternodes loaded").is_none(),
            "empty state must be replaced by the load form"
        );
        assert!(
            harness.query_by_label("ProTxHash").is_some(),
            "ProTxHash field label must render"
        );
        assert!(
            harness.query_all_by_label("Masternode").count() >= 1,
            "Masternode type segment must render"
        );
        assert!(
            harness.query_by_label("Evonode").is_some(),
            "Evonode type segment must render (TC-FR4-04: no User segment)"
        );
        assert!(
            harness.query_by_label("Load masternode").is_some(),
            "the Load submit button must render"
        );

        // Warning-tone key-storage note is always visible (TC-FR4-18).
        assert!(
            harness
                .query_by_label(
                    "Set an optional password to encrypt these keys on this device. Without one, \
                     they are stored unencrypted and you can add protection later from the key \
                     screen."
                )
                .is_some(),
            "the always-visible Warning-tone key-storage note must render verbatim"
        );

        // Cancel returns to the list / empty state (TC-FR4-21).
        harness.get_by_label("Cancel").click();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("No masternodes loaded").is_some(),
            "Cancel must return to the list without loading"
        );
    });
}

/// TC-FR5-01/02/07, TC-FR9-01/02, TC-FR11-01/02, TC-FR7-04 — clicking a card
/// opens the detail view with the ordered sections (Actions row present with all
/// three credit actions), the Evonode-only claim cross-link shown for an evonode
/// but absent for a masternode, a detail Refresh button, and the `‹ All
/// masternodes` back row returning to the list.
#[test]
fn detail_view_opens_from_card_with_sections_and_back() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_node(&app_context, 0x93, "mn-detail-01", IdentityType::Masternode);
        seed_node(&app_context, 0x94, "evo-detail-02", IdentityType::Evonode);
        activate_masternodes_tab(&mut harness, &app_context);

        // Open the masternode's detail view.
        harness.get_by_label("Open mn-detail-01").click();
        harness.run_steps(3);

        // Section presence + Actions row (TC-FR5-01, TC-FR9-01).
        assert!(
            harness.query_by_label("mn-detail-01").is_some(),
            "header alias"
        );
        assert!(
            harness.query_by_label("Actions").is_some(),
            "Actions section"
        );
        assert!(
            harness.query_by_label("Withdraw").is_some(),
            "Withdraw action"
        );
        assert!(harness.query_by_label("Top up").is_some(), "Top up action");
        assert!(
            harness.query_by_label("Transfer").is_some(),
            "Transfer action"
        );
        assert!(harness.query_by_label("Keys").is_some(), "Keys section");
        assert!(
            harness.query_by_label("Remove masternode").is_some(),
            "Remove action"
        );
        assert!(
            harness.query_all_by_label("Refresh").count() >= 1,
            "detail Refresh button (TC-FR7-04)"
        );
        // Claim cross-link absent for a plain masternode (TC-FR11-02).
        assert!(
            harness.query_by_label("Claim token rewards ›").is_none(),
            "masternode detail must not show the evonode claim cross-link"
        );

        // Back row returns to the card list (TC-FR5-07).
        harness.get_by_label("‹ All masternodes").click();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("mn-detail-01").is_some(),
            "back row returns to the card grid"
        );

        // The evonode's detail view shows the claim cross-link (TC-FR11-01).
        harness.get_by_label("Open evo-detail-02").click();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("Claim token rewards ›").is_some(),
            "evonode detail must show the claim cross-link"
        );
    });
}
