//! The Send form's flows (`docs/ai-design/2026-10-23-send-screen-redesign/`
//! wireframes W1–W14), rendered headless over seeded balances. Each test
//! settles the harness with `run()` before asserting, so none depends on a
//! fixed number of frames between an action and its effect.

use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::model::address::{ValidatedAddress, encode_shielded_address};
use dash_evo_tool::model::amount::Amount;
use dash_evo_tool::model::secret::Secret;
use dash_evo_tool::model::spend_limit::CoreSendShortfall;
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::model::wallet::{Wallet, WalletSeedHash};
use dash_evo_tool::ui::wallets::send_screen::{
    SendBalances, SendFlow, SourceSelection, WalletSendScreen,
};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::{Address, Network, PublicKey};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, RwLock};

const ONE_DASH_DUFFS: u64 = 100_000_000;
const ONE_DASH_CREDITS: u64 = ONE_DASH_DUFFS * CREDITS_PER_DUFF;

fn core_address() -> Address {
    core_address_on(Network::Testnet)
}

fn core_address_on(network: Network) -> Address {
    let secret_key = loop {
        let bytes: [u8; 32] = rand::random();
        if let Ok(key) = dash_sdk::dpp::secp256k1::SecretKey::from_secret_bytes(bytes) {
            break key;
        }
    };
    Address::p2pkh(&PublicKey::new(secret_key.public_key()), network)
}

fn platform_destination() -> ValidatedAddress {
    let address = PlatformAddress::try_from(core_address()).expect("Platform address");
    ValidatedAddress::Platform {
        address,
        bech32m: address.to_bech32m_string(Network::Testnet),
    }
}

/// Someone else's private address.
fn foreign_shielded_destination() -> ValidatedAddress {
    let seed: [u8; 64] = rand::random();
    let keys =
        platform_wallet::wallet::shielded::OrchardKeySet::from_seed(&seed, Network::Testnet, 0)
            .expect("Orchard keys");
    ValidatedAddress::Shielded(
        encode_shielded_address(&keys.address_at(0).to_raw_address_bytes(), Network::Testnet)
            .expect("shielded address"),
    )
}

/// 2 DASH on Core and 3 DASH on Platform.
fn balances() -> SendBalances {
    let core = core_address();
    let platform = core_address();
    SendBalances {
        core_duffs: 2 * ONE_DASH_DUFFS,
        core_addresses: vec![(core, 2 * ONE_DASH_DUFFS)],
        platform: vec![(
            platform.clone(),
            PlatformAddress::try_from(platform).expect("Platform address"),
            3 * ONE_DASH_CREDITS,
        )],
        shielded_credits: 0,
    }
}

struct Fixture {
    _runtime: tokio::runtime::Runtime,
    screen: WalletSendScreen,
    seed_hash: WalletSeedHash,
}

fn fixture(password: Option<&str>) -> Fixture {
    let (runtime, app_context) = fresh_app_context();
    app_context.set_user_role(UserRole::Everyday);
    let password = password.map(Secret::new);
    let mut wallet = Wallet::new_from_seed(
        rand::random(),
        Network::Testnet,
        Some("Main".to_string()),
        password.as_ref(),
    )
    .expect("wallet fixture");
    if password.is_some() {
        wallet.wallet_seed.close();
    }
    let seed_hash = wallet.seed_hash();
    let mut screen = WalletSendScreen::new(&app_context, Arc::new(RwLock::new(wallet)));
    screen.set_balances_for_test(balances());
    Fixture {
        _runtime: runtime,
        screen,
        seed_hash,
    }
}

type Observed = Rc<RefCell<AppAction>>;

fn harness(screen: WalletSendScreen, width: f32) -> (Harness<'static, WalletSendScreen>, Observed) {
    let observed: Observed = Rc::new(RefCell::new(AppAction::None));
    let captured = observed.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(width, 1200.0))
        .build_ui_state(
            move |ui, screen: &mut WalletSendScreen| {
                let action = screen.render_form_for_test(ui);
                if !matches!(action, AppAction::None) {
                    *captured.borrow_mut() = action;
                }
            },
            screen,
        );
    harness.run();
    (harness, observed)
}

