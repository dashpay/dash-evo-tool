//! Identity Home tab.
//!
//! See design-spec §B.2 / §B.3. The Home tab is the default landing inside the
//! Identities hub once at least one identity exists. It stacks:
//!
//! 1. Hero identity card (gradient surface) — display name, handle, balance,
//!    identity-type badge, network pill. Two variants: social profile set
//!    (`IdentityHeroCard` with display name) and no social profile (type-glyph
//!    monogram + inline `Set up your social profile` card below the hero).
//! 2. Quick-actions row: **Send**, **Receive**, **Add contact**. `Add contact`
//!    is gated behind a social profile (see §B.3).
//! 3. Secondary actions row (ghost buttons): `Add funds`, `Send to wallet`,
//!    `Send to another identity`. All three visible for all personas (§B.2).
//! 4. Onboarding checklist strip (until all three steps are complete or the
//!    user dismisses it).
//! 5. Recent activity preview (up to 5 rows). This task T8 scaffolds an
//!    empty-state preview and wires the `See all activity` link to the
//!    Activity tab — richer content is parked until the activity aggregator
//!    lands (feature `identity-hub-activity-feed`).
//! 6. Advanced details expander (raw Identity ID, revision, last updated).
//!
//! Strings are taken verbatim from §B.2 / §B.3 and the wording audit in §C.
//!
//! This module is state-less per-frame: the only persisted state is the
//! `dismissed_checklist` flag owned by the calling hub screen and passed in
//! via [`HomeState`]. Everything else is recomputed from `AppContext`.

use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::ui::ScreenType;
use crate::ui::components::identity_hero_card::{HeroIdentityKind, IdentityHeroCard};
use crate::ui::components::onboarding_checklist::{
    ChecklistAction, ChecklistStep, OnboardingChecklist,
};
use crate::ui::identities::register_dpns_name_screen::RegisterDpnsNameSource;
use crate::ui::identity::tabs::IdentityHubTab;
use crate::ui::theme::{DashColors, ResponseExt, Shape, Spacing};
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use std::sync::Arc;

/// Mutable state owned by the hub screen and passed by reference to the Home
/// tab each frame. Kept tiny so the hub stays the single source of truth.
#[derive(Debug, Default, Clone)]
pub struct HomeState {
    /// Whether the user has dismissed the onboarding checklist for this
    /// session. Dismissal is intentionally ephemeral for now (no DB schema
    /// change); if this needs to persist across restarts, the follow-up is
    /// an additive `Settings` DB column with `DEFAULT false`.
    pub dismissed_checklist: bool,
    /// Whether the user has dismissed the inline social profile card. Same
    /// ephemeral rationale as `dismissed_checklist` — honoured only for the
    /// current session.
    pub skipped_social_profile: bool,
    /// Whether the Advanced expander on Home is open. Persisted in memory so
    /// toggling state survives tab switches without a DB write.
    pub advanced_open: bool,
}

/// Intent returned by the Home tab so the hub screen can act on it without
/// needing deep knowledge of which button the user pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeOutcome {
    /// Nothing happened this frame.
    None,
    /// User wants to switch to the Activity tab.
    GoToActivity,
    /// User wants to switch to the Contacts tab (via `Add contact`).
    GoToContacts,
    /// User dismissed the onboarding checklist.
    DismissChecklist,
    /// User dismissed the inline social profile card.
    SkipSocialProfile,
    /// User toggled the Advanced expander.
    ToggleAdvanced,
}

