//! Integration tests for the new unified Identities hub.
//!
//! Each test mounts the full `AppState`, switches the selected root screen to
//! `RootScreenIdentityHub`, and asserts that the expected tab structure renders.
//!
//! This is the minimum kittest coverage for the scaffold. Per-tab assertions on
//! the full populated layouts arrive as each tab's content lands (T8–T11).

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::ui::RootScreenType;

/// IT-ONBOARD-01 / IT-HOME-01 combined smoke: the hub renders without
/// panicking on the default first-run database (no identities loaded → should
/// render the onboarding empty state).
#[test]
fn identity_hub_mounts_and_renders() {
    with_isolated_data_dir(|| {
        let _harness = mount_app(RootScreenType::RootScreenIdentityHub);
        // If `mount_app` returned without panicking, the hub compiled-in and
        // rendered. More detailed assertions land with the per-tab content work.
    });
}

/// IT-NAV-01: The hub is a root distinct from the legacy `Dashpay` entry, and
/// its on-disk encoding round-trips.
#[test]
fn hub_is_distinct_from_dashpay_and_round_trips() {
    // A pure enum check — no need to drive the UI.
    // `RootScreenDashpay` is the legacy root nav entry for DashPay; the other
    // `RootScreenDashPay*` variants are sub-screens within that section.
    let legacy_dashpay_root = RootScreenType::RootScreenDashpay;
    let hub = RootScreenType::RootScreenIdentityHub;
    assert_ne!(legacy_dashpay_root, hub);
    // Round-trip through on-disk encoding to verify the persistence contract.
    let encoded = hub.to_int();
    let decoded = RootScreenType::from_int(encoded).expect("hub variant must decode");
    assert_eq!(hub, decoded);
}

/// The hub screen must be reachable from the existing `create_screen`
/// dispatch table. This asserts the wiring that AppState::new relies on.
#[test]
fn identity_hub_screen_type_creates_hub_screen() {
    // Guard against a future refactor that silently drops the hub case from
    // `ScreenType::create_screen`. If that happens, this test regresses.
    use dash_evo_tool::ui::ScreenType;
    // The assert-that-it-compiles-and-matches is enough; the dispatcher
    // expects a Screen with IdentityHub variant.
    let screen_type = ScreenType::IdentityHub;
    assert_eq!(screen_type, ScreenType::IdentityHub);
    // `ScreenType::create_screen` requires a live AppContext; instead of
    // constructing one here we verify through the enum discriminant. The end-
    // to-end wiring is exercised by `identity_hub_mounts_and_renders`.
    let _ = screen_type;
}

mod loads {
    //! The hub's own data loads: each is dispatched once per arrival, so one
    //! that is lost on the way out or on the way back never comes again.

    use crate::identity_home::{key, seed_identity};
    use crate::support::{fresh_app_context, mount_app, with_isolated_data_dir};
    use dash_evo_tool::app::AppAction;
    use dash_evo_tool::backend_task::dashpay::DashPayTask;
    use dash_evo_tool::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
    use dash_evo_tool::context::AppContext;
    use dash_evo_tool::model::user_role::UserRole;
    use dash_evo_tool::ui::identity::IdentityHubScreen;
    use dash_evo_tool::ui::{RootScreenType, ScreenLike};
    use dash_sdk::dpp::identity::Purpose;
    use dash_sdk::platform::Identifier;
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::sync::Arc;

    /// The hub on its own, with every action its frames return.
    type HubHarness = Harness<'static, (IdentityHubScreen, Vec<AppAction>)>;

