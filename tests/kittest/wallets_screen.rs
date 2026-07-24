use crate::support::{fresh_app_context, with_isolated_data_dir};
#[cfg(feature = "testing")]
use dash_evo_tool::app::AppAction;
use dash_evo_tool::backend_task::BackendTaskSuccessResult;
#[cfg(feature = "testing")]
use dash_evo_tool::backend_task::error::TaskError;
#[cfg(feature = "testing")]
use dash_evo_tool::backend_task::wallet::WalletTask;
#[cfg(feature = "testing")]
use dash_evo_tool::backend_task::{BackendTask, BackendTaskContext};
use dash_evo_tool::model::secret::Secret;
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::model::wallet::birth_height::WalletOrigin;
#[cfg(feature = "testing")]
use dash_evo_tool::ui::components::MessageBanner;
use dash_evo_tool::ui::wallets::wallets_screen::WalletsBalancesScreen;
use dash_evo_tool::ui::{MessageType, ScreenLike};
#[cfg(feature = "testing")]
use dash_sdk::dashcore_rpc::dashcore::Network;
#[cfg(feature = "testing")]
use dash_sdk::dpp::address_funds::PlatformAddress;
#[cfg(feature = "testing")]
use dash_sdk::dpp::dashcore::PrivateKey;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
#[cfg(feature = "testing")]
use std::cell::{Cell, RefCell};
#[cfg(feature = "testing")]
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use zeroize::Zeroize;

const WALLET_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(5);

fn build_wallet_screen_harness(
    runtime: tokio::runtime::Runtime,
    app_context: Arc<dash_evo_tool::context::AppContext>,
) -> Harness<'static, WalletsBalancesScreen> {
    let screen = WalletsBalancesScreen::new(&app_context);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(
            move |ui, screen: &mut WalletsBalancesScreen| {
                let _runtime = &runtime;
                screen.ui(ui);
            },
            screen,
        );
    harness.run();
    harness
}

fn wallet_screen_harness(password: Option<&Secret>) -> Harness<'static, WalletsBalancesScreen> {
    let (runtime, app_context) = fresh_app_context();
    let mut seed: [u8; 64] = rand::random();
    let mut wallet = Wallet::new_from_seed(
        seed,
        app_context.network(),
        Some("Dialog wallet".to_string()),
        password,
    )
    .expect("create wallet fixture");
    seed.zeroize();
    if password.is_some() {
        wallet.wallet_seed.close();
    }
    let seed_hash = wallet.seed_hash();
    app_context
        .wallets()
        .write()
        .expect("wallet map")
        .insert(seed_hash, Arc::new(RwLock::new(wallet)));

    build_wallet_screen_harness(runtime, app_context)
}

fn registered_wallet_screen_harness() -> Harness<'static, WalletsBalancesScreen> {
    let (runtime, app_context) = fresh_app_context();
    let mut seed: [u8; 64] = rand::random();
    let wallet = Wallet::new_from_seed(
        seed,
        app_context.network(),
        Some("Dialog wallet".to_string()),
        None,
    )
    .expect("create wallet fixture");
    let (seed_hash, _) = {
        let _guard = runtime.enter();
        app_context
            .register_wallet(wallet, &seed, WalletOrigin::Imported)
            .expect("register wallet fixture")
    };
    seed.zeroize();
    let backend = app_context
        .wallet_backend()
        .expect("wallet backend must be wired");
    let monitored_addresses = runtime.block_on(async {
        tokio::time::timeout(WALLET_REGISTRATION_TIMEOUT, async {
            loop {
                let addresses = backend.snapshot_monitored_receive_addresses(&seed_hash);
                if !addresses.is_empty() {
                    return addresses;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("wallet registration must publish monitored receive addresses")
    });
    assert!(
        !monitored_addresses.is_empty(),
        "the fixture must have a monitored receive address before the click"
    );

    build_wallet_screen_harness(runtime, app_context)
}

fn click_in_one_frame(harness: &mut Harness<'_, WalletsBalancesScreen>, label: &str) {
    let pos = harness.get_by_label(label).rect().center();
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        },
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        },
    ]);
    harness.step();
}

