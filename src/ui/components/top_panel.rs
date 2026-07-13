use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::context::connection_status::OverallConnectionState;
use crate::ui::components::global_nav_switcher::{self, GlobalNavEffect};
use crate::ui::state::global_nav::{PageNavSpec, PillConsumption};
use crate::ui::state::hub_selection::HubSelection;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Shadow, Shape};
use crate::ui::{RootScreenType, ScreenType};
use egui::{Align2, FontId, Frame, Margin, Panel, RichText, Ui};
use std::sync::Arc;

fn add_location_view(ui: &mut Ui, location: Vec<(&str, AppAction)>, dark_mode: bool) -> AppAction {
    let mut action = AppAction::None;
    let font_id = egui::FontId::proportional(22.0);

    // Wrap in a container that can be positioned vertically
    ui.allocate_ui(ui.available_size(), |ui| {
        // Apply negative vertical offset to move text up
        let offset = egui::vec2(0.0, -7.0);

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                ui.cursor().min + offset,
                ui.available_size(),
            )),
            |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.horizontal(|ui| {
                        let len = location.len();
                        for (idx, (text, loc_action)) in location.into_iter().enumerate() {
                            if ui
                                .button(
                                    RichText::new(text)
                                        .font(font_id.clone())
                                        .color(DashColors::text_primary(dark_mode)),
                                )
                                .clicked()
                            {
                                action = loc_action;
                            }
                            if idx < len - 1 {
                                ui.label(
                                    RichText::new(">")
                                        .font(font_id.clone())
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                            }
                        }
                    });
                });
            },
        );
    });
    action
}

fn add_connection_indicator(ui: &mut Ui, app_context: &Arc<AppContext>) {
    let status = app_context.connection_status();
    let overall = status.overall_state();

    let dark_mode = ui.style().visuals.dark_mode;
    let circle_size = 14.0;

    // Five-state color: green (synced), orange (syncing/connecting), magenta (error), red (disconnected)
    let color = match overall {
        OverallConnectionState::Synced => DashColors::success_color(dark_mode),
        OverallConnectionState::Connecting | OverallConnectionState::Syncing => {
            DashColors::warning_color(dark_mode)
        }
        OverallConnectionState::Error => DashColors::sync_error_color(dark_mode),
        OverallConnectionState::Disconnected => DashColors::error_color(dark_mode),
    };

    // Pulsation per state
    let time = ui.ctx().input(|i| i.time as f32);

    let pulse_scale = match overall {
        OverallConnectionState::Synced => 1.0 + 0.2 * (time * 2.0).sin(),
        OverallConnectionState::Connecting => 1.0 + 0.2 * (time * 2.5).sin(),
        OverallConnectionState::Syncing => 1.0 + 0.15 * (time * 1.2).sin(),
        OverallConnectionState::Error => 1.0 + 0.25 * (time * 0.8).sin(),
        OverallConnectionState::Disconnected => 1.0, // No pulse when disconnected
    };

    // Wrap in a container that can be positioned vertically
    ui.allocate_ui(ui.available_size(), |ui| {
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                ui.cursor().min,
                ui.available_size(),
            )),
            |ui| {
                ui.horizontal(|ui| {
                    // Hover-only: the tooltip is the indicator's whole interaction.
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(circle_size, circle_size),
                        egui::Sense::hover(),
                    );
                    let center = rect.center();

                    // Draw the background circle with pulsating effect
                    let bg_radius = if overall != OverallConnectionState::Disconnected {
                        (circle_size / 2.0 + 3.0) * pulse_scale
                    } else {
                        circle_size / 2.0 // Same size as main circle when disconnected
                    };
                    ui.painter()
                        .circle_filled(center, bg_radius, color.linear_multiply(0.3));

                    // Draw the main circle
                    ui.painter().circle_filled(center, circle_size / 2.0, color);

                    // Draw "!" glyph on error state
                    if overall == OverallConnectionState::Error {
                        ui.painter().text(
                            center,
                            Align2::CENTER_CENTER,
                            "!",
                            FontId::proportional(10.0),
                            egui::Color32::WHITE,
                        );
                    }

                    // Request repaint for animation (Synced, Syncing, and Error states pulse)
                    if overall != OverallConnectionState::Disconnected {
                        app_context.repaint_animation(ui.ctx());
                    }
                    let tip = status.tooltip_text();
                    let _resp = resp.info_tooltip(tip);
                });
            },
        );
    });
}

