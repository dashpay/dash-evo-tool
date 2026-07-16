//! Identity picker grid — rendered when the hub detects ≥ 2 identities on the
//! active network. See design-spec
//! `docs/ai-design/2026-04-22-identity-dashpay-redesign/design-spec.md` §B.14.
//!
//! Layout: a responsive grid of [`IdentityPickerCard`]s followed by an
//! [`IdentityPickerAddCard`]. Cards flow left-to-right, wrapping based on the
//! available panel width. Clicking an identity card is reported to the caller
//! so the hub can route to Identity Home. Clicking the add card routes to the
//! **existing** `AddNewIdentityScreen` via `AppAction::AddScreen` — no new
//! navigation surface is introduced.
//!
//! This module is the UI shell only. No backend tasks are dispatched here —
//! identity lookup is handled upstream by `IdentityHubScreen::landing()`.

use super::identity_picker_add_card::IdentityPickerAddCard;
use super::identity_picker_card::{CARD_MIN_WIDTH, IdentityPickerCard};
use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::Screen;
use crate::ui::identities::add_new_identity_screen::AddNewIdentityScreen;
use crate::ui::theme::DashColors;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::{RichText, Ui};
use std::sync::Arc;

/// Minimum horizontal gap between cards in the grid.
const GRID_GAP: f32 = 16.0;

/// Default fallback balance label when the identity has no known credits — a
/// complete sentence fragment, never empty.
const EMPTY_BALANCE_LABEL: &str = "No balance";

/// Rendered the picker grid. Returns the `AppAction` the caller must propagate:
///
/// * `AppAction::None` on hover / no interaction.
/// * `AppAction::AddScreen(Screen::AddNewIdentityScreen(...))` when the
///   "Add a new identity" card is clicked.
/// * For identity-card clicks the action is `AppAction::None` by default —
///   identity selection is deferred to a follow-up task (Home-tab routing
///   lands in T8); the visible banner is left to the hub screen.
///
/// `selected_id_out`, if `Some`, is populated with the Base58 identity id of
/// the clicked card so a composing `IdentityHubScreen` can remember the
/// selection and flip its landing to Home.
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identities: &[QualifiedIdentity],
    selected_id_out: Option<&mut Option<String>>,
) -> AppAction {
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;
    let mut action = AppAction::None;
    let mut captured_selection: Option<String> = None;

    // Heading + sub-heading — verbatim from design-spec §B.14.
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        ui.label(
            RichText::new("Pick an identity")
                .size(24.0)
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Each identity has its own balance, keys, and optional social profile. \
                 Choose one to open it, or add a new identity.",
            )
            .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(20.0);
    });

    // Grid layout: horizontal flow with manual wrapping.
    //
    // We do not use `egui::Grid` because it requires a fixed column count;
    // the design-spec `repeat(auto-fill, minmax(260px, 1fr))` rule needs the
    // column count to depend on the available width.
    let available_width = ui.available_width();
    let columns = compute_column_count(available_width);

    ui.vertical(|ui| {
        let cells: Vec<PickerCell<'_>> = identities
            .iter()
            .map(PickerCell::Identity)
            .chain(std::iter::once(PickerCell::Add))
            .collect();

        for row in cells.chunks(columns.max(1)) {
            ui.horizontal(|ui| {
                for (i, cell) in row.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(GRID_GAP);
                    }
                    match cell {
                        PickerCell::Identity(identity) => {
                            let card = build_card(identity);
                            let response = card.show(ui);
                            if response.clicked {
                                captured_selection = Some(response.identity_id.clone());
                            }
                        }
                        PickerCell::Add => {
                            let card = IdentityPickerAddCard::new();
                            let response = card.show(ui);
                            if response.add_requested {
                                // Navigate to the existing AddNewIdentityScreen —
                                // no duplicated screen, no new backend task.
                                action = AppAction::AddScreen(Screen::AddNewIdentityScreen(
                                    AddNewIdentityScreen::new(app_context),
                                ));
                            }
                        }
                    }
                }
            });
            ui.add_space(GRID_GAP);
        }
    });

    // Propagate captured selection to the caller, if they supplied a slot.
    // Only update when an identity was actually clicked — never overwrite with
    // `None` on frames where no click occurred (T17).
    if let Some(slot) = selected_id_out
        && let Some(sel) = captured_selection
    {
        *slot = Some(sel);
    }

    action
}

/// Compute how many cards fit per row at the given available width. Matches
/// the design-spec `minmax(260px, 1fr)` rule — at least 260 px per column,
/// stretching to fill the remaining space when extra room exists.
pub fn compute_column_count(available_width: f32) -> usize {
    let min_width = CARD_MIN_WIDTH + GRID_GAP;
    let columns = (available_width / min_width).floor() as usize;
    columns.max(1)
}

enum PickerCell<'a> {
    Identity(&'a QualifiedIdentity),
    Add,
}

/// Build a [`IdentityPickerCard`] from a [`QualifiedIdentity`].
///
/// Display-name is not yet a first-class field on `QualifiedIdentity` (it
/// comes from the DashPay social profile, loaded asynchronously). Until that
/// integration lands we use the local nickname (`alias`) as a stand-in for
/// display-name so users who have labelled their identities still see a
/// familiar heading. When a real social-profile display name becomes
/// available upstream, wiring it into this function is a one-line change.
fn build_card(identity: &QualifiedIdentity) -> IdentityPickerCard {
    let id_base58 = identity.identity.id().to_string(Encoding::Base58);

    // Balance formatting: design-spec §B.14 uses `{amount} DASH` with tabular
    // numerals. We keep the formatting simple here so the picker does not
    // duplicate fee-estimation logic; identities with a zero balance still
    // render a readable label ("No balance") rather than the misleading
    // "0 DASH".
    let balance_credits = identity.identity.balance();
    let balance_label = if balance_credits == 0 {
        EMPTY_BALANCE_LABEL.to_string()
    } else {
        let dash = balance_credits as f64 * 1e-11;
        format!("{dash:.6} DASH")
    };

    let mut card =
        IdentityPickerCard::new(id_base58.clone(), identity.identity_type, balance_label)
            .with_tooltip(
                "Open this identity. You can switch between identities anytime from the \
             breadcrumb.",
            );

    // Heading priority: local alias → DPNS → shortened id.
    if let Some(alias) = identity.alias.as_deref().filter(|s| !s.trim().is_empty()) {
        card = card.with_display_name(alias);
    }
    if let Some(dpns) = identity.dpns_names.first().map(|n| n.name.as_str())
        && !dpns.is_empty()
    {
        card = card.with_dpns_handle(dpns);
    }

    card
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_count_narrow_panel_is_one() {
        // 260 px viewport — exactly one column worth of space.
        assert_eq!(compute_column_count(260.0), 1);
    }

    #[test]
    fn column_count_standard_panel_is_three() {
        // A reasonably wide panel fits three 260 + 16 = 276 px columns.
        assert_eq!(compute_column_count(900.0), 3);
    }

    #[test]
    fn column_count_zero_or_negative_returns_one() {
        // Defensive: never report zero columns — we'd divide by it elsewhere.
        assert_eq!(compute_column_count(0.0), 1);
        assert_eq!(compute_column_count(-10.0), 1);
    }

    #[test]
    fn column_count_grows_with_width() {
        // Monotonic: wider panel can never fit fewer columns.
        let a = compute_column_count(600.0);
        let b = compute_column_count(1200.0);
        assert!(b >= a);
    }
}
