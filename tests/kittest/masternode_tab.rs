//! IT-MN-TAB — Masternodes root tab: Expert-Mode nav gate + live de-gating (B2),
//! empty state + card grid (B3).

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::{RootScreenType, ScreenLike};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
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

/// Seed a masternode that has an associated voter identity, inserting BOTH the
/// node and its (separately stored) voter identity into the local DB. Returns
/// the voter identity's id so a test can assert its deletion. The node id is
/// `[byte; 32]`; the voter id is `[byte ^ 0xFF; 32]`.
fn seed_node_with_voter(app_context: &Arc<AppContext>, byte: u8, alias: &str) -> Identifier {
    let pv = PlatformVersion::latest();
    let voter_id = Identifier::from([byte ^ 0xFF; 32]);
    let voter_identity =
        Identity::create_basic_identity(voter_id, pv).expect("voter basic identity");
    let voter_key = dash_sdk::platform::IdentityPublicKey::random_key(1, Some(1), pv);

    // The voter identity is stored as its own local record so its deletion on
    // node removal is observable.
    let voter_qi = QualifiedIdentity {
        identity: voter_identity.clone(),
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some(format!("{alias}-voter")),
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
        .insert_local_qualified_identity(&voter_qi, &None)
        .expect("seed voter insert");

    let node_identity = Identity::create_basic_identity(Identifier::from([byte; 32]), pv)
        .expect("node basic identity");
    let node_qi = QualifiedIdentity {
        identity: node_identity,
        associated_voter_identity: Some((voter_identity, voter_key)),
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
        .insert_local_qualified_identity(&node_qi, &None)
        .expect("seed node-with-voter insert");
    voter_id
}

/// Seed a masternode whose associated voter identity carries one public key,
/// so the detail view's "Manage keys" list renders a deterministic
/// "Voting key ›" button (voter-target keys are always labelled "Voting").
fn seed_node_with_voter_key(app_context: &Arc<AppContext>, byte: u8, alias: &str) {
    let pv = PlatformVersion::latest();
    let mut voter_identity =
        Identity::create_basic_identity(Identifier::from([byte ^ 0xFF; 32]), pv)
            .expect("voter basic identity");
    let voter_key = dash_sdk::platform::IdentityPublicKey::random_key(1, Some(1), pv);
    voter_identity.add_public_key(voter_key.clone());

    let node_identity = Identity::create_basic_identity(Identifier::from([byte; 32]), pv)
        .expect("node basic identity");
    let node_qi = QualifiedIdentity {
        identity: node_identity,
        associated_voter_identity: Some((voter_identity, voter_key)),
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
        .insert_local_qualified_identity(&node_qi, &None)
        .expect("seed node-with-voter-key insert");
}

/// Seed a masternode with one locally held key. Insertion migrates the clear
/// key into the keyless vault, matching a loaded node with no password.
fn seed_node_with_unprotected_held_key(app_context: &Arc<AppContext>, byte: u8, alias: &str) {
    let pv = PlatformVersion::latest();
    let key = IdentityPublicKey::random_key(1, Some(1), pv);
    let identity = Identity::new_with_id_and_keys(
        Identifier::from([byte; 32]),
        BTreeMap::from([(key.id(), key.clone())]),
        pv,
    )
    .expect("masternode identity with key");
    let private_keys = KeyStorage {
        private_keys: BTreeMap::from([(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, key.id()),
            (
                QualifiedIdentityPublicKey::from(key),
                PrivateKeyData::Clear([byte; 32]),
            ),
        )]),
    };
    let node_qi = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::Masternode,
        alias: Some(alias.to_string()),
        private_keys,
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: app_context.network(),
    };
    app_context
        .insert_local_qualified_identity(&node_qi, &None)
        .expect("seed node-with-unprotected-key insert");
}

/// TC-FR1-01…04 — the Masternodes nav entry is absent below the Power role and
/// present at Power or above. Raising the role flips the gate; the nav rail
/// re-evaluates the per-entry `FeatureGate::Masternodes` skip each frame.
/// Counted with `query_all_by_label` because the nav button exposes both a
/// Button and an inner Label node for the same text.
#[test]
fn nav_gated_by_expert_mode() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();

        app_context.set_user_role(UserRole::Everyday);
        harness.run_steps(3);
        assert_eq!(
            harness.query_all_by_label("Masternodes").count(),
            0,
            "the Masternodes nav entry must be absent below the Power role"
        );

        app_context.set_user_role(UserRole::Power);
        harness.run_steps(3);
        assert!(
            harness.query_all_by_label("Masternodes").count() >= 1,
            "the Masternodes nav entry must appear at the Power role"
        );
    });
}

