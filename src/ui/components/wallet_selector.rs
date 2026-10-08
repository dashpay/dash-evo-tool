//! Shared wallet picker: one row per wallet with its type, its name, and the
//! balance the calling screen counts.

use crate::model::address::AddressKind;
use crate::model::fee_estimation::format_duffs_as_dash;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::balance_summary::{
    CoreFigure, WalletBalanceEntry, WalletBalanceSummary, WalletChoice,
};
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::theme::ResponseExt;
use dash_sdk::dpp::balances::credits::Credits;
use egui::{Button, ComboBox, InnerResponse, Response, Ui};
use std::collections::BTreeMap;

const PLACEHOLDER: &str = "Select a wallet";
const NO_WALLETS: &str = "No wallets available.";
const UNNAMED_WALLET: &str = "Unnamed wallet";

/// What one wallet's row shows.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    label: String,
    /// The counted balance, one line per kind.
    breakdown: String,
    /// Why the wallet cannot be picked, when it cannot.
    unavailable: Option<String>,
}

/// Result of showing a [`WalletSelector`] for one frame.
#[derive(Clone)]
pub struct WalletSelectorResponse {
    /// The egui response of the selector.
    pub response: Response,
    changed: bool,
    selected: Option<WalletChoice>,
}

impl ComponentResponse for WalletSelectorResponse {
    type DomainType = WalletChoice;

    fn has_changed(&self) -> bool {
        self.changed
    }

    fn is_valid(&self) -> bool {
        true
    }

    /// The picked wallet; `None` after a change means "all wallets".
    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.selected
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// A dropdown of wallets. Each row reads `HD: {name} — {balance}` (or `SK:` for
/// an imported key); the balance is the sum of the kinds the screen counts and
/// hovering a row breaks it down by kind. The screen owns the choice: it sets
/// the rows and the current wallet every frame and acts on a reported change.
///
/// ```ignore
/// let selector = self.wallet_selector.get_or_insert_with(|| {
///     WalletSelector::new("select_wallet").with_balance_kinds(&[AddressKind::Core])
/// });
/// selector.set_entries(self.app_context.wallet_selector_entries(false));
/// selector.set_selected(current);
/// let response = selector.show(ui).inner;
/// ```
pub struct WalletSelector {
    id_salt: String,
    label: Option<String>,
    kinds: Vec<AddressKind>,
    core_figure: CoreFigure,
    all_wallets_label: Option<String>,
    entries: Vec<WalletBalanceEntry>,
    identity_credits: BTreeMap<WalletSeedHash, Credits>,
    unavailable: BTreeMap<WalletChoice, String>,
    selected: Option<WalletChoice>,
}

impl WalletSelector {
    /// A selector that counts every balance kind; `id_salt` must be unique on
    /// the screen.
    pub fn new(id_salt: impl Into<String>) -> Self {
        Self {
            id_salt: id_salt.into(),
            label: None,
            kinds: AddressKind::ALL.to_vec(),
            core_figure: CoreFigure::default(),
            all_wallets_label: None,
            entries: Vec::new(),
            identity_credits: BTreeMap::new(),
            unavailable: BTreeMap::new(),
            selected: None,
        }
    }

    /// Show `label` above the selector and use it as its accessible name.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Count only these balance kinds. See [`Self::set_balance_kinds`].
    pub fn with_balance_kinds(mut self, kinds: &[AddressKind]) -> Self {
        self.set_balance_kinds(kinds);
        self
    }

    /// Which Core figure counts as the Core balance.
    pub fn with_core_figure(mut self, core_figure: CoreFigure) -> Self {
        self.core_figure = core_figure;
        self
    }

    /// Offer a first row, labelled `label`, that stands for no single wallet.
    pub fn with_all_wallets_option(mut self, label: impl Into<String>) -> Self {
        self.all_wallets_label = Some(label.into());
        self
    }

    /// Count only these balance kinds. An empty list is ignored: a row always
    /// shows a balance.
    pub fn set_balance_kinds(&mut self, kinds: &[AddressKind]) {
        if !kinds.is_empty() {
            self.kinds = kinds.to_vec();
        }
    }

