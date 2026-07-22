use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::backend_task::shielded::ShieldedTask;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::model::address::{ValidatedAddress, encode_shielded_address};
use dash_evo_tool::model::amount::Amount;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::ui::wallets::send_screen::{SourceSelection, WalletSendScreen};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::secp256k1::{Secp256k1, SecretKey};
use dash_sdk::dpp::dashcore::{Address, Network, PublicKey};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

const ONE_DASH_CREDITS: u64 = CREDITS_PER_DUFF * 100_000_000;

fn core_address() -> Address {
    let secret_key = loop {
        let bytes: [u8; 32] = rand::random();
        if let Ok(key) = SecretKey::from_slice(&bytes) {
            break key;
        }
    };
    let public_key = PublicKey::new(secret_key.public_key(&Secp256k1::new()));
    Address::p2pkh(&public_key, Network::Testnet)
}

fn platform_address() -> PlatformAddress {
    PlatformAddress::try_from(core_address()).expect("valid Platform address")
}

fn shielded_address() -> String {
    let seed: [u8; 64] = rand::random();
    let keys =
        platform_wallet::wallet::shielded::OrchardKeySet::from_seed(&seed, Network::Testnet, 0)
            .expect("Orchard keys");
    encode_shielded_address(&keys.address_at(0).to_raw_address_bytes(), Network::Testnet)
        .expect("shielded address")
}

fn send_screen() -> (tokio::runtime::Runtime, WalletSendScreen) {
    let (runtime, app_context) = fresh_app_context();
    let wallet = Wallet::new_from_seed(
        rand::random(),
        Network::Testnet,
        Some("Send routing fixture".to_string()),
        None,
    )
    .expect("wallet fixture");
    (
        runtime,
        WalletSendScreen::new(&app_context, Arc::new(RwLock::new(wallet))),
    )
}

/// Wallet-less `SourceSelection::Identity` fixture. Only routing needs it — the
/// unsupported-route arms match `SourceSelection::Identity(_)` and never read the
/// identity's contents, so no keys, wallet association, or DB row are required.
/// Mirrors the `make_qi` pattern in `identity_selector.rs`.
fn identity_source(byte: u8) -> SourceSelection {
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), PlatformVersion::latest())
            .expect("basic identity");
    SourceSelection::Identity(Box::new(QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some("Send routing identity".to_string()),
        private_keys: KeyStorage::default(),
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::PendingCreation,
        network: Network::Testnet,
    }))
}

fn platform_source() -> Vec<(PlatformAddress, Address, u64)> {
    let core = core_address();
    vec![(
        PlatformAddress::try_from(core.clone()).expect("platform source"),
        core,
        3 * ONE_DASH_CREDITS,
    )]
}

fn assert_route(
    source: SourceSelection,
    destination: ValidatedAddress,
    button_label: &str,
) -> Result<AppAction, String> {
    with_isolated_data_dir(|| {
        let (_runtime, mut screen) = send_screen();
        screen.set_simple_send_input_for_test(
            Some(source),
            Some(destination),
            Some(Amount::new_dash(1.0)),
        );

        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| {
                screen.render_simple_send_button_for_test(ui);
            },
            screen,
        );
        harness.run();

        assert!(
            harness.query_by_label(button_label).is_some(),
            "expected send button label {button_label:?}"
        );
        harness.state_mut().validate_and_send_for_test()
    })
}

fn assert_validation_error(
    source: Option<SourceSelection>,
    destination: Option<ValidatedAddress>,
    amount: Option<Amount>,
    expected: &str,
) {
    with_isolated_data_dir(|| {
        let (_runtime, mut screen) = send_screen();
        screen.set_simple_send_input_for_test(source, destination, amount);

        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| {
                screen.render_simple_send_button_for_test(ui);
            },
            screen,
        );
        harness.run();

        assert_eq!(
            harness
                .state_mut()
                .validate_and_send_for_test()
                .expect_err("invalid send input"),
            expected
        );
    });
}

#[test]
fn routes_core_wallet_to_core_address() {
    let error = assert_route(
        SourceSelection::CoreWallet,
        ValidatedAddress::Core(core_address()),
        "Send DASH",
    )
    .expect_err("offline Core wallet has no spendable snapshot balance");
    assert!(error.starts_with("Insufficient balance. Need "), "{error}");
    assert!(!error.contains("including fee"), "{error}");
}

#[test]
fn routes_core_wallet_to_platform_address() {
    let address = platform_address();
    let error = assert_route(
        SourceSelection::CoreWallet,
        ValidatedAddress::Platform {
            address,
            bech32m: address.to_bech32m_string(Network::Testnet),
        },
        "Fund Platform Address",
    )
    .expect_err("offline Core wallet has no spendable snapshot balance");
    assert!(error.starts_with("Insufficient balance. Need "), "{error}");
    assert!(error.contains("including fee"), "{error}");
}