/// TC-EDGE-05/06 (§10.11) — live de-gating: with the Masternodes tab active,
/// flipping Expert Mode off falls the active tab back to the Identities hub
/// (the gated screen is never shown without its gate). Drives the guard in
/// `active_root_screen_mut` directly by selecting the tab, then revoking the
/// gate.
///
/// The fallback must land on a tab the nav still carries. It used to land on
/// the legacy standalone `Identities` screen, which no nav entry points at any
/// more — a de-gated user was stranded there with no highlighted nav entry and
/// no way back, the same dead end the "Just Explore" landing hit.
#[test]
fn de_gating_falls_back_to_identity_hub() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();

        // Power role on, select the Masternodes tab — it stays selected.
        app_context.set_user_role(UserRole::Power);
        harness.state_mut().selected_main_screen = RootScreenType::RootScreenMasternodes;
        harness.run_steps(3);
        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenMasternodes,
            "the Masternodes tab stays active at the Power role"
        );

        // Drop below Power — the active tab must fall back to the Identities hub.
        app_context.set_user_role(UserRole::Everyday);
        harness.run_steps(3);
        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenIdentityHub,
            "de-gating must fall the active tab back to the Identities hub, the \
             single identity entry the nav still carries"
        );
    });
}

/// Raise to the Power role, activate the Masternodes tab, and reload its cached
/// list (the direct field-set bypasses `set_main_screen`, so drive the screen's
/// arrival refresh explicitly — the same call `set_main_screen` makes).
fn activate_masternodes_tab(
    harness: &mut egui_kittest::Harness<'static, dash_evo_tool::app::AppState>,
    app_context: &Arc<AppContext>,
) {
    app_context.set_user_role(UserRole::Power);
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

        // Status label pairs colour with text — never colour-only (TC-NFR6-03) —
        // and names the subject explicitly, so the dot cannot be misread as Core
        // or PoSe health. The tab is Power-gated, so the status always renders here.
        assert!(
            harness
                .query_all_by_label("Platform identity: Pending Creation")
                .count()
                >= 1,
            "identity-status label must render as text alongside its dot, naming Platform identity"
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

        // Cancel returns to the list / empty state (TC-FR4-21). The Cancel
        // button sits at the bottom of the scrollable form; give the harness a
        // taller window so the whole form (back link + fields + actions row)
        // fits and the button is reachable in this headless viewport.
        harness.set_size(egui::vec2(1280.0, 1200.0));
        harness.run_steps(2);
        harness.get_by_label("Cancel").click();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("No masternodes loaded").is_some(),
            "Cancel must return to the list without loading"
        );
    });
}