    /// The wallets to offer, in display order.
    pub fn set_entries(&mut self, entries: Vec<WalletBalanceEntry>) {
        self.entries = entries;
    }

    /// The summed identity balance of each recovery-phrase wallet. Counted
    /// only when [`AddressKind::Identity`] is among the balance kinds.
    pub fn set_identity_credits(&mut self, credits: BTreeMap<WalletSeedHash, Credits>) {
        self.identity_credits = credits;
    }

    /// Wallets that cannot be picked right now, each with the reason shown to
    /// the user on its greyed-out row.
    pub fn set_unavailable(
        &mut self,
        unavailable: impl IntoIterator<Item = (WalletChoice, String)>,
    ) {
        self.unavailable = unavailable.into_iter().collect();
    }

    /// The wallet the screen currently uses; `None` for no wallet / all wallets.
    pub fn set_selected(&mut self, selected: Option<WalletChoice>) {
        self.selected = selected;
    }

    /// The entry's balances with its wallet's identity balance filled in.
    fn balances(&self, entry: &WalletBalanceEntry) -> WalletBalanceSummary {
        match entry.choice {
            WalletChoice::Hd(seed_hash) => entry
                .balances
                .with_identity_credits(self.identity_credits.get(&seed_hash).copied().unwrap_or(0)),
            WalletChoice::SingleKey(_) => entry.balances,
        }
    }