/// Shared top-island scaffold: the `Panel::top`, surface Frame + radius +
/// shadow + network accent, the 2-column layout, the connection indicator, and
/// the right-button grouping. The `left` closure renders the left/breadcrumb
/// region. Both public entry points delegate here so they render identically.
fn render_top_island(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    left: impl FnOnce(&mut Ui) -> AppAction,
    right_buttons: Vec<(&str, DesiredAppAction)>,
) -> AppAction {
    let ctx = ui.ctx().clone();
    let ctx = &ctx;
    let mut action = AppAction::None;
    let dark_mode = ctx.global_style().visuals.dark_mode;
    let network_accent = DashColors::network_accent(app_context.network, dark_mode);

    Panel::top("top_panel")
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::same(10)), // 10px margin on all sides
        )
        .show(ui, |ui| {
            // Create an island panel with rounded edges
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .inner_margin(Margin {
                    left: 20,
                    right: 20,
                    top: 10,
                    bottom: 10,
                })
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |ui| {
                    // Use columns for better control over layout
                    ui.columns(2, |columns| {
                        // Left column: connection indicator and location
                        columns[0].with_layout(
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                add_connection_indicator(ui, app_context);
                                action |= left(ui);
                            },
                        );

                        // Right column: buttons (right-aligned)
                        columns[1].with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                // Separate contract and document-related actions
                                let mut contract_actions = Vec::new();
                                let mut doc_actions = Vec::new();
                                let mut other_actions = Vec::new();

                                for (text, act) in right_buttons.into_iter() {
                                    match act {
                                        DesiredAppAction::AddScreenType(ref screen_type) => {
                                            match **screen_type {
                                                ScreenType::AddContracts
                                                | ScreenType::RegisterContract
                                                | ScreenType::UpdateContract => {
                                                    contract_actions.push((text, act));
                                                }
                                                ScreenType::CreateDocument
                                                | ScreenType::DeleteDocument
                                                | ScreenType::ReplaceDocument
                                                | ScreenType::TransferDocument
                                                | ScreenType::PurchaseDocument
                                                | ScreenType::SetDocumentPrice => {
                                                    doc_actions.push((text, act));
                                                }
                                                _ => {
                                                    other_actions.push((text, act));
                                                }
                                            }
                                        }
                                        _ => {
                                            other_actions.push((text, act));
                                        }
                                    }
                                }

                                // Grouped Documents menu
                                if !doc_actions.is_empty() {
                                    ui.add_space(3.0);

                                    let resp = ComponentStyles::add_toolbar_button(
                                        ui,
                                        "Documents",
                                        network_accent,
                                    );
                                    let popup_id = ui.make_persistent_id("docs_popup");

                                    let dark_mode = ui.style().visuals.dark_mode;
                                    egui::Popup::new(
                                        popup_id,
                                        ui.ctx().clone(),
                                        &resp,
                                        resp.layer_id,
                                    )
                                    .open_memory(
                                        resp.clicked().then_some(egui::SetOpenCommand::Toggle),
                                    )
                                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                    .frame(
                                        egui::Frame::popup(ui.style())
                                            .fill(DashColors::popup_fill(dark_mode)),
                                    )
                                    .show(|ui| {
                                        ui.set_min_width(150.0);
                                        for (text, da) in doc_actions {
                                            let btn = egui::Button::new(text)
                                                .min_size(egui::vec2(ui.available_width(), 0.0));
                                            if ComponentStyles::add_button(ui, btn).clicked() {
                                                action = da.create_action(app_context);
                                                ui.close();
                                            }
                                        }
                                    });
                                }

                                // Grouped Contracts menu
                                if !contract_actions.is_empty() {
                                    ui.add_space(3.0);

                                    let popup_id = ui.auto_id_with("contracts_popup");
                                    let resp = ComponentStyles::add_toolbar_button(
                                        ui,
                                        "Contracts",
                                        network_accent,
                                    );

                                    let dark_mode = ui.style().visuals.dark_mode;
                                    egui::Popup::new(
                                        popup_id,
                                        ui.ctx().clone(),
                                        &resp,
                                        resp.layer_id,
                                    )
                                    .open_memory(
                                        resp.clicked().then_some(egui::SetOpenCommand::Toggle),
                                    )
                                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                    .frame(
                                        egui::Frame::popup(ui.style())
                                            .fill(DashColors::popup_fill(dark_mode)),
                                    )
                                    .show(|ui| {
                                        ui.set_min_width(150.0);
                                        for (text, ca) in contract_actions {
                                            let btn = egui::Button::new(text)
                                                .min_size(egui::vec2(ui.available_width(), 0.0));
                                            if ComponentStyles::add_button(ui, btn).clicked() {
                                                action = ca.create_action(app_context);
                                                ui.close();
                                            }
                                        }
                                    });
                                }

                                // Render other buttons normally
                                for (text, btn_act) in other_actions.into_iter().rev() {
                                    ui.add_space(3.0);
                                    if ComponentStyles::add_toolbar_button(ui, text, network_accent)
                                        .clicked()
                                    {
                                        action = btn_act.create_action(app_context);
                                    }
                                }
                            },
                        );
                    });
                });
        });

    action
}