/// The load form carries the same `‹ All masternodes` back link as the detail
/// view (wireframe C): it renders at the top of the form and returns to the list
/// without loading anything. This is the always-visible navigation affordance,
/// distinct from the bottom Cancel button.
#[test]
fn load_form_back_link_returns_to_list() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        activate_masternodes_tab(&mut harness, &app_context);

        // Open the load form from the empty-state primary CTA.
        harness.get_by_label("Load a masternode").click();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("No masternodes loaded").is_none(),
            "empty state must be replaced by the load form"
        );

        // The back link is present at the top of the form and returns to the
        // list (empty state) without loading.
        assert!(
            harness.query_by_label("‹ All masternodes").is_some(),
            "the load form must render the `‹ All masternodes` back link"
        );
        harness.get_by_label("‹ All masternodes").click();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("No masternodes loaded").is_some(),
            "the back link must return to the list without loading"
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

        // Section presence + Actions row (TC-FR5-01, TC-FR9-01). The alias shows
        // both in the header and the page-scoped pill, so count ≥ 1.
        assert!(
            harness.query_all_by_label("mn-detail-01").count() >= 1,
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
        assert!(
            harness.query_by_label("Top up").is_none(),
            "Top up action was removed from the masternode detail screen"
        );
        assert!(
            harness.query_by_label("Transfer").is_none(),
            "Transfer action was removed from the masternode detail screen"
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
            harness.query_all_by_label("mn-detail-01").count() >= 1,
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

/// TC-DPNS-01/02/09/10 — the DPNS section is collapsed by default (its body is
/// not rendered), the header carries the open-contest count, and for a node with
/// no voter identity the expanded section shows the actionable missing-voter
/// message with an `Add voting key` action that opens a scoped in-place prompt
/// (not FR-4's load form — no ProTxHash field).
#[test]
fn dpns_section_missing_voter_scoped_prompt() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_node(&app_context, 0x95, "mn-vote-01", IdentityType::Masternode);
        activate_masternodes_tab(&mut harness, &app_context);
        harness.get_by_label("Open mn-vote-01").click();
        harness.run_steps(3);

        // With no voter key the "Add voting key" CTA and its
        // actionable message are rendered ABOVE, outside the collapsed-by-default
        // DPNS section, so they are visible immediately without expanding
        // anything. The empty DPNS header is omitted in this state (no contests
        // are possible without a voter).
        assert!(
            harness
                .query_by_label("DPNS name contests to vote on (0)")
                .is_none(),
            "the empty DPNS header must be omitted when the node has no voter key"
        );
        assert!(
            harness
                .query_by_label(
                    "This node has no voting key loaded. Add its voting private key to cast votes."
                )
                .is_some(),
            "the actionable missing-voter message must be visible without expanding"
        );
        assert!(
            harness.query_by_label("Add voting key").is_some(),
            "missing-voter state must offer an Add voting key action"
        );

        // Click Add voting key → scoped in-place prompt (Save/Cancel), NOT the
        // load form (no ProTxHash field) (TC-DPNS-10/11).
        harness.get_by_label("Add voting key").click();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("Save").is_some(),
            "scoped voter-key prompt must open with a Save action"
        );
        assert!(
            harness.query_by_label("ProTxHash").is_none(),
            "the scoped prompt must not be FR-4's load form (no ProTxHash re-entry)"
        );
    });
}

/// TC-NAV-12 / TC-FR6-07 (release-blocking) — selecting a masternode on the
/// Masternodes page (opening its detail via a card click) must NEVER write the
/// app-global identity selection. With no User identity loaded, the
/// app-global selection must stay `None` even after a masternode is in view, and
/// stay `None` after navigating away to Identities/Identity Hub.
#[test]
fn masternode_selection_never_leaks_to_app_global_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_node(&app_context, 0x96, "mn-leak-01", IdentityType::Masternode);

        // Baseline: no app-global identity selected.
        assert!(
            app_context.selected_identity_id().is_none(),
            "no app-global identity should be selected initially"
        );

        // Open the masternode's detail — this sets the page-scoped selection.
        activate_masternodes_tab(&mut harness, &app_context);
        harness.get_by_label("Open mn-leak-01").click();
        harness.run_steps(3);

        // FR-6 boundary: the masternode must NOT have become the app-global
        // identity, nor resolve as one.
        assert!(
            app_context.selected_identity_id().is_none(),
            "masternode selection must not write the app-global identity id"
        );
        assert!(
            app_context.resolve_selected_identity().is_none(),
            "a masternode must never resolve as the app-global identity"
        );

        // Navigate away to Identities and Identity Hub — still no leak.
        harness.state_mut().selected_main_screen = RootScreenType::RootScreenIdentities;
        harness.run_steps(3);
        assert!(
            app_context.selected_identity_id().is_none(),
            "the app-global identity stays unset on Identities after MN selection"
        );

        harness.state_mut().selected_main_screen = RootScreenType::RootScreenIdentityHub;
        harness.run_steps(3);
        assert!(
            app_context.resolve_selected_identity().is_none(),
            "the app-global identity stays unset on the Hub after MN selection"
        );
    });
}

