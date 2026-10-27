//! Kittest coverage for where a backend-task result goes on the real
//! `AppState`: to the screen that dispatched it, even while another screen
//! covers it, and to the screen in view only when no open screen sent it.
//!
//! No task reaches the backend: dispatches are entered through test seams and
//! their results are injected through the channel the app drains each frame.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::app::{AppState, TaskResult};
use dash_evo_tool::backend_task::dashpay::DashPayTask;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use dash_evo_tool::ui::identity::register_dpns_name_screen::RegisterDpnsNameSource;
use dash_evo_tool::ui::{RootScreenType, Screen, ScreenType};
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

type App = Harness<'static, AppState>;

fn in_app(test: impl FnOnce()) {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();
        test();
    });
}

fn deliver(harness: &mut App, result: TaskResult) {
    assert!(
        harness.state().task_result_sender.try_send(result).is_ok(),
        "the task result channel accepts the result"
    );
    harness.run_steps(3);
}

/// Open a username request page and start its status refresh, recorded as
/// that page's dispatch. Returns the dispatch.
fn refreshing_request_page(harness: &mut App) -> BackendTaskContext {
    let app_context = harness.state().current_app_context().clone();
    let page = ScreenType::UsernameRequestStatus {
        identity_id: Identifier::from([0x61; 32]),
        normalized_label: "alice".into(),
    }
    .create_screen(&app_context);
    harness.state_mut().screen_stack.push(page);
    let dispatch = request_page(harness)
        .begin_refresh_for_test()
        .expect("the page starts its refresh");
    harness.state_mut().test_record_visible_dispatch(&dispatch);
    harness.run_steps(3);
    dispatch
}

fn request_page(
    harness: &mut App,
) -> &mut dash_evo_tool::ui::identity::username_request_screen::UsernameRequestScreen {
    harness
        .state_mut()
        .screen_stack
        .iter_mut()
        .find_map(|screen| match screen {
            Screen::UsernameRequestScreen(page) => Some(page),
            _ => None,
        })
        .expect("the request page is open")
}

/// Cover the request page with the registration form, as "Register another
/// name" does.
fn cover_with_registration(harness: &mut App) {
    let app_context = harness.state().current_app_context().clone();
    let form = ScreenType::RegisterDpnsName(RegisterDpnsNameSource::Identities)
        .create_screen(&app_context);
    harness.state_mut().screen_stack.push(form);
    harness.run_steps(3);
}

/// A request page covered by another screen still settles its own refresh,
/// whether it succeeds or fails, and ignores refreshes it did not send.
#[test]
fn a_covered_request_page_settles_its_own_refresh() {
    for fail in [false, true] {
        in_app(|| {
            let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
            let dispatch = refreshing_request_page(&mut harness);
            cover_with_registration(&mut harness);

            deliver(
                &mut harness,
                TaskResult::Success {
                    context: BackendTaskContext::Unknown,
                    result: Box::new(BackendTaskSuccessResult::MyUsernameRequestsRefreshed),
                },
            );
            assert_eq!(
                request_page(&mut harness).begin_refresh_for_test(),
                None,
                "a refresh the page did not send leaves its own running"
            );

            let result = if fail {
                TaskResult::Error {
                    context: dispatch,
                    error: TaskError::DataContractNotFound,
                }
            } else {
                TaskResult::Success {
                    context: dispatch,
                    result: Box::new(BackendTaskSuccessResult::MyUsernameRequestsRefreshed),
                }
            };
            deliver(&mut harness, result);
            assert_eq!(
                harness.state().test_dispatch_owner_count(),
                0,
                "a dispatch reports once, and its sender is forgotten then"
            );

            harness.state_mut().screen_stack.pop();
            harness.run_steps(3);
            assert!(
                request_page(&mut harness)
                    .begin_refresh_for_test()
                    .is_some(),
                "back in view, the page can refresh again"
            );
        });
    }
}

