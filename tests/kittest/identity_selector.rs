//! Kittest coverage for `IdentitySelector` — the app-scoped write-back keystone.
//!
//! The critical write-back path — a genuine ComboBox picker
//! change must call `AppContext::set_selected_identity`, keeping "who am I signing
//! as" in sync across the breadcrumb and all 12 SYNC screens. This is a
//! component-level lock: testing `IdentitySelector` directly proves the shared
//! mechanism for all 12 SYNC callsites without needing private keys or a full
//! screen fixture for each.
//!
//! Two phases:
//! - **Phase 1 (seeding-not-write-back):** with the buffer pre-seeded to Alice's
//!   Base58, an initial render must NOT invoke `set_selected_identity`. The
//!   `combo_changed` flag stays false until a user interaction occurs.
//! - **Phase 2 (genuine ComboBox change):** click the ComboBox header (Alice),
//!   click Bob's dropdown item, and assert
//!   `ctx.selected_identity_id() == Some(bob_id)`.
//!
//! ## Setup
//!
//! We use a `build_eframe` harness (`run_steps(5)`) to fully initialize the
//! `AppContext` — specifically to let `ensure_wallet_backend` run and complete
//! `restore_selected_identity_from_kv`. Identity selection is set AFTER that
//! initialization so the KV restore does not race against our seed.
//! A separate `build_ui` harness then hosts the standalone `IdentitySelector`.
//!
//! ## egui-kittest ComboBox quirk (documented by Marvin's QA run)
//!
//! The ComboBox header is exposed to accesskit via its `selected_text` as the
//! **value** property → use `harness.get_by_value("Alice")` to click it open.
//! Items inside the popup are `selectable_label` widgets exposed as Buttons via
//! their text **label** → use `harness.get_by_label("Bob")` to select them.
//!
//! Per-screen click-through tests (which DO need private keys so the screen gates
//! on `has_suitable_keys`) remain deferred per the `TODO(WalletFixture)` comments
//! in `contract_screen.rs`, `dashpay_screen.rs`, and `tokens_screen.rs`.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::components::identity_selector::IdentitySelector;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

/// Build a wallet-less `QualifiedIdentity` in-memory. No DB insertion, no
/// private keys — only `id()` + `display_string()` (username) are exercised.
fn make_qi(byte: u8, alias: &str) -> QualifiedIdentity {
    let pv = PlatformVersion::latest();
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), pv).expect("basic identity");
    QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
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
        network: Network::Testnet,
    }
}