/// TC-US4-01/02/06/07 — the detail Remove flow: the danger button opens a
/// confirmation with the `Remove masternode` verb; confirming deletes only the
/// target node (its card disappears) and leaves other nodes intact (isolation).
#[test]
fn remove_flow_deletes_only_target_node() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_node(&app_context, 0x97, "mn-remove-me", IdentityType::Masternode);
        seed_node(&app_context, 0x98, "mn-keep-me", IdentityType::Masternode);
        activate_masternodes_tab(&mut harness, &app_context);

        // Two nodes present in the DB.
        assert_eq!(
            app_context
                .load_local_masternode_identities()
                .expect("load")
                .len(),
            2
        );

        // Open the target node's detail and click the danger Remove button
        // (unique before the dialog opens).
        harness.get_by_label("Open mn-remove-me").click();
        harness.run_steps(3);
        harness.get_by_label("Remove masternode").click();
        harness.run_steps(3);

        // The confirmation is open with the `Remove masternode` verb; confirm by
        // clicking the danger button (last node with that label — the section
        // button, dialog title, and confirm button all share the verb).
        let confirm = harness
            .query_all_by_label("Remove masternode")
            .last()
            .expect("confirm button present");
        confirm.click();
        harness.run_steps(3);

        // Only the target node was deleted; the other remains (isolation).
        let remaining = app_context
            .load_local_masternode_identities()
            .expect("load");
        assert_eq!(remaining.len(), 1, "exactly one node removed");
        assert_eq!(
            remaining[0].alias.as_deref(),
            Some("mn-keep-me"),
            "the untargeted node must survive"
        );
        assert!(
            harness.query_by_label("mn-remove-me").is_none(),
            "removed node's card must be gone"
        );
        assert!(
            harness.query_all_by_label("mn-keep-me").count() >= 1,
            "kept node's card must remain"
        );
    });
}

/// TC-US4-05 — removing a node also deletes its associated voter identity from
/// local storage, not just the node record. Seeds a masternode whose voter
/// identity is stored separately, removes the node through the confirm flow,
/// and asserts both the node and the voter identity are gone.
#[test]
fn remove_flow_deletes_associated_voter_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        let voter_id = seed_node_with_voter(&app_context, 0xA1, "mn-with-voter");
        activate_masternodes_tab(&mut harness, &app_context);

        // Precondition: both the node and its voter identity are stored.
        assert_eq!(
            app_context
                .load_local_masternode_identities()
                .expect("load")
                .len(),
            1,
            "the masternode must be seeded"
        );
        assert!(
            app_context
                .get_local_qualified_identity(&voter_id)
                .expect("voter read")
                .is_some(),
            "the voter identity must be seeded"
        );

        // Open the node's detail and confirm removal.
        harness.get_by_label("Open mn-with-voter").click();
        harness.run_steps(3);
        harness.get_by_label("Remove masternode").click();
        harness.run_steps(3);
        let confirm = harness
            .query_all_by_label("Remove masternode")
            .last()
            .expect("confirm button present");
        confirm.click();
        harness.run_steps(3);

        // Both the node and its voter identity are deleted.
        assert_eq!(
            app_context
                .load_local_masternode_identities()
                .expect("load")
                .len(),
            0,
            "the masternode must be removed"
        );
        assert!(
            app_context
                .get_local_qualified_identity(&voter_id)
                .expect("voter read")
                .is_none(),
            "the associated voter identity must be removed too (TC-US4-05)"
        );
    });
}

