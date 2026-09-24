//! Kittest coverage for the name step of the Import Wallet screen, for both
//! recovery-phrase (HD wallet) and private-key imports.
//!
//! The screen passes the raw name to the backend, which cleans it, fills in
//! the smallest unused "Wallet N" / "Key N" when it is blank, and rejects a
//! name another wallet of the same kind already uses.

use crate::support::{fresh_app_context, with_isolated_data_dir};
use bip39::Mnemonic;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::model::wallet::alias::AliasSource;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::wallets::import_mnemonic_screen::ImportMnemonicScreen;
use dash_sdk::dpp::dashcore::PrivateKey;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::sync::{Arc, RwLock};
use zeroize::Zeroize;

fn import_harness(
    runtime: tokio::runtime::Runtime,
    app_context: &Arc<AppContext>,
    setup: impl FnOnce(&mut ImportMnemonicScreen),
) -> Harness<'static, ImportMnemonicScreen> {
    let mut screen = ImportMnemonicScreen::new(app_context);
    setup(&mut screen);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 2400.0))
        .build_ui_state(
            move |ui, screen: &mut ImportMnemonicScreen| {
                let _runtime_guard = runtime.enter();
                screen.ui(ui);
            },
            screen,
        );
    harness.run();
    harness
}

fn with_test_phrase(screen: &mut ImportMnemonicScreen) {
    // Fixed test entropy — never a literal recovery phrase.
    screen.set_seed_phrase_for_test(&Mnemonic::from_entropy(&[0u8; 16]).expect("valid entropy"));
}

fn test_wif(app_context: &AppContext, byte: u8) -> String {
    PrivateKey::from_byte_array(&[byte; 32], app_context.network())
        .expect("valid private key")
        .to_wif()
}

fn insert_wallet(app_context: &AppContext, alias: &str) {
    let mut seed: [u8; 64] = rand::random();
    let wallet = Wallet::new_from_seed(seed, app_context.network(), Some(alias.to_owned()), None)
        .expect("wallet fixture");
    seed.zeroize();
    app_context
        .wallet_context()
        .insert_test_wallet(wallet.seed_hash(), Arc::new(RwLock::new(wallet)));
}

fn sorted_wallet_aliases(app_context: &AppContext) -> Vec<String> {
    let mut aliases: Vec<String> = app_context
        .wallet_context()
        .wallets()
        .values()
        .filter_map(|wallet| {
            app_context
                .wallet_context()
                .hd_alias(&wallet.read().expect("wallet").seed_hash())
        })
        .collect();
    aliases.sort();
    aliases
}

fn sorted_key_aliases(app_context: &AppContext) -> Vec<Option<String>> {
    let mut aliases: Vec<Option<String>> = app_context
        .wallet_backend()
        .expect("wallet backend")
        .single_key()
        .list()
        .into_iter()
        .map(|key| key.alias)
        .collect();
    aliases.sort();
    aliases
}

fn type_name(harness: &mut Harness<'_, ImportMnemonicScreen>, text: &str) {
    harness.get_by_label("Name").focus();
    harness.event(egui::Event::Text(text.to_owned()));
    harness.step();
}

#[test]
fn imported_wallet_name_is_cleaned_and_saved() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let mut harness = import_harness(runtime, &app_context, with_test_phrase);

        type_name(&mut harness, "\u{202E}Cold storage ");
        assert!(
            harness
                .query_by_label("12 of 64 characters used.")
                .is_some(),
            "the counter must count the cleaned name"
        );
        harness.get_by_label("Save Wallet").click();
        harness.run();

        assert_eq!(
            sorted_wallet_aliases(&app_context),
            vec!["Cold storage".to_owned()]
        );
    });
}

#[test]
fn blank_imported_wallet_name_takes_smallest_unused_default_name() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        insert_wallet(&app_context, "Wallet 1");
        let mut harness = import_harness(runtime, &app_context, with_test_phrase);

        harness.get_by_label("Save Wallet").click();
        harness.run();

        assert_eq!(
            sorted_wallet_aliases(&app_context),
            vec!["Wallet 1".to_owned(), "Wallet 2".to_owned()]
        );
    });
}

#[test]
fn blank_imported_key_name_takes_smallest_unused_default_name() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        {
            let _runtime_guard = runtime.enter();
            app_context
                .import_single_key_wif(
                    &test_wif(&app_context, 0x31),
                    AliasSource::UserEntered("Key 1".to_owned()),
                    Default::default(),
                )
                .expect("existing key");
        }
        let wif = test_wif(&app_context, 0x32);
        let mut harness = import_harness(runtime, &app_context, |screen| {
            screen.set_private_key_for_test(&wif);
        });

        harness.get_by_label("Import Key").click();
        harness.run();

        assert_eq!(
            sorted_key_aliases(&app_context),
            vec![Some("Key 1".to_owned()), Some("Key 2".to_owned())]
        );
    });
}