    fn row(&self, entry: &WalletBalanceEntry) -> Row {
        let balances = self.balances(entry);
        let name = entry
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(UNNAMED_WALLET);
        let balance = format_duffs_as_dash(balances.total_duffs(&self.kinds, self.core_figure));
        let label = match entry.choice {
            WalletChoice::Hd(_) => format!("HD: {name} — {balance}"),
            WalletChoice::SingleKey(_) => format!("SK: {name} — {balance}"),
        };
        let breakdown = AddressKind::ALL
            .into_iter()
            .filter(|kind| self.kinds.contains(kind))
            .map(|kind| {
                let amount = format_duffs_as_dash(balances.kind_duffs(kind, self.core_figure));
                match (kind, self.core_figure) {
                    (AddressKind::Core, CoreFigure::Total) => format!("Core: {amount}"),
                    (AddressKind::Core, CoreFigure::Usable) => {
                        format!("Core, ready to use: {amount}")
                    }
                    (AddressKind::Platform, _) => format!("Platform: {amount}"),
                    (AddressKind::Shielded, _) => format!("Shielded: {amount}"),
                    (AddressKind::Identity, _) => format!("Identities: {amount}"),
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        Row {
            label,
            breakdown,
            unavailable: self.unavailable.get(&entry.choice).cloned(),
        }
    }

    /// The row of the wallet in use, while it is still offered.
    fn selected_row(&self) -> Option<Row> {
        let selected = self.selected?;
        let entry = self.entries.iter().find(|entry| entry.choice == selected)?;
        Some(self.row(entry))
    }

    /// Text of the closed selector.
    fn selected_text(&self) -> String {
        match (self.selected_row(), &self.all_wallets_label) {
            (Some(row), _) => row.label,
            (None, Some(all_wallets)) if self.selected.is_none() => all_wallets.clone(),
            (None, _) => PLACEHOLDER.to_string(),
        }
    }

    /// Record a click on a row; reports whether the choice changed.
    fn pick(&mut self, clicked: Option<WalletChoice>) -> bool {
        let changed = self.selected != clicked;
        self.selected = clicked;
        changed
    }
}

impl Component for WalletSelector {
    type DomainType = WalletChoice;
    type Response = WalletSelectorResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        ui.vertical(|ui| {
            let label_id = self.label.as_deref().map(|label| ui.label(label).id);
            if self.entries.is_empty() && self.all_wallets_label.is_none() {
                return WalletSelectorResponse {
                    response: ui.label(NO_WALLETS),
                    changed: false,
                    selected: self.selected,
                };
            }

            let mut clicked = None;
            let selected_row = self.selected_row();
            let mut response = ComboBox::from_id_salt(self.id_salt.as_str())
                .selected_text(self.selected_text())
                .show_ui(ui, |ui| {
                    if let Some(all_wallets) = &self.all_wallets_label
                        && ui
                            .selectable_label(self.selected.is_none(), all_wallets)
                            .clicked()
                    {
                        clicked = Some(None);
                    }
                    for entry in &self.entries {
                        let row = self.row(entry);
                        let is_selected = self.selected == Some(entry.choice);
                        let button = Button::selectable(is_selected, row.label);
                        match row.unavailable {
                            Some(reason) => {
                                ui.add_enabled(false, button).disabled_tooltip(reason);
                            }
                            None => {
                                if ui.add(button).clickable_tooltip(row.breakdown).clicked() {
                                    clicked = Some(Some(entry.choice));
                                }
                            }
                        }
                    }
                })
                .response;
            if let Some(row) = selected_row {
                response = response.clickable_tooltip(row.breakdown);
            }
            if let Some(label_id) = label_id {
                response = response.labelled_by(label_id);
            }

            let changed = clicked.is_some_and(|choice| self.pick(choice));
            WalletSelectorResponse {
                response,
                changed,
                selected: self.selected,
            }
        })
    }

    fn current_value(&self) -> Option<Self::DomainType> {
        self.selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: WalletChoice = WalletChoice::Hd([1; 32]);
    const SAVINGS: WalletChoice = WalletChoice::Hd([2; 32]);
    const KEY: WalletChoice = WalletChoice::SingleKey([3; 32]);

    fn balances() -> WalletBalanceSummary {
        WalletBalanceSummary::default()
            .with_core_duffs(50_000_000, 20_000_000)
            .with_platform_duffs(10_000_000)
            .with_shielded_credits(5_000_000_000)
    }

    fn entry(choice: WalletChoice, name: Option<&str>) -> WalletBalanceEntry {
        WalletBalanceEntry {
            choice,
            name: name.map(str::to_string),
            balances: balances(),
        }
    }

    fn selector() -> WalletSelector {
        let mut selector = WalletSelector::new("test");
        selector.set_entries(vec![
            entry(MAIN, Some("Main")),
            entry(SAVINGS, Some("Savings")),
            entry(KEY, Some("Key 1")),
        ]);
        selector
    }

    #[test]
    fn row_names_the_wallet_type_and_the_counted_balance() {
        let selector = selector().with_balance_kinds(&[AddressKind::Core]);
        assert_eq!(
            selector.row(&entry(MAIN, Some("Main"))).label,
            "HD: Main — 0.5 DASH"
        );
        assert_eq!(
            selector.row(&entry(KEY, Some("Key 1"))).label,
            "SK: Key 1 — 0.5 DASH"
        );
    }

    #[test]
    fn row_counts_every_kind_by_default() {
        // 0.5 Core + 0.1 Platform + 0.05 shielded.
        assert_eq!(
            selector().row(&entry(MAIN, Some("Main"))).label,
            "HD: Main — 0.65 DASH"
        );
    }

    #[test]
    fn row_counts_only_the_kinds_the_screen_asked_for() {
        let selector = selector().with_balance_kinds(&[AddressKind::Platform]);
        let row = selector.row(&entry(MAIN, Some("Main")));
        assert_eq!(row.label, "HD: Main — 0.1 DASH");
        assert_eq!(row.breakdown, "Platform: 0.1 DASH");
    }

    #[test]
    fn usable_core_figure_replaces_the_core_total() {
        let selector = selector()
            .with_balance_kinds(&[AddressKind::Core])
            .with_core_figure(CoreFigure::Usable);
        let row = selector.row(&entry(MAIN, Some("Main")));
        assert_eq!(row.label, "HD: Main — 0.2 DASH");
        assert_eq!(row.breakdown, "Core, ready to use: 0.2 DASH");
    }

    #[test]
    fn breakdown_lists_each_counted_kind_on_its_own_line() {
        let mut selector = selector();
        selector.set_identity_credits(BTreeMap::from([([1; 32], 2_000_000_000)]));
        assert_eq!(
            selector.row(&entry(MAIN, Some("Main"))).breakdown,
            "Core: 0.5 DASH\nPlatform: 0.1 DASH\nShielded: 0.05 DASH\nIdentities: 0.02 DASH"
        );
    }

    #[test]
    fn identity_credits_count_only_for_their_own_wallet() {
        let mut selector = selector().with_balance_kinds(&[AddressKind::Identity]);
        selector.set_identity_credits(BTreeMap::from([([1; 32], 2_000_000_000)]));
        assert_eq!(
            selector.row(&entry(MAIN, Some("Main"))).label,
            "HD: Main — 0.02 DASH"
        );
        assert_eq!(
            selector.row(&entry(SAVINGS, Some("Savings"))).label,
            "HD: Savings — 0 DASH"
        );
        assert_eq!(
            selector.row(&entry(KEY, Some("Key 1"))).label,
            "SK: Key 1 — 0 DASH"
        );
    }

    #[test]
    fn an_empty_kind_list_keeps_every_kind_counted() {
        let selector = selector().with_balance_kinds(&[]);
        assert_eq!(
            selector.row(&entry(MAIN, Some("Main"))).label,
            "HD: Main — 0.65 DASH"
        );
    }

    #[test]
    fn a_wallet_without_a_name_gets_a_fallback_name() {
        let selector = selector().with_balance_kinds(&[AddressKind::Core]);
        assert_eq!(
            selector.row(&entry(MAIN, None)).label,
            "HD: Unnamed wallet — 0.5 DASH"
        );
        assert_eq!(
            selector.row(&entry(MAIN, Some("  "))).label,
            "HD: Unnamed wallet — 0.5 DASH"
        );
    }

    #[test]
    fn an_unavailable_wallet_carries_its_reason() {
        let mut selector = selector();
        selector.set_unavailable([(SAVINGS, "Not enough Dash.".to_string())]);
        assert_eq!(
            selector
                .row(&entry(SAVINGS, Some("Savings")))
                .unavailable
                .as_deref(),
            Some("Not enough Dash.")
        );
        assert_eq!(selector.row(&entry(MAIN, Some("Main"))).unavailable, None);
    }

    #[test]
    fn closed_selector_shows_the_selected_row() {
        let mut selector = selector().with_balance_kinds(&[AddressKind::Core]);
        selector.set_selected(Some(SAVINGS));
        assert_eq!(selector.selected_text(), "HD: Savings — 0.5 DASH");
    }

    #[test]
    fn closed_selector_without_a_wallet_shows_the_placeholder_or_the_all_wallets_label() {
        let mut selector = selector();
        assert_eq!(selector.selected_text(), "Select a wallet");

        // A wallet that is no longer offered reads as no wallet.
        selector.set_selected(Some(WalletChoice::Hd([9; 32])));
        assert_eq!(selector.selected_text(), "Select a wallet");

        let mut selector = selector.with_all_wallets_option("All unlocked wallets");
        selector.set_selected(None);
        assert_eq!(selector.selected_text(), "All unlocked wallets");
    }

    #[test]
    fn picking_another_wallet_is_a_change() {
        let mut selector = selector();
        selector.set_selected(Some(MAIN));
        assert!(selector.pick(Some(SAVINGS)));
        assert_eq!(selector.current_value(), Some(SAVINGS));
    }

    #[test]
    fn picking_the_current_wallet_is_not_a_change() {
        let mut selector = selector();
        selector.set_selected(Some(MAIN));
        assert!(!selector.pick(Some(MAIN)));
        assert_eq!(selector.current_value(), Some(MAIN));
    }

    #[test]
    fn picking_all_wallets_clears_the_choice() {
        let mut selector = selector().with_all_wallets_option("All unlocked wallets");
        selector.set_selected(Some(MAIN));
        assert!(selector.pick(None));
        assert_eq!(selector.current_value(), None);
    }
}
