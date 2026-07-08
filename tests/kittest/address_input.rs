//! Kittest coverage for `AddressInput` — the GitHub-style tag search, the
//! dynamic placeholder legend, and pill-based row rendering.
//!
//! Autocomplete rows are painted directly (no child widgets, so a full-width
//! click rect can own the row), which means their pills and text are not exposed
//! to accesskit as queryable widgets. Data assertions therefore use the
//! component's public introspection API (`effective_hint_text`, `rendered_rows`),
//! whose fields map one-to-one to render decisions, while a focused render pass
//! actually paints the popup — exercising the real pill layout/paint path and
//! guarding against panics there.

use dash_evo_tool::model::address::AddressKind;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::ui::components::Component;
use dash_evo_tool::ui::components::address_input::{AddressInput, WalletWithBalances};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, RwLock};

/// Build an in-memory wallet with a single derived receive address, paired with
/// empty display balances.
fn wallet(seed: u8, alias: &str) -> WalletWithBalances {
    let wallet = Wallet::new_from_seed([seed; 64], Network::Testnet, Some(alias.to_string()), None)
        .expect("wallet from seed");
    (Arc::new(RwLock::new(wallet)), BTreeMap::new())
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
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::PendingCreation,
        network: Network::Testnet,
    }
}

/// Render `input` in a headless harness, focus its field to open (and paint) the
/// autocomplete popup, then run `assert` against the component's public state.
fn with_rendered(input: AddressInput, assert: impl FnOnce(&AddressInput)) {
    let cell = Rc::new(RefCell::new(input));
    let cell_ui = Rc::clone(&cell);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(480.0, 400.0))
        .build_ui(move |ui| {
            let _ = cell_ui.borrow_mut().show(ui);
        });

    harness.run();
    // Focus the text field so the popup opens next frame and paints its rows.
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
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
    let six: Vec<WalletWithBalances> = (0u8..6).map(|i| wallet(i + 1, &format!("w{i}"))).collect();
    with_rendered(
        AddressInput::new(Network::Testnet).with_wallets(&six),
        |input| {
            let hint = input.effective_hint_text();
            assert!(hint.contains("wallet:w0|w1|w2|w3|w4"), "{hint}");
            assert!(hint.contains("(+1 more)"), "{hint}");
        },
    );

    let five: Vec<WalletWithBalances> = (0u8..5).map(|i| wallet(i + 1, &format!("w{i}"))).collect();
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
    let input = AddressInput::new(Network::Testnet)
        .with_wallets(&[wallet(1, "main")])
        .with_identities(&[identity("alice")])
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
    let input = AddressInput::new(Network::Testnet)
        .with_wallets(&[wallet(1, "alpha"), wallet(2, "beta")])
        .with_identities(&[identity("carol")])
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