#[test]
fn duplicate_imported_key_name_is_rejected_and_not_saved() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        {
            let _runtime_guard = runtime.enter();
            app_context
                .import_single_key_wif(
                    &test_wif(&app_context, 0x31),
                    AliasSource::UserEntered("Savings".to_owned()),
                    Default::default(),
                )
                .expect("existing key");
        }
        let wif = test_wif(&app_context, 0x32);
        let mut harness = import_harness(runtime, &app_context, |screen| {
            screen.set_private_key_for_test(&wif);
        });

        type_name(&mut harness, "Savings");
        harness.get_by_label("Import Key").click();
        harness.run();

        assert_eq!(
            sorted_key_aliases(&app_context),
            vec![Some("Savings".to_owned())],
            "a second key must not be saved under a name already in use"
        );
        assert!(
            harness
                .query_by_label_contains("already uses this name")
                .is_some(),
            "the rejection must be shown, not silently swallowed"
        );
    });
}

/// Regression: pressing "Save Wallet" when the backend rejects the import
/// must tell the user why. The recovery-phrase branch only rendered errors
/// containing "Invalid seed phrase", so every `register_wallet` rejection was
/// stored in `self.error` and never shown — the click looked like a no-op.
#[test]
fn duplicate_imported_wallet_name_is_reported_to_the_user() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        insert_wallet(&app_context, "Savings");
        let mut harness = import_harness(runtime, &app_context, with_test_phrase);

        type_name(&mut harness, "Savings");
        harness.get_by_label("Save Wallet").click();
        harness.run();

        assert_eq!(
            sorted_wallet_aliases(&app_context),
            vec!["Savings".to_owned()],
            "a second wallet must not be saved under a name already in use"
        );
        assert!(
            harness
                .query_by_label_contains("Another wallet already uses this name")
                .is_some(),
            "the rejection must be shown, not silently swallowed"
        );
    });
}

/// Regression: re-importing a recovery phrase that is already registered
/// must say so instead of doing nothing.
#[test]
fn reimported_recovery_phrase_is_reported_to_the_user() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        {
            let _runtime_guard = runtime.enter();
            // Fixed test entropy — never a literal recovery phrase.
            let seed = Mnemonic::from_entropy(&[0u8; 16])
                .expect("valid entropy")
                .to_seed("");
            let wallet = Wallet::new_from_seed(
                seed,
                app_context.network(),
                Some("Existing".to_owned()),
                None,
            )
            .expect("wallet fixture");
            app_context
                .register_wallet(
                    wallet,
                    &seed,
                    dash_evo_tool::model::wallet::birth_height::WalletOrigin::Imported,
                )
                .expect("first import");
        }
        let mut harness = import_harness(runtime, &app_context, with_test_phrase);

        type_name(&mut harness, "Second copy");
        harness.get_by_label("Save Wallet").click();
        harness.run();

        assert_eq!(
            sorted_wallet_aliases(&app_context),
            vec!["Existing".to_owned()]
        );
        assert!(
            harness
                .query_by_label_contains("already been imported")
                .is_some(),
            "the duplicate import must be shown, not silently swallowed"
        );
        assert!(
            harness
                .query_by_label_contains("Open it from the Wallets screen")
                .is_some(),
            "the duplicate-import message must tell the user what to do next"
        );
    });
}

/// Regression: a password below the vault minimum is refused by the wallet
/// model, not by `register_wallet`; that rejection must reach the user too.
#[test]
fn too_short_import_password_is_reported_to_the_user() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let mut harness = import_harness(runtime, &app_context, |screen| {
            with_test_phrase(screen);
            screen.set_password_for_test("short");
        });

        harness.get_by_label("Save Wallet").click();
        harness.run();

        assert!(
            sorted_wallet_aliases(&app_context).is_empty(),
            "no wallet may be saved with a password below the minimum"
        );
        assert!(
            harness
                .query_by_label_contains("Wallet passwords must be at least")
                .is_some(),
            "the password rejection must be shown, not silently swallowed"
        );
    });
}

/// A private-key import with a password fails this screen's own pre-checks;
/// the typed rejection must be shown and nothing imported.
#[test]
fn private_key_import_with_password_is_reported_to_the_user() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        let wif = test_wif(&app_context, 0x33);
        let mut harness = import_harness(runtime, &app_context, |screen| {
            screen.set_private_key_for_test(&wif);
            screen.set_password_for_test("long enough password");
        });

        harness.get_by_label("Import Key").click();
        harness.run();

        assert!(sorted_key_aliases(&app_context).is_empty());
        assert!(
            harness
                .query_by_label_contains("Per-key passwords are not supported")
                .is_some(),
            "the password rejection must be shown, not silently swallowed"
        );
    });
}

#[test]
fn invalid_recovery_phrase_message_follows_the_words() {
    with_isolated_data_dir(|| {
        let (runtime, app_context) = fresh_app_context();
        // Twelve copies of the first BIP39 word fail the checksum.
        let mut harness = import_harness(runtime, &app_context, |screen| {
            screen.set_seed_words_for_test(&["abandon"; 12]);
        });

        assert!(
            harness
                .query_by_label_contains("Invalid seed phrase")
                .is_some(),
            "a complete but invalid phrase must be flagged"
        );
        assert!(harness.query_by_label("Save Wallet").is_none());

        with_test_phrase(harness.state_mut());
        harness.run();

        assert!(
            harness
                .query_by_label_contains("Invalid seed phrase")
                .is_none(),
            "the message must clear once the phrase is valid"
        );
        assert!(harness.query_by_label("Save Wallet").is_some());
    });
}