#[cfg(feature = "testing")]
fn press_label(harness: &mut Harness<'_, WalletsBalancesScreen>, label: &str) {
    let pos = harness.get_by_label(label).rect().center();
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        },
    ]);
    harness.step();
}

#[cfg(feature = "testing")]
fn release_pointer_away(harness: &mut Harness<'_, WalletsBalancesScreen>) {
    let pos = egui::pos2(0.0, 0.0);
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        },
    ]);
    harness.step();
}

#[cfg(feature = "testing")]
fn platform_addresses(count: u8, network: Network) -> Vec<(String, u64)> {
    (1..=count)
        .map(|byte| {
            (
                PlatformAddress::P2pkh([byte; 20]).to_bech32m_string(network),
                0,
            )
        })
        .collect()
}

#[cfg(feature = "testing")]
#[derive(Debug, Clone)]
struct CapturedRename {
    task: WalletTask,
    context: BackendTaskContext,
}

#[cfg(feature = "testing")]
fn capture_rename_action(action: AppAction) -> Option<CapturedRename> {
    let (task, context) = match action {
        AppAction::BackendTask(BackendTask::WalletTask(task)) => {
            let backend_task = BackendTask::WalletTask(task.clone());
            (task, BackendTaskContext::from(&backend_task))
        }
        AppAction::BackendTaskWithContext {
            task: BackendTask::WalletTask(task),
            context,
        } => (task, context),
        _ => return None,
    };
    matches!(
        task,
        WalletTask::RenameHdWallet { .. } | WalletTask::RenameSingleKeyWallet { .. }
    )
    .then_some(CapturedRename { task, context })
}

#[cfg(feature = "testing")]
fn build_rename_harness(
    runtime: tokio::runtime::Runtime,
    app_context: Arc<dash_evo_tool::context::AppContext>,
) -> (
    Harness<'static, WalletsBalancesScreen>,
    Rc<RefCell<Vec<CapturedRename>>>,
) {
    let dispatched = Rc::new(RefCell::new(Vec::new()));
    let captured = dispatched.clone();
    let screen = WalletsBalancesScreen::new(&app_context);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(
            move |ui, screen: &mut WalletsBalancesScreen| {
                let _runtime = &runtime;
                if let Some(rename) = capture_rename_action(screen.ui(ui)) {
                    captured.borrow_mut().push(rename);
                }
            },
            screen,
        );
    harness.run();
    (harness, dispatched)
}

#[cfg(feature = "testing")]
fn enter_rename_alias(harness: &mut Harness<'_, WalletsBalancesScreen>, alias: &str) {
    click_in_one_frame(harness, "Rename");
    let input = harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .expect("rename input");
    input.focus();
    harness.event(egui::Event::Text(alias.to_string()));
    harness.step();
}

#[cfg(feature = "testing")]
fn take_rename_dispatch(dispatched: &Rc<RefCell<Vec<CapturedRename>>>) -> CapturedRename {
    let mut dispatched = dispatched.borrow_mut();
    assert_eq!(dispatched.len(), 1, "exactly one rename must dispatch");
    dispatched.pop().expect("rename dispatch")
}

#[cfg(feature = "testing")]
fn random_wif(network: Network) -> String {
    loop {
        let mut key_bytes: [u8; 32] = rand::random();
        if let Ok(private_key) = PrivateKey::from_byte_array(&key_bytes, network) {
            key_bytes.zeroize();
            return private_key.to_wif();
        }
        key_bytes.zeroize();
    }
}

