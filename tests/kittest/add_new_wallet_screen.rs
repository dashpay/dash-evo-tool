//! Kittest coverage for the wallet-name step of the Create Wallet screen.
//!
//! The screen hands the raw name to `AppContext::register_wallet`, which cleans
//! it, fills in the smallest unused "Wallet N" when it is blank, and rejects a
//! name another wallet already uses. These tests drive the real screen and the
//! real registration path.

use crate::support::{fresh_app_context, with_isolated_data_dir};
use bip39::Mnemonic;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::wallets::add_new_wallet_screen::AddNewWalletScreen;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::sync::{Arc, RwLock};
use zeroize::Zeroize;

fn create_wallet_harness(
    runtime: tokio::runtime::Runtime,
    app_context: &Arc<AppContext>,
) -> Harness<'static, AddNewWalletScreen> {
    let mut screen = AddNewWalletScreen::new(app_context);
    // Fixed test entropy — never a literal recovery phrase.
    screen.set_seed_phrase_for_test(Mnemonic::from_entropy(&[0u8; 16]).expect("valid entropy"));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 2400.0))
        .build_ui_state(
            move |ui, screen: &mut AddNewWalletScreen| {
                let _runtime_guard = runtime.enter();
                screen.ui(ui);
            },
            screen,
        );
    harness.run();
    harness
}

/// Put a wallet named `alias` straight into the in-memory map.
fn insert_wallet(app_context: &AppContext, alias: &str) {
    let mut seed: [u8; 64] = rand::random();
    let wallet = Wallet::new_from_seed(seed, app_context.network(), Some(alias.to_owned()), None)
        .expect("wallet fixture");
    seed.zeroize();
    app_context
        .wallets()
        .write()
        .expect("wallet map")
        .insert(wallet.seed_hash(), Arc::new(RwLock::new(wallet)));
}

fn sorted_aliases(app_context: &AppContext) -> Vec<String> {
    let mut aliases: Vec<String> = app_context
        .wallets()
        .read()
        .expect("wallet map")
        .values()
        .filter_map(|wallet| wallet.read().expect("wallet").alias.clone())
        .collect();
    aliases.sort();
    aliases
}

fn type_wallet_name(harness: &mut Harness<'_, AddNewWalletScreen>, text: &str) {
    harness.get_by_label("Wallet name").focus();
    harness.event(egui::Event::Text(text.to_owned()));
    harness.step();
}

#[test]
fn typed_wallet_name_is_cleaned_and_saved() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let mut harness = create_wallet_harness(runtime, &app_context);

        type_wallet_name(&mut harness, "  Savings\u{200B}  ");
        assert!(
            harness.query_by_label("7 of 64 characters used.").is_some(),
            "the counter must count the cleaned name"
        );
        harness.get_by_label("Save Wallet").click();
        harness.run();

        assert_eq!(sorted_aliases(&app_context), vec!["Savings".to_owned()]);
    });
}

/// A blank name becomes the smallest unused "Wallet N": with "Wallet 1" and
/// "Wallet 3" present, the old `count + 1` rule would have produced a second
/// "Wallet 3".
#[test]
fn blank_wallet_name_takes_smallest_unused_default_name() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        insert_wallet(&app_context, "Wallet 1");
        insert_wallet(&app_context, "Wallet 3");
        let mut harness = create_wallet_harness(runtime, &app_context);

        harness.get_by_label("Save Wallet").click();
        harness.run();

        assert_eq!(
            sorted_aliases(&app_context),
            vec![
                "Wallet 1".to_owned(),
                "Wallet 2".to_owned(),
                "Wallet 3".to_owned()
            ]
        );
    });
}

#[test]
fn duplicate_wallet_name_is_rejected_and_not_saved() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        insert_wallet(&app_context, "Savings");
        let mut harness = create_wallet_harness(runtime, &app_context);

        type_wallet_name(&mut harness, "Savings");
        harness.get_by_label("Save Wallet").click();
        harness.run();

        assert_eq!(
            sorted_aliases(&app_context),
            vec!["Savings".to_owned()],
            "a second wallet must not be saved under a name already in use"
        );
        assert!(
            harness.query_by_label("Close").is_some(),
            "the rejection must be shown to the user"
        );
    });
}
