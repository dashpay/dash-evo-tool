//! Activity tab — shell.
//!
//! The unified activity timeline (payments + funding + platform ops) depends
//! on a backend aggregator that does not exist yet. T10 ships the shell only:
//!
//! - a filter-chip row (All / Payments / Funding / Platform) so the tab has
//!   the visual shape called out in the design spec §B.6 and in Frame F6 of
//!   `wireframe.html`,
//! - a gated empty state pointing users to the legacy DashPay Payments screen
//!   when the `identity-hub-activity-feed` Cargo feature is off (default),
//! - a feature-gated placeholder that says `Unified activity feed coming soon`
//!   when the feature is on — the aggregator backend is deliberately out of
//!   scope for T10 (additive-only; no new backend task).
//!
//! Retry plumbing for failed rows is wired through the reusable
//! [`ActivityRow`](crate::ui::identity::activity_row::ActivityRow)
//! component — the caller decides what a retry means. Because T10 renders no
//! live rows, there is nothing to retry today; when the aggregator lands it
//! will feed real rows into the same component and surface
//! [`ActivityRowAction::Retry`](crate::ui::identity::activity_row::ActivityRowAction::Retry)
//! through the unified [`AppAction`] channel.
//!
//! See `docs/ai-design/2026-04-23-identity-hub-impl/04-dev-plan.md` T10 and
//! the test specs UT-ACTIVITY-ROW-01 / IT-ACTIVITY-01.

use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::RootScreenType;
use crate::ui::theme::{DashColors, ResponseExt};
use eframe::egui::{self, RichText, Sense, Ui};
use std::sync::Arc;

/// Filter categories for the unified activity timeline.
///
/// A standalone type so the filter state can be lifted into the calling screen
/// without depending on egui internals, and so the shell's unit tests can
/// exhaustively cover enum rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityFilter {
    /// Show every kind of activity (default).
    All,
    /// Payments sent or received via DashPay.
    Payments,
    /// Funding operations: add funds, send to wallet, send to another identity.
    Funding,
    /// Platform operations: DPNS, keys, contracts.
    Platform,
}

impl ActivityFilter {
    /// The label shown on the filter chip.
    ///
    /// Labels are i18n-ready standalone sentences-as-labels — no punctuation,
    /// no positional assumptions. Matches the strings called out in the
    /// design-spec tooltip table and in `wireframe.html` Frame F6.
    pub fn label(self) -> &'static str {
        match self {
            ActivityFilter::All => "All",
            ActivityFilter::Payments => "Payments",
            ActivityFilter::Funding => "Funding",
            ActivityFilter::Platform => "Platform",
        }
    }
}

