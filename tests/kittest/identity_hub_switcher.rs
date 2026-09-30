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
//! Regression coverage includes wallet-scoped picker navigation, card layout,
//! pointer cursors, and scrolling long identity lists.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::RootScreenType;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The hub tab bar's `Activity` tab label only renders in the Home view, never
/// in the Picker or Onboarding surfaces — a reliable "we are on Home" marker.
const HOME_ONLY_MARKER: &str = "Activity";
/// The Picker grid heading (verbatim, picker.rs / design-spec §B.14).
const PICKER_HEADING: &str = "Pick an identity";

/// Seed one wallet-less basic identity (alias = `alias`, id = `[byte; 32]`)
/// into the live per-network identity DB, and return its `Identifier`.
fn seed_identity(app_context: &Arc<AppContext>, byte: u8, alias: &str) -> Identifier {
    seed_identity_typed(app_context, byte, alias, IdentityType::User)
}

/// Seed one wallet-less identity of `identity_type` (alias = `alias`,
/// id = `[byte; 32]`) into the live per-network identity DB.
fn seed_identity_typed(
    app_context: &Arc<AppContext>,
    byte: u8,
    alias: &str,
    identity_type: IdentityType,
) -> Identifier {
    let pv = PlatformVersion::latest();
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), pv).expect("basic identity");
    let id = identity.id();
    let qi = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type,
        alias: Some(alias.to_string()),
        private_keys: KeyStorage::default(),
        dpns_names: vec![dash_evo_tool::model::qualified_identity::DPNSNameInfo {
            name: alias.to_string(),
            acquired_at: 0,
        }],
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
        let harness = mount_app(RootScreenType::RootScreenIdentityHub);

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

        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
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

        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
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

/// Selecting a wallet-less (imported-by-id) identity clears the derived
/// wallet pointer, so the breadcrumb wallet segment and the active identity
/// agree. Pre-fix, `set_selected_identity` left `selected_wallet_hash` stale
/// (the owner derivation returns `None` for a wallet-less identity) and the
/// breadcrumb pill showed a different identity than Home/operate-as used.
#[test]
fn qa_001_wallet_less_selection_clears_derived_wallet() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();

        let lonely = seed_identity(&app_context, 0x55, "Lonely Identity");

        // Simulate a stale derived wallet from a prior wallet-owned selection.
        app_context.set_selected_hd_wallet(Some([0x99; 32]));
        assert_eq!(
            app_context.selected_wallet_hash(),
            Some([0x99; 32]),
            "precondition: a stale wallet pointer is set"
        );

        // Select the wallet-less identity — the exact call the picker/dropdown
        // click handler makes.
        app_context.set_selected_identity(Some(lonely));

        // The derived wallet must reconcile to None (no owning wallet).
        assert_eq!(
            app_context.selected_wallet_hash(),
            None,
            "selecting a wallet-less identity must clear the derived wallet pointer"
        );
        // The active identity is the wallet-less one (what Home/operate-as use).
        assert_eq!(
            app_context
                .resolve_selected_identity()
                .map(|qi| qi.identity.id()),
            Some(lonely),
            "resolve_selected_identity must return the selected wallet-less identity"
        );

        // The breadcrumb now reflects it: the identity pill shows the alias and
        // the wallet segment is the wallet-less placeholder — never the stale
        // "no identity yet" the pre-fix path rendered.
        harness.run_steps(5);
        assert!(
            harness.query_by_label("(no identity yet)").is_none(),
            "an identity IS selected; the breadcrumb must not claim none"
        );
        assert!(
            harness.query_by_label_contains("Lonely Identity").is_some(),
            "the breadcrumb must display the selected wallet-less identity"
        );
    });
}