/// The result of a page closed before it arrived goes to the screen in view,
/// and a failure is still reported by the banner.
#[test]
fn a_closed_page_result_still_reports_its_failure() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let dispatch = refreshing_request_page(&mut harness);
        harness.state_mut().screen_stack.pop();
        harness.run_steps(3);

        let error = TaskError::DataContractNotFound;
        let message = error.to_string();
        deliver(
            &mut harness,
            TaskResult::Error {
                context: dispatch,
                error,
            },
        );
        assert!(
            harness
                .query_all_by_label_contains(&message)
                .next()
                .is_some(),
            "the failure of a closed page is reported"
        );
    });
}

/// Store a username request for `identity_id` so its page has something to
/// show, Refresh button included.
fn store_request(harness: &App, identity_id: Identifier) {
    use dash_evo_tool::model::dpns::ContestDurations;
    use dash_evo_tool::model::dpns_usernames::UsernameRequest;
    let request = UsernameRequest::submitted(
        "ali",
        dash_evo_tool::utils::time::now_ms().saturating_sub(8 * 86_400_000),
        ContestDurations {
            total: std::time::Duration::from_secs(14 * 86_400),
            join: std::time::Duration::from_secs(7 * 86_400),
        },
        None,
    );
    harness
        .state()
        .current_app_context()
        .store_username_requests(&identity_id, vec![request])
        .expect("store the request");
}

/// The frame loop itself records the screen behind a task its `ui()`
/// returns: a real click on Refresh, with the task captured instead of run.
#[test]
fn a_clicked_refresh_reaches_its_page_behind_another_screen() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let identity_id = Identifier::from([0x62; 32]);
        store_request(&harness, identity_id);
        harness.state_mut().test_capture_dispatches();
        let app_context = harness.state().current_app_context().clone();
        let page = ScreenType::UsernameRequestStatus {
            identity_id,
            normalized_label: dash_evo_tool::model::dpns::normalize_dpns_label("ali"),
        }
        .create_screen(&app_context);
        harness.state_mut().screen_stack.push(page);
        harness.run_steps(3);

        harness.get_by_label("Refresh status").click();
        harness.run_steps(3);
        let dispatches = harness.state().test_captured_dispatches();
        assert_eq!(dispatches.len(), 1, "the click dispatches one refresh");
        let dispatch = dispatches[0].1.clone();
        assert_eq!(
            harness.state().test_dispatch_owner_count(),
            1,
            "the frame loop records the page as the refresh's sender"
        );
        assert!(harness.query_by_label("Refreshing…").is_some());

        cover_with_registration(&mut harness);
        deliver(
            &mut harness,
            TaskResult::Success {
                context: dispatch,
                result: Box::new(BackendTaskSuccessResult::MyUsernameRequestsRefreshed),
            },
        );
        harness.state_mut().screen_stack.pop();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("Refresh status").is_some(),
            "the covered page settled its own refresh"
        );
    });
}

/// A failure that skips the general error path still reaches the covered
/// screen that dispatched the task, so it does not wait for good.
#[test]
fn a_covered_page_is_told_of_a_failure_outside_the_general_error_path() {
    use dash_evo_tool::backend_task::migration::MigrationError;
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        let dispatch = refreshing_request_page(&mut harness);
        cover_with_registration(&mut harness);

        deliver(
            &mut harness,
            TaskResult::Error {
                context: dispatch,
                error: TaskError::MigrationFailed {
                    source: std::sync::Arc::new(MigrationError::LegacyDataTooOld {
                        found: 1,
                        minimum_supported: 2,
                    }),
                },
            },
        );
        harness.state_mut().screen_stack.pop();
        harness.run_steps(3);
        assert!(
            request_page(&mut harness)
                .begin_refresh_for_test()
                .is_some(),
            "the page's refresh ended with the failure"
        );
    });
}