/// Render the Home tab.
///
/// Returns a pair: `(AppAction, HomeOutcome)`. Most screens in the codebase
/// return only `AppAction`, but the Home tab also needs to report hub-local
/// intents (tab switch, dismiss checklist, toggle expander) that the hub
/// screen owns — hence the extra channel.
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &HomeState,
) -> (AppAction, HomeOutcome) {
    let dark_mode = ui.ctx().style().visuals.dark_mode;
    let mut action = AppAction::None;
    let mut outcome = HomeOutcome::None;

    // Pick the first loaded identity for the hero; when the picker lands
    // (T7), the hub will track a selected identity and pass it in.
    let identity = match first_loaded_identity(app_context) {
        Some(qi) => qi,
        None => {
            render_empty(ui, dark_mode);
            return (action, outcome);
        }
    };

    // --- Hero card ----------------------------------------------------
    let hero = build_hero(app_context, &identity);
    let hero_has_social_profile = hero.has_social_profile();
    let hero_response = hero.show(ui);
    if hero_response.pick_username_clicked() {
        action |= AppAction::AddScreen(
            ScreenType::RegisterDpnsName(RegisterDpnsNameSource::Identities)
                .create_screen(app_context),
        );
    }
    ui.add_space(Spacing::MD);

    // --- Quick actions row --------------------------------------------
    ui.horizontal(|ui| {
        if primary_quick_action(ui, "Send", "Send Dash to a contact, username, or address.")
            .clicked()
        {
            // Send flow for this identity — reuse the existing Transfer screen
            // until the dedicated Send sheet (§B.7) lands.
            action |= AppAction::AddScreen(
                ScreenType::TransferScreen(identity.clone()).create_screen(app_context),
            );
        }
        ui.add_space(Spacing::SM);
        if primary_quick_action(
            ui,
            "Receive",
            "Show a QR code or your username so someone can pay you.",
        )
        .clicked()
        {
            // Receive / add funds — reuse the existing TopUpIdentity screen.
            action |= AppAction::AddScreen(
                ScreenType::TopUpIdentity(identity.clone()).create_screen(app_context),
            );
        }
        ui.add_space(Spacing::SM);

        // Add contact is gated behind a social profile per §B.3.
        let add_contact = egui::Button::new(
            RichText::new("Add contact")
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        )
        .fill(DashColors::surface(dark_mode))
        .stroke(Stroke::new(
            Shape::BORDER_WIDTH,
            DashColors::border(dark_mode),
        ))
        .min_size(egui::vec2(160.0, 40.0));
        if hero_has_social_profile {
            let resp = ui
                .add(add_contact)
                .clickable_tooltip("Find someone by username and add them to your contacts.");
            if resp.clicked() {
                outcome = HomeOutcome::GoToContacts;
            }
        } else {
            ui.add_enabled(false, add_contact).disabled_tooltip(
                "Set up a social profile first. Contacts need a display name and avatar \
                 so people can find you.",
            );
        }
    });
    ui.add_space(Spacing::MD);

    // --- Secondary actions row ----------------------------------------
    ui.horizontal(|ui| {
        if ghost_action(
            ui,
            "Add funds",
            "Move Dash from your wallet into this identity.",
            dark_mode,
        )
        .clicked()
        {
            action |= AppAction::AddScreen(
                ScreenType::TopUpIdentity(identity.clone()).create_screen(app_context),
            );
        }
        ui.add_space(Spacing::SM);
        if ghost_action(
            ui,
            "Send to wallet",
            "Convert your identity balance back to spendable Dash in your wallet.",
            dark_mode,
        )
        .clicked()
        {
            action |= AppAction::AddScreen(
                ScreenType::WithdrawalScreen(identity.clone()).create_screen(app_context),
            );
        }
        ui.add_space(Spacing::SM);
        if ghost_action(
            ui,
            "Send to another identity",
            "Transfer Dash directly from this identity to another identity.",
            dark_mode,
        )
        .clicked()
        {
            action |= AppAction::AddScreen(
                ScreenType::TransferScreen(identity.clone()).create_screen(app_context),
            );
        }
    });
    ui.add_space(Spacing::MD);

    // --- Inline "Set up your social profile" card (no-profile variant) -
    if !hero_has_social_profile && !state.skipped_social_profile {
        if paint_social_profile_card(ui, dark_mode) {
            action |= AppAction::AddScreen(
                ScreenType::RegisterDpnsName(RegisterDpnsNameSource::Identities)
                    .create_screen(app_context),
            );
        }
        ui.add_space(Spacing::MD);
    }

    // --- Onboarding checklist -----------------------------------------
    if !state.dismissed_checklist {
        let mut checklist = OnboardingChecklist::new();
        if identity
            .dpns_names
            .iter()
            .any(|n| !n.name.trim().is_empty())
        {
            checklist = checklist.mark_complete(ChecklistStep::PickUsername);
        }
        if hero_has_social_profile {
            checklist = checklist.mark_complete(ChecklistStep::SetDisplayName);
        } else if state.skipped_social_profile {
            checklist = checklist.hide(ChecklistStep::SetDisplayName);
        }
        // We don't yet know if the user has contacts without a DashPay load;
        // leave `AddFirstContact` in its default (pending) state. The checklist
        // honours mark_complete calls as they come from the contacts pipeline
        // once T9 wires contact counts into the hub state.

        if !checklist.all_complete() {
            let resp = checklist.show(ui);
            match resp.action() {
                Some(ChecklistAction::Dismissed) => {
                    outcome = HomeOutcome::DismissChecklist;
                }
                Some(ChecklistAction::Activated(step)) => match step {
                    ChecklistStep::PickUsername => {
                        action |= AppAction::AddScreen(
                            ScreenType::RegisterDpnsName(RegisterDpnsNameSource::Identities)
                                .create_screen(app_context),
                        );
                    }
                    ChecklistStep::SetDisplayName => {
                        outcome = HomeOutcome::SkipSocialProfile;
                        // Deliberately route via the no-profile card above —
                        // the DashPay profile editor isn't part of T8's scope.
                    }
                    ChecklistStep::AddFirstContact => {
                        outcome = HomeOutcome::GoToContacts;
                    }
                },
                None => {}
            }
            ui.add_space(Spacing::MD);
        }
    }

    // --- Recent activity preview --------------------------------------
    ui.push_id("home_recent_activity", |ui| {
        let frame = Frame::new()
            .fill(DashColors::surface(dark_mode))
            .stroke(Stroke::new(
                Shape::BORDER_WIDTH,
                DashColors::border_light(dark_mode),
            ))
            .corner_radius(CornerRadius::same(Shape::RADIUS_MD))
            .inner_margin(Margin::same(Spacing::MD as i8));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Recent activity")
                        .size(16.0)
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
            });
            ui.add_space(Spacing::SM);

            // Empty-state preview — wiring to real data is parked until the
            // unified activity aggregator lands under
            // `identity-hub-activity-feed`. We explicitly render the design-
            // spec empty-state sentence so reviewers can see the copy is
            // correct.
            ui.label(
                RichText::new(
                    "No activity yet. When you send or receive Dash, it will show up here.",
                )
                .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(Spacing::SM);
            if ui
                .add(
                    egui::Label::new(
                        RichText::new("See all activity")
                            .underline()
                            .color(DashColors::DASH_BLUE),
                    )
                    .sense(egui::Sense::click()),
                )
                .clickable_tooltip("Open the unified activity timeline for this identity.")
                .clicked()
            {
                outcome = HomeOutcome::GoToActivity;
            }
        });
    });

    ui.add_space(Spacing::MD);

    // --- Advanced expander --------------------------------------------
    let advanced_header = if state.advanced_open {
        "▾ Advanced details"
    } else {
        "▸ Advanced details"
    };
    let header_resp = ui
        .add(
            egui::Label::new(
                RichText::new(advanced_header).color(DashColors::text_secondary(dark_mode)),
            )
            .sense(egui::Sense::click()),
        )
        .clickable_tooltip("Show technical details like raw IDs, keys, and revision numbers.");
    if header_resp.clicked() {
        outcome = HomeOutcome::ToggleAdvanced;
    }
    if state.advanced_open {
        ui.add_space(Spacing::XS);
        let frame = Frame::new()
            .fill(DashColors::surface(dark_mode))
            .stroke(Stroke::new(
                Shape::BORDER_WIDTH,
                DashColors::border_light(dark_mode),
            ))
            .corner_radius(CornerRadius::same(Shape::RADIUS_SM))
            .inner_margin(Margin::same(Spacing::SM as i8));
        frame.show(ui, |ui| {
            let id_str = identity.identity.id().to_string(Encoding::Base58);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Identity ID:").color(DashColors::text_secondary(dark_mode)),
                );
                ui.add_space(Spacing::XS);
                ui.add(
                    egui::Label::new(
                        RichText::new(&id_str)
                            .monospace()
                            .color(DashColors::text_primary(dark_mode)),
                    )
                    .selectable(true),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Version:").color(DashColors::text_secondary(dark_mode)));
                ui.add_space(Spacing::XS);
                ui.label(
                    RichText::new(identity.identity.revision().to_string())
                        .color(DashColors::text_primary(dark_mode)),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Keys:").color(DashColors::text_secondary(dark_mode)));
                ui.add_space(Spacing::XS);
                ui.label(
                    RichText::new(identity.identity.public_keys().len().to_string())
                        .color(DashColors::text_primary(dark_mode)),
                );
            });
        });
    }

    (action, outcome)
}