/// TC-FR6-01/02 + TC-NAV-17 — the Identity Hub is an everyday-user surface:
/// its picker and identity-pill sources list User identities only. A seeded
/// Masternode + Evonode never appear; the User identity does. Exactly one User
/// is seeded so the hub lands on that identity's Home (not the picker), which is
/// itself the assertion that the two node identities did not inflate the
/// everyday-user identity count.
#[test]
fn fr6_hub_excludes_masternode_and_evonode() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();

        seed_identity_typed(
            &app_context,
            0x91,
            "Node Masternode",
            IdentityType::Masternode,
        );
        seed_identity_typed(&app_context, 0xE2, "Node Evonode", IdentityType::Evonode);
        let user = seed_identity(&app_context, 0x71, "Real User");
        harness.run_steps(5);

        // The everyday-user accessor lists only the User identity.
        let user_only = app_context
            .load_local_user_identities()
            .expect("load user identities");
        assert_eq!(
            user_only.len(),
            1,
            "hub sources list only the User identity"
        );
        assert_eq!(user_only[0].identity.id(), user);

        // The node identities are absent from the hub surface by alias.
        assert!(
            harness.query_by_label_contains("Node Masternode").is_none(),
            "a masternode must not appear on the Identity Hub"
        );
        assert!(
            harness.query_by_label_contains("Node Evonode").is_none(),
            "an evonode must not appear on the Identity Hub"
        );
        // But both remain in the masternode accessor (Masternodes-page source).
        assert_eq!(
            app_context
                .load_local_masternode_identities()
                .expect("load masternode identities")
                .len(),
            2,
            "MN + Evonode remain available to the Masternodes page"
        );
    });
}

/// Replaces a sham tautology test — the no-wallet group is identified
/// by `wallet_index.is_none()` on real loaded identities: a seeded wallet-less
/// identity appears in that filtered group (the exact predicate the breadcrumb
/// dropdown's "Identities without a wallet on this device" section uses).
#[test]
fn qa_002_no_wallet_group_filter_on_real_data() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();

        let imported = seed_identity(&app_context, 0x77, "Imported By Id");

        let loaded = app_context
            .load_local_qualified_identities()
            .expect("load identities");
        let no_wallet_group: Vec<Identifier> = loaded
            .iter()
            .filter(|qi| qi.wallet_index.is_none())
            .map(|qi| qi.identity.id())
            .collect();

        assert!(
            no_wallet_group.contains(&imported),
            "a wallet-less identity must appear in the wallet_index.is_none() group"
        );
    });
}

#[test]
fn ui_polish_picker_cards_stay_inside_window() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = rt.enter();
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        let aliases: Vec<String> = (1..=8)
            .map(|i| {
                format!(
                    "Identity {i} with a very long unbroken alias {}",
                    "x".repeat(70)
                )
            })
            .collect();
        for (index, alias) in aliases.iter().enumerate() {
            seed_identity(&app_context, index as u8 + 1, alias);
        }
        for width in [1640.0, 1000.0, 600.0] {
            harness.set_size(egui::vec2(width, 1400.0));
            harness.run_steps(5);
            for alias in &aliases {
                let card = harness.get_by_label(&format!("Open {alias}")).rect();
                assert!(card.right() <= width, "card {card:?} exceeds {width}");
            }
        }
    });
}

#[test]
fn ui_polish_picker_actions_show_pointer() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = rt.enter();
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        seed_identity(&app_context, 1, "Cursor Alpha");
        seed_identity(&app_context, 2, "Cursor Beta");
        harness.run_steps(5);
        harness
            .get_all_by_label("Identities")
            .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("top breadcrumb")
            .hover();
        harness.run_steps(3);
        assert_eq!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
        harness.get_by_label("Open Cursor Alpha").hover();
        harness.run_steps(3);
        assert_eq!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
        harness
            .get_all_by_label("Opens Identity Home →")
            .next()
            .expect("home hint")
            .hover();
        harness.run_steps(3);
        assert_eq!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Add a new identity")
            .hover();
        harness.run_steps(3);
        assert_eq!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
    });
}

#[test]
fn ui_polish_detail_breadcrumb_opens_picker_repeatedly() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = rt.enter();
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        let identity = seed_identity(&app_context, 1, "Detail Alpha");
        app_context.set_selected_identity(Some(identity));
        harness.run_steps(5);
        for _ in 0..2 {
            let qi = app_context.load_local_user_identities().unwrap().remove(0);
            harness
                .state_mut()
                .screen_stack
                .push(dash_evo_tool::ui::Screen::KeysScreen(
                    dash_evo_tool::ui::identity::keys::keys_screen::KeysScreen::new(
                        qi,
                        &app_context,
                    ),
                ));
            harness.run_steps(5);
            harness
                .get_all_by_label("Identities")
                .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
                .expect("top breadcrumb")
                .click();
            harness.run_steps(5);
            assert!(harness.state().screen_stack.is_empty());
            assert!(
                harness.query_by_label(PICKER_HEADING).is_some(),
                "breadcrumb must open picker even for one identity"
            );
            harness.get_by_label("Open Detail Alpha").click();
            harness.run_steps(5);
            assert!(harness.query_by_label(HOME_ONLY_MARKER).is_some());
        }
    });
}