/// Execution-level: clicking a per-key "Manage keys" button in the masternode
/// detail view opens the interactive `KeyInfoScreen` (not the static read-only
/// `KeysScreen`). Seeds a node whose voter identity carries one key so
/// a deterministic "Voting key ›" button renders, clicks it, and asserts a
/// `KeyInfoScreen` is pushed and its "Key Information" heading renders.
#[test]
fn manage_keys_button_opens_key_info_screen() {
    use dash_evo_tool::ui::Screen;

    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_node_with_voter_key(&app_context, 0x71, "mn-keys-01");
        activate_masternodes_tab(&mut harness, &app_context);

        harness.get_by_label("Open mn-keys-01").click();
        harness.run_steps(3);

        // The per-key "Manage keys" list renders a Voting key button.
        assert!(
            harness.query_by_label("Voting key ›").is_some(),
            "the Keys section must render a per-key 'Voting key ›' button"
        );
        // No KeyInfoScreen is on the stack yet.
        assert!(
            !harness
                .state()
                .screen_stack
                .iter()
                .any(|s| matches!(s, Screen::KeyInfoScreen(_))),
            "no KeyInfoScreen should be open before the click"
        );

        harness.get_by_label("Voting key ›").click();
        harness.run_steps(3);

        // Clicking it pushes the interactive KeyInfoScreen.
        assert!(
            matches!(
                harness.state().screen_stack.last(),
                Some(Screen::KeyInfoScreen(_))
            ),
            "clicking a per-key button must push KeyInfoScreen"
        );
        assert!(
            harness.query_by_label("Key Information").is_some(),
            "the pushed KeyInfoScreen must render its 'Key Information' heading"
        );
    });
}

/// Clicking the aggregate protection CTA opens `KeyInfoScreen` with its
/// confirmation dialog already visible, without a second click in that screen.
#[test]
fn add_password_protection_opens_confirmation_dialog() {
    use dash_evo_tool::ui::Screen;

    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_node_with_unprotected_held_key(&app_context, 0x72, "mn-protect-01");
        activate_masternodes_tab(&mut harness, &app_context);

        harness.get_by_label("Open mn-protect-01").click();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("Add password protection…").is_some(),
            "an unprotected vault key must offer the aggregate protection action"
        );

        harness.get_by_label("Add password protection…").click();
        harness.run_steps(3);

        assert!(
            matches!(
                harness.state().screen_stack.last(),
                Some(Screen::KeyInfoScreen(_))
            ),
            "the aggregate protection action must push KeyInfoScreen"
        );
        assert!(
            harness
                .query_by_label("Protect this identity's keys with a password?")
                .is_some(),
            "the protection confirmation dialog must be visible immediately"
        );
        assert!(
            harness.query_by_label("Yes, add protection").is_some(),
            "the visible dialog must expose its confirmation action"
        );
    });
}

#[test]
fn left_nav_return_to_masternodes_resets_detail_to_list() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_node(
            &app_context,
            0x72,
            "mn-nav-reset-01",
            IdentityType::Masternode,
        );
        activate_masternodes_tab(&mut harness, &app_context);

        harness.get_by_label("Open mn-nav-reset-01").click();
        harness.run_steps(3);
        assert!(harness.query_by_label("‹ All masternodes").is_some());

        harness
            .get_by_role_and_label(Role::Button, "Wallets")
            .click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(Role::Button, "Masternodes")
            .click();
        harness.run_steps(3);

        assert!(
            harness.query_by_label("Open mn-nav-reset-01").is_some(),
            "returning through the left nav must show the masternode list"
        );
        assert!(harness.query_by_label("‹ All masternodes").is_none());
    });
}

#[test]
fn active_masternodes_left_nav_resets_detail_to_list() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_node(
            &app_context,
            0x74,
            "mn-active-nav-reset-01",
            IdentityType::Masternode,
        );
        activate_masternodes_tab(&mut harness, &app_context);

        harness.get_by_label("Open mn-active-nav-reset-01").click();
        harness.run_steps(3);
        assert!(harness.query_by_label("‹ All masternodes").is_some());

        harness
            .get_by_role_and_label(Role::Button, "Masternodes")
            .click();
        harness.run_steps(3);

        assert!(
            harness
                .query_by_label("Open mn-active-nav-reset-01")
                .is_some(),
            "clicking the already-active Masternodes nav entry must show the list"
        );
        assert!(harness.query_by_label("‹ All masternodes").is_none());
    });
}