/// Convenience for callers: apply the `HomeOutcome` to the hub state.
pub fn apply_outcome(state: &mut HomeState, outcome: HomeOutcome) -> Option<IdentityHubTab> {
    match outcome {
        HomeOutcome::None => None,
        HomeOutcome::GoToActivity => Some(IdentityHubTab::Activity),
        HomeOutcome::GoToContacts => Some(IdentityHubTab::Contacts),
        HomeOutcome::DismissChecklist => {
            state.dismissed_checklist = true;
            None
        }
        HomeOutcome::SkipSocialProfile => {
            state.skipped_social_profile = true;
            None
        }
        HomeOutcome::ToggleAdvanced => {
            state.advanced_open = !state.advanced_open;
            None
        }
    }
}

/// Pick an identity to render in the hero. Returns the first loaded
/// qualified identity on the active network, or `None` if the load fails or
/// the user has no identities. The hub already handles the latter via
/// `HubLanding::Onboarding`, so this is only reached when at least one
/// identity exists.
fn first_loaded_identity(app_context: &Arc<AppContext>) -> Option<QualifiedIdentity> {
    app_context
        .load_local_qualified_identities()
        .ok()
        .and_then(|list| list.into_iter().next())
}

/// Build the [`IdentityHeroCard`] from a qualified identity. Keeps the
/// rendering code in `render` readable.
fn build_hero(app_context: &Arc<AppContext>, qi: &QualifiedIdentity) -> IdentityHeroCard {
    let kind: HeroIdentityKind = qi.identity_type.into();
    let balance_dash = format_credits_as_dash(qi.identity.balance());
    let handle = qi
        .dpns_names
        .first()
        .map(|n| n.name.clone())
        .filter(|n| !n.trim().is_empty());

    // Try to pick up the DashPay display name from the local cache. Silently
    // skip on any DB error — this is a best-effort read for the hero.
    let display_name = load_display_name_opt(app_context, qi);

    let mut card = IdentityHeroCard::new(kind, balance_dash);
    if let Some(handle) = handle {
        card = card.with_dpns_handle(handle);
    }
    if let Some(name) = display_name {
        card = card.with_display_name(name);
    }
    let network_label = network_label(app_context.network());
    card = card
        .with_network_label(network_label)
        .with_network_tooltip(format!(
            "You are on {network_label}. Identities and balances are separate per network.",
        ));
    card
}