/// A screen that handles its own error in place keeps the banner away only
/// while it is in view. Out of view, the user would never see how it was
/// handled, so the banner still reports the failure.
#[test]
fn a_hidden_screen_handling_its_error_still_raises_the_banner() {
    use dash_evo_tool::backend_task::BackendTask;
    use dash_evo_tool::backend_task::identity::IdentityTask;
    for hidden in [false, true] {
        in_app(|| {
            let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
            let dispatch = BackendTaskContext::for_dispatch_for_test(&BackendTask::IdentityTask(
                IdentityTask::RefreshMyUsernameRequests,
            ));
            harness.state_mut().test_record_visible_dispatch(&dispatch);
            if hidden {
                harness.state_mut().selected_main_screen =
                    RootScreenType::RootScreenWalletsBalances;
                harness.run_steps(3);
            }

            // The hub handles this one quietly: its refresh retries by itself.
            let error = TaskError::UsernameRequestRefreshFailed {
                source: Box::new(dash_sdk::Error::Generic("timeout".into())),
            };
            let message = error.to_string();
            deliver(
                &mut harness,
                TaskResult::Error {
                    context: dispatch,
                    error,
                },
            );
            assert_eq!(
                harness.query_all_by_label_contains(&message).count(),
                usize::from(hidden),
                "the banner appears only when the hub is out of view"
            );
        });
    }
}

/// A Send form for a fresh wallet.
fn send_form(harness: &App) -> dash_evo_tool::ui::wallets::send_screen::WalletSendScreen {
    use dash_evo_tool::model::wallet::Wallet;
    use std::sync::{Arc, RwLock};
    let app_context = harness.state().current_app_context().clone();
    let wallet = Wallet::new_from_seed([0x57; 64], app_context.network(), None, None)
        .expect("wallet fixture");
    dash_evo_tool::ui::wallets::send_screen::WalletSendScreen::new(
        &app_context,
        Arc::new(RwLock::new(wallet)),
    )
}

/// A send whose form was closed is not the next form's send. Its outcome is
/// reported by a banner and leaves the form in view as it was.
#[test]
fn a_closed_send_form_result_leaves_a_new_form_alone() {
    use dash_evo_tool::backend_task::BackendTask;
    use dash_evo_tool::backend_task::wallet::WalletTask;
    for fail in [false, true] {
        in_app(|| {
            let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
            let form = Screen::WalletSendScreen(send_form(&harness));
            harness.state_mut().screen_stack.push(form);
            harness.run_steps(3);
            let send = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
                seed_hash: [0x57; 32],
            });
            let dispatch = match harness.state_mut().screen_stack.last_mut() {
                Some(Screen::WalletSendScreen(form)) => form.confirm_send_for_test(send),
                _ => panic!("the Send form is in view"),
            }
            .expect("the confirmed send is the form's dispatch");
            harness.state_mut().test_record_visible_dispatch(&dispatch);
            harness.run_steps(3);

            // The user closes the form while the send runs and opens a new one.
            harness.state_mut().screen_stack.pop();
            let form = Screen::WalletSendScreen(send_form(&harness));
            harness.state_mut().screen_stack.push(form);
            harness.run_steps(3);

            let (result, message) = if fail {
                let error = TaskError::DataContractNotFound;
                let message = error.to_string();
                (
                    TaskResult::Error {
                        context: dispatch,
                        error,
                    },
                    message,
                )
            } else {
                (
                    TaskResult::Success {
                        context: dispatch,
                        result: Box::new(BackendTaskSuccessResult::WalletPayment {
                            txid: "00".repeat(32),
                            recipients: vec![("yTestRecipient".into(), 100_000)],
                            total_amount: 100_000,
                        }),
                    },
                    format!(
                        "Sent {} to yTestRecipient",
                        dash_evo_tool::model::fee_estimation::format_duffs_as_dash(100_000)
                    ),
                )
            };
            deliver(&mut harness, result);
            // The form shows no outcome (below), so this one is the banner.
            assert_eq!(
                harness.query_all_by_label_contains(&message).count(),
                1,
                "a banner tells how the closed form's send ended"
            );
            assert_eq!(
                harness.query_all_by_label_contains("Send Another").count(),
                0,
                "the new form did not send it and does not claim it"
            );
        });
    }
}

