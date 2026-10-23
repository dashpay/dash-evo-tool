//! Rendering of the Send form: From, To, Amount, Summary and the Send button
//! (spec `docs/ai-design/2026-10-23-send-screen-redesign/02-ux-spec.md` §2).

use super::resolve::DestinationStatus;
use super::*;
use crate::model::send_route::{AutoMax, AutoSource, PayFromKind, RouteRefusal};
use crate::model::wallet::balance_summary::{WalletChoice, format_selector_balance};
use crate::ui::components::choice_chips::{ChoiceChip, ChoiceChips};
use crate::ui::components::wallet_selector::WalletSelector;
use crate::ui::state::send_form::{PayFrom, PinNotice, ReceiveInto, ToMode};
use crate::ui::theme::{ResponseExt, Shape};

/// Width of the label column in the wide layout, in points.
const LABEL_WIDTH: f32 = 100.0;
/// From this panel width on, labels sit left of their controls.
const WIDE_LAYOUT_WIDTH: f32 = 640.0;
/// Below this panel width, chips drop their balances.
const NARROW_LAYOUT_WIDTH: f32 = 420.0;

/// The responsive layout for the current panel width (spec §13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FormLayout {
    wide: bool,
    narrow: bool,
}

impl FormLayout {
    pub(super) fn for_width(width: f32) -> Self {
        Self {
            wide: width >= WIDE_LAYOUT_WIDTH,
            narrow: width < NARROW_LAYOUT_WIDTH,
        }
    }
}

/// A labelled row: label left of the controls when wide, above them otherwise.
fn form_row<R>(
    ui: &mut Ui,
    layout: FormLayout,
    label: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let dark_mode = ui.style().visuals.dark_mode;
    let label = RichText::new(label).color(DashColors::text_secondary(dark_mode));
    if layout.wide {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(LABEL_WIDTH, 20.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.label(label),
            );
            ui.vertical(add_contents).inner
        })
        .inner
    } else {
        ui.label(label);
        add_contents(ui)
    }
}

/// A section heading ("From", "To", "Amount").
fn section_heading(ui: &mut Ui, text: &str) {
    let dark_mode = ui.style().visuals.dark_mode;
    ui.add_space(4.0);
    ui.label(
        RichText::new(text)
            .color(DashColors::text_primary(dark_mode))
            .strong()
            .size(15.0),
    );
    ui.add_space(4.0);
}

/// A status line under a chip row.
fn status_line(ui: &mut Ui, text: impl Into<String>) {
    status_label(ui, text);
}

/// A status line whose response the caller decorates.
fn status_label(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    let dark_mode = ui.style().visuals.dark_mode;
    ui.label(
        RichText::new(text.into())
            .color(DashColors::text_secondary(dark_mode))
            .size(13.0),
    )
}

/// An inline problem under the field it concerns.
fn inline_warning(ui: &mut Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(format!("⚠ {}", text.into()))
            .color(DashColors::VALIDATION_WARNING)
            .size(13.0),
    );
}

/// A one-shot informational line.
fn info_line(ui: &mut Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(format!("ⓘ {}", text.into()))
            .color(DashColors::INFO)
            .size(13.0),
    );
}

/// Short name of a balance kind (spec D-14).
pub(super) fn kind_label(kind: PayFromKind) -> &'static str {
    match kind {
        PayFromKind::Core => "Core",
        PayFromKind::Platform => "Platform",
        PayFromKind::Identity => "Identity",
        PayFromKind::Shielded => "Shielded",
    }
}

/// What a balance kind means, in one sentence (spec §14).
fn kind_tooltip(kind: PayFromKind) -> &'static str {
    match kind {
        PayFromKind::Core => "Your regular Dash balance. Works with any Dash address.",
        PayFromKind::Platform => {
            "Your balance on Dash Platform. Fast and cheap to send to Platform addresses and identities."
        }
        PayFromKind::Identity => "Credits held by one of your identities.",
        PayFromKind::Shielded => {
            "Your private balance. Sending from it to a non-private address reveals the amount."
        }
    }
}

/// An address shortened in the middle, e.g. `yX12pQ…9fab`.
pub(super) fn short_address(address: &str) -> String {
    let chars: Vec<char> = address.chars().collect();
    if chars.len() <= 16 {
        return address.to_string();
    }
    let head: String = chars[..8].iter().collect();
    let tail: String = chars[chars.len() - 6..].iter().collect();
    format!("{head}…{tail}")
}

/// Why a route is refused, as the inline sentence under To.
fn refusal_message(refusal: RouteRefusal, own_pool: bool) -> &'static str {
    match refusal {
        RouteRefusal::IdentityToShielded if own_pool => {
            "You can't send from an identity to a private address. Choose Auto or Platform under Pay from."
        }
        RouteRefusal::IdentityToShielded => {
            "You can't send from an identity to a private address. Choose Shielded under Pay from."
        }
        RouteRefusal::ShieldedToIdentity => {
            "Private funds can't go straight to an identity. Send them to a Platform address first, then top up the identity."
        }
        RouteRefusal::ShieldedNotOwnPool => {
            "Core and Platform funds can only move into your own private balance. To pay this private address, choose Shielded under Pay from."
        }
    }
}

impl WalletSendScreen {
    /// The alias of a wallet, for sentences.
    pub(super) fn wallet_name(&self, seed_hash: &WalletSeedHash) -> String {
        self.app_context
            .wallet_context()
            .hd_alias(seed_hash)
            .unwrap_or_else(|| "Unnamed wallet".to_string())
    }

    fn paying_wallet_name(&self) -> String {
        self.selected_wallet_seed_hash
            .map(|seed_hash| self.wallet_name(&seed_hash))
            .unwrap_or_else(|| "Unnamed wallet".to_string())
    }

    fn is_expert(&self) -> bool {
        self.app_context.user_role().at_least(UserRole::Power)
    }

    /// Switch the paying wallet. The To section is kept (spec SR-16).
    fn change_from_wallet(&mut self, seed_hash: WalletSeedHash) {
        let wallet = self
            .app_context
            .wallet_context()
            .wallets()
            .get(&seed_hash)
            .cloned();
        self.selected_wallet = wallet;
        self.selected_wallet_seed_hash = Some(seed_hash);
        self.wallet_open_attempted = false;
        self.asset_lock_balance.invalidate();
        self.spend_limits.invalidate();
        self.address_input_snapshot_signature = None;
        self.form.touched();
    }