/// Keystone write-back lock for the app-scoped identity migration.
///
/// A genuine ComboBox picker change on an `IdentitySelector` configured with
/// `.syncing_global(ctx)` must propagate to `ctx.selected_identity_id()`.
///
/// No private keys, no wallet backend write-path, no full screen fixture needed:
/// `set_selected_identity` with wallet-less identities only updates in-memory
/// mutexes; the selector's `identities` slice is built in-memory. The component-
/// level test covers the write-back for all 12 SYNC screens in one assertion.
#[test]
fn combo_change_writes_selection_to_app_context() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        // Step 1: fully initialize AppContext via build_eframe.
        //
        // `ensure_wallet_backend` calls `restore_selected_identity_from_kv` on
        // first wiring. Waiting for the storage-preparation gate to lift puts
        // that BEFORE the seed below, so the seed is not overwritten by the
        // async initialization.
        let mut setup = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("AppState builds")
                .with_animations(false)
        });
        crate::support::wait_for_screens(&mut setup);
        let ctx = setup.state().current_app_context().clone();

        let alice = make_qi(0xAA, "Alice");
        let bob = make_qi(0xBB, "Bob");
        let alice_id = alice.identity.id();
        let bob_id = bob.identity.id();

        // Seed global AFTER KV restore; wallet_backend is now wired and
        // restore_selected_identity_from_kv will not run again (idempotent).
        ctx.set_selected_identity(Some(alice_id));

        // Step 2: build a standalone build_ui harness hosting the IdentitySelector.
        //
        // Rc<RefCell<>> shared state: the closure captures the inner Rc clones,
        // the outer clones remain accessible for post-harness assertions.
        let ids = vec![alice.clone(), bob.clone()];
        let buf = Rc::new(RefCell::new(alice_id.to_string(Encoding::Base58)));
        let sel: Rc<RefCell<Option<QualifiedIdentity>>> = Rc::new(RefCell::new(None));

        let buf_inner = Rc::clone(&buf);
        let sel_inner = Rc::clone(&sel);
        let ctx_inner = Arc::clone(&ctx);

        let mut harness = Harness::builder()
            .with_size(egui::vec2(400.0, 80.0))
            .build_ui(move |ui| {
                let mut b = buf_inner.borrow_mut();
                let mut s = sel_inner.borrow_mut();
                let _ = ui.add(
                    IdentitySelector::new("qa001_combo", &mut b, &ids)
                        .selected_identity(&mut s)
                        .expect("selected_identity")
                        .other_option(false)
                        .syncing_global(Arc::clone(&ctx_inner)),
                );
            });

        // ── Phase 1: initial render ───────────────────────────────────────────
        // The buffer is pre-seeded to Alice's Base58. No user interaction has
        // occurred, so combo_changed=false and sync_to_global is NOT called.
        // The global must remain Alice after the render.
        harness.run();
        assert_eq!(
            ctx.selected_identity_id(),
            Some(alice_id),
            "Phase 1: initial render must not invoke set_selected_identity (seeding ≠ write-back)"
        );

        // ── Phase 2: genuine ComboBox change to Bob ───────────────────────────
        // ComboBox header selected_text = "Alice" → accesskit value = "Alice".
        // selectable_label("Bob") popup item → accesskit label = "Bob".
        harness.get_by_value("Alice").click(); // open the ComboBox popup
        harness.run(); // render the open popup (Alice + Bob as selectable items)
        harness.get_by_label("Bob").click(); // select Bob's item
        harness.run(); // combo_changed=true → sync_to_global() → set_selected_identity(bob)

        assert_eq!(
            ctx.selected_identity_id(),
            Some(bob_id),
            "Phase 2: selecting Bob via ComboBox must propagate bob_id to AppContext"
        );
    });
}