/// Render the standard top panel with a plain-text breadcrumb location and the
/// grouped right-side action buttons. Unchanged public API — delegates to the
/// shared [`render_top_island`] scaffold.
pub fn add_top_panel(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    location: Vec<(&str, AppAction)>,
    right_buttons: Vec<(&str, DesiredAppAction)>,
) -> AppAction {
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;
    render_top_island(
        ui,
        app_context,
        |ui| add_location_view(ui, location, dark_mode),
        right_buttons,
    )
}

/// Render the top panel with a custom left/breadcrumb region — the Identities
/// hub switcher injects its three-segment breadcrumb here. Same island, accent,
/// connection indicator, and right column as [`add_top_panel`].
pub fn add_top_panel_with_breadcrumb(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    breadcrumb: impl FnOnce(&mut Ui) -> AppAction,
    right_buttons: Vec<(&str, DesiredAppAction)>,
) -> AppAction {
    render_top_island(ui, app_context, breadcrumb, right_buttons)
}

/// Standard how-to-change tooltip for an unwired wallet pill (FR-GLOBAL-NAV-2
/// rule 3). A single translation unit; keep it a complete sentence.
const TT_WALLET_UNWIRED: &str = "Change the active wallet from the Wallets tab.";
/// Standard how-to-change tooltip for an unwired identity pill.
const TT_IDENTITY_UNWIRED: &str = "Change the active identity from the Identity Hub.";

/// An everyday-page global-nav spec with both pills subdued (unwired) — the
/// Phase-A rollout default. Segment-1 links to the page's own root.
pub fn subdued_everyday_spec(label: impl Into<String>, target: RootScreenType) -> PageNavSpec {
    PageNavSpec::unwired_everyday(label, target, TT_WALLET_UNWIRED, TT_IDENTITY_UNWIRED)
}

/// A wallet-only global-nav spec (no identity/object pill) with the wallet pill
/// **interactive** (`Consumed`): picking a wallet from it drives the app-global
/// selection via [`apply_global_nav_effect`]. For pages with no identity context
/// that own the wallet-selection surface, e.g. Wallets — FR-GLOBAL-NAV-2 rules 2
/// and 4.
pub fn wallet_only_spec(label: impl Into<String>, target: RootScreenType) -> PageNavSpec {
    PageNavSpec::new(label, target).with_wallet_pill(PillConsumption::Consumed)
}

