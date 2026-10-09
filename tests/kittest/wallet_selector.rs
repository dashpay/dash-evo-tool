//! Kittest coverage for `WalletSelector`, the shared wallet picker, driven
//! with synthetic rows so no wallet or network is needed.
//!
//! ComboBox quirk: the closed selector exposes its text as the accesskit
//! **value** (`get_by_value`), the rows of the open popup as **labels**.

use dash_evo_tool::model::address::AddressKind;
use dash_evo_tool::model::wallet::balance_summary::{
    CoreFigure, WalletBalanceEntry, WalletBalanceSummary, WalletChoice,
};
use dash_evo_tool::ui::components::wallet_selector::WalletSelector;
use dash_evo_tool::ui::components::{Component, ComponentResponse};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::cell::RefCell;
use std::rc::Rc;

const MAIN: WalletChoice = WalletChoice::Hd([1; 32]);
const SAVINGS: WalletChoice = WalletChoice::Hd([2; 32]);
const KEY: WalletChoice = WalletChoice::SingleKey([3; 32]);
const NOT_ENOUGH: &str = "This wallet does not have enough Dash.";

fn entry(choice: WalletChoice, name: &str, core_duffs: u64) -> WalletBalanceEntry {
    WalletBalanceEntry {
        choice,
        name: Some(name.to_string()),
        balances: WalletBalanceSummary::default()
            .with_core_duffs(core_duffs, core_duffs / 2)
            .with_platform_duffs(10_000_000),
    }
}

fn entries() -> Vec<WalletBalanceEntry> {
    vec![
        entry(MAIN, "Main", 50_000_000),
        entry(SAVINGS, "Savings", 100_000_000),
        entry(KEY, "Key 1", 25_000_000),
    ]
}

/// What the hosting screen keeps: the wallet in use and every reported change.
#[derive(Default)]
struct Picked {
    current: Option<WalletChoice>,
    changes: Vec<Option<WalletChoice>>,
}

/// Host `selector` the way a screen does: feed the rows and the current wallet
/// each frame, and apply a reported change.
fn mount(
    mut selector: WalletSelector,
    entries: Vec<WalletBalanceEntry>,
    visuals: egui::Visuals,
) -> (Harness<'static>, Rc<RefCell<Picked>>) {
    let picked = Rc::new(RefCell::new(Picked::default()));
    let shared = Rc::clone(&picked);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(move |ui| {
            ui.ctx().set_visuals(visuals.clone());
            let mut picked = shared.borrow_mut();
            selector.set_entries(entries.clone());
            selector.set_selected(picked.current);
            let response = selector.show(ui).inner;
            if response.has_changed() {
                picked.changes.push(*response.changed_value());
            }
            response.update(&mut picked.current);
        });
    harness.run();
    (harness, picked)
}

fn core_selector() -> WalletSelector {
    WalletSelector::new("test_wallet_selector").with_balance_kinds(&[AddressKind::Core])
}

fn open(harness: &mut Harness<'static>, closed_text: &str) {
    harness.get_by_value(closed_text).click();
    harness.run();
}

/// Hover `label` and step past the tooltip delay.
fn hover(harness: &mut Harness<'static>, label: &str) {
    harness.get_by_label(label).hover();
    for _ in 0..30 {
        harness.step();
    }
    harness.run();
}

#[test]
fn rows_show_wallet_type_name_and_balance_in_light_and_dark_mode() {
    for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
        let (mut harness, _picked) = mount(core_selector(), entries(), visuals);
        open(&mut harness, "Select a wallet");

        for row in [
            "HD: Main — 0.5000 DASH",
            "HD: Savings — 1.0000 DASH",
            "SK: Key 1 — 0.2500 DASH",
        ] {
            assert!(harness.query_by_label(row).is_some(), "missing row {row}");
        }
    }
}

#[test]
fn rows_show_the_sum_of_the_kinds_the_screen_counts() {
    let selector = WalletSelector::new("test_wallet_selector")
        .with_balance_kinds(&[AddressKind::Core, AddressKind::Platform])
        .with_core_figure(CoreFigure::Usable);
    let (mut harness, _picked) = mount(selector, entries(), egui::Visuals::light());
    open(&mut harness, "Select a wallet");

    // 0.25 usable Core + 0.1 Platform.
    assert!(harness.query_by_label("HD: Main — 0.3500 DASH").is_some());
}