/// The Documents screen in view.
fn documents(
    harness: &mut App,
) -> &mut dash_evo_tool::ui::contracts_documents::contracts_documents_screen::DocumentQueryScreen {
    match harness
        .state_mut()
        .main_screens
        .get_mut(&RootScreenType::RootScreenDocumentQuery)
    {
        Some(Screen::DocumentQueryScreen(screen)) => screen,
        _ => panic!("the Documents screen is open"),
    }
}

/// A send whose form was closed must not disturb the screen in view: a
/// document query running there keeps waiting for its own answer, and takes
/// it when it comes.
#[test]
fn a_closed_send_form_result_leaves_a_running_query_alone() {
    use dash_evo_tool::backend_task::BackendTask;
    use dash_evo_tool::backend_task::wallet::WalletTask;
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenDocumentQuery);
        let form = Screen::WalletSendScreen(send_form(&harness));
        harness.state_mut().screen_stack.push(form);
        harness.run_steps(3);
        let send = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
            seed_hash: [0x57; 32],
        });
        let dispatch = match harness.state_mut().screen_stack.last_mut() {
            Some(Screen::WalletSendScreen(form)) => form.confirm_send_for_test(send),
            _ => panic!("the Send form is in view"),
        }
        .expect("the confirmed send is the form's dispatch");
        harness.state_mut().test_record_visible_dispatch(&dispatch);

        // The user closes the form and starts a query on the Documents screen.
        harness.state_mut().screen_stack.pop();
        harness.run_steps(3);
        let query = documents(&mut harness)
            .begin_fetch_for_test()
            .expect("the query is the screen's dispatch");
        harness.state_mut().test_record_visible_dispatch(&query);

        deliver(
            &mut harness,
            TaskResult::Success {
                context: dispatch,
                result: Box::new(BackendTaskSuccessResult::WalletPayment {
                    txid: "00".repeat(32),
                    recipients: vec![("yTestRecipient".into(), 100_000)],
                    total_amount: 100_000,
                }),
            },
        );
        assert!(
            documents(&mut harness).fetch_pending_for_test(),
            "the query still waits for its own answer"
        );

        deliver(
            &mut harness,
            TaskResult::Success {
                context: query,
                result: Box::new(BackendTaskSuccessResult::PageDocuments(
                    Default::default(),
                    None,
                )),
            },
        );
        assert!(
            !documents(&mut harness).fetch_pending_for_test(),
            "the query took its own answer"
        );
    });
}

#[test]
fn a_closed_batch_purchase_does_not_complete_the_next_form() {
    use dash_evo_tool::app::{AppAction, BackendTasksExecutionMode};
    use dash_evo_tool::backend_task::tokens::TokenTask;
    use dash_evo_tool::backend_task::{BackendTask, FeeResult};
    use dash_evo_tool::model::qualified_contract::QualifiedContract;
    use dash_evo_tool::ui::tokens::direct_token_purchase_screen::PurchaseTokenScreen;
    use dash_evo_tool::ui::tokens::tokens_screen::IdentityTokenInfo;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dash_sdk::dpp::identity::Purpose;
    use std::sync::Arc;
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenMyTokenBalances);
        harness.state_mut().test_capture_dispatches();
        let context = harness.state().current_app_context().clone();
        let key = crate::identity_home::key(0, Purpose::AUTHENTICATION);
        let id = crate::identity_home::seed_identity(&context, vec![key.clone()], vec![]);
        let info = IdentityTokenInfo {
            token_id: Identifier::from([0x23; 32]),
            token_alias: "Test token".into(),
            identity: context.get_identity_by_id(&id).unwrap().unwrap(),
            data_contract: QualifiedContract {
                contract: dash_sdk::dpp::system_data_contracts::load_system_data_contract(
                    dash_sdk::dpp::system_data_contracts::SystemDataContract::DPNS,
                    dash_sdk::dpp::version::PlatformVersion::latest(),
                )
                .unwrap(),
                alias: None,
            },
            token_config: TokenConfigurationV0::default_most_restrictive().into(),
            token_position: 0,
        };
        harness
            .state_mut()
            .screen_stack
            .push(Screen::PurchaseTokenScreen(PurchaseTokenScreen::new(
                info.clone(),
                &context,
            )));
        harness
            .state_mut()
            .test_dispatch_screen_action(AppAction::BackendTasks(
                vec![
                    BackendTask::TokenTask(Box::new(TokenTask::PurchaseTokens {
                        identity: info.identity.clone(),
                        data_contract: Arc::new(info.data_contract.contract.clone()),
                        token_position: 0,
                        signing_key: key,
                        amount: 1,
                        total_agreed_price: 1,
                    })),
                    BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
                ],
                BackendTasksExecutionMode::Sequential,
            ));
        let dispatch = harness.state().test_captured_dispatches()[0].1.clone();
        harness.state_mut().screen_stack.pop();
        harness
            .state_mut()
            .screen_stack
            .push(Screen::PurchaseTokenScreen(PurchaseTokenScreen::new(
                info, &context,
            )));
        deliver(
            &mut harness,
            TaskResult::Success {
                context: dispatch,
                result: Box::new(BackendTaskSuccessResult::PurchasedTokens(FeeResult::new(
                    1, 1,
                ))),
            },
        );
        assert!(
            harness.query_by_label("Purchase Successful!").is_none(),
            "the newly opened form did not make the previous form's purchase"
        );
    });
}

