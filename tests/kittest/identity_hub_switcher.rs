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
            harness
                .query_all_by_label_contains("Lonely Identity")
                .next()
                .is_some(),
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

fn with_identity_hub(
    test: impl FnOnce(egui_kittest::Harness<'static, dash_evo_tool::app::AppState>, Arc<AppContext>),
) {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = rt.enter();
        let harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        test(harness, app_context);
    });
}

fn assert_pointer<State>(harness: &mut egui_kittest::Harness<'_, State>) {
    harness.run_steps(3);
    assert_eq!(
        harness.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
}

fn open_picker<State>(harness: &mut egui_kittest::Harness<'_, State>) {
    harness
        .get_all_by_label("Identities")
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("top breadcrumb")
        .click();
    harness.run_steps(5);
}

fn scroll_to_bottom<State>(harness: &mut egui_kittest::Harness<'_, State>) {
    harness.run_steps(2);
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        phase: egui::TouchPhase::Move,
        delta: egui::vec2(0.0, -10000.0),
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(10);
}

#[test]
fn ui_polish_picker_cards_stay_inside_window() {
    with_identity_hub(|mut harness, app_context| {
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
    with_identity_hub(|mut harness, app_context| {
        seed_identity(&app_context, 1, "Cursor Alpha");
        seed_identity(&app_context, 2, "Cursor Beta");
        harness.run_steps(5);
        for label in ["Identities", "Open Cursor Alpha", "Opens Identity Home →"] {
            harness
                .get_all_by_label(label)
                .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
                .expect("hover target")
                .hover();
            assert_pointer(&mut harness);
        }
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Add a new identity")
            .hover();
        assert_pointer(&mut harness);
    });
}

#[test]
fn ui_polish_detail_breadcrumb_opens_picker_repeatedly() {
    with_identity_hub(|mut harness, app_context| {
        let identity = seed_identity(&app_context, 1, "Detail Alpha");
        app_context.set_selected_identity(Some(identity));
        harness.run_steps(5);
        for _ in 0..2 {
            app_context.set_selected_hd_wallet(Some([0x11; 32]));
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
            open_picker(&mut harness);
            assert!(harness.state().screen_stack.is_empty());
            assert_eq!(app_context.selected_wallet_hash(), None);
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

/// The request status page is a pushed screen, so its root crumb has to dismiss
/// it — selecting the hub underneath would leave the page on screen.
#[test]
fn username_request_breadcrumb_dismisses_the_page_and_opens_the_picker() {
    with_identity_hub(|mut harness, app_context| {
        let identity = seed_identity(&app_context, 1, "Request Alpha");
        app_context.set_selected_identity(Some(identity));
        harness.run_steps(5);
        harness
            .state_mut()
            .screen_stack
            .push(dash_evo_tool::ui::Screen::UsernameRequestScreen(
                dash_evo_tool::ui::identity::username_request_screen::UsernameRequestScreen::new(
                    &app_context,
                    identity,
                    "a11ce".to_owned(),
                ),
            ));
        harness.run_steps(5);
        open_picker(&mut harness);
        assert!(
            harness.state().screen_stack.is_empty(),
            "breadcrumb must dismiss the request status page"
        );
        assert!(harness.query_by_label(PICKER_HEADING).is_some());
    });
}

#[test]
fn ui_polish_username_registration_breadcrumb_opens_all_identities() {
    with_identity_hub(|mut harness, app_context| {
        let alpha = seed_identity(&app_context, 1, "Username Alpha");
        let mut unnamed = app_context.load_local_user_identities().unwrap().remove(0);
        unnamed.dpns_names.clear();
        app_context
            .insert_local_qualified_identity(&unnamed, &None)
            .unwrap();
        seed_identity(&app_context, 2, "Username Beta");
        app_context.set_selected_identity(Some(alpha));
        harness.run_steps(5);
        harness
            .get_all_by_label("Pick a username")
            .next()
            .expect("username link")
            .click();
        harness.run_steps(5);
        assert!(matches!(
            harness.state().screen_stack.last(),
            Some(dash_evo_tool::ui::Screen::RegisterDpnsNameScreen(_))
        ));
        app_context.set_selected_hd_wallet(Some([0x11; 32]));
        open_picker(&mut harness);
        assert!(
            harness.state().screen_stack.is_empty(),
            "breadcrumb must dismiss Register Name"
        );
        assert_eq!(app_context.selected_wallet_hash(), None);
        assert!(harness.query_by_label(PICKER_HEADING).is_some());
        for name in [unnamed.display_string(), "Username Beta".to_string()] {
            assert!(harness.query_by_label(&format!("Open {name}")).is_some());
        }
    });
}

#[test]
fn ui_polish_picker_scopes_to_selected_wallet_and_handles_empty_wallet() {
    with_identity_hub(|mut harness, app_context| {
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
        harness.run_steps(5);
        open_picker(&mut harness);
        for (wallet, visible) in [
            (Some([0x11; 32]), vec!["Wallet Alpha"]),
            (Some([0x22; 32]), vec!["Wallet Beta"]),
            (Some([0x33; 32]), vec![]),
            (None, vec!["Wallet Alpha", "Wallet Beta", "Imported Gamma"]),
        ] {
            app_context.set_selected_hd_wallet(wallet);
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
    with_identity_hub(|mut harness, app_context| {
        for byte in 1..=30 {
            seed_identity(&app_context, byte, &format!("Scroll identity {byte:02}"));
        }
        harness.run_steps(5);
        harness.get_by_label("Open Scroll identity 01").hover();
        scroll_to_bottom(&mut harness);
        let add =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, "Add a new identity");
        assert!(
            add.rect().bottom() <= 800.0,
            "scroll must bring Add into viewport: {:?}",
            add.rect()
        );
        add.click();
        harness.run_steps(5);
        // The card opens its create/load menu; picking an item opens the screen.
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Load an existing identity")
            .click();
        harness.run_steps(5);
        assert!(
            !harness.state().screen_stack.is_empty(),
            "Add card must be clickable after scrolling"
        );
    });
}

#[test]
fn ui_polish_all_wallets_dropdown_includes_owned_and_walletless_users_once() {
    with_identity_hub(|mut harness, app_context| {
        for (byte, name) in [(1, "Dropdown Owned Alpha"), (2, "Dropdown Owned Beta")] {
            let id = seed_identity(&app_context, byte, name);
            let identity = app_context
                .load_local_user_identities()
                .unwrap()
                .into_iter()
                .find(|identity| identity.identity.id() == id)
                .unwrap();
            app_context
                .insert_local_qualified_identity(&identity, &Some(([byte; 32], 0)))
                .unwrap();
        }
        seed_identity(&app_context, 3, "Dropdown Imported");
        seed_identity_typed(&app_context, 4, "Dropdown Node", IdentityType::Masternode);
        harness.run_steps(5);
        harness.get_by_label("Open Dropdown Imported").click();
        harness.run_steps(5);
        assert_eq!(app_context.selected_wallet_hash(), None);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Link, "Dropdown Imported")
            .click();
        harness.run_steps(5);
        for name in [
            "Dropdown Owned Alpha",
            "Dropdown Owned Beta",
            "Dropdown Imported",
        ] {
            assert_eq!(
                harness
                    .query_all_by_role_and_label(egui::accesskit::Role::Button, name)
                    .count(),
                1,
                "All wallets must list {name} exactly once"
            );
        }
        assert!(harness.query_by_label("Dropdown Node").is_none());
    });
}

#[test]
fn ui_polish_many_walletless_identities_popup_scroll_reaches_load() {
    with_identity_hub(|mut harness, app_context| {
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
        scroll_to_bottom(&mut harness);
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
    with_identity_hub(|mut harness, app_context| {
        use dash_evo_tool::model::wallet::Wallet;
        use std::sync::RwLock;
        use zeroize::Zeroize;

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
        open_picker(&mut harness);
        assert_eq!(app_context.selected_wallet_hash(), None);
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
        harness
            .get_by_role_and_label(egui::accesskit::Role::Link, "All wallets")
            .click();
        harness.run_steps(3);
        harness.get_by_label("💼 Second wallet").click();
        harness.run_steps(5);
        assert_eq!(app_context.selected_wallet_hash(), Some(hashes[1]));
        assert!(
            harness
                .query_by_label("Open First owned identity")
                .is_none()
        );
        assert!(
            harness
                .query_by_label("Open Second owned identity")
                .is_some()
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
        app_context.set_selected_hd_wallet(Some(hashes[0]));
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Wallets")
            .click();
        harness.run_steps(5);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Link, "First wallet")
            .click();
        harness.run_steps(3);
        harness.get_by_label("💼 Second wallet").click();
        harness.run_steps(5);
        assert_eq!(app_context.selected_wallet_hash(), Some(hashes[1]));
        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenWalletsBalances
        );
        assert!(harness.query_by_label(PICKER_HEADING).is_none());
    });
}

#[test]
fn ui_polish_picker_uses_profile_avatar_and_keeps_missing_avatar_fallback() {
    with_isolated_data_dir(|| {
        use dash_evo_tool::app::AppAction;
        use dash_evo_tool::backend_task::{
            BackendTask, BackendTaskContext, BackendTaskSuccessResult, dashpay::DashPayTask,
        };
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
                |ui,
                 state: &mut (
                    IdentityHubScreen,
                    Option<(Identifier, BackendTaskContext)>,
                    Vec<String>,
                )| {
                    let tasks = match state.0.ui(ui) {
                        AppAction::BackendTaskWithContext { task, context } => {
                            if let BackendTask::DashPayTask(profile_task) = &task
                                && let DashPayTask::LoadProfile { identity } = profile_task.as_ref()
                            {
                                state.1 = Some((identity.identity.id(), context));
                            }
                            vec![task]
                        }
                        AppAction::BackendTask(task) => vec![task],
                        AppAction::BackendTasks(tasks, _) => tasks,
                        _ => Vec::new(),
                    };
                    for task in tasks {
                        if let BackendTask::DashPayTask(task) = task
                            && let DashPayTask::FetchAvatar { url } = *task
                        {
                            state.2.push(url);
                        }
                    }
                },
                (screen, None, Vec::new()),
            );
        harness.run_steps(5);
        let (owner, context) = harness.state_mut().1.take().expect("profile dispatch");
        assert_eq!(owner, avatar_id);
        let url = "https://example.invalid/synthetic-avatar.png";
        harness.state_mut().0.display_backend_task_result(
            &context,
            BackendTaskSuccessResult::DashPayProfile(
                dash_evo_tool::model::dashpay::ProfileSnapshot {
                    network: app_context.network(),
                    owner: avatar_id,
                    revision: app_context.identity_profile_revision(avatar_id),
                    profile: Some(("Profile name".into(), String::new(), url.into())),
                },
            ),
        );
        open_picker(&mut harness);
        assert_eq!(
            harness.state().2,
            [url],
            "picker must request the avatar before completion"
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
        harness.run_steps(5);
        assert_eq!(
            harness.state().2,
            [url],
            "a loaded avatar must not be fetched again"
        );
        let card = harness.get_by_label("Open Profile name").rect();
        assert!(harness.query_by_label("Open Avatar identity").is_none());
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

        harness.state_mut().1 = None;
        harness.state_mut().2.clear();
        harness.state_mut().0.refresh();
        harness.run_steps(3);
        let (owner, mut context) = harness.state_mut().1.take().expect("profile reload");
        if owner != avatar_id {
            harness.state_mut().0.display_backend_task_result(
                &context,
                BackendTaskSuccessResult::DashPayProfile(
                    dash_evo_tool::model::dashpay::ProfileSnapshot {
                        network: app_context.network(),
                        owner,
                        revision: app_context.identity_profile_revision(owner),
                        profile: None,
                    },
                ),
            );
            harness.run_steps(3);
            let (owner, next) = harness.state_mut().1.take().expect("avatar owner's reload");
            assert_eq!(owner, avatar_id);
            context = next;
        }
        harness.state_mut().0.display_backend_task_result(
            &context,
            BackendTaskSuccessResult::DashPayProfile(
                dash_evo_tool::model::dashpay::ProfileSnapshot {
                    network: app_context.network(),
                    owner: avatar_id,
                    revision: app_context.identity_profile_revision(avatar_id),
                    profile: Some(("".into(), String::new(), url.into())),
                },
            ),
        );
        harness.run_steps(5);
        assert_eq!(
            harness.state().2,
            [url],
            "refresh must request a new avatar before failure"
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
            harness.state().2,
            [url],
            "a failed avatar must not trigger repeated fetches"
        );
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
        let result = BackendTaskSuccessResult::DashPayProfile(
            dash_evo_tool::model::dashpay::ProfileSnapshot {
                network: app_context.network(),
                owner: identities[1].identity.id(),
                revision: app_context.identity_profile_revision(identities[1].identity.id()),
                profile: Some((
                    "Beta".into(),
                    String::new(),
                    "https://example.invalid/beta.png".into(),
                )),
            },
        );
        assert!(
            !profiles.record_result(&app_context, &old, &result),
            "late completion after reset must not populate another identity"
        );
        profiles.record_error(&old);
        assert!(
            matches!(profiles.dispatch_pending(), AppAction::None),
            "late error must not clear the current request"
        );
        assert!(profiles.record_result(&app_context, &current, &result));
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

#[test]
fn hidden_hub_receives_its_profile_save_and_releases_the_next_draft() {
    use dash_evo_tool::app::{AppAction, TaskResult};
    use dash_evo_tool::backend_task::{
        BackendTask, BackendTaskContext, BackendTaskSuccessResult, dashpay::DashPayTask,
    };
    use dash_evo_tool::model::dashpay::ProfileSnapshot;
    use dash_evo_tool::ui::{
        Screen, ScreenLike,
        identity::{IdentityHubScreen, IdentityHubTab},
    };
    with_isolated_data_dir(|| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut app = mount_app(RootScreenType::RootScreenWalletsBalances);
        let context = app.state().current_app_context().clone();
        let owner = seed_identity(&context, 81, "Profile owner");
        context.set_selected_identity(Some(owner));
        let mut hub = IdentityHubScreen::new(&context);
        hub.select_tab(IdentityHubTab::Settings);
        let mut editor = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, state: &mut (IdentityHubScreen, Option<BackendTaskContext>)| {
                    if let AppAction::BackendTaskWithContext {
                        task: BackendTask::DashPayTask(task),
                        context,
                    } = state.0.ui(ui)
                        && matches!(*task, DashPayTask::UpdateProfile { .. })
                    {
                        state.1 = Some(context);
                    }
                },
                (hub, None),
            );
        editor.run();
        editor
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .unwrap()
            .focus();
        editor.run();
        editor
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .unwrap()
            .type_text("Saved name");
        editor.run();
        editor.get_by_label("Save social profile").click();
        editor.run();
        let dispatch = editor.state_mut().1.take().expect("save dispatch");
        let hub = std::mem::replace(&mut editor.state_mut().0, IdentityHubScreen::new(&context));
        app.state_mut().main_screens.insert(
            RootScreenType::RootScreenIdentityHub,
            Screen::IdentityHubScreen(hub),
        );
        runtime
            .block_on(app.state().task_result_sender.send(TaskResult::Success {
                context: dispatch,
                result: Box::new(BackendTaskSuccessResult::DashPayProfileUpdated(
                    ProfileSnapshot {
                        network: context.network(),
                        owner,
                        revision: context.identity_profile_revision(owner),
                        profile: Some(("Saved name".into(), String::new(), String::new())),
                    },
                )),
            }))
            .unwrap();
        app.run_steps(3);
        let Screen::IdentityHubScreen(hub) = app
            .state_mut()
            .main_screens
            .remove(&RootScreenType::RootScreenIdentityHub)
            .unwrap()
        else {
            panic!("hub")
        };
        editor.state_mut().0 = hub;
        editor.run();
        editor.get_by_label("Save social profile").click();
        editor.run();
        assert!(
            editor.state().1.is_none(),
            "confirmed fields must be the clean baseline"
        );
        editor
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .unwrap()
            .focus();
        editor.run();
        editor
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .unwrap()
            .type_text(" next");
        editor.run();
        editor.get_by_label("Save social profile").click();
        editor.run();
        assert!(
            editor.state().1.is_some(),
            "hidden completion must release its pending save"
        );
    });
}

/// With identities already loaded and none selected (the picker), the hub's
/// top-bar "Add" menu offers both add flows: "Create a new identity" pushes
/// `AddNewIdentityScreen` and "Load an existing identity" pushes
/// `AddExistingIdentityScreen`.
#[test]
fn picker_top_bar_add_menu_creates_or_loads_an_identity() {
    with_isolated_data_dir(|| {
        use dash_evo_tool::ui::Screen;
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        seed_identity(&app_context, 0xC1, "Add Menu Alpha");
        seed_identity(&app_context, 0xC2, "Add Menu Beta");
        harness.run_steps(5);
        assert!(harness.query_by_label(PICKER_HEADING).is_some());

        harness.get_by_label("Add ▾").click();
        harness.run_steps(3);
        harness.get_by_label("Load an existing identity").click();
        harness.run_steps(3);
        assert!(
            matches!(
                harness.state().screen_stack.last(),
                Some(Screen::AddExistingIdentityScreen(_))
            ),
            "\"Load an existing identity\" must push AddExistingIdentityScreen"
        );

        harness.state_mut().screen_stack.clear();
        harness.run_steps(3);
        harness.get_by_label("Add ▾").click();
        harness.run_steps(3);
        harness.get_by_label("Create a new identity").click();
        harness.run_steps(3);
        assert!(
            matches!(
                harness.state().screen_stack.last(),
                Some(Screen::AddNewIdentityScreen(_))
            ),
            "\"Create a new identity\" must push AddNewIdentityScreen"
        );
    });
}

#[test]
fn picker_create_entries_preserve_selected_wallet() {
    with_identity_hub(|mut harness, app_context| {
        use dash_evo_tool::model::wallet::Wallet;
        use dash_evo_tool::model::wallet::birth_height::WalletOrigin;
        use dash_evo_tool::ui::Screen;
        use zeroize::Zeroize;

        let mut wallets = Vec::new();
        for alias in ["First candidate", "Second candidate"] {
            let mut seed: [u8; 64] = rand::random();
            let wallet =
                Wallet::new_from_seed(seed, app_context.network(), Some(alias.into()), None)
                    .expect("wallet fixture");
            let hash = wallet.seed_hash();
            app_context
                .register_wallet(wallet, &seed, WalletOrigin::Imported)
                .expect("register wallet fixture");
            seed.zeroize();
            wallets.push((hash, alias));
        }
        wallets.sort_by_key(|(hash, _)| *hash);
        let (selected_hash, selected_alias) = wallets[1];
        assert_ne!(
            app_context
                .wallet_context()
                .first_hd()
                .unwrap()
                .read()
                .unwrap()
                .seed_hash(),
            selected_hash
        );
        for byte in [0xC3, 0xC4] {
            let id = seed_identity(&app_context, byte, &format!("Owned identity {byte}"));
            let identity = app_context
                .load_local_user_identities()
                .unwrap()
                .into_iter()
                .find(|identity| identity.identity.id() == id)
                .unwrap();
            app_context
                .insert_local_qualified_identity(&identity, &Some((selected_hash, 0)))
                .unwrap();
        }
        app_context.set_selected_hd_wallet(Some(wallets[0].0));
        harness.run_steps(5);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Link, wallets[0].1)
            .click();
        harness.run_steps(3);
        harness
            .get_by_label(&format!("💼 {selected_alias}"))
            .click();
        harness.run_steps(5);

        for entry in ["Add ▾", "Add a new identity"] {
            assert_eq!(app_context.selected_wallet_hash(), Some(selected_hash));
            assert!(harness.query_by_label(PICKER_HEADING).is_some());
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, entry)
                .click();
            harness.run_steps(3);
            harness.get_by_label("Create a new identity").click();
            harness.run_steps(3);
            assert!(matches!(
                harness.state().screen_stack.last(),
                Some(Screen::AddNewIdentityScreen(_))
            ));
            assert!(
                harness
                    .query_by_value(&format!("HD: {selected_alias} — 0 DASH"))
                    .is_some(),
                "{entry} must preselect the chosen wallet in the creation form"
            );
            harness.state_mut().screen_stack.clear();
            harness.run_steps(3);
        }
    });
}