fn has(harness: &Harness<'_, WalletSendScreen>, text: &str) -> bool {
    harness.query_by_label_contains(text).is_some()
}

/// Whether pressing Send gets anywhere: a confirmation or a password prompt.
/// A disabled Send only takes hover, so the press is the observable test.
fn send_responds(harness: &mut Harness<'_, WalletSendScreen>) -> bool {
    click_send(harness);
    harness.state().has_send_confirmation_for_test() || harness.state().is_unlock_open_for_test()
}

fn click_send(harness: &mut Harness<'_, WalletSendScreen>) {
    harness
        .get_all_by_label("Send")
        .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
        .expect("a Send button")
        .click();
    harness.run();
}

fn enter(screen: &mut WalletSendScreen, destination: ValidatedAddress, dash: f64) {
    screen.set_simple_send_input_for_test(None, Some(destination), Some(Amount::new_dash(dash)));
}

#[test]
fn w1_opens_on_auto_and_someone_else_with_send_waiting_for_a_recipient() {
    with_isolated_data_dir(|| {
        let fixture = fixture(None);
        let (mut harness, _) = harness(fixture.screen, 900.0);

        assert!(has(&harness, "✓ Auto"), "Pay from starts on Auto");
        assert!(harness.query_by_label("Someone else").is_some());
        assert!(harness.query_by_label("My wallets").is_some());
        assert!(has(
            &harness,
            "Auto: picks a balance once you enter who you're paying."
        ));
        assert!(has(&harness, "Enter who you're paying."));
        assert!(!send_responds(&mut harness), "Send waits for a recipient");
        assert!(
            !has(&harness, "Advanced"),
            "the Advanced source is an Expert option"
        );
    });
}

#[test]
fn w2_auto_pays_a_platform_recipient_from_platform_and_confirms_before_sending() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        enter(&mut fixture.screen, platform_destination(), 1.0);
        let (mut harness, observed) = harness(fixture.screen, 900.0);

        assert!(has(&harness, "Auto: paying from Platform"));
        assert!(has(&harness, "Summary"));
        assert!(has(&harness, "· Platform to tdash1"));
        assert!(matches!(
            harness.state().resolved_route_for_test().0,
            Some(SourceSelection::PlatformAddresses(_))
        ));
        click_send(&mut harness);
        assert!(harness.state().has_send_confirmation_for_test());
        assert!(has(&harness, "Send 1 DASH?"));
        assert!(has(
            &harness,
            "Check the recipient. Payments can't be undone."
        ));
        assert!(
            matches!(*observed.borrow(), AppAction::None),
            "Send only opens the confirmation"
        );
    });
}

#[test]
fn w3_an_explicit_source_is_kept() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture
            .screen
            .seed_asset_lock_max_amount_for_test(fixture.seed_hash, 2 * ONE_DASH_DUFFS);
        enter(&mut fixture.screen, platform_destination(), 0.5);
        let (mut harness, _) = harness(fixture.screen, 900.0);

        harness.get_by_label_contains("Core ·").click();
        harness.run();

        assert!(has(&harness, "✓ Core"));
        assert!(
            !has(&harness, "Auto: paying from"),
            "an explicit source shows no Auto status"
        );
        assert!(matches!(
            harness.state().resolved_route_for_test().0,
            Some(SourceSelection::CoreWallet)
        ));
    });
}

#[test]
fn w5_auto_waits_for_the_core_quote_then_falls_back_to_platform() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        enter(
            &mut fixture.screen,
            ValidatedAddress::Core(core_address()),
            1.0,
        );
        let (mut harness, _) = harness(fixture.screen, 900.0);

        assert!(has(&harness, "Auto: checking balances…"));
        assert!(
            !send_responds(&mut harness),
            "nothing is sent while Auto checks"
        );

        harness
            .state_mut()
            .seed_core_send_limit_for_test(ONE_DASH_CREDITS / 2);
        harness.run();

        assert!(has(
            &harness,
            "Auto: paying from Platform · Core can't cover 1 DASH plus the fee."
        ));
        assert!(matches!(
            harness.state().resolved_route_for_test().0,
            Some(SourceSelection::PlatformAddresses(_))
        ));
    });
}