/// Render the Activity tab shell.
///
/// The selection state for the filter chips lives in egui `data` storage so
/// the shell remains a pure function for T10. A proper component upgrade is
/// planned when the aggregator lands.
pub fn render(ui: &mut Ui, _app_context: &Arc<AppContext>) -> AppAction {
    let dark_mode = ui.ctx().style().visuals.dark_mode;
    let mut action = AppAction::None;

    ui.vertical_centered(|ui| {
        ui.add_space(12.0);

        // Filter chips — multi-select with an `All` reset. `All` is selected
        // by default when no other chip is active.
        let id = ui.make_persistent_id("identity_hub_activity_filters");
        let mut filters: FilterSet = ui
            .ctx()
            .data(|d| d.get_temp::<FilterSet>(id).unwrap_or_default());

        ui.horizontal(|ui| {
            for filter in [
                ActivityFilter::All,
                ActivityFilter::Payments,
                ActivityFilter::Funding,
                ActivityFilter::Platform,
            ] {
                let selected = filters.is_selected(filter);
                if ui.selectable_label(selected, filter.label()).clicked() {
                    filters.toggle(filter);
                }
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(id, filters));

        ui.add_space(16.0);

        #[cfg(feature = "identity-hub-activity-feed")]
        {
            // Feature-gated placeholder. No live aggregator yet — T10 is
            // additive-only and does not introduce a new backend task.
            ui.label(
                RichText::new("Unified activity feed coming soon.")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.add_space(8.0);
            action |= legacy_payments_link(ui);
        }

        #[cfg(not(feature = "identity-hub-activity-feed"))]
        {
            // Default (feature off): gated empty state. Point users to the
            // legacy DashPay Payments screen so they can still see their
            // activity while the aggregator is built.
            ui.label(
                RichText::new("Unified activity is coming soon.")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new("For now, view activity on the existing DashPay Payments screen:")
                    .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(4.0);
            action |= legacy_payments_link(ui);
        }
    });

    action
}

/// Render an underlined, Dash-blue link to the legacy DashPay Payments screen.
/// Emits `AppAction::SetMainScreen(RootScreenDashPayPayments)` when clicked so
/// the user can see their movements today while the unified aggregator is
/// built. No new backend task and no new screen are introduced.
fn legacy_payments_link(ui: &mut Ui) -> AppAction {
    let resp = ui
        .add(
            egui::Label::new(
                RichText::new("Open DashPay Payments")
                    .underline()
                    .color(DashColors::DASH_BLUE),
            )
            .sense(Sense::click()),
        )
        .clickable_tooltip("Open the legacy DashPay Payments screen in a new view.");
    if resp.clicked() {
        return resolve_activity_button(ActivityButton::LegacyPaymentsLink);
    }
    AppAction::None
}

/// Every clickable affordance on the Activity tab. Today there is only one —
/// the legacy-payments link — but keeping an enum here mirrors the Home /
/// Contacts dispatcher pattern so reviewers see a single shape. When the
/// aggregator lands and activity rows gain retry affordances, the rows'
/// `ActivityRowAction::Retry` will plug into this enum as
/// `ActivityButton::Retry(ActivityRowId)` without changing the UI shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityButton {
    /// `Open DashPay Payments` link, rendered in both the empty state
    /// (feature off) and the feed placeholder (feature on).
    LegacyPaymentsLink,
}

/// What an [`ActivityButton`] click produces, as a pure enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityButtonKind {
    /// Swap the root screen to `RootScreenDashPayPayments`.
    SetRootScreen(RootScreenType),
}

/// Pure dispatcher — unit-testable, no `AppContext` needed.
pub fn activity_button_kind(button: ActivityButton) -> ActivityButtonKind {
    match button {
        ActivityButton::LegacyPaymentsLink => {
            ActivityButtonKind::SetRootScreen(RootScreenType::RootScreenDashPayPayments)
        }
    }
}

/// Materialise an [`ActivityButton`] into a concrete [`AppAction`].
fn resolve_activity_button(button: ActivityButton) -> AppAction {
    match activity_button_kind(button) {
        ActivityButtonKind::SetRootScreen(root) => AppAction::SetMainScreen(root),
    }
}

/// Bitset-style filter state for the chip row.
///
/// A tiny `Copy` value so the shell can stash it in egui's `data` storage
/// without heap allocations. Exactly one of the three category flags is set
/// at any time in the default configuration; toggling `All` clears the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct FilterSet {
    payments: bool,
    funding: bool,
    platform: bool,
}

impl FilterSet {
    fn any_selected(self) -> bool {
        self.payments || self.funding || self.platform
    }

    fn is_selected(self, filter: ActivityFilter) -> bool {
        match filter {
            ActivityFilter::All => !self.any_selected(),
            ActivityFilter::Payments => self.payments,
            ActivityFilter::Funding => self.funding,
            ActivityFilter::Platform => self.platform,
        }
    }

    fn toggle(&mut self, filter: ActivityFilter) {
        match filter {
            ActivityFilter::All => {
                *self = FilterSet::default();
            }
            ActivityFilter::Payments => self.payments = !self.payments,
            ActivityFilter::Funding => self.funding = !self.funding,
            ActivityFilter::Platform => self.platform = !self.platform,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter_set_selects_all() {
        let set = FilterSet::default();
        assert!(set.is_selected(ActivityFilter::All));
        assert!(!set.is_selected(ActivityFilter::Payments));
        assert!(!set.is_selected(ActivityFilter::Funding));
        assert!(!set.is_selected(ActivityFilter::Platform));
    }

    #[test]
    fn toggling_payments_deselects_all() {
        let mut set = FilterSet::default();
        set.toggle(ActivityFilter::Payments);
        assert!(set.is_selected(ActivityFilter::Payments));
        assert!(!set.is_selected(ActivityFilter::All));
    }

    #[test]
    fn toggling_all_clears_every_selection() {
        let mut set = FilterSet::default();
        set.toggle(ActivityFilter::Payments);
        set.toggle(ActivityFilter::Funding);
        set.toggle(ActivityFilter::All);
        assert!(set.is_selected(ActivityFilter::All));
        assert!(!set.is_selected(ActivityFilter::Payments));
        assert!(!set.is_selected(ActivityFilter::Funding));
    }

    #[test]
    fn filter_labels_are_i18n_ready_single_words() {
        assert_eq!(ActivityFilter::All.label(), "All");
        assert_eq!(ActivityFilter::Payments.label(), "Payments");
        assert_eq!(ActivityFilter::Funding.label(), "Funding");
        assert_eq!(ActivityFilter::Platform.label(), "Platform");
    }

    // ---------------------------------------------------------------
    // Dead-button regression test.
    // ---------------------------------------------------------------

    const ALL_ACTIVITY_BUTTONS: &[ActivityButton] = &[ActivityButton::LegacyPaymentsLink];

    #[test]
    fn activity_all_buttons_list_is_exhaustive() {
        for button in ALL_ACTIVITY_BUTTONS {
            let _: () = match *button {
                ActivityButton::LegacyPaymentsLink => (),
            };
        }
    }

    #[test]
    fn legacy_payments_link_routes_to_dashpay_payments_screen() {
        assert_eq!(
            activity_button_kind(ActivityButton::LegacyPaymentsLink),
            ActivityButtonKind::SetRootScreen(RootScreenType::RootScreenDashPayPayments),
        );
    }
}