#[test]
#[cfg(feature = "testing")]
fn hd_rename_dispatches_exact_task_and_applies_success() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let mut seed: [u8; 64] = rand::random();
        let wallet =
            Wallet::new_from_seed(seed, app_context.network(), None, None).expect("wallet");
        seed.zeroize();
        let seed_hash = wallet.seed_hash();
        let wallet = Arc::new(RwLock::new(wallet));
        app_context
            .wallets()
            .write()
            .expect("wallet map")
            .insert(seed_hash, wallet.clone());
        app_context.set_selected_hd_wallet(Some(seed_hash));

        let (mut harness, dispatched) = build_rename_harness(runtime, app_context);
        enter_rename_alias(&mut harness, "Renamed HD");
        harness.get_by_label("Save").click();
        harness.run();

        let dispatch = take_rename_dispatch(&dispatched);
        assert_eq!(
            dispatch.task,
            WalletTask::RenameHdWallet {
                seed_hash,
                alias: "Renamed HD".into(),
            }
        );
        assert!(
            harness.query_all_by_value("Renamed HD").next().is_some(),
            "the attempted alias must remain visible while saving"
        );
        assert!(
            harness
                .query_all_by_role(egui::accesskit::Role::TextInput)
                .next()
                .expect("rename input")
                .accesskit_node()
                .is_disabled(),
            "the alias input must be disabled while saving"
        );
        assert!(
            harness.get_by_label("Save").accesskit_node().is_disabled()
                && harness
                    .get_by_label("Cancel")
                    .accesskit_node()
                    .is_disabled(),
            "Save and Cancel must be disabled while saving"
        );
        let outside = egui::pos2(0.0, 0.0);
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(outside),
            egui::Event::PointerButton {
                pos: outside,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: outside,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        harness.step();
        assert!(
            harness.query_by_label("Enter new wallet name:").is_some(),
            "outside clicks must not dismiss a rename while it is saving"
        );

        harness.state_mut().display_backend_task_result(
            &dispatch.context,
            BackendTaskSuccessResult::WalletAliasRenamed {
                seed_hash,
                alias: "Renamed HD".into(),
            },
        );
        harness.run();

        assert_eq!(
            wallet.read().expect("wallet").alias.as_deref(),
            Some("Renamed HD")
        );
        assert!(
            harness.query_by_label("Enter new wallet name:").is_none(),
            "success must close the rename dialog"
        );
    });
}

#[test]
#[cfg(feature = "testing")]
fn rename_button_is_disabled_and_inert_while_save_is_pending() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let mut seed: [u8; 64] = rand::random();
        let wallet =
            Wallet::new_from_seed(seed, app_context.network(), None, None).expect("wallet");
        seed.zeroize();
        let seed_hash = wallet.seed_hash();
        app_context
            .wallets()
            .write()
            .expect("wallet map")
            .insert(seed_hash, Arc::new(RwLock::new(wallet)));
        app_context.set_selected_hd_wallet(Some(seed_hash));

        let (mut harness, dispatched) = build_rename_harness(runtime, app_context);
        enter_rename_alias(&mut harness, "Pending rename");
        harness.get_by_label("Save").click();
        harness.run();

        assert_eq!(dispatched.borrow().len(), 1, "the rename must dispatch");
        assert!(
            harness
                .get_by_label("Rename")
                .accesskit_node()
                .is_disabled(),
            "the Rename entry point must be disabled while saving"
        );

        click_in_one_frame(&mut harness, "Rename");
        assert!(
            harness
                .query_all_by_value("Pending rename")
                .next()
                .is_some(),
            "clicking the disabled entry point must preserve the pending alias"
        );
        assert!(
            harness.get_by_label("Save").accesskit_node().is_disabled(),
            "clicking the disabled entry point must not reset the saving state"
        );
    });
}

#[test]
#[cfg(feature = "testing")]
fn single_key_rename_dispatches_exact_task_and_applies_success() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let (_imported, wallet) = app_context
            .import_single_key_wif(&random_wif(app_context.network()), None, Default::default())
            .expect("import key");
        let (key_hash, address) = {
            let wallet = wallet.read().expect("wallet");
            (wallet.key_hash, wallet.address.to_string())
        };
        app_context.set_selected_single_key_wallet(Some(key_hash));

        let (mut harness, dispatched) = build_rename_harness(runtime, app_context);
        enter_rename_alias(&mut harness, "Renamed key");
        harness.get_by_label("Save").click();
        harness.run();

        let dispatch = take_rename_dispatch(&dispatched);
        assert_eq!(
            dispatch.task,
            WalletTask::RenameSingleKeyWallet {
                address: address.clone(),
                alias: "Renamed key".into(),
            }
        );
        assert!(
            harness.query_all_by_value("Renamed key").next().is_some(),
            "the attempted alias must remain visible while saving"
        );

        harness.state_mut().display_backend_task_result(
            &dispatch.context,
            BackendTaskSuccessResult::SingleKeyAliasRenamed {
                address,
                alias: "Renamed key".into(),
            },
        );
        harness.run();

        assert_eq!(
            wallet.read().expect("wallet").alias.as_deref(),
            Some("Renamed key")
        );
        assert!(
            harness.query_by_label("Enter new wallet name:").is_none(),
            "success must close the rename dialog"
        );
    });
}