#[test]
fn w6_my_wallets_pays_an_own_address_and_auto_on_receive_pins_pay_from() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        // The address the backend derives is on the context's network.
        let own = core_address_on(fixture.screen.app_context.network());
        fixture
            .screen
            .seed_own_core_address_for_test(fixture.seed_hash, own.to_string());
        let (mut harness, _) = harness(fixture.screen, 900.0);

        harness.get_by_label("My wallets").click();
        harness.run();

        // Same wallet, more on Platform: Receive into starts on Core.
        assert!(has(&harness, "✓ Core"));
        assert!(has(&harness, "To a new Core address in "));
        assert_eq!(
            harness.state().resolved_route_for_test().1,
            Some(ValidatedAddress::Core(own))
        );

        // Auto on Receive into while Pay from is Auto pins Pay from.
        let receive_auto = harness
            .get_all_by_label("Auto")
            .last()
            .expect("Receive into Auto chip");
        receive_auto.click();
        harness.run();
        assert!(has(&harness, "so Receive into can choose automatically."));
    });
}

#[test]
fn w8_the_advanced_source_lists_addresses_in_expert_view() {
    with_isolated_data_dir(|| {
        let fixture = fixture(None);
        fixture.screen.app_context.set_user_role(UserRole::Power);
        let (mut harness, _) = harness(fixture.screen, 900.0);

        harness.get_by_label("Advanced").click();
        harness.run();

        assert!(has(&harness, "Spend from"));
        assert!(harness.query_by_label("Core addresses").is_some());
        assert!(harness.query_by_label("Platform addresses").is_some());
        assert!(has(
            &harness,
            "Core pays from the whole wallet. The ticked addresses only limit how much you can send."
        ));
        assert!(harness.query_by_label("+ Add another recipient").is_some());
    });
}

#[test]
fn w9_max_with_auto_takes_the_largest_balance() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture
            .screen
            .seed_asset_lock_max_amount_for_test(fixture.seed_hash, 2 * ONE_DASH_DUFFS);
        fixture
            .screen
            .set_simple_send_input_for_test(None, Some(platform_destination()), None);
        let (mut harness, _) = harness(fixture.screen, 900.0);

        harness.get_by_label("Max").click();
        harness.run();

        assert!(has(
            &harness,
            "Auto: paying from Platform · Max uses the largest balance."
        ));
        assert!(has(&harness, "Everything available in Platform"));
        assert!(send_responds(&mut harness));
    });
}

#[test]
fn w10_core_cannot_pay_someone_elses_private_address() {
    with_isolated_data_dir(|| {
        let fixture = fixture(None);
        let (mut harness, observed) = harness(fixture.screen, 900.0);
        harness.get_by_label_contains("Core ·").click();
        harness.run();

        enter(harness.state_mut(), foreign_shielded_destination(), 0.5);
        harness.run();

        assert!(has(
            &harness,
            "Core and Platform funds can only move into your own private balance."
        ));
        assert!(has(&harness, "Fix the highlighted field."));
        assert!(!send_responds(&mut harness));
        assert!(matches!(*observed.borrow(), AppAction::None));
    });
}

#[test]
fn w11_a_locked_wallet_asks_for_its_password_when_send_is_pressed() {
    with_isolated_data_dir(|| {
        // Generated per run: no wallet secret is ever committed.
        let password = format!("{:032x}", rand::random::<u128>());
        let mut fixture = fixture(Some(&password));
        enter(&mut fixture.screen, platform_destination(), 1.0);
        let (mut harness, observed) = harness(fixture.screen, 900.0);

        assert!(has(
            &harness,
            "Locked. You'll be asked for the password when you send."
        ));
        // A locked wallet does not disable Send: pressing it asks first.
        click_send(&mut harness);
        assert!(harness.state().is_unlock_open_for_test());
        assert!(!harness.state().has_send_confirmation_for_test());
        assert!(matches!(*observed.borrow(), AppAction::None));
    });
}

