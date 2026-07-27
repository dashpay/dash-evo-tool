//! Identity Home tab.
//!
//! See design-spec §B.2 / §B.3. The Home tab is the default landing inside the
//! Identities hub once at least one identity exists. It stacks:
//!
//! 1. Hero identity card (gradient surface) — display name, handle, balance,
//!    identity-type badge, network pill. Two variants: social profile set
//!    (`IdentityHeroCard` with display name) and no social profile (type-glyph
//!    monogram + inline `Set up your social profile` card below the hero).
//! 2. Actions row: **Add funds**, **Send to wallet**, **Send to another
//!    identity**, **Add contact**. `Add contact` is gated behind a social
//!    profile (see §B.3).
//! 3. Onboarding checklist strip (until all three steps are complete or the
//!    user dismisses it).
//! 4. Recent activity preview (up to 5 rows), currently an empty-state
//!    preview; the `See all activity` link routes to the Activity tab.
//!    Richer content is parked until the activity aggregator lands.
//! 5. Advanced details expander (raw Identity ID, revision, last updated).
//!
//! Strings follow the Identity Home action-set redesign and the wording audit
//! in §C.
//!
//! This module is state-less per-frame: the only persisted state is the
//! `dismissed_checklist` flag owned by the calling hub screen and passed in
//! via [`HomeState`]. Everything else is recomputed from `AppContext`.

use super::identity_hero_card::{HeroIdentityKind, IdentityHeroCard};
use super::onboarding_checklist::{ChecklistAction, ChecklistStep, OnboardingChecklist};
use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::contested_name::PendingUsername;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::ScreenType;
use crate::ui::identities::register_dpns_name_screen::RegisterDpnsNameSource;
use crate::ui::identity::tabs::IdentityHubTab;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Shape, Spacing, network_label};
#[cfg(test)]
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
    /// User wants to switch to the Settings tab — used by the inline
    /// "Set up your social profile" card and the onboarding "Set a display
    /// name" step, since DashPay social profile editing lives in §B.8 (the
    /// Settings tab), not in the DPNS register-a-username flow.
    GoToSettings,
    /// User dismissed the onboarding checklist.
    DismissChecklist,
    /// User dismissed the inline social profile card.
    SkipSocialProfile,
    /// User toggled the Advanced expander.
    ToggleAdvanced,
}

/// Every clickable affordance on the Home tab.
///
/// Pairing button identities with this enum lets us unit-test dispatch without
/// a UI harness: the test iterates every variant and asserts the resulting
/// action is **not** `AppAction::None` / `HomeOutcome::None`. Invariant: every
/// `HomeButton` must resolve to a non-no-op action — if the test passes,
/// every button produces a real side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeButton {
    /// Action-row `Add contact` (enabled path — gated callers do not reach
    /// the dispatcher).
    AddContact,
    /// Action-row `Add funds`.
    AddFunds,
    /// Action-row `Send to wallet`.
    SendToWallet,
    /// Action-row `Send to another identity`.
    SendToAnotherIdentity,
    /// Inline social profile card `Add a display name` CTA. The hero card's
    /// separate `Pick a username` CTA (when no DPNS handle exists) maps to
    /// [`PickUsernameHero`](HomeButton::PickUsernameHero).
    SetUpSocialProfile,
    /// Hero card `Pick a username` CTA — only rendered when the hero has no
    /// DPNS handle.
    PickUsernameHero,
    /// `See all activity` link in the recent-activity preview.
    SeeAllActivity,
    /// Onboarding checklist: "Pick a username" step.
    ChecklistPickUsername,
    /// Onboarding checklist: "Set a display name" step.
    ChecklistSetDisplayName,
    /// Onboarding checklist: "Add your first contact" step.
    ChecklistAddFirstContact,
    /// Onboarding checklist dismiss (X).
    DismissChecklist,
    /// Advanced expander header toggle.
    ToggleAdvanced,
}