    /// Open the password-less wallet once, as the old unlock gate did.
    fn open_password_less_wallet(&mut self, ui: &Ui) {
        if self.wallet_open_attempted {
            return;
        }
        self.wallet_open_attempted = true;
        let Some(wallet) = &self.selected_wallet else {
            return;
        };
        if wallet.read().map(|w| w.is_open()).unwrap_or(true) {
            return;
        }
        if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
            MessageBanner::set_global(ui.ctx(), &e, MessageType::Error).disable_auto_dismiss();
        }
    }

    fn render_wallet_picker(&mut self, ui: &mut Ui) {
        let selector = self
            .from_wallet_selector
            .get_or_insert_with(|| WalletSelector::new("send_from_wallet"));
        selector.set_entries(self.app_context.wallet_selector_entries(false));
        selector.set_selected(
            self.selected_wallet
                .as_deref()
                .map(WalletChoice::of_hd_wallet),
        );
        let response = selector.show(ui).inner;
        if response.has_changed()
            && let Some(WalletChoice::Hd(seed_hash)) = response.changed_value()
        {
            self.change_from_wallet(*seed_hash);
        }
        if let Some(wallet) = &self.selected_wallet
            && wallet_needs_unlock(wallet)
        {
            status_line(
                ui,
                "🔒 Locked. You'll be asked for the password when you send.",
            );
        }
    }

    /// Chips for Pay from, with the reasons disabled ones carry.
    fn pay_from_chips(&mut self) -> Vec<ChoiceChip<PayFrom>> {
        let wallet = self.paying_wallet_name();
        let destination = self.destination_kind();
        let own_pool = self.shielded_destination_is_own_pool();
        let core_duffs = self.get_core_balance();
        let platform_credits: u64 = self
            .platform_source_addresses()
            .iter()
            .map(|(_, _, balance)| *balance)
            .sum();
        let shield_preset = self.flow == SendFlow::Shield;
        let top_up = matches!(self.identity_flow, Some((_, IdentitySendMode::TopUp)));

        let mut chips = vec![ChoiceChip::new(PayFrom::Auto, "Auto").with_tooltip(
            "Let the app pick the cheapest balance that can pay, and say which one it picked.",
        )];
        let refused = |kind: PayFromKind| {
            destination.and_then(|destination| {
                crate::model::send_route::route_refusal(kind, destination, own_pool)
            })
        };
        let mut core = ChoiceChip::new(PayFrom::Core, "Core")
            .with_detail(format_selector_balance(core_duffs).replace(" DASH", ""))
            .with_tooltip(kind_tooltip(PayFromKind::Core));
        if core_duffs == 0 {
            core = core.disabled(format!("Core has no funds in {wallet}."));
        } else if let Some(refusal) = refused(PayFromKind::Core) {
            core = core.disabled(refusal_message(refusal, own_pool));
        }
        chips.push(core);
        let mut platform = ChoiceChip::new(PayFrom::Platform, "Platform")
            .with_detail(
                format_selector_balance(platform_credits / CREDITS_PER_DUFF).replace(" DASH", ""),
            )
            .with_tooltip(kind_tooltip(PayFromKind::Platform));
        if platform_credits == 0 {
            platform = platform.disabled(format!("Platform has no funds in {wallet}."));
        } else if let Some(refusal) = refused(PayFromKind::Platform) {
            platform = platform.disabled(refusal_message(refusal, own_pool));
        }
        chips.push(platform);

        if !shield_preset {
            let identities = self.paying_identities();
            if !identities.is_empty() {
                let mut identity = ChoiceChip::new(PayFrom::Identity, "Identity")
                    .with_tooltip(kind_tooltip(PayFromKind::Identity));
                if let Some(chosen) = self.chosen_paying_identity() {
                    identity = identity.with_detail(
                        format_selector_balance(chosen.identity.balance() / CREDITS_PER_DUFF)
                            .replace(" DASH", ""),
                    );
                    let has_key = if destination == Some(AddressKind::Core) {
                        chosen.resolve_withdrawal_signing_key(None).is_ok()
                    } else {
                        chosen.resolve_transfer_signing_key(None).is_ok()
                    };
                    if !has_key {
                        identity =
                            identity.disabled("This identity has no key that can send credits.");
                    } else if let Some(refusal) = refused(PayFromKind::Identity) {
                        identity = identity.disabled(refusal_message(refusal, own_pool));
                    }
                }
                chips.push(identity);
            }
            if !top_up && let Some((_, balance)) = self.spendable_shielded() {
                let mut shielded = ChoiceChip::new(PayFrom::Shielded, "Shielded")
                    .with_detail(
                        format_selector_balance(balance / CREDITS_PER_DUFF).replace(" DASH", ""),
                    )
                    .with_tooltip(kind_tooltip(PayFromKind::Shielded));
                if let Some(refusal) = refused(PayFromKind::Shielded) {
                    shielded = shielded.disabled(refusal_message(refusal, own_pool));
                }
                chips.push(shielded);
            }
            if self.flow == SendFlow::General && self.identity_flow.is_none() && self.is_expert() {
                chips.push(
                    ChoiceChip::new(PayFrom::Advanced, "Advanced")
                        .with_tooltip("Choose exactly which addresses to spend from."),
                );
            }
        }
        chips
    }

    /// What Auto under Receive into would pin Pay from to: the source Auto
    /// resolved to, else the larger of Core and Platform.
    fn source_pin(&self) -> PayFromKind {
        if let Some(source) = &self.selected_source {
            return Self::source_kind(source);
        }
        let platform: u64 = self
            .platform_source_addresses()
            .iter()
            .map(|(_, _, balance)| *balance)
            .sum();
        if platform > self.get_core_balance().saturating_mul(CREDITS_PER_DUFF) {
            PayFromKind::Platform
        } else {
            PayFromKind::Core
        }
    }

    /// What Auto under Pay from would pin Receive into to: the kind its Auto
    /// resolved to.
    fn receive_pin(&self) -> PayFromKind {
        self.route.receive_kind.unwrap_or(PayFromKind::Core)
    }

    /// The default Receive into for My wallets (spec §5).
    fn default_receive(&self) -> PayFromKind {
        let core = self.get_core_balance().saturating_mul(CREDITS_PER_DUFF);
        let platform: u64 = self
            .platform_source_addresses()
            .iter()
            .map(|(_, _, balance)| *balance)
            .sum();
        let to_wallet = self.form.to_wallet().or(self.selected_wallet_seed_hash);
        let same_wallet = to_wallet == self.selected_wallet_seed_hash;
        match crate::model::send_route::default_receive_kind(core, platform, same_wallet) {
            AddressKind::Platform => PayFromKind::Platform,
            _ => PayFromKind::Core,
        }
    }

    fn render_pay_from_status(&mut self, ui: &mut Ui) {
        if let Some(PinNotice::PayFromPinned(kind)) = self.form.pin_notice() {
            info_line(
                ui,
                format!(
                    "Pay from is set to {kind} so Receive into can choose automatically.",
                    kind = kind_label(kind)
                ),
            );
            return;
        }
        if self.form.pay_from() != PayFrom::Auto {
            return;
        }
        let wallet = self.paying_wallet_name();
        let amount = self
            .requested_credits()
            .map(format_credits_as_dash)
            .unwrap_or_default();
        if let Some(outcome) = self.route.auto_max.clone() {
            match outcome {
                AutoMax::Chosen { kind, .. } => status_line(
                    ui,
                    format!(
                        "Auto: paying from {source} · Max uses the largest balance.",
                        source = kind_label(kind)
                    ),
                ),
                AutoMax::Checking { .. } => status_line(ui, "Auto: checking balances…"),
                AutoMax::NoFunds => {
                    status_line(ui, format!("Auto: {wallet} has no funds to send."))
                }
            }
            return;
        }
        let foreign_private = self.destination_kind() == Some(AddressKind::Shielded)
            && !self.shielded_destination_is_own_pool();
        match self.route.auto.clone() {
            Some(AutoSource::Chosen {
                kind,
                available,
                passed_over,
            }) => match passed_over.first() {
                Some(passed) => status_line(
                    ui,
                    format!(
                        "Auto: paying from {source} · {passed} can't cover {amount} plus the fee.",
                        source = kind_label(kind),
                        passed = kind_label(*passed)
                    ),
                ),
                None => status_line(
                    ui,
                    format!(
                        "Auto: paying from {source} · {available} available",
                        source = kind_label(kind),
                        available = format_credits_as_dash(available)
                    ),
                ),
            },
            Some(AutoSource::Checking { .. }) => status_line(ui, "Auto: checking balances…"),
            Some(AutoSource::QuoteFailed { kind }) => {
                status_line(
                    ui,
                    format!(
                        "Auto: the available amount in {source} could not be checked.",
                        source = kind_label(kind)
                    ),
                );
                if ui.button("Retry available amount check").clicked() {
                    self.asset_lock_balance.invalidate();
                    self.spend_limits.invalidate();
                }
            }
            Some(AutoSource::Preview { .. }) | None => status_line(
                ui,
                "Auto: picks a balance once you enter who you're paying.",
            ),
            Some(AutoSource::NothingCovers { .. }) => status_line(
                ui,
                format!("Auto: no single balance in {wallet} covers {amount} plus the fee."),
            ),
            Some(AutoSource::NoFunds) if foreign_private => status_line(
                ui,
                "Auto: only your private balance can pay a private address, and it holds nothing.",
            ),
            Some(AutoSource::NoFunds) if !self.paying_identities().is_empty() => status_line(
                ui,
                format!(
                    "Auto: {wallet}'s balances are empty. Choose Identity to pay from an identity."
                ),
            ),
            Some(AutoSource::NoFunds) => {
                status_line(ui, format!("Auto: {wallet} has no funds to send."))
            }
        }
    }

    fn render_identity_picker(
        &mut self,
        ui: &mut Ui,
        id_salt: &str,
        identities: &[QualifiedIdentity],
        chosen: Option<&QualifiedIdentity>,
    ) -> Option<dash_sdk::platform::Identifier> {
        let label_of = |qi: &QualifiedIdentity| {
            let name = self.app_context.identity_display_label(qi);
            let balance = format_credits_as_dash(qi.identity.balance());
            format!("{name} — {balance}")
        };
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.label("Identity");
            egui::ComboBox::from_id_salt(id_salt)
                .selected_text(
                    chosen
                        .map(label_of)
                        .unwrap_or_else(|| "Choose an identity".into()),
                )
                .show_ui(ui, |ui| {
                    for identity in identities {
                        let selected = chosen
                            .is_some_and(|chosen| chosen.identity.id() == identity.identity.id());
                        if ui.selectable_label(selected, label_of(identity)).clicked() {
                            picked = Some(identity.identity.id());
                        }
                    }
                });
        });
        picked
    }

    fn render_advanced_panel(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        let network = self.app_context.network;
        let rows: Vec<(String, u64)> = match self.form.advanced_type() {
            AdvancedSourceType::Core => self
                .get_core_addresses()
                .into_iter()
                .map(|(address, duffs)| {
                    (address.to_string(), duffs.saturating_mul(CREDITS_PER_DUFF))
                })
                .collect(),
            AdvancedSourceType::Platform => self
                .platform_source_addresses()
                .into_iter()
                .map(|(platform, _, credits)| (platform.to_bech32m_string(network), credits))
                .collect(),
        };
        let offered: Vec<String> = rows.iter().map(|(address, _)| address.clone()).collect();
        self.form.retain_advanced(&offered);
        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .stroke(egui::Stroke::new(
                Shape::BORDER_WIDTH,
                DashColors::border_light(dark_mode),
            ))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(Shape::RADIUS_SM)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Spend from");
                    for kind in [AdvancedSourceType::Core, AdvancedSourceType::Platform] {
                        if ui
                            .radio(self.form.advanced_type() == kind, kind.to_string())
                            .clicked()
                        {
                            self.form.choose_advanced_type(kind);
                        }
                    }
                });
                ui.add_space(4.0);
                if rows.is_empty() {
                    status_line(ui, "No addresses of this kind hold any funds.");
                    return;
                }
                let platform = self.form.advanced_type() == AdvancedSourceType::Platform;
                let ticked_count = self.form.ticked_advanced().len();
                let mut selected_total = 0u64;
                for (address, balance) in &rows {
                    let row = self.form.advanced_row(address);
                    if row.ticked {
                        selected_total = selected_total.saturating_add(*balance);
                    }
                    ui.horizontal(|ui| {
                        let limit_reached =
                            platform && !row.ticked && ticked_count >= MAX_PLATFORM_INPUTS;
                        let mut ticked = row.ticked;
                        let checkbox = ui
                            .add_enabled(!limit_reached, egui::Checkbox::new(&mut ticked, ""))
                            .disabled_tooltip(format!(
                                "One payment can use at most {MAX_PLATFORM_INPUTS} Platform addresses."
                            ));
                        if checkbox.changed() {
                            self.form.set_advanced_ticked(address, ticked);
                        }
                        ui.label(
                            RichText::new(short_address(address))
                                .monospace()
                                .color(DashColors::text_primary(dark_mode)),
                        )
                        .info_tooltip(address.as_str());
                        ui.label(
                            RichText::new(format_credits_as_dash(*balance))
                                .color(DashColors::success_color(dark_mode)),
                        );
                        if row.ticked {
                            let mut amount = row.amount.clone();
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut amount)
                                    .hint_text(RichText::new("Amount (optional)").color(Color32::GRAY))
                                    .desired_width(130.0),
                            );
                            if edit.changed() {
                                self.form.set_advanced_amount(address, amount);
                            }
                        }
                    });
                }
                ui.add_space(4.0);
                status_line(
                    ui,
                    format!(
                        "Selected: {total}",
                        total = format_credits_as_dash(selected_total)
                    ),
                );
                status_line(
                    ui,
                    "Leave the amounts empty to spread the payment over the ticked addresses.",
                );
            });
    }

    /// From: wallet picker, Pay from chips, status line, option panel.
    pub(super) fn render_from_section(&mut self, ui: &mut Ui, layout: FormLayout) {
        section_heading(ui, "From");
        let dark_mode = ui.style().visuals.dark_mode;
        if let Some((identity, IdentitySendMode::Send | IdentitySendMode::MoveToWallet)) =
            self.identity_flow.clone()
        {
            form_row(ui, layout, "Identity", |ui| {
                ui.label(
                    RichText::new(self.app_context.identity_display_label(&identity))
                        .color(DashColors::text_primary(dark_mode))
                        .strong(),
                );
                status_line(
                    ui,
                    format!(
                        "Identity balance: {balance} (set by {flow})",
                        balance = format_credits_as_dash(identity.identity.balance()),
                        flow = self.heading()
                    ),
                );
            });
            return;
        }

        self.open_password_less_wallet(ui);
        form_row(ui, layout, "Wallet", |ui| self.render_wallet_picker(ui));
        ui.add_space(6.0);

        if self.form.locks().pay_from {
            let balance = match &self.selected_source {
                Some(SourceSelection::Shielded(_, balance)) => *balance,
                _ => 0,
            };
            form_row(ui, layout, "Pay from", |ui| {
                ui.label(
                    RichText::new(format!(
                        "Shielded · {balance}",
                        balance = format_credits_as_dash(balance)
                    ))
                    .color(DashColors::text_primary(dark_mode))
                    .strong(),
                );
                status_line(ui, format!("Set by {flow}", flow = self.heading()));
            });
            return;
        }

        form_row(ui, layout, "Pay from", |ui| {
            let chip_list = self.pay_from_chips();
            let chips = self
                .pay_from_chips_widget
                .get_or_insert_with(|| ChoiceChips::new("send_pay_from"));
            chips.set_chips(chip_list);
            chips.set_selected(Some(self.form.pay_from()));
            chips.set_compact(layout.narrow);
            if let Some(choice) = *chips.show(ui).inner.changed_value() {
                let pin = self.receive_pin();
                self.form.choose_pay_from(choice, pin);
            }

            if self.form.pay_from() == PayFrom::Identity {
                let identities = self.paying_identities();
                if identities.len() > 1 {
                    let chosen = self.chosen_paying_identity();
                    if let Some(id) = self.render_identity_picker(
                        ui,
                        "send_pay_identity",
                        &identities,
                        chosen.as_ref(),
                    ) {
                        self.form.choose_pay_identity(id);
                    }
                }
            }
            if self.form.pay_from() == PayFrom::Advanced {
                ui.add_space(4.0);
                self.render_advanced_panel(ui);
            }
            self.render_pay_from_status(ui);
        });
    }

    fn receive_chips(&mut self, to_wallet: Option<WalletSeedHash>) -> Vec<ChoiceChip<ReceiveInto>> {
        let same_wallet = to_wallet.is_some() && to_wallet == self.selected_wallet_seed_hash;
        let explicit = self.form.explicit_source_kind().or_else(|| {
            self.identity_flow
                .as_ref()
                .filter(|(_, mode)| *mode != IdentitySendMode::TopUp)
                .map(|_| PayFromKind::Identity)
        });
        let unshield = self.flow == SendFlow::Unshield;
        let mut chips = vec![
            ChoiceChip::new(ReceiveInto::Auto, "Auto")
                .with_tooltip("Let the app pick where the funds land, and say where."),
        ];
        let mut kinds = vec![PayFromKind::Core, PayFromKind::Platform];
        if !unshield
            && let Some(to_wallet) = to_wallet
            && !self.receiving_identities(&to_wallet).is_empty()
        {
            kinds.push(PayFromKind::Identity);
        }
        if !unshield && FeatureGate::ShieldedOperations.is_available(&self.app_context) {
            kinds.push(PayFromKind::Shielded);
        }
        for kind in kinds {
            let mut chip = ChoiceChip::new(ReceiveInto::from_kind(kind), kind_label(kind))
                .with_tooltip(kind_tooltip(kind));
            if same_wallet && explicit == Some(kind) {
                chip = chip.disabled(
                    "You're paying from this balance. Pick a different one, or a different wallet.",
                );
            } else if let Some(source) = explicit {
                match crate::model::send_route::route_refusal(
                    source,
                    kind.address_kind(),
                    same_wallet,
                ) {
                    Some(RouteRefusal::ShieldedToIdentity) => {
                        chip = chip.disabled(
                            "Private funds can't go straight to an identity. Receive into Platform first, then top up the identity.",
                        );
                    }
                    Some(RouteRefusal::IdentityToShielded) => {
                        chip = chip.disabled(
                            "Identity credits can't go straight to a private balance. Receive into Platform first, then shield from there.",
                        );
                    }
                    Some(RouteRefusal::ShieldedNotOwnPool) => {
                        chip = chip.disabled(
                            "Core and Platform funds can only move into the paying wallet's own private balance. Choose the same wallet, or pay from Shielded.",
                        );
                    }
                    None => {}
                }
            }
            chips.push(chip);
        }
        chips
    }

    fn render_receive_status(&mut self, ui: &mut Ui, to_wallet: Option<WalletSeedHash>) {
        if let Some(PinNotice::ReceiveIntoPinned(kind)) = self.form.pin_notice() {
            info_line(
                ui,
                format!(
                    "Receive into is set to {kind} so Pay from can choose automatically.",
                    kind = kind_label(kind)
                ),
            );
        }
        let Some(to_wallet) = to_wallet else {
            status_line(ui, "Choose the wallet that receives.");
            return;
        };
        let wallet = self.wallet_name(&to_wallet);
        let auto = self.form.receive_into() == ReceiveInto::Auto;
        let prefix = if auto { "Auto: to" } else { "To" };
        match self.route.destination_status.clone() {
            DestinationStatus::Ready => {
                match (&self.validated_destination, self.route.receive_kind) {
                    (Some(ValidatedAddress::Shielded(address)), _) if address.is_empty() => {
                        status_line(ui, format!("{prefix} your private balance in {wallet}"))
                    }
                    (Some(ValidatedAddress::Shielded(_)), _) => {
                        status_line(ui, format!("{prefix} the private balance of {wallet}"))
                    }
                    (Some(ValidatedAddress::Identity { .. }), _) => {}
                    (Some(destination), Some(kind)) => {
                        let address = destination.to_address_string();
                        status_label(
                            ui,
                            format!(
                                "{prefix} a new {kind} address in {wallet} · {address}",
                                kind = kind_label(kind),
                                address = short_address(&address)
                            ),
                        )
                        .info_tooltip(address.as_str());
                    }
                    _ => {}
                }
            }
            DestinationStatus::Fetching { .. } => {
                status_line(ui, format!("Getting a new address in {wallet}…"))
            }
            DestinationStatus::NeedsUnlock { wallet: seed_hash } => {
                status_line(ui, format!("Unlock {wallet} to get a receiving address."));
                if ui.button("Unlock").clicked()
                    && let Some(target) = self.app_context.wallet_context().hd_wallet(&seed_hash)
                {
                    self.unlock_target = Some(target);
                    self.unlock_then_send = false;
                    self.wallet_unlock_popup.open();
                }
            }
            DestinationStatus::NoIdentity => {
                status_line(ui, format!("{wallet} has no identity to top up."))
            }
            DestinationStatus::Missing => {}
        }
        if auto
            && Some(to_wallet) == self.selected_wallet_seed_hash
            && self.route.destination_status == DestinationStatus::Ready
        {
            status_line(ui, "(same wallet, so Auto chose the other balance)");
        }
    }

    fn render_my_wallets(&mut self, ui: &mut Ui, layout: FormLayout) {
        let to_wallet = self.form.to_wallet().or(self.selected_wallet_seed_hash);
        let locked_receive = self.form.locks().receive_into;
        form_row(ui, layout, "Wallet", |ui| {
            let selector = self
                .to_wallet_selector
                .get_or_insert_with(|| WalletSelector::new("send_to_wallet"));
            selector.set_entries(self.app_context.wallet_selector_entries(false));
            selector.set_selected(to_wallet.map(WalletChoice::Hd));
            let response = selector.show(ui).inner;
            if response.has_changed()
                && let Some(WalletChoice::Hd(seed_hash)) = response.changed_value()
            {
                self.form.choose_to_wallet(*seed_hash);
            }
        });
        ui.add_space(6.0);
        form_row(ui, layout, "Receive into", |ui| {
            if locked_receive {
                let dark_mode = ui.style().visuals.dark_mode;
                ui.label(
                    RichText::new("Core")
                        .color(DashColors::text_primary(dark_mode))
                        .strong(),
                );
            } else {
                let chip_list = self.receive_chips(to_wallet);
                let chips = self
                    .receive_chips_widget
                    .get_or_insert_with(|| ChoiceChips::new("send_receive_into"));
                chips.set_chips(chip_list);
                chips.set_selected(Some(self.form.receive_into()));
                chips.set_compact(layout.narrow);
                if let Some(choice) = *chips.show(ui).inner.changed_value() {
                    let pin = self.source_pin();
                    self.form.choose_receive_into(choice, pin);
                }
            }
            if self.route.receive_kind == Some(PayFromKind::Identity)
                && let Some(to_wallet) = to_wallet
            {
                let identities = self.receiving_identities(&to_wallet);
                let chosen = self.chosen_receiving_identity(&to_wallet);
                if let Some(id) = self.render_identity_picker(
                    ui,
                    "send_receive_identity",
                    &identities,
                    chosen.as_ref(),
                ) {
                    self.form.choose_receive_identity(id);
                }
            }
            self.render_receive_status(ui, to_wallet);
        });
    }

    fn render_recipient_rows(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        let kinds: Vec<Option<AddressKind>> = self
            .form
            .recipients()
            .iter()
            .map(|row| AddressKind::detect(row.address.trim()))
            .collect();
        let first_kind = kinds.iter().flatten().next().copied();
        let mut remove = None;
        for (index, kind) in kinds.iter().enumerate() {
            ui.horizontal(|ui| {
                let row = &mut self.form.recipients_mut()[index];
                ui.add(
                    egui::TextEdit::singleline(&mut row.address)
                        .hint_text("Address")
                        .desired_width(260.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut row.amount)
                        .hint_text(RichText::new("0.0").color(Color32::GRAY))
                        .desired_width(90.0),
                );
                ui.label(RichText::new("DASH").color(DashColors::text_secondary(dark_mode)));
                if let Some(kind) = kind {
                    ui.label(
                        RichText::new(kind.short_label())
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0),
                    );
                }
                if ui
                    .button("✕")
                    .on_hover_text("Remove this recipient")
                    .clicked()
                {
                    remove = Some(index);
                }
            });
            match kind {
                Some(AddressKind::Core | AddressKind::Platform) if *kind != first_kind => {
                    inline_warning(
                        ui,
                        "All recipients in one payment must be the same kind. Send to this address separately.",
                    );
                }
                Some(AddressKind::Core | AddressKind::Platform) => {}
                Some(_) => inline_warning(
                    ui,
                    "Several recipients must all be Core addresses or all be Platform addresses.",
                ),
                None if !self.form.recipients()[index].address.trim().is_empty() => inline_warning(
                    ui,
                    "This isn't a Dash address. Check it and paste it again.",
                ),
                None => {}
            }
        }
        if let Some(index) = remove
            && let Some(survivor) = self.form.remove_recipient(index)
        {
            // Back to one recipient: its address and amount carry over.
            self.typed_destination = None;
            self.address_input = None;
            self.pending_typed_address = Some(survivor.address);
            let amount = Amount::parse(&survivor.amount, DASH_DECIMAL_PLACES)
                .ok()
                .map(|amount| amount.with_unit_name("DASH"));
            self.amount = amount.clone();
            if let Some(amount) = amount {
                self.amount_input
                    .get_or_insert_with(Self::new_amount_input)
                    .set_value(amount);
            } else {
                self.amount_input = None;
            }
        }
    }

    /// Kinds the Someone else field accepts for this entry point.
    pub(super) fn someone_else_kinds(&self) -> Vec<AddressKind> {
        if let Some(kinds) = self.flow.preset_destination_kinds() {
            return kinds;
        }
        self.general_destination_kinds()
    }

    fn render_someone_else(&mut self, ui: &mut Ui) {
        if self.form.has_several_recipients() {
            self.render_recipient_rows(ui);
        } else {
            let kinds = self.someone_else_kinds();
            self.render_destination_input_with_kinds(ui, &kinds);
        }
        let own_pool = self.shielded_destination_is_own_pool();
        if let Some(refusal) = self.route.refusal {
            inline_warning(ui, refusal_message(refusal, own_pool));
        } else if let Some(source) = &self.selected_source
            && self.validated_destination.is_some()
            && self.form.pay_from() != PayFrom::Auto
            && self
                .route
                .destination
                .is_some_and(|d| d.same_account == Some(Self::source_kind(source)))
        {
            inline_warning(
                ui,
                format!(
                    "This address belongs to {wallet} · {kind}, the balance you're paying from. You'd only pay a fee.",
                    wallet = self.paying_wallet_name(),
                    kind = kind_label(Self::source_kind(source))
                ),
            );
        }
        let can_add = self.flow == SendFlow::General
            && self.identity_flow.is_none()
            && self.is_expert()
            && self.form.to_mode() == ToMode::SomeoneElse;
        if can_add && ui.button("+ Add another recipient").clicked() {
            let address = self
                .typed_destination
                .as_ref()
                .map(ValidatedAddress::to_address_string)
                .unwrap_or_default();
            let amount = self
                .amount
                .as_ref()
                .map(Amount::to_string_without_unit)
                .unwrap_or_default();
            self.form.add_recipient(address, amount);
        }
    }

    /// To: Someone else | My wallets, then the field or the wallet options.
    pub(super) fn render_to_section(&mut self, ui: &mut Ui, layout: FormLayout) {
        section_heading(ui, "To");
        let dark_mode = ui.style().visuals.dark_mode;
        if let Some((identity, IdentitySendMode::TopUp)) = self.identity_flow.clone() {
            form_row(ui, layout, "Identity", |ui| {
                ui.label(
                    RichText::new(self.app_context.identity_display_label(&identity))
                        .color(DashColors::text_primary(dark_mode))
                        .strong(),
                );
                status_line(ui, format!("Set by {flow}", flow = self.heading()));
            });
            return;
        }
        if self.flow == SendFlow::Shield {
            form_row(ui, layout, "Receive into", |ui| {
                ui.label(
                    RichText::new(format!(
                        "Your private balance in {wallet}",
                        wallet = self.paying_wallet_name()
                    ))
                    .color(DashColors::text_primary(dark_mode))
                    .strong(),
                );
                status_line(ui, format!("Set by {flow}", flow = self.heading()));
            });
            return;
        }

        if !self.form.locks().to_mode {
            let move_to_wallet = matches!(
                self.identity_flow,
                Some((_, IdentitySendMode::MoveToWallet))
            );
            let someone_else = if move_to_wallet {
                "Choose an address"
            } else {
                "Someone else"
            };
            ui.horizontal_wrapped(|ui| {
                if layout.wide {
                    ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
                }
                let mut mode = self.form.to_mode();
                let changed = ui
                    .radio_value(&mut mode, ToMode::SomeoneElse, someone_else)
                    .changed()
                    | ui.radio_value(&mut mode, ToMode::MyWallets, "My wallets")
                        .changed();
                if changed {
                    let from = self.selected_wallet_seed_hash;
                    let default_receive = self.default_receive();
                    self.form.choose_to_mode(mode, from, default_receive);
                }
            });
            ui.add_space(4.0);
        }
        match self.form.to_mode() {
            ToMode::SomeoneElse => {
                if layout.wide {
                    ui.horizontal_top(|ui| {
                        ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
                        ui.vertical(|ui| self.render_someone_else(ui));
                    });
                } else {
                    self.render_someone_else(ui);
                }
            }
            ToMode::MyWallets => {
                self.render_my_wallets(ui, layout);
                if let Some(refusal) = self.route.refusal {
                    inline_warning(
                        ui,
                        refusal_message(refusal, self.shielded_destination_is_own_pool()),
                    );
                }
            }
        }
    }

    pub(super) fn new_amount_input() -> AmountInput {
        AmountInput::new(Amount::new_dash(0.0))
            .with_label("Amount")
            .with_hint_text("0.00")
            .with_max_button(true)
            .with_desired_width(150.0)
    }

    /// The Amount field with Max, its limits and the inline errors under it.
    pub(super) fn render_amount_input(&mut self, ui: &mut Ui) {
        let limits = self
            .selected_source
            .clone()
            .map(|source| self.amount_limits_for(&source, self.validated_destination.as_ref()));

        if limits.as_ref().is_some_and(|limits| limits.fee_not_covered) {
            self.amount = None;
            self.amount_input = None;
            ui.colored_label(DashColors::WARNING, FUNDING_FEE_NOT_COVERED);
            return;
        }
        let max_credits = limits.as_ref().and_then(|limits| limits.max_credits);
        let amount_input = self.amount_input.get_or_insert_with(Self::new_amount_input);
        amount_input.set_max_amount(max_credits);
        amount_input.set_caption(limits.as_ref().and_then(|limits| limits.hint.clone()));
        amount_input.set_min_amount(limits.as_ref().and_then(|limits| limits.min_credits));

        // Max is an intent: it follows the source's maximum until the amount
        // is edited (spec §6).
        if self.form.max_intent() {
            match max_credits {
                Some(max) if self.amount.as_ref().map(Amount::value) != Some(max) => {
                    let amount = Amount::new(max, DASH_DECIMAL_PLACES).with_unit_name("DASH");
                    amount_input.set_value(amount);
                }
                Some(_) => {}
                None => self.amount = None,
            }
        }

        let response = amount_input.show(ui);
        if response.inner.max_clicked {
            self.form.press_max();
        } else if response.inner.response.changed() {
            self.form.amount_edited();
        }
        response.inner.update(&mut self.amount);
        if self.form.max_intent()
            && let (Some(source), Some(max)) = (&self.selected_source, max_credits)
        {
            status_line(
                ui,
                format!(
                    "Everything available in {source} · ≈ {max}",
                    source = kind_label(Self::source_kind(source)),
                    max = format_credits_as_dash(max)
                ),
            );
        }
    }

    /// Amount: the field, quote status, and the no-single-balance offer.
    pub(super) fn render_amount_section(&mut self, ui: &mut Ui, layout: FormLayout) {
        section_heading(ui, "Amount");
        form_row(ui, layout, "", |ui| {
            self.render_asset_lock_balance_status(ui);
            self.render_spend_limit_status(ui);
            self.render_amount_input(ui);
            if let Some(AutoSource::NothingCovers {
                best: Some((kind, best)),
            }) = self.route.auto.clone()
            {
                inline_warning(
                    ui,
                    format!(
                        "The most one payment can send is {best}, from {source}.",
                        best = format_credits_as_dash(best),
                        source = kind_label(kind)
                    ),
                );
                if ui
                    .button(format!(
                        "Send {best} instead",
                        best = format_credits_as_dash(best)
                    ))
                    .clicked()
                {
                    self.apply_amount(best);
                }
            } else if let (Some(amount), Some(source)) = (
                self.amount.as_ref().map(Amount::value),
                self.selected_source.clone(),
            ) && self.form.pay_from() != PayFrom::Auto
                && let Some(max) = self
                    .amount_limits_for(&source, self.validated_destination.as_ref())
                    .max_credits
                && amount > max
            {
                inline_warning(
                    ui,
                    format!(
                        "{source} can send at most {max} after the fee.",
                        source = kind_label(Self::source_kind(&source)),
                        max = format_credits_as_dash(max)
                    ),
                );
                if ui
                    .button(format!("Use {max}", max = format_credits_as_dash(max)))
                    .clicked()
                {
                    self.apply_amount(max);
                }
            }
        });
    }

    /// Put `credits` in the Amount field as an exact amount.
    fn apply_amount(&mut self, credits: u64) {
        let amount = Amount::new(credits, DASH_DECIMAL_PLACES).with_unit_name("DASH");
        self.amount_input
            .get_or_insert_with(Self::new_amount_input)
            .set_value(amount.clone());
        self.amount = Some(amount);
        self.form.amount_edited();
    }

    /// How the destination reads in sentences.
    pub(super) fn destination_label(&self) -> String {
        if self.form.has_several_recipients() {
            let count = self.form.recipients().len();
            return format!("{count} recipients");
        }
        match &self.validated_destination {
            Some(ValidatedAddress::Shielded(address)) if address.is_empty() => format!(
                "your private balance in {wallet}",
                wallet = self.paying_wallet_name()
            ),
            Some(ValidatedAddress::Identity { id, dpns_name }) => {
                let id =
                    id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);
                match dpns_name {
                    Some(name) => format!("{name} ({short})", short = short_address(&id)),
                    None => short_address(&id),
                }
            }
            Some(destination) => short_address(&destination.to_address_string()),
            None => String::new(),
        }
    }

    /// How the source reads in sentences, e.g. "Main · Core".
    pub(super) fn source_label(&self) -> String {
        match &self.selected_source {
            Some(SourceSelection::Identity(qi)) => self.app_context.identity_display_label(qi),
            Some(source) => format!(
                "{wallet} · {kind}",
                wallet = self.paying_wallet_name(),
                kind = kind_label(Self::source_kind(source))
            ),
            None => self.paying_wallet_name(),
        }
    }

    /// The one route-specific note, if any (spec §7).
    pub(super) fn route_note(&self) -> Option<&'static str> {
        // TODO(#1106): once the Core→Shielded drain lands, add its note:
        // "Moves your whole Core balance. If the app closes, you can finish it
        // from the Shielded tab."
        match (
            self.selected_source.as_ref().map(Self::source_kind),
            self.destination_kind(),
        ) {
            (
                Some(PayFromKind::Platform | PayFromKind::Identity | PayFromKind::Shielded),
                Some(AddressKind::Core),
            ) => Some("Withdrawals to a Dash address can take a while to arrive."),
            (
                Some(PayFromKind::Core),
                Some(AddressKind::Platform | AddressKind::Identity | AddressKind::Shielded),
            ) => Some(
                "Your Dash is moved to Platform first, so this takes a little longer than a regular payment.",
            ),
            _ => None,
        }
    }

    /// Whether the Fee options row applies (spec SR-22).
    fn shows_fee_options(&self) -> bool {
        let explicit =
            self.form.pay_from() == PayFrom::Advanced || self.form.has_several_recipients();
        let platform_involved =
            self.form.explicit_source_kind() == Some(PayFromKind::Platform)
                || matches!(
                    &self.selected_source,
                    Some(SourceSelection::PlatformAddresses(_))
                )
                || self.destination_kind() == Some(AddressKind::Platform)
                || self.form.recipients().iter().any(|row| {
                    AddressKind::detect(row.address.trim()) == Some(AddressKind::Platform)
                });
        self.is_expert() && explicit && platform_involved
    }

    fn render_fee_options(&mut self, ui: &mut Ui) {
        let strategy = self.form.fee_strategy();
        egui::CollapsingHeader::new(format!("Fee options: {strategy}"))
            .id_salt("send_fee_options")
            .default_open(false)
            .show(ui, |ui| {
                for option in PlatformFeeStrategy::ALL {
                    let tooltip = match option {
                        PlatformFeeStrategy::DeductFromFirstInput => {
                            "The first address paying covers the fee."
                        }
                        PlatformFeeStrategy::DeductFromLastInput => {
                            "The last address paying covers the fee."
                        }
                        PlatformFeeStrategy::ReduceFirstOutput => {
                            "The first recipient receives the amount minus the fee."
                        }
                        PlatformFeeStrategy::ReduceLastOutput => {
                            "The last recipient receives the amount minus the fee."
                        }
                    };
                    if ui
                        .radio(strategy == option, option.to_string())
                        .clickable_tooltip(tooltip)
                        .clicked()
                    {
                        self.form.choose_fee_strategy(option);
                    }
                }
            });
    }

    fn render_summary_details(&self, ui: &mut Ui) {
        let Some(SourceSelection::PlatformAddresses(addresses)) = &self.selected_source else {
            return;
        };
        let Some(amount_credits) = self.requested_credits() else {
            return;
        };
        let destination = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_platform().copied());
        let allocation = allocate_platform_addresses(
            &self.app_context.fee_estimator(),
            addresses,
            amount_credits,
            destination.as_ref(),
        );
        if allocation.inputs.is_empty() {
            return;
        }
        let network = self.app_context.network;
        egui::CollapsingHeader::new("Details")
            .id_salt("send_summary_details")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("send_summary_details_grid")
                    .num_columns(2)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for (address, amount) in &allocation.inputs {
                            let address = address.to_bech32m_string(network);
                            ui.label(RichText::new(short_address(&address)).monospace())
                                .info_tooltip(address.as_str());
                            ui.label(format_credits_as_dash(*amount));
                            ui.end_row();
                        }
                    });
                if allocation.sorted_addresses.len() > MAX_PLATFORM_INPUTS {
                    status_line(
                        ui,
                        format!(
                            "Limited to the {MAX_PLATFORM_INPUTS} largest Platform addresses. Send the rest in another payment."
                        ),
                    );
                }
            });
    }

    /// The one Summary card (spec §7).
    pub(super) fn render_summary(&mut self, ui: &mut Ui) {
        let Some(amount_credits) = self.requested_credits().filter(|v| *v > 0) else {
            return;
        };
        if self.validated_destination.is_none() && !self.form.has_several_recipients() {
            return;
        }
        let Some(source) = self.selected_source.clone() else {
            return;
        };
        let dark_mode = ui.style().visuals.dark_mode;
        let explicit = self.uses_explicit_payment();
        let preview = if explicit {
            self.advanced_fee_estimate_credits()
                .map(|fee| FeePreview::on_top(amount_credits, fee))
        } else {
            self.current_fee_preview()
        };
        let preview = preview.map(|mut preview| {
            if explicit && self.form.fee_strategy().reduces_output() {
                preview = FeePreview::deducted_from_amount(amount_credits, preview.fee_credits);
            }
            preview
        });
        let kind = kind_label(Self::source_kind(&source));
        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .stroke(egui::Stroke::new(
                Shape::BORDER_WIDTH,
                DashColors::border_light(dark_mode),
            ))
            .inner_margin(Margin::symmetric(14, 10))
            .corner_radius(Shape::RADIUS_SM)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Summary")
                        .color(DashColors::text_primary(dark_mode))
                        .strong(),
                );
                status_line(
                    ui,
                    format!(
                        "Pay from {source} to {destination}",
                        source = self.source_label(),
                        destination = self.destination_label()
                    ),
                );
                ui.add_space(4.0);
                egui::Grid::new("send_summary_grid")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        let row = |ui: &mut Ui, label: &str, value: String, strong: bool| {
                            ui.label(
                                RichText::new(label).color(DashColors::text_secondary(dark_mode)),
                            );
                            let value =
                                RichText::new(value).color(DashColors::text_primary(dark_mode));
                            ui.label(if strong { value.strong() } else { value });
                            ui.end_row();
                        };
                        row(
                            ui,
                            "You send",
                            format_credits_as_dash(amount_credits),
                            false,
                        );
                        match preview {
                            Some(preview) => {
                                row(
                                    ui,
                                    "Network fee",
                                    format!("≈ {}", format_credits_as_dash(preview.fee_credits)),
                                    false,
                                );
                                if let Some(receives) = preview.recipient_receives_credits {
                                    row(
                                        ui,
                                        "Recipient gets",
                                        format!("≈ {}", format_credits_as_dash(receives)),
                                        false,
                                    );
                                }
                                row(
                                    ui,
                                    &format!("Total from {kind}"),
                                    format!(
                                        "≈ {}",
                                        format_credits_as_dash(preview.total_debit_credits)
                                    ),
                                    true,
                                );
                            }
                            None => {
                                row(ui, "Network fee", "Calculated when you send".into(), false);
                            }
                        }
                    });
                if let Some(note) = self.route_note() {
                    ui.add_space(4.0);
                    status_line(ui, note);
                }
                if self.shows_fee_options() {
                    self.render_fee_options(ui);
                }
                if self.is_expert() && !explicit {
                    self.render_summary_details(ui);
                }
            });
    }

    /// The first missing step, for the hint above a disabled Send (spec §8).
    pub(super) fn missing_step(&self) -> Option<&'static str> {
        let has_destination =
            self.validated_destination.is_some() || self.form.has_several_recipients();
        if !has_destination {
            return Some(match self.route.destination_status {
                DestinationStatus::Fetching { .. } => "Waiting for a receiving address.",
                DestinationStatus::NeedsUnlock { .. } => "Unlock the receiving wallet first.",
                _ => "Enter who you're paying.",
            });
        }
        if self.requested_credits().filter(|v| *v > 0).is_none() {
            return Some(match self.route.auto_max {
                Some(AutoMax::Checking { .. }) => "Checking the available amount…",
                _ => "Enter an amount.",
            });
        }
        if self.route.refusal.is_some() || self.selected_source.is_none() {
            return Some(match self.route.auto {
                Some(AutoSource::Checking { .. }) => "Checking the available amount…",
                Some(AutoSource::NoFunds) => "Add funds to this wallet first.",
                _ => "Fix the highlighted field.",
            });
        }
        None
    }

    /// The Send form: From, To, Amount, Summary and the actions.
    pub(super) fn render_form(&mut self, ui: &mut Ui) -> AppAction {
        self.sync_route();
        self.sync_explicit_payment();
        let layout = FormLayout::for_width(ui.available_width());
        self.render_from_section(ui, layout);
        ui.add_space(12.0);
        self.render_to_section(ui, layout);
        ui.add_space(12.0);
        if !self.form.has_several_recipients() {
            self.render_amount_section(ui, layout);
            ui.add_space(12.0);
        }
        self.render_summary(ui);
        ui.add_space(12.0);
        let hint = self.missing_step();
        if let Some(hint) = hint {
            status_line(ui, hint);
            ui.add_space(4.0);
        }
        self.render_send_button(ui)
    }

    /// Cancel and Send. Send always says "Send" (spec SR-17); a locked wallet
    /// asks for its password first (spec SR-18).
    pub(super) fn render_send_button(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let is_sending = matches!(self.send_status, SendStatus::WaitingForResult);
        let missing = self.missing_step();
        let can_send = !is_sending && missing.is_none();
        let narrow = ui.available_width() < NARROW_LAYOUT_WIDTH;
        let mut clicked = false;
        let mut layout = |ui: &mut Ui| {
            let label = if is_sending { "Sending..." } else { "Send" };
            let mut send = ComponentStyles::add_primary_button_enabled(ui, can_send, label);
            if let Some(missing) = missing {
                send = send.disabled_tooltip(missing);
            }
            clicked = send.clicked();
        };
        if narrow {
            ui.vertical(|ui| {
                layout(ui);
                if ui.button("Cancel").clicked() {
                    action = AppAction::PopScreen;
                }
            });
        } else {
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    action = AppAction::PopScreen;
                }
                ui.add_space(10.0);
                layout(ui);
            });
        }
        if clicked && can_send {
            self.press_send(ui.ctx());
        }
        action
    }

    /// Send was pressed: unlock first if the wallet needs it, then confirm.
    pub(super) fn press_send(&mut self, ctx: &Context) {
        let wallet_source = !matches!(self.selected_source, Some(SourceSelection::Identity(_)));
        if wallet_source
            && let Some(wallet) = &self.selected_wallet
            && !wallet.read().map(|w| w.is_open()).unwrap_or(false)
            && wallet_needs_unlock(wallet)
        {
            self.unlock_target = Some(wallet.clone());
            self.unlock_then_send = true;
            self.wallet_unlock_popup.open();
            return;
        }
        match self.prepare_send() {
            Ok((send_action, message)) => self.open_send_confirmation(send_action, message),
            Err(e) => {
                MessageBanner::set_global(ctx, &e, MessageType::Error);
                self.send_status = SendStatus::Error;
            }
        }
    }
}