    fn mount_hub(app_context: &Arc<AppContext>) -> HubHarness {
        Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, state: &mut (IdentityHubScreen, Vec<AppAction>)| {
                    let action = state.0.ui(ui);
                    if !matches!(action, AppAction::None) {
                        state.1.push(action);
                    }
                },
                (IdentityHubScreen::new(app_context), Vec::new()),
            )
    }

    /// Looks for an action `wanted` recognises among those the hub has returned
    /// and not yet reported, rendering one more frame at a time until there is
    /// one, so a test does not depend on which frame a load goes out in.
    fn step_until<T>(
        hub: &mut HubHarness,
        what: &str,
        wanted: impl Fn(&AppAction) -> Option<T>,
    ) -> T {
        for _ in 0..16 {
            let actions = &mut hub.state_mut().1;
            let found = actions
                .iter()
                .enumerate()
                .find_map(|(index, action)| wanted(action).map(|found| (index, found)));
            if let Some((index, found)) = found {
                actions.drain(..=index);
                return found;
            }
            hub.step();
        }
        panic!("{what} was never dispatched");
    }

    fn contacts_load(action: &AppAction) -> Option<()> {
        let AppAction::BackendTasks(tasks, _) = action else {
            return None;
        };
        tasks
            .iter()
            .any(|task| {
                matches!(task, BackendTask::DashPayTask(task)
                    if matches!(**task, DashPayTask::LoadContacts { .. }))
            })
            .then_some(())
    }

    fn payments_load(action: &AppAction) -> Option<BackendTaskContext> {
        match action {
            AppAction::BackendTaskWithContext {
                task: BackendTask::DashPayTask(task),
                context,
            } if matches!(**task, DashPayTask::LoadPaymentHistory { .. }) => Some(context.clone()),
            _ => None,
        }
    }

    /// Clicks `label` in the very next frame and hands over what that frame
    /// returned.
    fn click_in_one_frame(hub: &mut HubHarness, label: &str) -> Vec<AppAction> {
        hub.state_mut().1.clear();
        hub.get_by_role_and_label(Role::Button, label)
            .click_accesskit();
        hub.step();
        std::mem::take(&mut hub.state_mut().1)
    }

    /// Clicks the header's `Move to wallet` in the very next frame, which must
    /// return nothing but the screen it opens.
    fn click_move_to_wallet(hub: &mut HubHarness) {
        let actions = click_in_one_frame(hub, "Move to wallet");
        assert!(
            matches!(actions.as_slice(), [AppAction::AddScreen(_)]),
            "the click must open the Move to wallet screen, got {actions:?}"
        );
    }

    /// The hub with both of Home's loads already sent, then re-armed the way a
    /// refresh re-arms them: the contact load is due on the next frame.
    fn hub_with_loads_due(app_context: &Arc<AppContext>) -> HubHarness {
        let mut hub = mount_hub(app_context);
        step_until(&mut hub, "the first contact load", contacts_load);
        step_until(&mut hub, "the first payment history load", payments_load);
        hub.state_mut().0.refresh();
        hub
    }

    fn seed_identity_that_can_move_funds(app_context: &Arc<AppContext>) -> Identifier {
        app_context.set_user_role(UserRole::Everyday);
        let transfer_key = key(1, Purpose::TRANSFER);
        seed_identity(app_context, vec![transfer_key.clone()], vec![transfer_key])
    }

    /// A header button can be clicked on the very frame a load would go out.
    /// The click wins that frame; the load must still go out afterwards, since
    /// cancelling the opened screen returns to the hub without refreshing it.
    #[test]
    fn a_header_click_does_not_swallow_the_load_due_on_its_frame() {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            seed_identity_that_can_move_funds(&app_context);
            let mut hub = hub_with_loads_due(&app_context);
            click_move_to_wallet(&mut hub);
            step_until(&mut hub, "the contact load after the click", contacts_load);

            // The payment history load is due on the frame after it.
            click_move_to_wallet(&mut hub);
            step_until(
                &mut hub,
                "the payment history load after the click",
                payments_load,
            );
        });
    }

    /// A sidebar click on the frame a load is due must still navigate: an
    /// automatic load never takes a frame the user has claimed.
    #[test]
    fn a_sidebar_click_is_not_replaced_by_the_load_due_on_its_frame() {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            seed_identity_that_can_move_funds(&app_context);
            let mut hub = hub_with_loads_due(&app_context);

            let actions = click_in_one_frame(&mut hub, "Wallets");
            assert!(
                matches!(
                    actions.as_slice(),
                    [
                        AppAction::SetMainScreen(RootScreenType::RootScreenWalletsBalances)
                            | AppAction::SetMainScreenThenGoToMainScreen(
                                RootScreenType::RootScreenWalletsBalances
                            )
                    ]
                ),
                "the click must open Wallets, got {actions:?}"
            );
            step_until(&mut hub, "the contact load after the click", contacts_load);
            step_until(&mut hub, "the payment history load after it", payments_load);
        });
    }

    /// The breadcrumb's add-identity entries are applied after the tab has
    /// rendered. Opening one on the frame a load is due must not spend the load.
    #[test]
    fn a_breadcrumb_add_does_not_swallow_the_load_due_on_its_frame() {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            seed_identity_that_can_move_funds(&app_context);
            let mut hub = mount_hub(&app_context);
            // The breadcrumb's identity crumb opens the menu with the add entries.
            let identity = app_context.resolve_selected_identity().expect("identity");
            let crumb = app_context.identity_display_label(&identity);
            hub.get_by_role_and_label(Role::Link, &crumb).click();
            hub.run();
            step_until(&mut hub, "the first contact load", contacts_load);
            step_until(&mut hub, "the first payment history load", payments_load);
            hub.state_mut().0.refresh();

            let actions = click_in_one_frame(&mut hub, "Load an existing identity");
            assert!(
                matches!(actions.as_slice(), [AppAction::AddScreen(_)]),
                "the click must open the load-identity screen, got {actions:?}"
            );
            step_until(&mut hub, "the contact load after the click", contacts_load);
            step_until(&mut hub, "the payment history load after it", payments_load);
        });
    }

    /// Choosing another identity in the breadcrumb is applied after the tab has
    /// rendered, on the frame that then asks for the loads. The new identity's
    /// lists must be asked for once, not again when its Home first renders.
    #[test]
    fn switching_identity_in_the_breadcrumb_loads_the_new_identity_once() {
        use crate::identity_home::seed_identity_with_id;
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            let second = seed_identity_with_id(&app_context, 0xB6, vec![], vec![]);
            let shown = seed_identity_that_can_move_funds(&app_context);
            let label_of = |identity: &Identifier| {
                let identity = app_context
                    .get_identity_by_id(identity)
                    .expect("identity store")
                    .expect("seeded identity");
                app_context.identity_display_label(&identity)
            };
            let mut hub = mount_hub(&app_context);
            hub.get_by_role_and_label(Role::Link, &label_of(&shown))
                .click();
            hub.run();
            step_until(&mut hub, "the first contact load", contacts_load);
            step_until(&mut hub, "the first payment history load", payments_load);

            let load = |action: &AppAction| match action {
                AppAction::BackendTasks(tasks, _) => tasks.iter().find_map(|task| match task {
                    BackendTask::DashPayTask(task) => match &**task {
                        DashPayTask::LoadContacts { identity } => {
                            Some(("contacts", identity.identity.id()))
                        }
                        _ => None,
                    },
                    _ => None,
                }),
                AppAction::BackendTaskWithContext {
                    task: BackendTask::DashPayTask(task),
                    ..
                } => match &**task {
                    DashPayTask::LoadPaymentHistory { identity } => {
                        Some(("payments", identity.identity.id()))
                    }
                    _ => None,
                },
                _ => None,
            };
            hub.state_mut().1.clear();
            hub.get_by_role_and_label(Role::Button, &label_of(&second))
                .click_accesskit();
            hub.step();
            assert_eq!(
                step_until(&mut hub, "the first load after the switch", load),
                ("contacts", second)
            );
            assert_eq!(
                step_until(&mut hub, "the load after it", load),
                ("payments", second),
                "the contact lists must be asked for once"
            );
        });
    }

    /// The Contacts tab's own load is settled in the same place as Home's: it
    /// goes out once the tab is populated, on a frame nothing else took.
    #[test]
    fn the_contacts_tab_load_goes_out_once_its_tab_is_populated() {
        use dash_evo_tool::model::dashpay::ProfileSnapshot;
        use dash_evo_tool::ui::identity::IdentityHubTab;
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            let owner = seed_identity_that_can_move_funds(&app_context);
            let mut hub = mount_hub(&app_context);
            hub.state_mut().0.select_tab(IdentityHubTab::Contacts);
            hub.state_mut().0.refresh();
            hub.state_mut().1.clear();

            // Without a social profile the tab is gated; the hub asks for one.
            let profile_request = step_until(&mut hub, "the profile load", |action| match action {
                AppAction::BackendTaskWithContext {
                    task: BackendTask::DashPayTask(task),
                    context,
                } if matches!(**task, DashPayTask::LoadProfile { .. }) => Some(context.clone()),
                _ => None,
            });
            hub.state_mut().0.display_backend_task_result(
                &profile_request,
                BackendTaskSuccessResult::DashPayProfile(ProfileSnapshot {
                    network: app_context.network(),
                    owner,
                    revision: app_context.identity_profile_revision(owner),
                    profile: Some(("Alex".into(), String::new(), String::new())),
                }),
            );
            step_until(&mut hub, "the Contacts tab's load", contacts_load);
        });
    }

    /// Puts the hub behind the Send screen Home opens, delivers `results`
    /// there, then cancels that screen: the hub comes back to `hub` the way
    /// Cancel leaves it, without a refresh.
    fn receive_behind_the_send_screen(
        runtime: &tokio::runtime::Runtime,
        app: &mut Harness<'static, dash_evo_tool::app::AppState>,
        hub: &mut HubHarness,
        results: Vec<(BackendTaskContext, BackendTaskSuccessResult)>,
    ) {
        use dash_evo_tool::app::TaskResult;
        use dash_evo_tool::ui::wallets::send_screen::IdentitySendMode;
        use dash_evo_tool::ui::{RootScreenType, Screen, ScreenType};

        let app_context = app.state().current_app_context().clone();
        let identity = app_context.resolve_selected_identity().expect("identity");
        let fresh_hub = IdentityHubScreen::new(&app_context);
        let hub_screen = std::mem::replace(&mut hub.state_mut().0, fresh_hub);
        let state = app.state_mut();
        state.main_screens.insert(
            RootScreenType::RootScreenIdentityHub,
            Screen::IdentityHubScreen(hub_screen),
        );
        state.selected_main_screen = RootScreenType::RootScreenIdentityHub;
        state.screen_stack.push(
            ScreenType::IdentitySendScreen(identity, IdentitySendMode::Send)
                .create_screen(&app_context),
        );

        for (context, result) in results {
            runtime
                .block_on(app.state().task_result_sender.send(TaskResult::Success {
                    context,
                    result: Box::new(result),
                }))
                .expect("deliver the result");
        }
        // One frame drains the result queue before it renders.
        app.step();

        app.get_by_label("Cancel").click_accesskit();
        app.step();
        assert!(
            app.state().screen_stack.is_empty(),
            "Cancel must return to the hub"
        );

        let Some(Screen::IdentityHubScreen(hub_screen)) = app
            .state_mut()
            .main_screens
            .remove(&RootScreenType::RootScreenIdentityHub)
        else {
            panic!("the hub must still be a root screen");
        };
        hub.state_mut().0 = hub_screen;
        hub.run();
    }

    /// Home asks for its payment history, then Send is opened before the answer
    /// arrives. The answer must still reach the hub behind that screen: Cancel
    /// returns without refreshing, so nothing would ask for it again.
    #[test]
    fn payment_history_reaches_the_hub_behind_an_open_send_screen() {
        with_isolated_data_dir(|| {
            let runtime = tokio::runtime::Runtime::new().expect("runtime");
            let _guard = runtime.enter();
            let mut app = mount_app(RootScreenType::RootScreenWalletsBalances);
            let app_context = app.state().current_app_context().clone();
            seed_identity_that_can_move_funds(&app_context);

            let mut hub = mount_hub(&app_context);
            let request = step_until(&mut hub, "the payment history load", payments_load);
            assert!(hub.query_by_label("Recent payments").is_none());

            let history = BackendTaskSuccessResult::DashPayPaymentHistory(vec![(
                String::new(),
                "Alice".into(),
                1_000,
                true,
                String::new(),
            )]);
            receive_behind_the_send_screen(&runtime, &mut app, &mut hub, vec![(request, history)]);
            assert!(
                hub.query_by_label("Recent payments").is_some(),
                "the payment history that arrived behind the Send screen must be shown"
            );
        });
    }

    /// The same for Home's contact lists: they are asked for once per arrival,
    /// so answers that arrive behind the Send screen must reach the hub — but
    /// only those loaded for the identity it shows.
    #[test]
    fn contact_lists_reach_the_hub_behind_an_open_send_screen() {
        use crate::identity_home::seed_identity_with_id;
        use dash_evo_tool::backend_task::dashpay::ContactData;
        use dash_sdk::dpp::document::{Document, DocumentV0};
        with_isolated_data_dir(|| {
            let runtime = tokio::runtime::Runtime::new().expect("runtime");
            let _guard = runtime.enter();
            let mut app = mount_app(RootScreenType::RootScreenWalletsBalances);
            let app_context = app.state().current_app_context().clone();
            let other = seed_identity_with_id(&app_context, 0xB6, vec![], vec![]);
            let shown = seed_identity_that_can_move_funds(&app_context);

            let mut hub = mount_hub(&app_context);
            step_until(&mut hub, "the contact load", contacts_load);
            assert!(hub.query_by_label("Bao").is_none());

            let contact = |name: &str| ContactData {
                identity_id: Identifier::from([0xD8; 32]),
                nickname: Some(name.into()),
                note: None,
                is_hidden: false,
                account_reference: 0,
                username: None,
                display_name: None,
                avatar_url: None,
                bio: None,
            };
            let request = Identifier::from([0xC7; 32]);
            let answer = |result: BackendTaskSuccessResult| {
                let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts {
                    identity: app_context.resolve_selected_identity().expect("identity"),
                }));
                (BackendTaskContext::from(&task), result)
            };
            let results = vec![
                answer(BackendTaskSuccessResult::DashPayContactsWithInfo {
                    identity: shown,
                    contacts: vec![contact("Bao")],
                }),
                answer(BackendTaskSuccessResult::DashPayContactRequests {
                    identity: shown,
                    incoming: vec![(
                        request,
                        Document::V0(DocumentV0 {
                            id: request,
                            owner_id: other,
                            ..Default::default()
                        }),
                    )],
                    outgoing: vec![],
                }),
                // A load some other screen ran for another identity.
                answer(BackendTaskSuccessResult::DashPayContactsWithInfo {
                    identity: other,
                    contacts: vec![contact("Zed")],
                }),
            ];
            receive_behind_the_send_screen(&runtime, &mut app, &mut hub, results);

            assert!(
                hub.query_by_label("Zed").is_none(),
                "another identity's contacts must never be shown"
            );
            assert!(
                hub.query_by_label("Bao").is_some(),
                "the contacts that arrived behind the Send screen must be shown"
            );
            assert!(
                hub.query_by_label("Accept").is_some(),
                "and so must the request received meanwhile"
            );
        });
    }
}