#[test]
fn w13_narrow_chips_drop_their_balances() {
    with_isolated_data_dir(|| {
        let fixture = fixture(None);
        let (harness, _) = harness(fixture.screen, 380.0);

        assert!(harness.query_by_label("Core").is_some());
        assert!(!has(&harness, "Core ·"));
    });
}

#[test]
fn w14_the_shield_preset_fixes_the_destination() {
    with_isolated_data_dir(|| {
        let fixture = fixture(None);
        let screen = fixture.screen.with_flow(SendFlow::Shield);
        let (harness, _) = harness(screen, 900.0);

        assert!(has(&harness, "Your private balance in "));
        assert!(harness.query_by_label("Someone else").is_none());
        assert!(
            !has(&harness, "Identity"),
            "the Shield preset pays from wallet balances"
        );
    });
}

/// No funds anywhere.
fn empty_balances() -> SendBalances {
    SendBalances {
        core_duffs: 0,
        core_addresses: Vec::new(),
        platform: Vec::new(),
        shielded_credits: 0,
    }
}

fn text_inputs(harness: &Harness<'_, WalletSendScreen>) -> usize {
    harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .count()
}

#[test]
fn w1_the_recipient_field_is_focused_and_asks_in_plain_words() {
    with_isolated_data_dir(|| {
        let fixture = fixture(None);
        let (mut harness, _) = harness(fixture.screen, 900.0);
        harness.run();

        let field = harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .find(|node| {
                node.accesskit_node().placeholder() == Some("Address, username or identity ID")
            })
            .expect("the Send to field asks for an address, username or identity ID");
        assert!(
            field.accesskit_node().is_focused(),
            "W1: the cursor waits in Send to"
        );
        assert!(
            !has(&harness, "type:"),
            "search tags are not shown to Everyday users"
        );
    });
}

#[test]
fn w8_advanced_on_an_empty_wallet_keeps_send_disabled() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture.screen.app_context.set_user_role(UserRole::Power);
        fixture.screen.set_balances_for_test(empty_balances());
        enter(
            &mut fixture.screen,
            ValidatedAddress::Core(core_address()),
            0.1,
        );
        let (mut harness, _) = harness(fixture.screen, 900.0);

        harness.get_by_label("Advanced").click();
        harness.run();

        assert!(has(&harness, "No addresses of this kind hold any funds."));
        assert!(has(&harness, "Choose the addresses to pay from."));
        assert!(!send_responds(&mut harness));
    });
}

#[test]
fn w10_an_amount_above_an_explicit_source_says_how_much_it_can_send() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture
            .screen
            .set_simple_send_input_for_test(None, Some(platform_destination()), None);
        let (mut harness, _) = harness(fixture.screen, 900.0);

        harness.get_by_label("Platform · 3.0000").click();
        harness.run();
        let amount = harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .find(|node| node.accesskit_node().placeholder() == Some("0.00"))
            .expect("the Amount field");
        amount.focus();
        harness.run();
        harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .find(|node| node.accesskit_node().placeholder() == Some("0.00"))
            .expect("the Amount field")
            .type_text("5");
        harness.run();

        assert!(has(&harness, "⚠ Platform can send at most"));
        assert!(harness.query_by_label_contains("Use ").is_some());
        assert!(has(&harness, "Fix the highlighted field."));
        assert!(!has(&harness, "Enter an amount."));
        assert!(!send_responds(&mut harness));
    });
}

#[test]
fn w10_the_amount_field_stays_when_no_amount_covers_the_fee() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture
            .screen
            .seed_asset_lock_max_amount_for_test(fixture.seed_hash, 1_000);
        enter(&mut fixture.screen, platform_destination(), 0.5);
        let (mut harness, _) = harness(fixture.screen, 900.0);

        harness.get_by_label_contains("Core ·").click();
        harness.run();

        assert!(has(&harness, "⚠ The amount you can use is too small"));
        assert_eq!(text_inputs(&harness), 2, "Send to and Amount both stay");
        assert!(!send_responds(&mut harness));
    });
}

