use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::model::secret::Secret;
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::wallets::wallets_screen::WalletsBalancesScreen;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::sync::{Arc, RwLock};

fn wallet_screen_harness(password: Option<&Secret>) -> Harness<'static, WalletsBalancesScreen> {
    let (runtime, app_context) = fresh_app_context();
    let mut wallet = Wallet::new_from_seed(
        [0x42; 64],
        app_context.network(),
        Some("Dialog wallet".to_string()),
        password,
    )
    .expect("create wallet fixture");
    if password.is_some() {
        wallet.wallet_seed.close();
    }
    let seed_hash = wallet.seed_hash();
    app_context
        .wallets()
        .write()
        .expect("wallet map")
        .insert(seed_hash, Arc::new(RwLock::new(wallet)));

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