#[test]
fn clicking_a_row_reports_that_wallet_once_and_shows_it_when_closed() {
    let (mut harness, picked) = mount(core_selector(), entries(), egui::Visuals::light());
    open(&mut harness, "Select a wallet");
    harness.get_by_label("HD: Savings — 1.0000 DASH").click();
    harness.run();

    assert_eq!(picked.borrow().current, Some(SAVINGS));
    assert_eq!(picked.borrow().changes, vec![Some(SAVINGS)]);
    assert!(
        harness
            .query_by_value("HD: Savings — 1.0000 DASH")
            .is_some(),
        "the closed selector must show the picked wallet with its balance"
    );
}

#[test]
fn clicking_the_wallet_already_in_use_reports_no_change() {
    let (mut harness, picked) = mount(core_selector(), entries(), egui::Visuals::light());
    picked.borrow_mut().current = Some(MAIN);
    harness.run();
    open(&mut harness, "HD: Main — 0.5000 DASH");
    harness.get_by_label("HD: Main — 0.5000 DASH").click();
    harness.run();

    assert_eq!(picked.borrow().current, Some(MAIN));
    assert!(picked.borrow().changes.is_empty());
}

#[test]
fn hovering_a_row_breaks_its_balance_down_by_kind() {
    let selector = WalletSelector::new("test_wallet_selector")
        .with_balance_kinds(&[AddressKind::Core, AddressKind::Platform]);
    let (mut harness, _picked) = mount(selector, entries(), egui::Visuals::light());
    open(&mut harness, "Select a wallet");
    hover(&mut harness, "HD: Main — 0.6000 DASH");

    assert!(
        harness
            .query_by_label("Core: 0.5000 DASH\nPlatform: 0.1000 DASH")
            .is_some(),
        "hovering a row must show the per-kind breakdown"
    );
}

#[test]
fn hovering_the_closed_selector_breaks_the_chosen_wallets_balance_down() {
    let selector = WalletSelector::new("test_wallet_selector")
        .with_balance_kinds(&[AddressKind::Core, AddressKind::Platform]);
    let (mut harness, picked) = mount(selector, entries(), egui::Visuals::light());
    picked.borrow_mut().current = Some(MAIN);
    harness.run();

    harness.get_by_value("HD: Main — 0.6000 DASH").hover();
    for _ in 0..30 {
        harness.step();
    }
    harness.run();

    assert!(
        harness
            .query_by_label("Core: 0.5000 DASH\nPlatform: 0.1000 DASH")
            .is_some(),
        "the chosen wallet's breakdown must be reachable without reopening the list"
    );
}

#[test]
fn unavailable_row_is_disabled_and_says_why() {
    let mut selector = core_selector();
    selector.set_unavailable([(SAVINGS, NOT_ENOUGH.to_string())]);
    let (mut harness, picked) = mount(selector, entries(), egui::Visuals::light());
    open(&mut harness, "Select a wallet");

    assert!(
        harness
            .get_by_label("HD: Savings — 1.0000 DASH")
            .accesskit_node()
            .is_disabled()
    );
    hover(&mut harness, "HD: Savings — 1.0000 DASH");
    assert!(
        harness.query_by_label(NOT_ENOUGH).is_some(),
        "a greyed-out wallet must say why it cannot be picked"
    );

    harness.get_by_label("HD: Savings — 1.0000 DASH").click();
    harness.run();
    assert_eq!(
        picked.borrow().current,
        None,
        "a disabled row cannot be picked"
    );
}

#[test]
fn all_wallets_row_clears_the_choice() {
    let selector = core_selector().with_all_wallets_option("All unlocked wallets");
    let (mut harness, picked) = mount(selector, entries(), egui::Visuals::light());
    picked.borrow_mut().current = Some(MAIN);
    harness.run();
    open(&mut harness, "HD: Main — 0.5000 DASH");
    harness.get_by_label("All unlocked wallets").click();
    harness.run();

    assert_eq!(picked.borrow().current, None);
    assert_eq!(picked.borrow().changes, vec![None]);
    assert!(harness.query_by_value("All unlocked wallets").is_some());
}

#[test]
fn no_wallets_shows_a_message_instead_of_an_empty_selector() {
    let (harness, _picked) = mount(core_selector(), Vec::new(), egui::Visuals::light());

    assert!(harness.query_by_label("No wallets available.").is_some());
    assert!(harness.query_by_value("Select a wallet").is_none());
}

#[test]
fn label_is_the_selectors_accessible_name() {
    let selector = core_selector().with_label("Wallet to search");
    let (harness, _picked) = mount(selector, entries(), egui::Visuals::light());

    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::ComboBox, "Wallet to search")
            .is_some(),
        "the label must name the selector for assistive technology"
    );
}
