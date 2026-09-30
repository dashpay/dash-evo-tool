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
use crate::ui::identity::add_new_identity_screen::AddNewIdentityScreen;
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
    profiles: &mut super::profile_cache::ProfileCache,
    avatars: &mut crate::ui::state::AvatarCache,
    pending_avatars: &mut Vec<crate::backend_task::BackendTask>,
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

    egui::ScrollArea::vertical()
        .id_salt(("identity_picker", app_context.selected_wallet_hash()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let available_width = ui.available_width();
            let columns = compute_column_count(available_width);
            let card_width = (available_width - GRID_GAP * (columns - 1) as f32) / columns as f32;
            if identities.is_empty() && app_context.selected_wallet_hash().is_some() {
                ui.label("This wallet has no identities yet. Add an identity to get started.");
                ui.add_space(GRID_GAP);
            }
            let cells: Vec<PickerCell<'_>> = identities
                .iter()
                .map(PickerCell::Identity)
                .chain(std::iter::once(PickerCell::Add))
                .collect();

            for row in cells.chunks(columns.max(1)) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GRID_GAP;
                    for cell in row {
                        match cell {
                            PickerCell::Identity(identity) => {
                                let mut card =
                                    build_card(app_context, identity).with_width(card_width);
                                if let Some(Some(profile)) = profiles.get_or_request(identity) {
                                    if let Some(name) = app_context.identity_display_name_or(
                                        identity.identity.id(),
                                        profile.display_name_opt(),
                                    ) {
                                        card = card.with_display_name(name);
                                    }
                                    card = card.with_avatar_url(&profile.avatar_url);
                                }
                                let response = card.show(ui, avatars);
                                if let Some(fetch) = response.avatar_fetch {
                                    pending_avatars.push(fetch);
                                }
                                if response.clicked {
                                    captured_selection = Some(response.identity_id.clone());
                                }
                            }
                            PickerCell::Add => {
                                let card = IdentityPickerAddCard::new().with_width(card_width);
                                let response = card.show(ui);
                                if response.add_requested {
                                    // Navigate to the existing AddNewIdentityScreen —
                                    // no duplicated screen, no new backend task.
                                    action = AppAction::AddScreen(Screen::AddNewIdentityScreen(
                                        AddNewIdentityScreen::new_with_wallet(
                                            app_context,
                                            app_context.selected_wallet_hash(),
                                        ),
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
    let columns = ((available_width + GRID_GAP) / min_width).floor() as usize;
    columns.max(1)
}

enum PickerCell<'a> {
    Identity(&'a QualifiedIdentity),
    Add,
}

/// Build a picker card using the same profile name as navigation.
fn build_card(app_context: &AppContext, identity: &QualifiedIdentity) -> IdentityPickerCard {
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

    if let Some(name) = app_context.identity_display_name(identity.identity.id()) {
        card = card.with_display_name(name);
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
