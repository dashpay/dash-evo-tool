//! Kittest coverage for `AddressInput` — the GitHub-style tag search, the
//! dynamic placeholder legend, and pill-based row rendering.
//!
//! Autocomplete rows are painted directly (no child widgets, so a full-width
//! click rect can own the row), which means their pills and text are not exposed
//! to accesskit as queryable widgets. Data assertions therefore use the
//! component's public introspection API (`effective_hint_text`, `rendered_rows`),
//! whose fields map one-to-one to render decisions, while a typing render pass
//! actually paints the popup — exercising the real pill layout/paint path and
//! guarding against panics there.

use dash_evo_tool::model::address::AddressKind;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::ui::components::Component;
use dash_evo_tool::ui::components::address_input::{AddressInput, WalletWithSnapshot};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, RwLock};

/// Build an in-memory wallet paired with its display snapshot: empty balances
/// and exactly one BIP-44 receive address, distinct per `seed`. The snapshot
/// paths — not the wallet's own bookkeeping — are what the component offers.
fn wallet(seed: u8, alias: &str) -> WalletWithSnapshot {
    use dash_sdk::dpp::dashcore::{Address, PublicKey};
    use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};

    let wallet = Wallet::new_from_seed([seed; 64], Network::Testnet, Some(alias.to_string()), None)
        .expect("wallet from seed");

    let mut sk_bytes = [1u8; 32];
    sk_bytes[31] = seed.max(1);
    let sk =
        dash_sdk::dpp::secp256k1::SecretKey::from_secret_bytes(sk_bytes).expect("valid secret key");
    let address = Address::p2pkh(&PublicKey::new(sk.public_key()), Network::Testnet);

    // m/44'/1'/0'/0/0 — a BIP-44 external receive path.
    let path = DerivationPath::from(
        [
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 0 },
        ]
        .as_slice(),
    );

    (
        Arc::new(RwLock::new(wallet)),
        BTreeMap::new(),
        BTreeMap::from([(address, path)]),
        Some(alias.to_owned()),
    )
}

/// Build a wallet-less `QualifiedIdentity` whose display name is its alias.
fn identity(alias: &str) -> QualifiedIdentity {
    let identity =
        Identity::create_basic_identity(Identifier::from([0x11; 32]), PlatformVersion::latest())
            .expect("basic identity");
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

fn identity_input(name: &str) -> AddressInput {
    crate::support::with_isolated_data_dir(|| {
        let (_runtime, context) = crate::support::fresh_app_context();
        AddressInput::new(Network::Testnet).with_identities(&context, &[identity(name)])
    })
}

/// Render `input` in a headless harness, type into its field to open (and
/// paint) the autocomplete popup, then run `assert` against the component's
/// public state. The typed space leaves the query unchanged.
fn with_rendered(input: AddressInput, assert: impl FnOnce(&AddressInput)) {
    let cell = Rc::new(RefCell::new(input));
    let cell_ui = Rc::clone(&cell);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(480.0, 400.0))
        .build_ui(move |ui| {
            let _ = cell_ui.borrow_mut().show(ui);
        });

    harness.run();
    // Typing opens the popup; it paints its rows on the next frame.
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
    harness.run();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text(" ");
    harness.run();
    harness.run();

    assert(&cell.borrow());
}

#[test]
fn hint_lists_enabled_kinds_when_no_wallets() {
    with_rendered(AddressInput::new(Network::Testnet), |input| {
        assert_eq!(
            input.effective_hint_text(),
            "type:core|platform|shielded|identity"
        );
        assert!(!input.effective_hint_text().contains("wallet:"));
    });
}

#[test]
fn hint_reflects_restricted_kinds() {
    let input = AddressInput::new(Network::Testnet)
        .with_address_kinds(&[AddressKind::Core, AddressKind::Shielded]);
    with_rendered(input, |input| {
        assert_eq!(input.effective_hint_text(), "type:core|shielded");
    });
}

#[test]
fn hint_lists_single_and_few_wallets() {
    with_rendered(
        AddressInput::new(Network::Testnet).with_wallets(&[wallet(1, "solo")]),
        |input| assert!(input.effective_hint_text().contains("wallet:solo")),
    );

    let three = AddressInput::new(Network::Testnet).with_wallets(&[
        wallet(1, "a"),
        wallet(2, "b"),
        wallet(3, "c"),
    ]);
    with_rendered(three, |input| {
        let hint = input.effective_hint_text();
        assert!(hint.contains("wallet:a|b|c"), "{hint}");
        assert!(!hint.contains("more"), "no trim below six wallets: {hint}");
    });
}

#[test]
fn hint_trims_wallets_above_five() {
    let six: Vec<WalletWithSnapshot> = (0u8..6).map(|i| wallet(i + 1, &format!("w{i}"))).collect();
    with_rendered(
        AddressInput::new(Network::Testnet).with_wallets(&six),
        |input| {
            let hint = input.effective_hint_text();
            assert!(hint.contains("wallet:w0|w1|w2|w3|w4"), "{hint}");
            assert!(hint.contains("(+1 more)"), "{hint}");
        },
    );

    let five: Vec<WalletWithSnapshot> = (0u8..5).map(|i| wallet(i + 1, &format!("w{i}"))).collect();
    with_rendered(
        AddressInput::new(Network::Testnet).with_wallets(&five),
        |input| {
            let hint = input.effective_hint_text();
            assert!(hint.contains("wallet:w0|w1|w2|w3|w4"), "{hint}");
            assert!(
                !hint.contains("more"),
                "no trim at exactly five wallets: {hint}"
            );
        },
    );
}