#[test]
#[cfg(feature = "testing")]
fn hd_rename_success_updates_original_wallet_after_selection_change() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let mut target_seed: [u8; 64] = rand::random();
        let target = Wallet::new_from_seed(
            target_seed,
            app_context.network(),
            Some("Target".into()),
            None,
        )
        .expect("target wallet");
        target_seed.zeroize();
        let target_hash = target.seed_hash();
        let target = Arc::new(RwLock::new(target));

        let mut other_seed: [u8; 64] = rand::random();
        let other = Wallet::new_from_seed(
            other_seed,
            app_context.network(),
            Some("Other".into()),
            None,
        )
        .expect("other wallet");
        other_seed.zeroize();
        let other_hash = other.seed_hash();
        let other = Arc::new(RwLock::new(other));

        {
            let mut wallets = app_context.wallets().write().expect("wallet map");
            wallets.insert(target_hash, target.clone());
            wallets.insert(other_hash, other);
        }
        app_context.set_selected_hd_wallet(Some(target_hash));

        let (mut harness, dispatched) = build_rename_harness(runtime, app_context);
        click_in_one_frame(&mut harness, "Rename");
        let input = harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .expect("rename input");
        input.focus();
        harness.event(egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        harness.event(egui::Event::Text("Target renamed".into()));
        harness.step();
        harness.get_by_label("Save").click();
        harness.run();
        let dispatch = take_rename_dispatch(&dispatched);

        harness.get_by_value("HD: Target").click();
        harness.run();
        harness.get_by_label("HD: Other (0.0000 DASH)").click();
        harness.run();

        harness.state_mut().display_backend_task_result(
            &dispatch.context,
            BackendTaskSuccessResult::WalletAliasRenamed {
                seed_hash: target_hash,
                alias: "Target renamed".into(),
            },
        );

        assert_eq!(
            target.read().expect("target").alias.as_deref(),
            Some("Target renamed"),
            "the canonical target wallet must update independently of selection"
        );
    });
}

#[test]
#[cfg(feature = "testing")]
fn single_key_rename_success_updates_original_wallet_after_selection_change() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let (_target_imported, target) = app_context
            .import_single_key_wif(
                &random_wif(app_context.network()),
                Some("Target key".into()),
                Default::default(),
            )
            .expect("target key");
        let (_other_imported, other) = app_context
            .import_single_key_wif(
                &random_wif(app_context.network()),
                Some("Other key".into()),
                Default::default(),
            )
            .expect("other key");
        let (target_hash, target_address) = {
            let target = target.read().expect("target");
            (target.key_hash, target.address.to_string())
        };
        app_context.set_selected_single_key_wallet(Some(target_hash));

        let (mut harness, dispatched) = build_rename_harness(runtime, app_context);
        click_in_one_frame(&mut harness, "Rename");
        let input = harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .expect("rename input");
        input.focus();
        harness.event(egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        harness.event(egui::Event::Text("Target key renamed".into()));
        harness.step();
        harness.get_by_label("Save").click();
        harness.run();
        let dispatch = take_rename_dispatch(&dispatched);

        harness.get_by_value("SK: Target key").click();
        harness.run();
        harness.get_by_label("SK: Other key (0.0000 DASH)").click();
        harness.run();

        harness.state_mut().display_backend_task_result(
            &dispatch.context,
            BackendTaskSuccessResult::SingleKeyAliasRenamed {
                address: target_address,
                alias: "Target key renamed".into(),
            },
        );

        assert_eq!(
            target.read().expect("target").alias.as_deref(),
            Some("Target key renamed"),
            "the canonical target key must update independently of selection"
        );
        assert_eq!(
            other.read().expect("other").alias.as_deref(),
            Some("Other key")
        );
    });
}