fn load_display_name_opt(app_context: &Arc<AppContext>, qi: &QualifiedIdentity) -> Option<String> {
    let network = network_db_key(app_context.network());
    let stored = app_context
        .db
        .load_dashpay_profile(&qi.identity.id(), network)
        .ok()??;
    stored.display_name.filter(|n| !n.trim().is_empty())
}

/// DB convention for network keys. Mirrors the existing `dashpay_profiles`
/// table column usage (see `src/database/dashpay.rs`).
fn network_db_key(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
        _ => "mainnet",
    }
}

/// Alex-facing network label, stable across tabs.
fn network_label(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "Mainnet",
        Network::Testnet => "Testnet",
        Network::Devnet => "Devnet",
        Network::Regtest => "Regtest",
        _ => "Unknown",
    }
}

/// Format a credit balance (u64, Platform credits) as a DASH amount string
/// with four significant decimals. Mirrors the pattern used in the legacy
/// identities screen so the two hubs agree on the unit conversion.
fn format_credits_as_dash(credits: u64) -> String {
    // 1 DASH = 10^11 credits (DASH_DECIMAL_PLACES). See `model/amount.rs`.
    let dash = credits as f64 * 1e-11;
    format!("{dash:.4}")
}

/// Render the empty-state placeholder shown when no loaded identity can be
/// resolved. In practice the hub routes to `HubLanding::Onboarding` in that
/// situation — this path is defensive and only triggers on a mid-frame load
/// failure.
fn render_empty(ui: &mut Ui, dark_mode: bool) {
    ui.vertical_centered(|ui| {
        ui.add_space(Spacing::LG);
        ui.label(
            RichText::new("No identity selected.").color(DashColors::text_secondary(dark_mode)),
        );
    });
}

/// Build a primary (filled, Dash-blue) button used in the quick-actions row.
/// Returns the `Response` so the caller can attach click handling inline.
fn primary_quick_action(ui: &mut Ui, label: &str, tooltip: &str) -> egui::Response {
    let btn = egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
        .fill(DashColors::DASH_BLUE)
        .min_size(egui::vec2(140.0, 40.0));
    ui.add(btn).clickable_tooltip(tooltip)
}