#[test]
fn w10_an_empty_wallet_says_so_under_the_amount_even_after_max() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture.screen.set_balances_for_test(empty_balances());
        enter(
            &mut fixture.screen,
            ValidatedAddress::Core(core_address()),
            5.0,
        );
        fixture.screen.seed_core_send_limit_for_test(0);
        let (mut harness, _) = harness(fixture.screen, 900.0);

        // The fixture wallet is not registered, so it reads "Unnamed wallet".
        let no_funds = "⚠ There are no funds in Unnamed wallet to send. Add funds first.";
        assert!(has(&harness, no_funds));

        harness.get_by_label("Max").click();
        harness.run();

        assert!(has(&harness, no_funds), "Max keeps the typed amount");
        assert!(!has(&harness, "Enter an amount."));
        assert!(!send_responds(&mut harness));
    });
}

#[test]
fn w6_a_pinned_pay_from_returns_to_auto_when_receive_into_is_chosen() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        let own = core_address_on(fixture.screen.app_context.network());
        fixture
            .screen
            .seed_own_core_address_for_test(fixture.seed_hash, own.to_string());
        let (mut harness, _) = harness(fixture.screen, 900.0);
        harness.get_by_label("My wallets").click();
        harness.run();
        harness
            .get_all_by_label("Auto")
            .last()
            .expect("Receive into Auto chip")
            .click();
        harness.run();
        assert!(has(&harness, "so Receive into can choose automatically."));

        harness
            .get_all_by_label("Core")
            .last()
            .expect("Receive into Core chip")
            .click();
        harness.run();

        assert_eq!(
            harness.query_all_by_label_contains("✓ Auto").count(),
            1,
            "Pay from is back on Auto"
        );
    });
}

#[test]
fn w8_several_recipients_count_only_filled_rows_and_align_their_remove_buttons() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture.screen.app_context.set_user_role(UserRole::Power);
        // A row must hold an address for the active network.
        let network = fixture.screen.app_context.network();
        let address =
            PlatformAddress::try_from(core_address_on(network)).expect("Platform address");
        let bech32m = address.to_bech32m_string(network);
        let destination = ValidatedAddress::Platform {
            address,
            bech32m: bech32m.clone(),
        };
        enter(&mut fixture.screen, destination, 0.5);
        let (mut harness, _) = harness(fixture.screen, 900.0);

        harness.get_by_label("+ Add another recipient").click();
        harness.run();

        assert!(has(&harness, "Summary"));
        assert!(
            !has(&harness, "2 recipients"),
            "a blank row is not a recipient"
        );
        assert!(has(&harness, &format!("to {}", &bech32m[..8])));

        let removes: Vec<f32> = harness
            .get_all_by_label("✕")
            .map(|node| node.rect().center().x)
            .collect();
        assert_eq!(removes.len(), 2);
        assert!(
            (removes[0] - removes[1]).abs() < 0.5,
            "the ✕ buttons share a column: {removes:?}"
        );
    });
}

#[test]
fn w3_choosing_an_explicit_source_does_not_shift_the_form() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        enter(&mut fixture.screen, platform_destination(), 0.5);
        let (mut harness, _) = harness(fixture.screen, 900.0);
        let before = harness.get_by_label("To").rect().top();

        harness.get_by_label("Platform · 3.0000").click();
        harness.run();

        let after = harness.get_by_label("To").rect().top();
        assert!(
            (before - after).abs() < 0.5,
            "To moved from {before} to {after}"
        );
    });
}

#[test]
fn w2_the_auto_status_shows_the_balance_like_the_chips() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        enter(&mut fixture.screen, platform_destination(), 1.0);
        let (harness, _) = harness(fixture.screen, 900.0);

        let status = harness
            .get_by_label_contains("Auto: paying from Platform ·")
            .accesskit_node()
            .value()
            .expect("status text");
        let amount = status
            .split(" · ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .expect("an amount");
        let decimals = amount.split('.').nth(1).map_or(0, str::len);
        assert_eq!(decimals, 4, "4 decimals like the chips: {status}");
    });
}