#[test]
#[cfg(feature = "testing")]
fn failed_hd_rename_keeps_prefilled_dialog_open_and_shows_banner() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let mut seed: [u8; 64] = rand::random();
        let wallet =
            Wallet::new_from_seed(seed, app_context.network(), None, None).expect("wallet");
        seed.zeroize();
        let seed_hash = wallet.seed_hash();
        app_context
            .wallets()
            .write()
            .expect("wallet map")
            .insert(seed_hash, Arc::new(RwLock::new(wallet)));
        app_context.set_selected_hd_wallet(Some(seed_hash));

        let (mut harness, dispatched) = build_rename_harness(runtime, app_context);
        enter_rename_alias(&mut harness, "Retry this alias");
        harness.get_by_label("Save").click();
        harness.run();
        let dispatch = take_rename_dispatch(&dispatched);

        let error = TaskError::WalletNotFound;
        harness
            .state_mut()
            .display_backend_task_error(&dispatch.context, &error);
        let message = error.to_string();
        MessageBanner::set_global(&harness.ctx, &message, MessageType::Error)
            .disable_auto_dismiss();
        harness
            .state_mut()
            .display_message(&message, MessageType::Error);
        harness.run();

        assert!(
            harness.query_by_label(&message).is_some(),
            "the actionable typed error must appear in the global banner"
        );
        assert!(
            harness
                .query_all_by_value("Retry this alias")
                .next()
                .is_some(),
            "the attempted alias must remain available for retry"
        );
        assert!(
            !harness.get_by_label("Save").accesskit_node().is_disabled(),
            "Save must be re-enabled after a matching failure"
        );
        assert!(
            !harness
                .get_by_label("Cancel")
                .accesskit_node()
                .is_disabled()
                && !harness
                    .query_all_by_role(egui::accesskit::Role::TextInput)
                    .next()
                    .expect("rename input")
                    .accesskit_node()
                    .is_disabled(),
            "Cancel and alias editing must be re-enabled after failure"
        );
    });
}

#[test]
#[cfg(feature = "testing")]
fn fund_platform_dialog_last_popup_row_stays_open_until_fund_is_clicked() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let mut seed: [u8; 64] = rand::random();
        let wallet = Wallet::new_from_seed(
            seed,
            app_context.network(),
            Some("Dialog wallet".to_string()),
            None,
        )
        .expect("create wallet fixture");
        seed.zeroize();
        let seed_hash = wallet.seed_hash();
        app_context
            .wallets()
            .write()
            .expect("wallet map")
            .insert(seed_hash, Arc::new(RwLock::new(wallet)));

        let addresses = platform_addresses(5, app_context.network());
        let last_address = addresses.last().expect("five addresses").0.clone();
        let selected_text = format!("{}... (0.0000 DASH)", &last_address[..12]);
        let mut screen = WalletsBalancesScreen::new(&app_context);
        screen.display_task_result(BackendTaskSuccessResult::TrackedAssetLocks {
            seed_hash,
            locks: Vec::new(),
        });
        screen.open_fund_platform_dialog_for_test(addresses);

        let funding_tasks = Rc::new(Cell::new(0));
        let task_counter = funding_tasks.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_ui_state(
                move |ui, screen: &mut WalletsBalancesScreen| {
                    let _runtime = &runtime;
                    if matches!(
                        screen.ui(ui),
                        AppAction::BackendTask(BackendTask::WalletTask(
                            WalletTask::FundPlatformAddressFromAssetLock { .. }
                        ))
                    ) {
                        task_counter.set(task_counter.get() + 1);
                    }
                },
                screen,
            );

        harness.run();
        assert_eq!(funding_tasks.get(), 0);

        harness.get_by_value("Select an address").click();
        harness.run();
        press_label(&mut harness, &selected_text);

        assert!(
            harness
                .query_by_label("Fund Platform Address from Asset Lock")
                .is_some(),
            "selecting the last popup row must leave the dialog open"
        );
        assert_eq!(
            funding_tasks.get(),
            0,
            "pressing an address row must not dispatch the funding task"
        );

        release_pointer_away(&mut harness);
        if harness.query_by_label(&selected_text).is_none() {
            harness.get_by_value("Select an address").click();
            harness.run();
        }
        harness.get_by_label(&selected_text).click();
        harness.run();

        assert!(
            harness.query_by_value(&selected_text).is_some(),
            "the fifth Platform address must become the selected value"
        );
        assert_eq!(
            funding_tasks.get(),
            0,
            "selecting an address must not dispatch the funding task"
        );

        harness.get_by_label("Fund Address").click();
        harness.run();
        assert_eq!(
            funding_tasks.get(),
            1,
            "the funding task must dispatch only after the explicit button click"
        );
    });
}