/// Apply a generalized global-nav effect: wallet/identity selection updates the
/// **app-global** selection silently (no forced navigation — FR-GLOBAL-NAV-2
/// rule 1); segment-1 navigation and add-flows route to the existing screens.
///
/// The shared successor to the hub's `apply_breadcrumb_effect`. The hub keeps
/// its own richer applier (it also resets identity-scoped caches and opens the
/// picker); every other root page uses this one.
pub fn apply_global_nav_effect(
    app_context: &Arc<AppContext>,
    effect: GlobalNavEffect,
) -> AppAction {
    match effect {
        GlobalNavEffect::None => AppAction::None,
        GlobalNavEffect::NavigateToRoot(target) => AppAction::SetMainScreen(target),
        // Silent app-scoped write, NO forced navigation (FR-GLOBAL-NAV-2 rule
        // 1). `set_selected_hd_wallet` also reconciles the app-global *identity*
        // to the new wallet's identities as a side effect (keep-if-owned →
        // first → None). On a non-Hub page this cross-axis mutation is real and
        // intentional; combined with the resolution-layer MN/Evonode filter it
        // must never reconcile onto a masternode/evonode identity — the FR-6
        // boundary is enforced there, not re-checked here.
        GlobalNavEffect::SwitchWallet(hash) => {
            app_context.set_selected_hd_wallet(Some(hash));
            AppAction::None
        }
        GlobalNavEffect::SelectIdentity(id) => {
            app_context.set_selected_identity(Some(id));
            AppAction::None
        }
        // The page-scoped object selection is owned by its page (B7) and never
        // writes `AppContext::selected_identity_id`. Unwired pages never emit it.
        GlobalNavEffect::SelectPageObject(_) => AppAction::None,
        GlobalNavEffect::AddWallet => {
            AppAction::SetMainScreen(RootScreenType::RootScreenWalletsBalances)
        }
        GlobalNavEffect::AddIdentityCreate | GlobalNavEffect::CreateTestIdentities => {
            AppAction::AddScreen(ScreenType::AddNewIdentity.create_screen(app_context))
        }
        GlobalNavEffect::AddIdentityLoad => {
            AppAction::AddScreen(ScreenType::AddExistingIdentity.create_screen(app_context))
        }
    }
}

/// Render the top panel with the global-nav switcher for `spec`, then apply its
/// effect. The one-call entry point every non-Hub root screen uses in place of
/// [`add_top_panel`]. Unwired specs compose no interactive dropdown, so a
/// throwaway per-frame search buffer suffices; interactive pages (the Hub,
/// later Masternodes) own a persistent [`HubSelection`] and wire the effect
/// themselves.
pub fn add_top_panel_with_global_nav(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    spec: PageNavSpec,
    right_buttons: Vec<(&str, DesiredAppAction)>,
) -> AppAction {
    let mut effect = GlobalNavEffect::None;
    let mut selection = HubSelection::default();
    let mut action = render_top_island(
        ui,
        app_context,
        |ui| {
            effect = global_nav_switcher::render(ui, app_context, &spec, &mut selection);
            AppAction::None
        },
        right_buttons,
    );
    action |= apply_global_nav_effect(app_context, effect);
    action
}

/// Like [`add_top_panel_with_global_nav`], but also returns the raw
/// [`GlobalNavEffect`] so a page that **consumes** a selection can mirror it into
/// its own view state — the page half of the two-way binding (FR-GLOBAL-NAV-2
/// rule 2). The returned effect is **already applied**; a caller must only
/// synchronize its own state from it, never re-apply it.
///
/// Two consumers today: the Wallets page mirrors
/// [`GlobalNavEffect::SwitchWallet`] into its selected wallet, and the
/// Masternodes page mirrors [`GlobalNavEffect::SelectPageObject`] by opening
/// that node's detail view. `SelectPageObject` is *only* surfaced here — the
/// applier never writes it to `AppContext::selected_identity_id` (the FR-6
/// boundary).
pub fn add_top_panel_with_global_nav_capturing(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    spec: PageNavSpec,
    right_buttons: Vec<(&str, DesiredAppAction)>,
) -> (AppAction, GlobalNavEffect) {
    let mut effect = GlobalNavEffect::None;
    let mut selection = HubSelection::default();
    let mut action = render_top_island(
        ui,
        app_context,
        |ui| {
            effect = global_nav_switcher::render(ui, app_context, &spec, &mut selection);
            AppAction::None
        },
        right_buttons,
    );
    action |= apply_global_nav_effect(app_context, effect.clone());
    (action, effect)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TC-WALLETLINK-02: the Wallets page's spec exposes an **interactive**
    /// (`Consumed`) wallet pill with no how-to tooltip — the pill drives the
    /// selection here rather than pointing elsewhere.
    #[test]
    fn wallet_only_spec_pill_is_consumed_without_tooltip() {
        let spec = wallet_only_spec("Wallets", RootScreenType::RootScreenWalletsBalances);
        let pill = spec.wallet_pill().expect("wallet pill present");
        assert!(pill.is_consumed(), "the Wallets pill must be interactive");
        assert_eq!(
            pill.tooltip(),
            None,
            "an interactive pill shows no how-to tooltip"
        );
        assert!(
            spec.identity_pill().is_none(),
            "the Wallets page is wallet-only, with no identity pill"
        );
    }
}