#[test]
fn ui_polish_picker_scopes_to_selected_wallet_and_handles_empty_wallet() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = rt.enter();
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        for (byte, alias, wallet) in [
            (1, "Wallet Alpha", Some([0x11; 32])),
            (2, "Wallet Beta", Some([0x22; 32])),
            (3, "Imported Gamma", None),
        ] {
            seed_identity(&app_context, byte, alias);
            if let Some(wallet) = wallet {
                let qi = app_context
                    .load_local_user_identities()
                    .unwrap()
                    .into_iter()
                    .find(|qi| qi.identity.id() == Identifier::from([byte; 32]))
                    .unwrap();
                app_context
                    .insert_local_qualified_identity(&qi, &Some((wallet, 0)))
                    .unwrap();
            }
        }
        for (wallet, visible) in [
            (Some([0x11; 32]), vec!["Wallet Alpha"]),
            (Some([0x22; 32]), vec!["Wallet Beta"]),
            (Some([0x33; 32]), vec![]),
            (None, vec!["Wallet Alpha", "Wallet Beta", "Imported Gamma"]),
        ] {
            app_context.set_selected_hd_wallet(wallet);
            harness.run_steps(5);
            harness
                .get_all_by_label("Identities")
                .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
                .expect("top breadcrumb")
                .click();
            harness.run_steps(5);
            assert!(harness.query_by_label(PICKER_HEADING).is_some());
            for alias in ["Wallet Alpha", "Wallet Beta", "Imported Gamma"] {
                assert_eq!(
                    harness.query_by_label(&format!("Open {alias}")).is_some(),
                    visible.contains(&alias),
                    "wrong wallet scope for {alias}"
                );
            }
            assert!(
                harness
                    .query_by_role_and_label(egui::accesskit::Role::Button, "Add a new identity")
                    .is_some()
            );
        }
    });
}

#[test]
fn ui_polish_many_identities_picker_scroll_reaches_add_card() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = rt.enter();
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        for byte in 1..=30 {
            seed_identity(&app_context, byte, &format!("Scroll identity {byte:02}"));
        }
        harness.run_steps(5);
        harness.get_by_label("Open Scroll identity 01").hover();
        harness.run_steps(2);
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: egui::vec2(0.0, -10000.0),
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(10);
        let add =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, "Add a new identity");
        assert!(
            add.rect().bottom() <= 800.0,
            "scroll must bring Add into viewport: {:?}",
            add.rect()
        );
        add.click();
        harness.run_steps(5);
        assert!(
            !harness.state().screen_stack.is_empty(),
            "Add card must be clickable after scrolling"
        );
    });
}

#[test]
fn ui_polish_many_walletless_identities_popup_scroll_reaches_load() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = rt.enter();
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        for byte in 1..=40 {
            seed_identity(&app_context, byte, &format!("Popup identity {byte:02}"));
        }
        app_context.set_selected_identity(Some(Identifier::from([1; 32])));
        harness.run_steps(5);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Link, "Popup identity 01")
            .click();
        harness.run_steps(5);
        harness
            .get_by_label("Identities without a wallet on this device")
            .hover();
        harness.run_steps(2);
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: egui::vec2(0.0, -10000.0),
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(10);
        let load = harness.get_by_label("Load an existing identity");
        assert!(
            load.rect().bottom() <= 800.0,
            "scroll must bring Load into viewport: {:?}",
            load.rect()
        );
        load.click();
        harness.run_steps(5);
        assert!(
            !harness.state().screen_stack.is_empty(),
            "Load must be clickable after scrolling popup"
        );
    });
}