#[test]
fn create_asset_lock_button_is_below_empty_state() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let mut seed: [u8; 64] = rand::random();
        let wallet = Wallet::new_from_seed(
            seed,
            app_context.network(),
            Some("Dialog wallet".to_string()),
            None,
        )
        .expect("create wallet fixture");
        seed.zeroize();
        let seed_hash = wallet.seed_hash();
        app_context
            .wallets()
            .write()
            .expect("wallet map")
            .insert(seed_hash, Arc::new(RwLock::new(wallet)));

        let mut screen = WalletsBalancesScreen::new(&app_context);
        screen.display_task_result(BackendTaskSuccessResult::TrackedAssetLocks {
            seed_hash,
            locks: Vec::new(),
        });
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 1600.0))
            .build_ui_state(
                move |ui, screen: &mut WalletsBalancesScreen| {
                    let _runtime = &runtime;
                    screen.ui(ui);
                },
                screen,
            );

        harness.run();
        let empty_state = harness.get_by_label("No asset locks found").rect();
        let create_button = harness.get_by_label("Create Asset Lock").rect();
        assert!(
            create_button.top() > empty_state.bottom(),
            "the Create Asset Lock action must render below the asset-lock empty state"
        );
    });
}

#[test]
fn receive_dialog_stays_open_on_triggering_click() {
    with_isolated_data_dir(|| {
        let mut harness = wallet_screen_harness(None);

        click_in_one_frame(&mut harness, "Receive");
        assert!(
            harness.query_by_label("Core Address").is_some(),
            "the Receive dialog must survive the click that opened it"
        );

        harness.step();
        assert!(harness.query_by_label("Core Address").is_some());
    });
}

#[test]
fn add_receiving_address_button_opens_receive_dialog() {
    with_isolated_data_dir(|| {
        let mut harness = registered_wallet_screen_harness();

        click_in_one_frame(&mut harness, "➕ Add Receiving Address");
        assert!(
            harness.query_by_label("Core Address").is_some(),
            "clicking 'Add Receiving Address' must open the same Receive dialog as the \
             'Receive' button, not silently generate an address with no visible feedback"
        );
        assert!(
            harness
                .query_by_label("Generating a new address…")
                .is_some()
        );

        harness.step();
        assert!(harness.query_by_label("Core Address").is_some());
    });
}

#[test]
fn rename_dialog_stays_open_on_triggering_click() {
    with_isolated_data_dir(|| {
        let mut harness = wallet_screen_harness(None);

        click_in_one_frame(&mut harness, "Rename");
        assert!(
            harness.query_by_label("Enter new wallet name:").is_some(),
            "the Rename dialog must survive the click that opened it"
        );

        harness.step();
        assert!(harness.query_by_label("Enter new wallet name:").is_some());
    });
}

#[test]
fn unlock_dialog_stays_open_on_triggering_click() {
    with_isolated_data_dir(|| {
        let password = Secret::new("correct horse battery staple");
        let mut harness = wallet_screen_harness(Some(&password));

        click_in_one_frame(&mut harness, "Unlock");
        assert!(
            harness
                .query_by_label("Enter password to unlock \"Dialog wallet\":")
                .is_some(),
            "the password prompt must survive the Unlock click that opened it"
        );

        harness.step();
        assert!(
            harness
                .query_by_label("Enter password to unlock \"Dialog wallet\":")
                .is_some()
        );
    });
}

/// Test that the wallets screen can be rendered
#[test]
fn test_wallets_screen_renders() {
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

/// Test that the app can run many frames without issues
#[test]
fn test_app_stability_over_many_frames() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(200).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(1024.0, 768.0));

        // Run 50 frames to test stability
        harness.run_steps(50);
    });
}

/// Test rapid frame stepping
#[test]
fn test_rapid_frame_stepping() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(800.0, 600.0));

        // Run single steps rapidly
        for _ in 0..20 {
            harness.run_steps(1);
        }
    });
}
