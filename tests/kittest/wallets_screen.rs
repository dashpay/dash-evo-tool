use crate::support::{fresh_app_context, with_isolated_data_dir};
#[cfg(feature = "testing")]
use dash_evo_tool::app::AppAction;
#[cfg(feature = "testing")]
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::backend_task::BackendTaskSuccessResult;
#[cfg(feature = "testing")]
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::model::secret::Secret;
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::model::wallet::birth_height::WalletOrigin;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::wallets::wallets_screen::WalletsBalancesScreen;
#[cfg(feature = "testing")]
use dash_sdk::dashcore_rpc::dashcore::Network;
#[cfg(feature = "testing")]
use dash_sdk::dpp::address_funds::PlatformAddress;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
#[cfg(feature = "testing")]
use std::cell::Cell;
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