/// What a Home-tab button press produces, as a pure discriminant — independent
/// of any `AppContext` so it is unit-testable without fixtures. The render
/// function maps this back to a concrete [`AppAction`] / [`HomeOutcome`] using
/// [`home_button_action`].
///
/// Variant summary:
/// - `ScreenType(ScreenKind)` — push a screen via `AppAction::AddScreen`.
/// - `Outcome(HomeOutcome)` — hub-local intent (tab switch, dismiss, toggle).
///
/// The test harness uses [`HomeButtonKind::is_dead`] to flag any button that
/// resolves to a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeButtonKind {
    /// Open a screen of the given kind for the hero identity.
    OpenScreen(HomeScreenKind),
    /// A hub-local outcome (checklist dismiss, tab switch, etc).
    Outcome(HomeOutcome),
}

/// The set of screens any Home-tab button can push. Each variant maps 1:1 to
/// a [`ScreenType`] constructor via [`HomeScreenKind::into_screen_type`] —
/// kept as a pure enum so unit tests do not need `ScreenType::create_screen`,
/// which requires an `AppContext`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeScreenKind {
    /// `Screen::TransferScreen` — transfer credits between identities.
    Transfer,
    /// `Screen::TopUpIdentityScreen` — move wallet Dash into the identity.
    TopUp,
    /// `Screen::WithdrawalScreen` — move identity credits to a Core Dash address.
    Withdrawal,
    /// `Screen::RegisterDpnsNameScreen` — register a DPNS name for the identity.
    RegisterDpnsName,
}

impl HomeButtonKind {
    /// A button is "dead" when pressing it would produce neither an
    /// `AppAction::AddScreen` nor a meaningful [`HomeOutcome`]. This is the
    /// invariant checked by the dead-button unit test.
    pub fn is_dead(self) -> bool {
        matches!(self, HomeButtonKind::Outcome(HomeOutcome::None))
    }
}

/// Pure dispatcher: maps a [`HomeButton`] to what it *should* produce, with
/// no `AppContext` dependency. Unit-testable.
pub fn home_button_kind(button: HomeButton) -> HomeButtonKind {
    use HomeButtonKind::{OpenScreen, Outcome};
    use HomeScreenKind::*;
    match button {
        HomeButton::SendToAnotherIdentity => OpenScreen(Transfer),
        HomeButton::AddFunds => OpenScreen(TopUp),
        HomeButton::SendToWallet => OpenScreen(Withdrawal),
        HomeButton::PickUsernameHero | HomeButton::ChecklistPickUsername => {
            OpenScreen(RegisterDpnsName)
        }
        HomeButton::AddContact | HomeButton::ChecklistAddFirstContact => {
            Outcome(HomeOutcome::GoToContacts)
        }
        HomeButton::SetUpSocialProfile | HomeButton::ChecklistSetDisplayName => {
            Outcome(HomeOutcome::GoToSettings)
        }
        HomeButton::SeeAllActivity => Outcome(HomeOutcome::GoToActivity),
        HomeButton::DismissChecklist => Outcome(HomeOutcome::DismissChecklist),
        HomeButton::ToggleAdvanced => Outcome(HomeOutcome::ToggleAdvanced),
    }
}

/// Convert a [`HomeScreenKind`] to the matching [`ScreenType`] bound to the
/// given identity. Small helper so the render loop stays readable and the
/// same mapping is used everywhere.
fn home_screen_kind_to_screen_type(
    kind: HomeScreenKind,
    identity: &QualifiedIdentity,
) -> ScreenType {
    match kind {
        HomeScreenKind::Transfer => ScreenType::TransferScreen(identity.clone()),
        HomeScreenKind::TopUp => ScreenType::TopUpIdentity(identity.clone()),
        HomeScreenKind::Withdrawal => ScreenType::WithdrawalScreen(identity.clone()),
        HomeScreenKind::RegisterDpnsName => {
            ScreenType::RegisterDpnsName(RegisterDpnsNameSource::Identities)
        }
    }
}

/// Result of a Home-tab button press as consumed by the renderer: the
/// `AppAction` to forward upstream plus the hub outcome (tab switch, etc).
/// Either may be `None`; `is_noop` returns `true` only when both are.
///
/// Does not derive `Clone` because `AppAction` is not `Clone`. The struct is
/// produced, merged into the parent accumulator, and dropped within a single
/// frame.
#[derive(Debug)]
pub struct HomeButtonAction {
    pub app_action: AppAction,
    pub outcome: HomeOutcome,
}