#[test]
fn a_balance_cache_notification_keeps_a_document_query_pending() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenDocumentQuery);
        harness.state_mut().test_capture_dispatches();
        let query = documents(&mut harness).begin_fetch_for_test().unwrap();
        harness.state_mut().test_record_visible_dispatch(&query);
        // The notification emitted when QueryMyTokenBalances updates its cache.
        let network = harness.state().current_app_context().network();
        deliver(
            &mut harness,
            TaskResult::CacheUpdated(dash_evo_tool::backend_task::CacheUpdate {
                network,
                domain: dash_evo_tool::backend_task::CacheDomain::Tokens,
            }),
        );
        assert!(
            documents(&mut harness).fetch_pending_for_test(),
            "a balance cache update must not reset an unrelated query"
        );
        deliver(
            &mut harness,
            TaskResult::Success {
                context: query,
                result: Box::new(BackendTaskSuccessResult::PageDocuments(
                    Default::default(),
                    None,
                )),
            },
        );
        assert!(!documents(&mut harness).fetch_pending_for_test());
    });
}

#[test]
fn rejected_dispatch_settles_its_owner_even_when_the_result_channel_is_full() {
    use dash_evo_tool::app::AppAction;
    use dash_evo_tool::backend_task::{BackendTask, system_task::SystemTask};
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenDocumentQuery);
        tokio::runtime::Handle::current()
            .block_on(harness.state().subtasks.shutdown_async())
            .unwrap();
        while harness
            .state()
            .task_result_sender
            .try_send(TaskResult::Repaint)
            .is_ok()
        {}
        harness
            .state_mut()
            .test_dispatch_screen_action(AppAction::BackendTask(BackendTask::SystemTask(
                SystemTask::WipePlatformData,
            )));
        assert_eq!(harness.state().test_dispatch_owner_count(), 1);
        harness.run_steps(3);
        assert_eq!(harness.state().test_dispatch_owner_count(), 0);
        let message = TaskError::TaskManagerShuttingDown.to_string();
        assert!(
            harness
                .query_all_by_label_contains(&message)
                .next()
                .is_some()
        );
    });
}

/// The contact-list reads Home dispatched while it was in view.
fn home_contact_list_reads(harness: &App) -> Vec<BackendTaskContext> {
    harness
        .state()
        .test_captured_dispatches()
        .iter()
        .filter(|(task, _)| {
            matches!(task, BackendTask::DashPayTask(task)
                if matches!(**task, DashPayTask::LoadContacts { .. }))
        })
        .map(|(_, context)| context.clone())
        .collect()
}