#[test]
fn wallet_pill_only_on_wallet_scoped_rows() {
    let input = identity_input("alice")
        .with_wallets(&[wallet(1, "main")])
        .with_shielded_balance("tdash1zexampleshieldedaddress".to_string(), 0);

    with_rendered(input, |input| {
        let rows = input.rendered_rows();
        assert!(!rows.is_empty());
        for row in &rows {
            // The type pill (driven by `kind`) is unconditional; assert the
            // wallet pill appears only on wallet-scoped kinds.
            match row.kind {
                AddressKind::Core | AddressKind::Platform => assert!(
                    row.wallet_pill.is_some(),
                    "wallet-scoped row must carry a wallet pill: {row:?}"
                ),
                AddressKind::Identity | AddressKind::Shielded => assert!(
                    row.wallet_pill.is_none(),
                    "wallet-agnostic row must not carry a wallet pill: {row:?}"
                ),
            }
        }

        let identity_row = rows
            .iter()
            .find(|r| r.kind == AddressKind::Identity)
            .expect("identity row present");
        assert_eq!(
            identity_row.name.as_deref(),
            Some("alice"),
            "identity row shows its name"
        );
        let shielded_row = rows
            .iter()
            .find(|r| r.kind == AddressKind::Shielded)
            .expect("shielded row present");
        assert_eq!(shielded_row.name, None, "shielded row has no name");
    });
}

#[test]
fn tag_query_narrows_rows() {
    let input = identity_input("carol")
        .with_wallets(&[wallet(1, "alpha"), wallet(2, "beta")])
        .with_initial_value("type:core wallet:alpha");

    with_rendered(input, |input| {
        let rows = input.rendered_rows();
        assert_eq!(
            rows.len(),
            1,
            "only alpha's core address should match: {rows:?}"
        );
        assert_eq!(rows[0].kind, AddressKind::Core);
        assert_eq!(rows[0].wallet_pill.as_deref(), Some("alpha"));
    });
}

/// Whether the suggestion popup shows any row (rows are the only buttons).
fn popup_rows(harness: &Harness<'_>) -> usize {
    harness
        .query_all_by_role(egui::accesskit::Role::Button)
        .filter(|node| node.accesskit_node().label().as_deref() != Some("Elsewhere"))
        .count()
}

#[test]
fn suggestions_open_while_typing_and_close_on_select_or_click_elsewhere() {
    let cell = Rc::new(RefCell::new(
        AddressInput::new(Network::Testnet).with_wallets(&[wallet(1, "main")]),
    ));
    let cell_ui = Rc::clone(&cell);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(480.0, 400.0))
        .build_ui(move |ui| {
            let _ = ui.button("Elsewhere");
            let _ = cell_ui.borrow_mut().show(ui);
        });
    harness.run();

    let field = || egui::accesskit::Role::TextInput;
    harness.get_by_role(field()).focus();
    harness.run();
    assert_eq!(popup_rows(&harness), 0, "focus alone opens nothing");

    harness.get_by_role(field()).type_text("y");
    harness.run();
    assert!(popup_rows(&harness) > 0, "typing offers suggestions");

    harness.get_by_label("Elsewhere").click();
    harness.run();
    harness.run();
    assert_eq!(popup_rows(&harness), 0, "a click elsewhere closes them");

    harness.get_by_role(field()).focus();
    harness.run();
    assert_eq!(
        popup_rows(&harness),
        0,
        "focusing again does not reopen them"
    );

    harness.key_press(egui::Key::Backspace);
    harness.run();
    assert!(popup_rows(&harness) > 0, "editing reopens them");
    let row = harness
        .query_all_by_role(egui::accesskit::Role::Button)
        .find(|node| node.accesskit_node().label().as_deref() != Some("Elsewhere"))
        .expect("a suggestion row");
    row.click();
    harness.run();
    harness.run();
    assert_eq!(popup_rows(&harness), 0, "choosing a suggestion closes them");
}

/// A blocking dialog over the form closes the address suggestions: the popup
/// must not draw above the dialog and its dimmed backdrop.
#[test]
fn suggestions_do_not_draw_over_a_blocking_dialog() {
    use dash_evo_tool::ui::components::confirmation_dialog::ConfirmationDialog;

    let input = identity_input("dora").with_wallets(&[wallet(1, "main")]);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(480.0, 400.0))
        .build_ui_state(
            |ui, (input, dialog): &mut (AddressInput, Option<ConfirmationDialog>)| {
                let _ = input.show(ui);
                if let Some(dialog) = dialog {
                    let _ = dialog.show(ui);
                }
            },
            (input, None),
        );
    harness.run();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
    harness.run();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text(" ");
    harness.run();
    harness.run();
    assert!(
        harness.query_by_label_contains("(dora)").is_some(),
        "precondition: the suggestions are open"
    );

    harness.state_mut().1 =
        Some(ConfirmationDialog::new("Confirm this send.", "Send 1 DASH?").blocks_input(true));
    harness.run_steps(4);
    assert!(
        harness.query_by_label("Confirm this send.").is_some(),
        "precondition: the dialog is open"
    );
    assert!(
        harness.query_by_label_contains("(dora)").is_none(),
        "the suggestions must not draw over a blocking dialog"
    );
}