#[test]
fn w8_summary_details_say_the_addresses_are_paid_from() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture.screen.app_context.set_user_role(UserRole::Power);
        enter(&mut fixture.screen, platform_destination(), 1.0);
        let (mut harness, _) = harness(fixture.screen, 900.0);

        harness.get_by_label_contains("Details").click();
        harness.run();

        assert!(
            has(&harness, "Paid from these addresses"),
            "the source addresses are labelled as such"
        );
    });
}

const SPEND_IN_PROGRESS: &str = "Another transaction from this wallet is still in progress.";

#[test]
fn a_busy_core_source_withholds_only_max() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        enter(
            &mut fixture.screen,
            ValidatedAddress::Core(core_address()),
            0.5,
        );
        fixture.screen.mark_core_send_busy_for_test();
        let (mut harness, _) = harness(fixture.screen, 900.0);
        harness.get_by_label_contains("Core ·").click();
        harness.run();

        assert!(has(&harness, SPEND_IN_PROGRESS), "the form says why");
        assert!(
            harness
                .query_by_label("Retry available amount check")
                .is_none()
        );

        harness.get_by_label("Max").click();
        harness.run();

        assert!(
            !has(&harness, "Checking the available amount…"),
            "a busy wallet is not a pending quote"
        );
        assert!(!has(&harness, "Enter an amount."), "the typed amount stays");
        assert!(
            send_responds(&mut harness),
            "an explicit amount is not blocked"
        );
    });
}

/// A Core balance held by another transaction is not a reason to pay a Core
/// recipient from Platform: Auto waits and says why instead of switching.
#[test]
fn auto_waits_while_another_transaction_holds_core() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        enter(
            &mut fixture.screen,
            ValidatedAddress::Core(core_address()),
            1.0,
        );
        fixture.screen.mark_core_send_busy_for_test();
        let (harness, _) = harness(fixture.screen, 900.0);

        assert!(!has(&harness, "Auto: checking balances…"));
        assert!(!has(&harness, "Auto: paying from Platform"));
        assert_eq!(harness.state().resolved_route_for_test().0, None);
        assert!(has(&harness, "is still in progress"));
    });
}

/// Auto Max cannot claim the largest balance while one balance is held.
#[test]
fn auto_max_waits_while_another_transaction_holds_core() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        enter(
            &mut fixture.screen,
            ValidatedAddress::Core(core_address()),
            1.0,
        );
        fixture.screen.mark_core_send_busy_for_test();
        let (mut harness, _) = harness(fixture.screen, 900.0);
        harness.get_by_label("Max").click();
        harness.run();

        assert!(!has(&harness, "Max uses the largest balance"));
        assert_eq!(harness.state().resolved_route_for_test().0, None);
        assert!(has(&harness, "is still in progress"));
    });
}

/// When the wallet says why Core can send nothing, Auto repeats that reason
/// instead of claiming the wallet has no funds.
#[test]
fn auto_states_why_core_funds_cannot_be_sent() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture.screen.set_balances_for_test(SendBalances {
            core_duffs: ONE_DASH_DUFFS,
            core_addresses: vec![(core_address(), ONE_DASH_DUFFS)],
            platform: Vec::new(),
            shielded_credits: 0,
        });
        enter(
            &mut fixture.screen,
            ValidatedAddress::Core(core_address()),
            0.5,
        );
        fixture
            .screen
            .mark_core_send_error_for_test(&TaskError::CoreSendMaxUnavailable {
                reason: CoreSendShortfall::NothingSpendable,
            });
        let (harness, _) = harness(fixture.screen, 900.0);

        assert!(!has(&harness, "Add funds first"));
        assert!(
            harness
                .query_all_by_label_contains("CoinJoin funds cannot be sent here")
                .next()
                .is_some()
        );
    });
}