/// Home's contact reads report to the hub even while another screen covers
/// it, so a temporary failure there is retried after a plain Back instead of
/// leaving Home loading for good.
#[test]
fn home_retries_a_contact_read_that_failed_while_covered() {
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        let app_context = harness.state().current_app_context().clone();
        app_context.connection_status().set_masternodes_ready(true);
        harness.state_mut().test_capture_dispatches();
        crate::identity_home::seed_identity(&app_context, vec![], vec![]);
        harness.run_steps(3);
        let reads = home_contact_list_reads(&harness);
        assert_eq!(reads.len(), 1, "Home asks for its contact list once");
        // A reconnect drops readiness while the read is running.
        app_context.connection_status().set_masternodes_ready(false);

        let add_contact = ScreenType::DashPayAddContact.create_screen(&app_context);
        harness.state_mut().screen_stack.push(add_contact);
        harness.run_steps(3);
        deliver(
            &mut harness,
            TaskResult::Error {
                context: reads[0].clone(),
                error: TaskError::MasternodeListNotReady {
                    source_error: Box::new(dash_sdk::Error::Generic("syncing".into())),
                },
            },
        );

        // Back from Add Contact pops the screen without refreshing the hub.
        harness.state_mut().screen_stack.pop();
        app_context.connection_status().set_masternodes_ready(true);
        harness.run_steps(3);
        assert_eq!(
            home_contact_list_reads(&harness).len(),
            2,
            "the hub heard of the failure and retries once the network is ready"
        );
    });
}

/// The tasks dispatched since capture began that satisfy `pick`.
fn captured_contexts(
    harness: &App,
    pick: impl Fn(&BackendTask) -> bool,
) -> Vec<BackendTaskContext> {
    harness
        .state()
        .test_captured_dispatches()
        .iter()
        .filter(|(task, _)| pick(task))
        .map(|(_, context)| context.clone())
        .collect()
}

/// A balance refresh started from My Tokens settles when its own result
/// arrives through the production dispatch, so its progress banner goes away
/// and the next Refresh is sent instead of being taken for a duplicate.
#[test]
fn token_balance_refresh_settles_on_its_dispatched_result() {
    use dash_evo_tool::backend_task::tokens::TokenTask;
    let is_refresh = |task: &BackendTask| {
        matches!(task, BackendTask::TokenTask(task)
            if matches!(**task, TokenTask::QueryMyTokenBalances))
    };
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenMyTokenBalances);
        harness.state_mut().test_capture_dispatches();
        harness.get_by_label("Refresh").click();
        harness.run_steps(3);
        let refreshes = captured_contexts(&harness, is_refresh);
        assert_eq!(refreshes.len(), 1, "Refresh sends one balance query");
        assert!(
            harness
                .query_all_by_label_contains("Refreshing tokens")
                .next()
                .is_some()
        );

        deliver(
            &mut harness,
            TaskResult::Success {
                context: refreshes[0].clone(),
                result: Box::new(BackendTaskSuccessResult::FetchedTokenBalances),
            },
        );
        assert!(
            harness
                .query_all_by_label_contains("Refreshing tokens")
                .next()
                .is_none(),
            "the refresh's own result ends it"
        );

        harness.get_by_label("Refresh").click();
        harness.run_steps(3);
        assert_eq!(
            captured_contexts(&harness, is_refresh).len(),
            2,
            "a finished refresh does not block the next one"
        );
    });
}