#[test]
fn left_nav_return_preserves_pending_masternode_load_form_fields() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .expect("Failed to create tokio runtime");
        let _guard = rt.enter();
        let (blocking_release, blocking_wait) = std::sync::mpsc::channel::<()>();
        let _blocking_task = tokio::task::spawn_blocking(move || blocking_wait.recv());

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        activate_masternodes_tab(&mut harness, &app_context);

        harness.get_by_label("Load a masternode").click();
        harness.set_size(egui::vec2(1280.0, 1200.0));
        harness.run_steps(3);

        harness.get_by_label("Evonode").click();
        harness.run_steps(2);
        assert_eq!(
            harness.get_by_label("Evonode").accesskit_node().toggled(),
            Some(Toggled::True),
            "the non-default node type must be selected before submission"
        );

        let identity_id = Identifier::from([0x75; 32]);
        let pro_tx_hash = identity_id.to_string(Encoding::Hex);
        harness
            .query_all_by_role(Role::TextInput)
            .next()
            .expect("ProTxHash input")
            .focus();
        harness.event(egui::Event::Text(pro_tx_hash.clone()));
        harness.step();

        let alias = "mn-pending-nav-01";
        harness
            .query_all_by_role(Role::TextInput)
            .nth(1)
            .expect("alias input")
            .focus();
        harness.event(egui::Event::Text(alias.to_owned()));
        harness.step();
        assert!(
            harness.query_all_by_value(&pro_tx_hash).count() >= 1,
            "the ProTxHash must be entered before submission"
        );
        assert!(
            harness.query_all_by_value(alias).count() >= 1,
            "the alias must be entered before submission"
        );

        harness.get_by_label("Load masternode").click();
        harness.step();
        assert!(
            app_context
                .mark_identity_load_submitted(identity_id)
                .is_none(),
            "the load must be pending before navigation"
        );

        harness
            .get_by_role_and_label(Role::Button, "Wallets")
            .click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(Role::Button, "Masternodes")
            .click();
        harness.run_steps(3);

        assert!(
            harness.query_by_label("ProTxHash").is_some(),
            "returning while a load is pending must keep the load form open"
        );
        assert!(
            harness.query_all_by_value(&pro_tx_hash).count() >= 1,
            "the pending load must retain its entered ProTxHash"
        );
        assert!(
            harness.query_all_by_value(alias).count() >= 1,
            "the pending load must retain its entered alias"
        );
        assert_eq!(
            harness.get_by_label("Evonode").accesskit_node().toggled(),
            Some(Toggled::True),
            "the pending load must retain its entered node type"
        );

        drop(blocking_release);
        for _ in 0..10 {
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
}

#[test]
fn go_to_main_screen_from_key_info_preserves_masternode_detail() {
    use dash_evo_tool::ui::Screen;

    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_node_with_voter_key(&app_context, 0x73, "mn-nav-back-01");
        activate_masternodes_tab(&mut harness, &app_context);

        harness.get_by_label("Open mn-nav-back-01").click();
        harness.run_steps(3);
        harness.get_by_label("Voting key ›").click();
        harness.run_steps(3);
        assert!(matches!(
            harness.state().screen_stack.last(),
            Some(Screen::KeyInfoScreen(_))
        ));

        harness
            .query_all_by_role_and_label(Role::Button, "Identities")
            .min_by(|left, right| left.rect().top().total_cmp(&right.rect().top()))
            .expect("Key Info breadcrumb")
            .click();
        harness.run_steps(3);

        assert!(harness.state().screen_stack.is_empty());
        assert!(
            harness.query_by_label("‹ All masternodes").is_some(),
            "returning from Key Info must preserve the open detail view"
        );
        assert!(harness.query_by_label("Open mn-nav-back-01").is_none());
    });
}