/// Build a ghost (outlined) button used in the secondary-actions row.
fn ghost_action(ui: &mut Ui, label: &str, tooltip: &str, dark_mode: bool) -> egui::Response {
    let btn = egui::Button::new(RichText::new(label).color(DashColors::text_primary(dark_mode)))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(
            Shape::BORDER_WIDTH,
            DashColors::border(dark_mode),
        ))
        .min_size(egui::vec2(160.0, 36.0));
    ui.add(btn).clickable_tooltip(tooltip)
}

/// Render the inline social profile card shown below the hero when the user
/// has no display name. Returns `true` when the primary `Add a display name`
/// button is clicked.
fn paint_social_profile_card(ui: &mut Ui, dark_mode: bool) -> bool {
    let mut clicked = false;
    let frame = Frame::new()
        .fill(DashColors::surface(dark_mode))
        .stroke(Stroke::new(
            Shape::BORDER_WIDTH,
            DashColors::border_light(dark_mode),
        ))
        .corner_radius(CornerRadius::same(Shape::RADIUS_MD))
        .inner_margin(Margin::same(Spacing::MD as i8));
    frame.show(ui, |ui| {
        ui.label(
            RichText::new("Set up your social profile")
                .size(18.0)
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(Spacing::XS);
        ui.label(
            RichText::new(
                "Add a display name, bio, and avatar so people can find you on DashPay. \
                 This is optional — you can still use every other feature without it.",
            )
            .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(Spacing::SM);
        let btn = egui::Button::new(
            RichText::new("Add a display name")
                .strong()
                .color(Color32::WHITE),
        )
        .fill(DashColors::DASH_BLUE)
        .min_size(egui::vec2(200.0, 36.0));
        // Profile editing currently lives in the legacy Dashpay section.
        // Click is wired through the caller's `action` channel via the
        // return value; we don't route the user from inside the helper so
        // the caller retains full control of navigation.
        if ui
            .add(btn)
            .clickable_tooltip("Open the profile editor to pick a display name, bio, and avatar.")
            .clicked()
        {
            clicked = true;
        }
    });
    clicked
}

/// Expose the credit-formatter so unit tests (and future callers) can pin
/// the conversion. Kept crate-private.
#[cfg(test)]
fn format_credits_as_dash_for_tests(credits: u64) -> String {
    format_credits_as_dash(credits)
}

// Keep IdentityType in scope even when unused elsewhere so the `From` impl
// above is testable without a qualified path.
#[allow(dead_code)]
fn _assert_identity_type_is_in_scope(_t: IdentityType) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_credits_emits_four_decimals() {
        assert_eq!(format_credits_as_dash_for_tests(0), "0.0000");
        // 1.2345 DASH = 123_450_000_000 credits.
        assert_eq!(format_credits_as_dash_for_tests(123_450_000_000), "1.2345");
    }

    #[test]
    fn apply_outcome_none_returns_none() {
        let mut state = HomeState::default();
        assert_eq!(apply_outcome(&mut state, HomeOutcome::None), None);
        assert!(!state.dismissed_checklist);
    }

    #[test]
    fn apply_outcome_dismiss_checklist_sets_flag() {
        let mut state = HomeState::default();
        assert_eq!(
            apply_outcome(&mut state, HomeOutcome::DismissChecklist),
            None,
        );
        assert!(state.dismissed_checklist);
    }

    #[test]
    fn apply_outcome_toggle_advanced_flips_state() {
        let mut state = HomeState::default();
        apply_outcome(&mut state, HomeOutcome::ToggleAdvanced);
        assert!(state.advanced_open);
        apply_outcome(&mut state, HomeOutcome::ToggleAdvanced);
        assert!(!state.advanced_open);
    }

    #[test]
    fn apply_outcome_go_to_tabs_returns_tab() {
        let mut state = HomeState::default();
        assert_eq!(
            apply_outcome(&mut state, HomeOutcome::GoToActivity),
            Some(IdentityHubTab::Activity),
        );
        assert_eq!(
            apply_outcome(&mut state, HomeOutcome::GoToContacts),
            Some(IdentityHubTab::Contacts),
        );
    }

    #[test]
    fn network_label_returns_stable_strings() {
        assert_eq!(network_label(Network::Mainnet), "Mainnet");
        assert_eq!(network_label(Network::Testnet), "Testnet");
        assert_eq!(network_label(Network::Devnet), "Devnet");
        assert_eq!(network_label(Network::Regtest), "Regtest");
    }

    #[test]
    fn network_db_key_uses_lowercase_conventional_names() {
        assert_eq!(network_db_key(Network::Mainnet), "mainnet");
        assert_eq!(network_db_key(Network::Testnet), "testnet");
    }
}