#[test]
fn routes_platform_addresses_to_platform_address() {
    let address = platform_address();
    let source = platform_source();
    let action = assert_route(
        SourceSelection::PlatformAddresses(source),
        ValidatedAddress::Platform {
            address,
            bech32m: address.to_bech32m_string(Network::Testnet),
        },
        "Transfer Credits",
    )
    .expect("Platform transfer task");
    let AppAction::BackendTask(BackendTask::WalletTask(WalletTask::TransferPlatformCredits {
        outputs,
        ..
    })) = action
    else {
        panic!("expected Platform transfer task");
    };
    assert_eq!(outputs.get(&address), Some(&ONE_DASH_CREDITS));
}

#[test]
fn routes_platform_addresses_to_core_address() {
    let destination = core_address();
    let expected_script = CoreScript::new(destination.script_pubkey());
    let source = platform_source();
    let action = assert_route(
        SourceSelection::PlatformAddresses(source),
        ValidatedAddress::Core(destination),
        "Withdraw to Wallet",
    )
    .expect("Platform withdrawal task");
    let AppAction::BackendTask(BackendTask::WalletTask(WalletTask::WithdrawFromPlatformAddress {
        output_script,
        ..
    })) = action
    else {
        panic!("expected Platform withdrawal task");
    };
    assert_eq!(output_script, expected_script);
}

#[test]
fn routes_shielded_pool_to_shielded_address() {
    let seed_hash = rand::random();
    let destination = shielded_address();
    let recipient_address_bytes =
        dash_evo_tool::model::address::parse_shielded_recipient(&destination)
            .expect("valid shielded recipient");
    let action = assert_route(
        SourceSelection::Shielded(seed_hash, 3 * ONE_DASH_CREDITS),
        ValidatedAddress::Shielded(destination),
        "Private Send",
    )
    .expect("shielded transfer task");
    let AppAction::BackendTask(BackendTask::ShieldedTask(ShieldedTask::ShieldedTransfer {
        seed_hash: task_seed_hash,
        amount,
        recipient_address_bytes: task_recipient,
    })) = action
    else {
        panic!("expected shielded transfer task");
    };
    assert_eq!(task_seed_hash, seed_hash);
    assert_eq!(amount, ONE_DASH_CREDITS);
    assert_eq!(task_recipient, recipient_address_bytes);
}

#[test]
fn routes_shielded_pool_to_platform_address() {
    let address = platform_address();
    let seed_hash = rand::random();
    let action = assert_route(
        SourceSelection::Shielded(seed_hash, 3 * ONE_DASH_CREDITS),
        ValidatedAddress::Platform {
            address,
            bech32m: address.to_bech32m_string(Network::Testnet),
        },
        "Unshield Credits",
    )
    .expect("unshield task");
    let AppAction::BackendTask(BackendTask::ShieldedTask(ShieldedTask::UnshieldCredits {
        seed_hash: task_seed_hash,
        amount,
        to_platform_address,
    })) = action
    else {
        panic!("expected unshield task");
    };
    assert_eq!(task_seed_hash, seed_hash);
    assert_eq!(amount, ONE_DASH_CREDITS);
    assert_eq!(to_platform_address, address);
}

#[test]
fn rejects_missing_source_before_routing() {
    assert_validation_error(
        None,
        Some(ValidatedAddress::Core(core_address())),
        Some(Amount::new_dash(1.0)),
        "Please select a source",
    );
}

#[test]
fn rejects_missing_destination_before_routing() {
    assert_validation_error(
        Some(SourceSelection::CoreWallet),
        None,
        Some(Amount::new_dash(1.0)),
        "Invalid destination address. Use a Dash address (X.../y...) or Platform address (dash1.../tdash1...)",
    );
}

#[test]
fn rejects_missing_amount_before_routing() {
    assert_validation_error(
        Some(SourceSelection::CoreWallet),
        Some(ValidatedAddress::Core(core_address())),
        None,
        "Please enter an amount",
    );
}

#[test]
fn rejects_zero_amount_before_routing() {
    assert_validation_error(
        Some(SourceSelection::CoreWallet),
        Some(ValidatedAddress::Core(core_address())),
        Some(Amount::new_dash(0.0)),
        "Amount must be greater than 0",
    );
}

// The two `Err` arms of `validated_send_route` are pure routing decisions with no
// network dependency. Pinning their exact wording guards the user-facing "not yet
// supported" guidance against silent drift and keeps the routing match exhaustive.

#[test]
fn rejects_identity_to_shielded_route() {
    assert_validation_error(
        Some(identity_source(0xC1)),
        Some(ValidatedAddress::Shielded(shielded_address())),
        Some(Amount::new_dash(1.0)),
        "Sending from an identity to the shielded pool is not yet supported. \
         Transfer to a Platform address first, then shield from there.",
    );
}

#[test]
fn rejects_shielded_to_identity_route() {
    assert_validation_error(
        Some(SourceSelection::Shielded(
            rand::random(),
            3 * ONE_DASH_CREDITS,
        )),
        Some(ValidatedAddress::Identity {
            id: Identifier::from([0xD2; 32]),
            dpns_name: None,
        }),
        Some(Amount::new_dash(1.0)),
        "Sending from the shielded pool to an identity is not yet supported. \
         Transfer to a Platform address first, then top up the identity.",
    );
}