/// Seed a masternode whose voter identity carries a key whose purpose is **not**
/// `VOTING`, with its private half filed structurally on the voter identity.
///
/// This is the shape where the two target conventions disagree: the structural
/// target is the voter identity, while `impl From<Purpose> for PrivateKeyTarget`
/// sends anything that is not `VOTING` to the main identity. It is the shape
/// `role_label_and_tip`'s voter-identity override exists for — a key on a voter
/// identity is the voting key whatever its purpose field says.
fn seed_node_with_non_voting_purpose_voter_key(
    app_context: &Arc<AppContext>,
    byte: u8,
    alias: &str,
) {
    let pv = PlatformVersion::latest();
    // Purpose::AUTHENTICATION (0), not VOTING.
    let voter_key = IdentityPublicKey::random_key(1, Some(1), pv);
    assert_eq!(
        voter_key.purpose(),
        dash_sdk::dpp::identity::Purpose::AUTHENTICATION,
        "the premise: this key's purpose derives to the MAIN identity, while it \
         actually sits on the voter identity"
    );

    let mut voter_identity =
        Identity::create_basic_identity(Identifier::from([byte ^ 0xFF; 32]), pv)
            .expect("voter basic identity");
    voter_identity.add_public_key(voter_key.clone());

    let node_identity = Identity::create_basic_identity(Identifier::from([byte; 32]), pv)
        .expect("node basic identity");
    let node_qi = QualifiedIdentity {
        identity: node_identity,
        associated_voter_identity: Some((voter_identity, voter_key.clone())),
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::Masternode,
        alias: Some(alias.to_string()),
        private_keys: KeyStorage {
            private_keys: BTreeMap::from([(
                (PrivateKeyTarget::PrivateKeyOnVoterIdentity, voter_key.id()),
                (
                    QualifiedIdentityPublicKey::from(voter_key),
                    PrivateKeyData::Clear([byte; 32]),
                ),
            )]),
        },
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::PendingCreation,
        network: app_context.network(),
    };
    app_context
        .insert_local_qualified_identity(&node_qi, &None)
        .expect("seed node-with-non-voting-voter-key insert");
}

/// AC-6 on the masternode path: the detail view's key list and the Key Info page
/// it opens must call the same key the same thing.
///
/// The row knows where the material is because it looked there. The page it
/// opens, left to itself, re-derives that location from the key's purpose — which
/// disagrees for a voter-identity key whose purpose is not `VOTING`. The list
/// says "Voting key" (correct: a key on a voter identity is the voting key) and
/// the page said "Authentication key", one click apart, about one key. The same
/// wrong target also drives the page's own re-read, so it loses the private half
/// of a key the device holds.
#[test]
fn key_info_names_a_voter_key_as_the_masternode_key_list_does() {
    use dash_evo_tool::ui::Screen;

    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();
        seed_node_with_non_voting_purpose_voter_key(&app_context, 0x9a, "mn-voter-label-01");
        activate_masternodes_tab(&mut harness, &app_context);
        harness.get_by_label("Open mn-voter-label-01").click();
        harness.run_steps(3);

        // What the list calls it: the voter-identity override, from the
        // structural target.
        assert!(
            harness.query_by_label("Voting key \u{203a}").is_some(),
            "a key on the voter identity is the node's voting key, whatever its \
             purpose field says"
        );

        harness.get_by_label("Voting key \u{203a}").click();
        harness.run_steps(3);
        assert!(
            matches!(
                harness.state().screen_stack.last(),
                Some(Screen::KeyInfoScreen(_))
            ),
            "the row must open Key Info"
        );

        // What the page one click later calls it. The masternode surface is
        // Expert-gated, so the raw purpose is appended here — which is exactly
        // what makes the disagreement legible: purpose AUTHENTICATION, role
        // Voting.
        assert!(
            harness
                .query_by_label("Voting key (AUTHENTICATION)")
                .is_some(),
            "Key Info must name the key as the list that opened it does"
        );
        assert!(
            harness
                .query_by_label("Authentication key (AUTHENTICATION)")
                .is_none(),
            "and must not rename it by re-deriving its location from its purpose"
        );

        // The same wrong target would also lose the held private half on the
        // page's own re-read.
        let Some(Screen::KeyInfoScreen(key_info)) = harness.state().screen_stack.last() else {
            panic!("Key Info must be the open screen");
        };
        assert!(
            key_info.private_key_data.is_some(),
            "Key Info must keep the private half the row found for it"
        );
    });
}