/// Text inputs in the order they render: each recipient row is an address
/// field followed by its amount field.
fn type_into_input(harness: &mut Harness<'_, WalletSendScreen>, index: usize, text: &str) {
    let input = || {
        harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .nth(index)
            .expect("a text input")
    };
    input().focus();
    harness.run();
    harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .nth(index)
        .expect("a text input")
        .type_text(text);
    harness.run();
}

fn enter_two_core_rows(harness: &mut Harness<'_, WalletSendScreen>) {
    let network = harness.state().app_context.network();
    type_into_input(harness, 0, &core_address_on(network).to_string());
    type_into_input(harness, 2, &core_address_on(network).to_string());
    type_into_input(harness, 1, "0.1");
    type_into_input(harness, 3, "0.2");
}

/// Several recipients entered from scratch: Auto resolves against the rows.
#[test]
fn auto_pays_several_core_recipients_entered_from_scratch() {
    with_isolated_data_dir(|| {
        let fixture = fixture(None);
        fixture.screen.app_context.set_user_role(UserRole::Power);
        let (mut harness, _) = harness(fixture.screen, 900.0);
        harness.get_by_label("+ Add another recipient").click();
        harness.run();
        enter_two_core_rows(&mut harness);
        harness
            .state_mut()
            .seed_core_send_limit_for_test(ONE_DASH_CREDITS);
        harness.run();

        assert!(matches!(
            harness.state().resolved_route_for_test().0,
            Some(SourceSelection::CoreWallet)
        ));
        assert!(send_responds(&mut harness), "Auto pays two Core recipients");
    });
}

/// The single recipient typed before adding rows does not steer Auto once
/// the rows say otherwise.
#[test]
fn auto_follows_the_rows_not_the_first_typed_recipient() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture.screen.app_context.set_user_role(UserRole::Power);
        enter(&mut fixture.screen, platform_destination(), 0.5);
        let (mut harness, _) = harness(fixture.screen, 900.0);
        harness.get_by_label("+ Add another recipient").click();
        harness.run();
        harness.get_by_label("+ Add another recipient").click();
        harness.run();
        // Drop the Platform row: two blank rows remain.
        harness
            .get_all_by_label("✕")
            .next()
            .expect("remove")
            .click();
        harness.run();
        enter_two_core_rows(&mut harness);
        harness
            .state_mut()
            .seed_core_send_limit_for_test(ONE_DASH_CREDITS);
        harness.run();

        assert!(matches!(
            harness.state().resolved_route_for_test().0,
            Some(SourceSelection::CoreWallet)
        ));
        assert!(send_responds(&mut harness), "Auto pays two Core recipients");
    });
}

/// The confirmation names where a Shield lands instead of an empty line.
#[test]
fn shield_confirmation_names_the_private_balance() {
    with_isolated_data_dir(|| {
        let fixture = fixture(None);
        let mut screen = fixture.screen.with_flow(SendFlow::Shield);
        screen.set_simple_send_input_for_test(None, None, Some(Amount::new_dash(0.5)));
        let (mut harness, _) = harness(screen, 900.0);
        harness.get_by_label("Platform · 3.0000").click();
        harness.run();
        click_send(&mut harness);

        assert!(harness.state().has_send_confirmation_for_test());
        assert!(has(&harness, "To: your private balance in "));
    });
}

/// The confirmation shows the whole recipient address: a lookalike that
/// shares only its first and last characters must not pass the check.
#[test]
fn confirmation_shows_the_whole_recipient_address() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        let recipient = core_address();
        enter(
            &mut fixture.screen,
            ValidatedAddress::Core(recipient.clone()),
            0.5,
        );
        fixture
            .screen
            .seed_core_send_limit_for_test(ONE_DASH_CREDITS);
        let (mut harness, _) = harness(fixture.screen, 900.0);
        click_send(&mut harness);

        assert!(harness.state().has_send_confirmation_for_test());
        assert!(has(&harness, &format!("To: {recipient}\n")));
    });
}