#[test]
fn ui_polish_wallet_dropdown_opens_scoped_picker() {
    with_isolated_data_dir(|| {
        use dash_evo_tool::model::wallet::Wallet;
        use std::sync::RwLock;
        use zeroize::Zeroize;

        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = rt.enter();
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        let mut hashes = Vec::new();
        for (byte, wallet_alias, identity_alias) in [
            (1, "First wallet", "First owned identity"),
            (2, "Second wallet", "Second owned identity"),
        ] {
            let mut seed: [u8; 64] = rand::random();
            let wallet =
                Wallet::new_from_seed(seed, app_context.network(), Some(wallet_alias.into()), None)
                    .expect("wallet fixture");
            seed.zeroize();
            let hash = wallet.seed_hash();
            app_context
                .wallet_context()
                .insert_test_wallet(hash, Arc::new(RwLock::new(wallet)));
            hashes.push(hash);
            let id = seed_identity(&app_context, byte, identity_alias);
            let qi = app_context
                .load_local_user_identities()
                .unwrap()
                .into_iter()
                .find(|qi| qi.identity.id() == id)
                .unwrap();
            app_context
                .insert_local_qualified_identity(&qi, &Some((hash, 0)))
                .unwrap();
        }
        app_context.set_selected_hd_wallet(Some(hashes[0]));
        harness.run_steps(5);
        assert!(harness.query_by_label(HOME_ONLY_MARKER).is_some());
        harness
            .get_by_role_and_label(egui::accesskit::Role::Link, "First wallet")
            .click();
        harness.run_steps(3);
        harness.get_by_label("💼 Second wallet").click();
        harness.run_steps(5);
        assert_eq!(app_context.selected_wallet_hash(), Some(hashes[1]));
        assert!(harness.query_by_label(PICKER_HEADING).is_some());
        assert!(
            harness
                .query_by_label("Open Second owned identity")
                .is_some()
        );
        assert!(
            harness
                .query_by_label("Open First owned identity")
                .is_none()
        );
        harness.get_by_label("Open Second owned identity").click();
        harness.run_steps(5);
        assert_eq!(
            app_context.selected_wallet_hash(),
            Some(hashes[1]),
            "opening a wallet-owned identity must preserve its stored wallet scope"
        );
        harness
            .get_all_by_label("Identities")
            .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("top breadcrumb")
            .click();
        harness.run_steps(5);
        assert!(
            harness
                .query_by_label("Open Second owned identity")
                .is_some()
        );
        assert!(
            harness
                .query_by_label("Open First owned identity")
                .is_none()
        );
        harness
            .get_by_role_and_label(egui::accesskit::Role::Link, "Second wallet")
            .click();
        harness.run_steps(3);
        harness.get_by_label("All wallets").click();
        harness.run_steps(5);
        assert_eq!(app_context.selected_wallet_hash(), None);
        assert!(
            harness
                .query_by_role_and_label(egui::accesskit::Role::Link, "All wallets")
                .is_some()
        );
        assert!(
            harness
                .query_by_label("Open First owned identity")
                .is_some()
        );
        assert!(
            harness
                .query_by_label("Open Second owned identity")
                .is_some()
        );
    });
}