/// A restore from the previous version that fails through the production
/// dispatch frees the Restore button again.
#[test]
fn failed_restore_from_previous_version_frees_its_button() {
    fn chooser(
        harness: &mut App,
    ) -> &mut dash_evo_tool::ui::network_chooser_screen::NetworkChooserScreen {
        match harness
            .state_mut()
            .main_screens
            .get_mut(&RootScreenType::RootScreenNetworkChooser)
        {
            Some(Screen::NetworkChooserScreen(screen)) => screen,
            _ => panic!("the network chooser is open"),
        }
    }
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenNetworkChooser);
        harness.state_mut().test_capture_dispatches();
        let action = chooser(&mut harness).begin_restore_for_test();
        harness.state_mut().test_dispatch_screen_action(action);
        let restores = captured_contexts(&harness, |_| true);
        assert_eq!(restores.len(), 1, "the restore is dispatched once");

        deliver(
            &mut harness,
            TaskResult::Error {
                context: restores[0].clone(),
                error: TaskError::TaskManagerShuttingDown,
            },
        );
        assert!(
            !chooser(&mut harness).restore_in_progress_for_test(),
            "the failed restore releases the button"
        );
    });
}

/// A document query sent on one network whose page arrives after a switch to
/// another reaches none of the new network's screens: the Documents screen,
/// back in view, neither shows the old page nor keeps waiting for it.
#[test]
fn a_result_sent_before_a_network_switch_stays_off_the_new_network() {
    use dash_evo_tool::app::BootPhase;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
    use std::time::{Duration, Instant};
    in_app(|| {
        let mut harness = mount_app(RootScreenType::RootScreenDocumentQuery);
        let query = documents(&mut harness).begin_fetch_for_test().unwrap();
        harness.state_mut().test_record_visible_dispatch(&query);

        let first = harness.state().current_app_context().network();
        let second = if first == Network::Testnet {
            Network::Mainnet
        } else {
            Network::Testnet
        };
        harness.state_mut().change_network(second);
        let deadline = Instant::now() + Duration::from_secs(60);
        while harness.state().current_app_context().network() != second
            || harness.state().boot_phase() != BootPhase::Ready
        {
            assert!(
                Instant::now() < deadline,
                "the switch did not finish in time"
            );
            harness.step();
        }
        harness.state_mut().selected_main_screen = RootScreenType::RootScreenDocumentQuery;
        harness.run_steps(3);

        deliver(
            &mut harness,
            TaskResult::Success {
                context: query,
                result: Box::new(BackendTaskSuccessResult::PageDocuments(
                    Default::default(),
                    Some(Start::StartAfter(vec![1])),
                )),
            },
        );
        assert!(
            harness.query_by_label("Next Page").is_none(),
            "the previous network's page is not shown on the new network"
        );
        assert!(
            !documents(&mut harness).fetch_pending_for_test(),
            "the query that ended with the previous network is no longer awaited"
        );
    });
}

fn token_creator(harness: &mut App) -> &mut dash_evo_tool::ui::tokens::tokens_screen::TokensScreen {
    match harness
        .state_mut()
        .main_screens
        .get_mut(&RootScreenType::RootScreenTokenCreator)
    {
        Some(Screen::TokensScreen(screen)) => screen,
        _ => panic!("the Token Creator is open"),
    }
}

/// Switch to the other of Testnet and Mainnet and wait until it is ready.
fn switch_network(harness: &mut App) {
    use dash_evo_tool::app::BootPhase;
    use dash_sdk::dpp::dashcore::Network;
    use std::time::{Duration, Instant};
    let first = harness.state().current_app_context().network();
    let second = if first == Network::Testnet {
        Network::Mainnet
    } else {
        Network::Testnet
    };
    harness.state_mut().change_network(second);
    let deadline = Instant::now() + Duration::from_secs(60);
    while harness.state().current_app_context().network() != second
        || harness.state().boot_phase() != BootPhase::Ready
    {
        assert!(
            Instant::now() < deadline,
            "the switch did not finish in time"
        );
        harness.step();
    }
}