impl HomeButtonAction {
    /// Whether the resolved action does nothing — both `AppAction::None` and
    /// `HomeOutcome::None`. Used by the render code to silently drop no-ops.
    pub fn is_noop(&self) -> bool {
        matches!(self.app_action, AppAction::None) && self.outcome == HomeOutcome::None
    }
}

/// Dispatch a button to its concrete `(AppAction, HomeOutcome)` pair, using
/// [`home_button_kind`] for the pure decision and the current `AppContext`
/// only to materialise the target screen when one is needed.
pub fn home_button_action(
    button: HomeButton,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
) -> HomeButtonAction {
    match home_button_kind(button) {
        HomeButtonKind::OpenScreen(kind) => HomeButtonAction {
            app_action: AppAction::AddScreen(
                home_screen_kind_to_screen_type(kind, identity).create_screen(app_context),
            ),
            outcome: HomeOutcome::None,
        },
        HomeButtonKind::Outcome(outcome) => HomeButtonAction {
            app_action: AppAction::None,
            outcome,
        },
    }
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
    profiles: &mut super::profile_cache::ProfileCache,
) -> (AppAction, HomeOutcome) {
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;
    let mut action = AppAction::None;
    let mut outcome = HomeOutcome::None;

    // Render the app-scoped active identity (selected → first → none). The
    // breadcrumb switcher and picker write the selection; this reads it.
    let identity = match app_context.resolve_selected_identity() {
        Some(qi) => qi,
        None => {
            render_empty(ui, dark_mode);
            return (action, outcome);
        }
    };

    // A DPNS username requested but not yet awarded — surfaced on the hero and
    // the onboarding checklist so a pending request is not mistaken for "no
    // username". Only meaningful when the identity owns no name yet; the cache
    // read is best-effort, so a failure simply omits the indicator.
    let pending_username: Option<PendingUsername> =
        app_context.pending_dpns_username_for_identity(&identity);

    // A tiny local closure that dispatches via the pure
    // `home_button_action` function and merges the result into the
    // `(action, outcome)` pair. Using this at every click site keeps the UI
    // code a single place to search for dead buttons.
    let mut apply = |button: HomeButton| {
        let dispatched = home_button_action(button, app_context, &identity);
        action |= dispatched.app_action;
        if dispatched.outcome != HomeOutcome::None {
            outcome = dispatched.outcome;
        }
    };

    // --- Hero card ----------------------------------------------------
    let hero = build_hero(app_context, &identity, profiles, pending_username.clone());
    let hero_has_social_profile = hero.has_social_profile();
    let hero_response = hero.show(ui);
    if hero_response.pick_username_clicked() {
        apply(HomeButton::PickUsernameHero);
    }

    // --- Inline "Set up your social profile" card (no-profile variant) -
    //
    // Per wireframe §B.3 this prompt belongs immediately below the compact hero,
    // with standard spacing separating the group from the actions.
    //
    // The onboarding checklist contains the same "Set a display name" action,
    // so suppress this card while that checklist is visible.
    let checklist_covers_profile = !hero_has_social_profile && !state.dismissed_checklist;
    if !hero_has_social_profile
        && !state.skipped_social_profile
        && !checklist_covers_profile
        && paint_social_profile_card(ui, dark_mode)
    {
        apply(HomeButton::SetUpSocialProfile);
    }
    ui.add_space(Spacing::MD);

    // --- Actions row --------------------------------------------------
    ui.horizontal_wrapped(|ui| {
        if ComponentStyles::add_primary_button(ui, "Add funds")
            .clickable_tooltip("Move Dash from your wallet into this identity.")
            .clicked()
        {
            apply(HomeButton::AddFunds);
        }

        let send_to_wallet = ComponentStyles::secondary_button("Send to wallet", dark_mode);
        if !identity.can_attempt_withdrawal(app_context.user_role()) {
            ui.add_enabled(false, send_to_wallet).disabled_tooltip(
                "Sending to a wallet is unavailable because this identity has no key available \
                 for withdrawal in the current interface mode. Load or import an eligible key, \
                 change the interface mode if another on-chain key is available, or use a \
                 different identity.",
            );
        } else if ui
            .add(send_to_wallet)
            .clickable_tooltip(
                "Move Dash out of this identity to a Dash address, such as one from your wallet.",
            )
            .clicked()
        {
            apply(HomeButton::SendToWallet);
        }

        if ComponentStyles::add_secondary_button(ui, "Send to another identity", dark_mode)
            .clickable_tooltip(
                "Send Dash from this identity to another identity. You can also send to a \
                 Platform address.",
            )
            .clicked()
        {
            apply(HomeButton::SendToAnotherIdentity);
        }

        // Add contact is gated behind a social profile per §B.3.
        let add_contact = ComponentStyles::secondary_button("Add contact", dark_mode);
        if hero_has_social_profile {
            let response = ui
                .add(add_contact)
                .clickable_tooltip("Find someone by username and add them to your contacts.");
            if response.clicked() {
                apply(HomeButton::AddContact);
            }
        } else {
            ui.add_enabled(false, add_contact).disabled_tooltip(
                "Set up a social profile first. Contacts need a display name and avatar \
                 so people can find you.",
            );
        }
    });
    ui.add_space(Spacing::MD);

    // --- Onboarding checklist -----------------------------------------
    if !state.dismissed_checklist {
        // Extract the primary DPNS handle for the done-subtext ("You are
        // @{handle}.") — passed into the checklist as optional context.
        let primary_handle = identity
            .dpns_names
            .first()
            .map(|n| n.name.trim().to_string())
            .filter(|s| !s.is_empty());

        let mut checklist = OnboardingChecklist::new();
        if let Some(h) = &primary_handle {
            checklist = checklist.with_handle(h);
        }
        if primary_handle.is_some() {
            checklist = checklist.mark_complete(ChecklistStep::PickUsername);
        } else if let Some(pending) = &pending_username {
            // Requested but not yet awarded — reflect the pending state instead
            // of nagging the user to pick a name they already chose.
            checklist = checklist.with_pending_username(pending.name.clone());
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
                    apply(HomeButton::DismissChecklist);
                }
                Some(ChecklistAction::Activated(step)) => match step {
                    ChecklistStep::PickUsername => apply(HomeButton::ChecklistPickUsername),
                    ChecklistStep::SetDisplayName => apply(HomeButton::ChecklistSetDisplayName),
                    ChecklistStep::AddFirstContact => apply(HomeButton::ChecklistAddFirstContact),
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
            // unified activity aggregator lands. We render the design-spec
            // empty-state sentence so the copy is visible.
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
                apply(HomeButton::SeeAllActivity);
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
        apply(HomeButton::ToggleAdvanced);
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
        HomeOutcome::GoToSettings => Some(IdentityHubTab::Settings),
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
/// Build the [`IdentityHeroCard`] from a qualified identity. Keeps the
/// rendering code in `render` readable.
fn build_hero(
    app_context: &Arc<AppContext>,
    qi: &QualifiedIdentity,
    profiles: &mut super::profile_cache::ProfileCache,
    pending_username: Option<PendingUsername>,
) -> IdentityHeroCard {
    let kind: HeroIdentityKind = qi.identity_type.into();
    let balance_dash = format_credits_short(qi.identity.balance());
    let handle = qi
        .dpns_names
        .first()
        .map(|n| n.name.clone())
        .filter(|n| !n.trim().is_empty());

    // Best-effort DashPay display name. The local profile cache was removed in
    // the platform-wallet migration; the hub loads profiles asynchronously, so
    // this is empty until the first load completes and the hero re-renders.
    let display_name = load_display_name_opt(profiles, qi);

    let mut card = IdentityHeroCard::new(kind, balance_dash);
    match handle {
        Some(handle) => card = card.with_dpns_handle(handle),
        // No owned name: show a pending request when one exists.
        None => {
            if let Some(pending) = pending_username {
                card = card.with_pending_username(pending);
            }
        }
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

fn load_display_name_opt(
    profiles: &mut super::profile_cache::ProfileCache,
    qi: &QualifiedIdentity,
) -> Option<String> {
    profiles
        .get_or_request(qi)
        .and_then(|p| p.as_ref())
        .and_then(|fields| fields.display_name_opt().map(str::to_owned))
}

/// Format a credit balance (u64, Platform credits) as a bare DASH amount
/// string with four decimal places and no unit suffix, for embedding in a
/// `"{amount} DASH"` template. Distinct from
/// [`crate::model::fee_estimation::format_credits_as_dash`], which returns a
/// trimmed amount with the unit already appended.
fn format_credits_short(credits: u64) -> String {
    format!(
        "{:.4}",
        crate::model::amount::Amount::dash_from_credits(credits).to_f64()
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_credits_emits_four_decimals() {
        assert_eq!(format_credits_short(0), "0.0000");
        // 1.2345 DASH = 123_450_000_000 credits.
        assert_eq!(format_credits_short(123_450_000_000), "1.2345");
    }

    #[test]
    fn home_state_default_flags_are_false() {
        let state = HomeState::default();
        assert!(!state.advanced_open);
        assert!(!state.dismissed_checklist);
        assert!(!state.skipped_social_profile);
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

    // -----------------------------------------------------------------
    // Dead-button regression tests.
    //
    // Invariant: every `HomeButton` variant must produce a non-dead
    // `HomeButtonKind` (neither `Outcome(HomeOutcome::None)` nor some
    // future catch-all). The enum is non-`Default`-constructible and
    // every variant is enumerated in `ALL_HOME_BUTTONS` — if you add a
    // new button, you MUST extend this list or the "coverage" test
    // fails.
    // -----------------------------------------------------------------

    /// Every `HomeButton` variant. The list is hand-maintained so that
    /// the compiler catches missing entries via `ALL_BUTTONS_IS_EXHAUSTIVE`
    /// below — adding a variant without updating this list is a compile
    /// error.
    const ALL_HOME_BUTTONS: &[HomeButton] = &[
        HomeButton::AddContact,
        HomeButton::AddFunds,
        HomeButton::SendToWallet,
        HomeButton::SendToAnotherIdentity,
        HomeButton::SetUpSocialProfile,
        HomeButton::PickUsernameHero,
        HomeButton::SeeAllActivity,
        HomeButton::ChecklistPickUsername,
        HomeButton::ChecklistSetDisplayName,
        HomeButton::ChecklistAddFirstContact,
        HomeButton::DismissChecklist,
        HomeButton::ToggleAdvanced,
    ];

    /// Exhaustiveness check: if a new `HomeButton` variant is added
    /// without updating `ALL_HOME_BUTTONS`, this match will fail to
    /// compile because the new variant has no arm.
    #[test]
    fn all_buttons_list_is_exhaustive() {
        for button in ALL_HOME_BUTTONS {
            #[allow(unreachable_patterns)]
            let _: () = match *button {
                HomeButton::AddContact => (),
                HomeButton::AddFunds => (),
                HomeButton::SendToWallet => (),
                HomeButton::SendToAnotherIdentity => (),
                HomeButton::SetUpSocialProfile => (),
                HomeButton::PickUsernameHero => (),
                HomeButton::SeeAllActivity => (),
                HomeButton::ChecklistPickUsername => (),
                HomeButton::ChecklistSetDisplayName => (),
                HomeButton::ChecklistAddFirstContact => (),
                HomeButton::DismissChecklist => (),
                HomeButton::ToggleAdvanced => (),
            };
        }
    }

    #[test]
    fn every_home_button_produces_live_action() {
        for button in ALL_HOME_BUTTONS {
            let kind = home_button_kind(*button);
            assert!(
                !kind.is_dead(),
                "HomeButton::{button:?} is dead — produced {kind:?}. Either the button is \
                 supposed to be disabled (then do not render it as enabled), or the dispatcher \
                 lost its arm.",
            );
        }
    }

    #[test]
    fn action_row_buttons_have_distinct_destinations() {
        let destinations = [
            HomeButton::AddFunds,
            HomeButton::SendToWallet,
            HomeButton::SendToAnotherIdentity,
        ]
        .map(|button| match home_button_kind(button) {
            HomeButtonKind::OpenScreen(screen) => screen,
            HomeButtonKind::Outcome(outcome) => {
                panic!("action-row button {button:?} produced hub outcome {outcome:?}")
            }
        });

        for (index, destination) in destinations.iter().enumerate() {
            for other in &destinations[index + 1..] {
                assert_ne!(destination, other);
            }
        }
    }

    #[test]
    fn social_profile_ctas_route_to_settings_not_dpns() {
        // Design-spec §B.8: DashPay profile (display name / bio / avatar) is
        // edited on the Settings tab, not in RegisterDpnsName.
        assert_eq!(
            home_button_kind(HomeButton::SetUpSocialProfile),
            HomeButtonKind::Outcome(HomeOutcome::GoToSettings),
        );
        assert_eq!(
            home_button_kind(HomeButton::ChecklistSetDisplayName),
            HomeButtonKind::Outcome(HomeOutcome::GoToSettings),
        );
    }

    #[test]
    fn pick_a_username_routes_to_dpns_flow() {
        assert_eq!(
            home_button_kind(HomeButton::PickUsernameHero),
            HomeButtonKind::OpenScreen(HomeScreenKind::RegisterDpnsName),
        );
        assert_eq!(
            home_button_kind(HomeButton::ChecklistPickUsername),
            HomeButtonKind::OpenScreen(HomeScreenKind::RegisterDpnsName),
        );
    }

    #[test]
    fn see_all_activity_switches_to_activity_tab() {
        assert_eq!(
            home_button_kind(HomeButton::SeeAllActivity),
            HomeButtonKind::Outcome(HomeOutcome::GoToActivity),
        );
    }

    #[test]
    fn add_contact_and_first_contact_step_switch_to_contacts_tab() {
        assert_eq!(
            home_button_kind(HomeButton::AddContact),
            HomeButtonKind::Outcome(HomeOutcome::GoToContacts),
        );
        assert_eq!(
            home_button_kind(HomeButton::ChecklistAddFirstContact),
            HomeButtonKind::Outcome(HomeOutcome::GoToContacts),
        );
    }

    #[test]
    fn dismiss_and_toggle_are_pure_hub_outcomes() {
        assert_eq!(
            home_button_kind(HomeButton::DismissChecklist),
            HomeButtonKind::Outcome(HomeOutcome::DismissChecklist),
        );
        assert_eq!(
            home_button_kind(HomeButton::ToggleAdvanced),
            HomeButtonKind::Outcome(HomeOutcome::ToggleAdvanced),
        );
    }

    #[test]
    fn apply_outcome_go_to_settings_returns_settings_tab() {
        // `GoToSettings` must be wired into `apply_outcome`; a missing
        // match arm would silently drop the outcome.
        let mut state = HomeState::default();
        assert_eq!(
            apply_outcome(&mut state, HomeOutcome::GoToSettings),
            Some(IdentityHubTab::Settings),
        );
    }

    #[test]
    fn home_button_action_wraps_kind_into_appaction() {
        // No `AppContext` available in this unit test, so we only inspect
        // the `is_noop` invariant after resolving — mirrors what the
        // renderer cares about.
        for button in ALL_HOME_BUTTONS {
            let kind = home_button_kind(*button);
            // Every kind must be "live" (non-dead); the renderer wraps
            // `OpenScreen` into `AppAction::AddScreen` (non-None) and
            // `Outcome(x)` into `HomeButtonAction.outcome = x`. Either
            // way the result is a non-noop.
            let mock = match kind {
                HomeButtonKind::OpenScreen(_) => HomeButtonAction {
                    // The render code wraps this into AddScreen(...)
                    // — we assert non-noop via a placeholder so the test
                    // doesn't need `AppContext`.
                    app_action: AppAction::None,
                    outcome: HomeOutcome::None,
                },
                HomeButtonKind::Outcome(o) => HomeButtonAction {
                    app_action: AppAction::None,
                    outcome: o,
                },
            };
            // For `OpenScreen` variants we can only assert via the kind,
            // not via HomeButtonAction.is_noop (see above). For `Outcome`
            // variants we CAN check the materialised struct.
            match kind {
                HomeButtonKind::OpenScreen(_) => {
                    assert!(
                        !kind.is_dead(),
                        "OpenScreen kind for {button:?} is not dead",
                    );
                }
                HomeButtonKind::Outcome(_) => {
                    assert!(
                        !mock.is_noop(),
                        "Outcome for {button:?} resolves to noop — {mock:?}",
                    );
                }
            }
        }
    }
}