#[test]
fn ui_polish_picker_uses_profile_avatar_and_keeps_missing_avatar_fallback() {
    with_isolated_data_dir(|| {
        use dash_evo_tool::app::AppAction;
        use dash_evo_tool::backend_task::{BackendTaskContext, BackendTaskSuccessResult};
        use dash_evo_tool::ui::ScreenLike;
        use dash_evo_tool::ui::identity::IdentityHubScreen;
        let (runtime, app_context) = crate::support::fresh_app_context();
        let _guard = runtime.enter();
        let avatar_id = seed_identity(&app_context, 1, "Avatar identity");
        seed_identity(&app_context, 2, "Initial identity");
        app_context.set_selected_identity(Some(avatar_id));
        let screen = IdentityHubScreen::new(&app_context);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_ui_state(
                |ui, state: &mut (IdentityHubScreen, Option<BackendTaskContext>)| {
                    if let AppAction::BackendTaskWithContext { context, .. } = state.0.ui(ui) {
                        state.1 = Some(context);
                    }
                },
                (screen, None),
            );
        harness.run_steps(5);
        let context = harness.state().1.clone().expect("profile dispatch");
        let url = "https://example.invalid/synthetic-avatar.png";
        harness.state_mut().0.display_backend_task_result(
            &context,
            BackendTaskSuccessResult::DashPayProfile(Some((
                "Profile name".into(),
                String::new(),
                url.into(),
            ))),
        );
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 128, 255, 255]))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("synthetic PNG");
        harness
            .state_mut()
            .0
            .display_task_result(BackendTaskSuccessResult::DashPayAvatar {
                url: url.into(),
                bytes: Some(bytes.into_inner()),
            });
        harness
            .get_all_by_label("Identities")
            .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("top breadcrumb")
            .click();
        harness.run_steps(5);
        let card = harness.get_by_label("Open Avatar identity").rect();
        assert_eq!(
            harness
                .query_all_by_role(egui::accesskit::Role::Image)
                .filter(|image| card.contains(image.rect().center()))
                .count(),
            1,
            "configured avatar must render inside its identity card"
        );
        let fallback = harness.get_by_label("Open Initial identity").rect();
        assert_eq!(
            harness
                .query_all_by_role(egui::accesskit::Role::Image)
                .filter(|image| fallback.contains(image.rect().center()))
                .count(),
            0,
            "identity without avatar retains monogram"
        );

        harness.state_mut().0.refresh();
        harness.run_steps(3);
        let context = harness.state().1.clone().expect("profile reload");
        harness.state_mut().0.display_backend_task_result(
            &context,
            BackendTaskSuccessResult::DashPayProfile(Some(("".into(), String::new(), url.into()))),
        );
        harness
            .state_mut()
            .0
            .display_task_result(BackendTaskSuccessResult::DashPayAvatar {
                url: url.into(),
                bytes: None,
            });
        harness.run_steps(5);
        assert_eq!(
            harness
                .query_all_by_role(egui::accesskit::Role::Image)
                .filter(|image| card.contains(image.rect().center()))
                .count(),
            0,
            "unavailable avatar falls back without a broken image"
        );
    });
}

#[test]
fn ui_polish_profile_completion_matches_dispatch_and_errors_release_queue() {
    with_isolated_data_dir(|| {
        use dash_evo_tool::app::AppAction;
        use dash_evo_tool::backend_task::BackendTaskSuccessResult;
        use dash_evo_tool::ui::identity::profile_cache::ProfileCache;
        let (runtime, app_context) = crate::support::fresh_app_context();
        let _guard = runtime.enter();
        seed_identity(&app_context, 1, "Profile Alpha");
        seed_identity(&app_context, 2, "Profile Beta");
        let identities = app_context.load_local_user_identities().unwrap();
        let mut profiles = ProfileCache::default();
        profiles.get_or_request(&identities[0]);
        let AppAction::BackendTaskWithContext { context: old, .. } = profiles.dispatch_pending()
        else {
            panic!("profile dispatch");
        };
        profiles.reset();
        profiles.get_or_request(&identities[1]);
        let AppAction::BackendTaskWithContext {
            context: current, ..
        } = profiles.dispatch_pending()
        else {
            panic!("profile dispatch");
        };
        let result = BackendTaskSuccessResult::DashPayProfile(Some((
            "Beta".into(),
            String::new(),
            "https://example.invalid/beta.png".into(),
        )));
        assert!(
            !profiles.record_result(&old, &result),
            "late completion after reset must not populate another identity"
        );
        profiles.record_error(&old);
        assert!(
            matches!(profiles.dispatch_pending(), AppAction::None),
            "late error must not clear the current request"
        );
        assert!(profiles.record_result(&current, &result));
        assert_eq!(
            profiles
                .get_or_request(&identities[1])
                .unwrap()
                .as_ref()
                .unwrap()
                .display_name,
            "Beta"
        );
        profiles.reset();
        profiles.get_or_request(&identities[0]);
        let AppAction::BackendTaskWithContext { context, .. } = profiles.dispatch_pending() else {
            panic!("profile dispatch");
        };
        profiles.get_or_request(&identities[1]);
        profiles.record_error(&context);
        assert!(
            matches!(
                profiles.dispatch_pending(),
                AppAction::BackendTaskWithContext { .. }
            ),
            "one failed profile must not block other avatars"
        );
    });
}