/// A token registration sent on one network whose outcome arrives after a
/// switch, while the Token Creator registers another token on the new
/// network, leaves that registration alone: a success neither completes it
/// nor raises a banner, and a failure neither ends its progress nor reaches
/// its screen. The failure is still reported by a banner, as for a closed
/// sender.
#[test]
fn a_registration_result_from_the_previous_network_leaves_the_token_creator_alone() {
    use dash_evo_tool::ui::components::message_banner::MessageBanner;
    use dash_evo_tool::ui::tokens::tokens_screen::TokenCreatorStatus;
    for fails in [false, true] {
        in_app(|| {
            let mut harness = mount_app(RootScreenType::RootScreenTokenCreator);
            let previous = token_creator(&mut harness).begin_token_registration_for_test();
            harness.state_mut().test_record_visible_dispatch(&previous);

            switch_network(&mut harness);
            harness.state_mut().selected_main_screen = RootScreenType::RootScreenTokenCreator;
            harness.run_steps(3);
            let ctx = harness.ctx.clone();
            MessageBanner::clear_all_global(&ctx);
            let current = token_creator(&mut harness).begin_token_registration_for_test();
            harness.state_mut().test_record_visible_dispatch(&current);
            harness.run_steps(3);

            let late = if fails {
                TaskResult::Error {
                    context: previous,
                    error: TaskError::TaskManagerShuttingDown,
                }
            } else {
                TaskResult::Success {
                    context: previous,
                    result: Box::new(BackendTaskSuccessResult::RegisteredTokenContract),
                }
            };
            deliver(&mut harness, late);

            let creator = token_creator(&mut harness);
            assert_eq!(
                *creator.token_creator_status_for_test(),
                TokenCreatorStatus::WaitingForResult,
                "the new network's registration is still waiting (fails: {fails})"
            );
            assert!(
                creator.registration_banner_live_for_test(),
                "its progress banner stays (fails: {fails})"
            );
            let failure_reported = harness
                .query_all_by_label_contains(&TaskError::TaskManagerShuttingDown.to_string())
                .next()
                .is_some();
            assert_eq!(
                failure_reported, fails,
                "only the failure raises a banner, as for a closed sender"
            );
        });
    }
}

/// A scheduled-vote sweep that ran on the previous network reaches no screen
/// of the new one when it ends after a switch. The Token Creator, in view and
/// registering a token on the new network, keeps its progress banner through
/// the sweep's failure and its completion. The failure is still reported by
/// a banner.
#[test]
fn a_scheduled_vote_sweep_from_the_previous_network_stays_off_the_new_network() {
    use dash_evo_tool::ui::components::message_banner::MessageBanner;
    use dash_evo_tool::ui::tokens::tokens_screen::TokenCreatorStatus;
    for fails in [false, true] {
        in_app(|| {
            let mut harness = mount_app(RootScreenType::RootScreenTokenCreator);
            let previous_network = harness.state().current_app_context().network();

            switch_network(&mut harness);
            harness.state_mut().selected_main_screen = RootScreenType::RootScreenTokenCreator;
            harness.run_steps(3);
            let ctx = harness.ctx.clone();
            MessageBanner::clear_all_global(&ctx);
            let current = token_creator(&mut harness).begin_token_registration_for_test();
            harness.state_mut().test_record_visible_dispatch(&current);
            harness.run_steps(3);

            let context = BackendTaskContext::ScheduledVoteSweep {
                network: previous_network,
            };
            let failure = || TaskError::ScheduledVoteSweepFailed {
                network: previous_network,
                source: Box::new(TaskError::TaskManagerShuttingDown),
            };
            let late = if fails {
                TaskResult::Error {
                    context,
                    error: failure(),
                }
            } else {
                TaskResult::Success {
                    context,
                    result: Box::new(BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
                        network: previous_network,
                        preserve_eligibility_since_ms: None,
                    }),
                }
            };
            deliver(&mut harness, late);

            let creator = token_creator(&mut harness);
            assert_eq!(
                *creator.token_creator_status_for_test(),
                TokenCreatorStatus::WaitingForResult,
                "the new network's registration is still waiting (fails: {fails})"
            );
            assert!(
                creator.registration_banner_live_for_test(),
                "its progress banner stays (fails: {fails})"
            );
            let failure_reported = harness
                .query_all_by_label_contains(&failure().to_string())
                .next()
                .is_some();
            assert_eq!(failure_reported, fails, "only the failure raises a banner");
        });
    }
}