/// Advanced Core: Max offers only what the ticked addresses hold.
#[test]
fn advanced_core_max_offers_only_the_ticked_addresses() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        fixture.screen.app_context.set_user_role(UserRole::Power);
        fixture.screen.set_balances_for_test(SendBalances {
            core_duffs: 2 * ONE_DASH_DUFFS,
            core_addresses: vec![
                (core_address(), ONE_DASH_DUFFS),
                (core_address(), ONE_DASH_DUFFS),
            ],
            platform: Vec::new(),
            shielded_credits: 0,
        });
        enter(
            &mut fixture.screen,
            ValidatedAddress::Core(core_address()),
            0.1,
        );
        fixture
            .screen
            .seed_core_send_limit_for_test(2 * ONE_DASH_CREDITS - 1_000_000);
        let (mut harness, _) = harness(fixture.screen, 900.0);
        harness.get_by_label("Advanced").click();
        harness.run();
        harness
            .query_all_by_role(egui::accesskit::Role::CheckBox)
            .next()
            .expect("a checkbox")
            .click();
        harness.run();
        harness.get_by_label("Max").click();
        harness.run();
        harness.run();

        assert!(!has(&harness, "The ticked addresses hold"));
        assert!(has(&harness, "Everything available in Core · ≈ 1 DASH"));
    });
}

/// A recipient row missing its address or amount keeps Send disabled: the
/// payment would otherwise leave that row out.
#[test]
fn an_incomplete_recipient_row_keeps_send_disabled() {
    with_isolated_data_dir(|| {
        let fixture = fixture(None);
        fixture.screen.app_context.set_user_role(UserRole::Power);
        let (mut harness, _) = harness(fixture.screen, 900.0);
        harness.get_by_label("+ Add another recipient").click();
        harness.run();
        let network = harness.state().app_context.network();
        type_into_input(&mut harness, 0, &core_address_on(network).to_string());
        type_into_input(&mut harness, 1, "0.1");
        // The second row has an amount but nobody to pay.
        type_into_input(&mut harness, 3, "0.2");
        harness
            .state_mut()
            .seed_core_send_limit_for_test(ONE_DASH_CREDITS);
        harness.run();

        assert!(!send_responds(&mut harness));
        assert!(has(
            &harness,
            "Enter an address and an amount for every recipient"
        ));
    });
}

/// Removing the second recipient keeps the first one's valid address as the
/// recipient, so Send works without editing it again.
#[test]
fn the_remaining_recipient_stays_valid_after_removing_a_row() {
    with_isolated_data_dir(|| {
        let fixture = fixture(None);
        fixture.screen.app_context.set_user_role(UserRole::Power);
        let (mut harness, _) = harness(fixture.screen, 900.0);
        harness.get_by_label("+ Add another recipient").click();
        harness.run();
        enter_two_core_rows(&mut harness);
        harness
            .get_all_by_label("✕")
            .last()
            .expect("remove")
            .click();
        harness.run();
        harness
            .state_mut()
            .seed_core_send_limit_for_test(ONE_DASH_CREDITS);
        harness.run();

        assert!(harness.state().resolved_route_for_test().1.is_some());
        assert!(send_responds(&mut harness));
    });
}

/// A failed Core quote under Auto Max keeps the Max intent: once the retry
/// answers, Max fills in without pressing it again.
#[test]
fn auto_max_survives_a_failed_quote_and_its_retry() {
    with_isolated_data_dir(|| {
        let mut fixture = fixture(None);
        enter(
            &mut fixture.screen,
            ValidatedAddress::Core(core_address()),
            0.5,
        );
        let (mut harness, _) = harness(fixture.screen, 900.0);
        harness.get_by_label("Max").click();
        harness.run();
        harness
            .state_mut()
            .mark_core_send_error_for_test(&TaskError::SpendLimitTimedOut);
        harness.run();
        assert!(has(&harness, "could not be checked"));

        harness.get_by_label("Retry available amount check").click();
        harness.run();
        harness
            .state_mut()
            .seed_core_send_limit_for_test(ONE_DASH_CREDITS);
        harness.run();
        harness.run();

        assert!(has(&harness, "Max uses the largest balance"));
    });
}