#[test]
fn profile_screen_rejects_late_picker_profile_for_other_identity() {
    use dash_evo_tool::app::AppAction;
    use dash_evo_tool::backend_task::BackendTaskSuccessResult;
    use dash_evo_tool::ui::ScreenLike;
    use dash_evo_tool::ui::dashpay::{DashPayScreen, DashPaySubscreen};
    use dash_evo_tool::ui::identity::profile_cache::ProfileCache;
    with_isolated_data_dir(|| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut harness =
            crate::support::mount_app(dash_evo_tool::ui::RootScreenType::RootScreenIdentityHub);
        let ctx = harness.state().current_app_context().clone();
        let first = make_qi(1, "Alice username");
        let second = make_qi(2, "Bob username");
        ctx.insert_local_qualified_identity(&first, &None).unwrap();
        ctx.insert_local_qualified_identity(&second, &None).unwrap();
        ctx.set_selected_identity(Some(second.identity.id()));
        let mut screen = DashPayScreen::new(&ctx, DashPaySubscreen::Profile);
        let own_context = match screen.profile_screen.trigger_load_profile() {
            AppAction::BackendTaskWithContext { context, .. } => context,
            AppAction::BackendTask(task) => (&task).into(),
            _ => panic!("own dispatch"),
        };
        screen.display_backend_task_result(
            &own_context,
            BackendTaskSuccessResult::DashPayProfile(
                dash_evo_tool::model::dashpay::ProfileSnapshot {
                    network: ctx.network(),
                    owner: second.identity.id(),
                    revision: ctx.identity_profile_revision(second.identity.id()),
                    profile: Some((
                        "Bob correct profile".into(),
                        "Bob biography".into(),
                        String::new(),
                    )),
                },
            ),
        );
        let mut picker_load = ProfileCache::default();
        picker_load.get_or_request(&first);
        let AppAction::BackendTaskWithContext {
            context: picker_context,
            ..
        } = picker_load.dispatch_pending()
        else {
            panic!("picker dispatch");
        };
        assert_eq!(
            screen
                .profile_screen
                .selected_identity
                .as_ref()
                .unwrap()
                .identity
                .id(),
            second.identity.id()
        );
        harness.state_mut().main_screens.insert(
            dash_evo_tool::ui::RootScreenType::RootScreenDashPayProfile,
            dash_evo_tool::ui::Screen::DashPayScreen(screen),
        );
        harness.state_mut().selected_main_screen =
            dash_evo_tool::ui::RootScreenType::RootScreenDashPayProfile;
        runtime
            .block_on(harness.state().task_result_sender.send(
                dash_evo_tool::app::TaskResult::Success {
                    context: picker_context,
                    result: Box::new(BackendTaskSuccessResult::DashPayProfile(
                        dash_evo_tool::model::dashpay::ProfileSnapshot {
                            network: ctx.network(),
                            owner: first.identity.id(),
                            revision: ctx.identity_profile_revision(first.identity.id()),
                            profile: Some((
                                "Alice late picker profile".into(),
                                "Alice biography".into(),
                                String::new(),
                            )),
                        },
                    )),
                },
            ))
            .unwrap();
        harness.run_steps(5);
        assert!(
            harness
                .query_by_label("Alice late picker profile")
                .is_none(),
            "another identity's late picker profile must not be displayed for Bob"
        );
        assert!(harness.query_by_label("Bob correct profile").is_some());
        let root = dash_evo_tool::ui::RootScreenType::RootScreenDashPayProfile;
        let dash_evo_tool::ui::Screen::DashPayScreen(screen) =
            harness.state_mut().main_screens.get_mut(&root).unwrap()
        else {
            panic!("profile screen")
        };
        let AppAction::BackendTaskWithContext {
            context: old_load, ..
        } = screen.profile_screen.trigger_load_profile()
        else {
            panic!("old load")
        };
        let AppAction::BackendTaskWithContext {
            context: current_load,
            ..
        } = screen.profile_screen.trigger_load_profile()
        else {
            panic!("current load")
        };
        for result in [
            dash_evo_tool::app::TaskResult::Success {
                context: old_load.clone(),
                result: Box::new(BackendTaskSuccessResult::DashPayProfile(
                    dash_evo_tool::model::dashpay::ProfileSnapshot {
                        network: ctx.network(),
                        owner: second.identity.id(),
                        revision: ctx.identity_profile_revision(second.identity.id()),
                        profile: Some(("Bob stale profile".into(), String::new(), String::new())),
                    },
                )),
            },
            dash_evo_tool::app::TaskResult::Error {
                context: old_load,
                error: dash_evo_tool::backend_task::error::TaskError::WalletBackendNotYetWired,
            },
            dash_evo_tool::app::TaskResult::Error {
                context: dash_evo_tool::backend_task::BackendTaskContext::Unknown,
                error: dash_evo_tool::backend_task::error::TaskError::WalletBackendNotYetWired,
            },
        ] {
            runtime
                .block_on(harness.state().task_result_sender.send(result))
                .unwrap();
        }
        harness.run_steps(5);
        assert!(
            harness.query_by_label("Loading profile...").is_some(),
            "stale results and unrelated failures cannot end the current load"
        );
        assert!(harness.query_by_label("Bob stale profile").is_none());
        runtime
            .block_on(harness.state().task_result_sender.send(
                dash_evo_tool::app::TaskResult::Success {
                    context: current_load,
                    result: Box::new(BackendTaskSuccessResult::DashPayProfile(
                        dash_evo_tool::model::dashpay::ProfileSnapshot {
                            network: ctx.network(),
                            owner: second.identity.id(),
                            revision: ctx.identity_profile_revision(second.identity.id()),
                            profile: Some((
                                "Bob current profile".into(),
                                String::new(),
                                String::new(),
                            )),
                        },
                    )),
                },
            ))
            .unwrap();
        harness.run_steps(5);
        assert!(harness.query_by_label("Bob current profile").is_some());
        assert!(harness.query_by_label("Loading profile...").is_none());
    });
}
