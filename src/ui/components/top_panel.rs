use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::core::CoreTask;
use crate::backend_task::BackendTask;
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::context::AppContext;
use crate::ui::ScreenType;
use dash_sdk::dashcore_rpc::dashcore::Network;
use egui::{Align, Color32, Context, Frame, Layout, Margin, RichText, Stroke, TopBottomPanel, Ui};
use std::sync::Arc;

fn add_location_view(ui: &mut Ui, location: Vec<(&str, AppAction)>) -> AppAction {
    let mut action = AppAction::None;
    let font_id = egui::FontId::proportional(22.0);

    ui.add_space(2.0);
    egui::menu::bar(ui, |ui| {
        ui.horizontal(|ui| {
            let len = location.len();
            for (idx, (text, loc_action)) in location.into_iter().enumerate() {
                if ui
                    .button(
                        RichText::new(text)
                            .font(font_id.clone())
                            .color(Color32::WHITE),
                    )
                    .clicked()
                {
                    action = loc_action;
                }
                if idx < len - 1 {
                    ui.label(
                        RichText::new(">")
                            .font(font_id.clone())
                            .color(Color32::WHITE),
                    );
                }
            }
        });
    });
    action
}

fn add_connection_indicator(ui: &mut Ui, app_context: &Arc<AppContext>) -> AppAction {
    let mut action = AppAction::None;
    let connected = app_context
        .zmq_connection_status
        .lock()
        .map(|status| matches!(*status, ZMQConnectionEvent::Connected))
        .unwrap_or(false);
    let circle_size = 14.0;
    let color = if connected {
        Color32::DARK_GREEN
    } else {
        Color32::DARK_RED
    };

    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(circle_size, circle_size), egui::Sense::click());
        let center = rect.center() + egui::vec2(0.0, 5.0);
        ui.painter()
            .circle_filled(center, circle_size / 2.0 + 3.0, color.linear_multiply(0.3));
        ui.painter().circle_filled(center, circle_size / 2.0, color);
        let tip = if connected {
            "Connected to Dash Core Wallet"
        } else {
            "Disconnected from Dash Core Wallet. Click to start it."
        };
        let resp = resp.on_hover_text(tip);

        if resp.clicked() && !connected {
            let settings = app_context.db.get_settings().ok().flatten();
            let (custom_path, overwrite) = settings
                .map(|(_, _, _, custom_path, overwrite)| (custom_path, overwrite))
                .unwrap_or((None, true));
            action |= AppAction::BackendTask(BackendTask::CoreTask(CoreTask::StartDashQT(
                app_context.network,
                custom_path,
                overwrite,
            )));
        }
    });
    action
}

pub fn add_top_panel(
    ctx: &Context,
    app_context: &Arc<AppContext>,
    location: Vec<(&str, AppAction)>,
    right_buttons: Vec<(&str, DesiredAppAction)>,
) -> AppAction {
    let mut action = AppAction::None;
    let color = match app_context.network {
        Network::Dash => Color32::from_rgb(21, 101, 192),
        Network::Testnet => Color32::from_rgb(255, 165, 0),
        Network::Devnet => Color32::DARK_RED,
        Network::Regtest => Color32::from_rgb(139, 69, 19),
        _ => Color32::BLACK,
    };

    TopBottomPanel::top("top_panel")
        .frame(
            Frame::new()
                .fill(color)
                .inner_margin(Margin::symmetric(10, 10)),
        )
        .exact_height(50.0)
        .show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                action |= add_connection_indicator(ui, app_context);
                action |= add_location_view(ui, location);

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(10.0);

                    // Separate document-related actions into dropdown
                    let (doc_actions, other_actions): (Vec<_>, Vec<_>) =
                        right_buttons.into_iter().partition(|(_, act)| {
                            matches!(
                                act,
                                DesiredAppAction::AddScreenType(ScreenType::CreateDocument)
                                    | DesiredAppAction::AddScreenType(ScreenType::DeleteDocument)
                                    | DesiredAppAction::AddScreenType(ScreenType::ReplaceDocument)
                                    | DesiredAppAction::AddScreenType(ScreenType::TransferDocument)
                                    | DesiredAppAction::AddScreenType(ScreenType::PurchaseDocument)
                                    | DesiredAppAction::AddScreenType(ScreenType::SetDocumentPrice)
                            )
                        });

                    // Grouped Documents menu
                    if !doc_actions.is_empty() {
                        ui.add_space(3.0);

                        // give it the same style as your other buttons
                        let docs_btn =
                            egui::Button::new(RichText::new("Documents").color(Color32::WHITE))
                                .fill(Color32::from_rgb(0, 128, 255))
                                .frame(true)
                                .corner_radius(3.0)
                                .stroke(Stroke::new(1.0, Color32::WHITE))
                                .min_size(egui::vec2(100.0, 30.0));

                        // a unique ID for the popup
                        let popup_id = ui.auto_id_with("documents_popup");
                        let resp = ui.add(docs_btn);
                        if resp.clicked() {
                            ui.memory_mut(|mem| mem.toggle_popup(popup_id));
                        }

                        // open the popup directly below the button
                        egui::popup::popup_below_widget(
                            ui,
                            popup_id,
                            &resp,
                            egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                            |ui| {
                                ui.set_min_width(150.0);
                                for (text, da) in doc_actions {
                                    if ui.button(text).clicked() {
                                        action = da.create_action(app_context);
                                        ui.close_menu();
                                    }
                                }
                            },
                        );
                    }

                    // Render other buttons normally
                    for (text, btn_act) in other_actions.into_iter().rev() {
                        ui.add_space(3.0);
                        let font = egui::FontId::proportional(16.0);
                        let text_size = ui
                            .fonts(|f| {
                                f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
                            })
                            .size();
                        let width = text_size.x + 12.0;

                        let button = egui::Button::new(RichText::new(text).color(Color32::WHITE))
                            .fill(Color32::from_rgb(0, 128, 255))
                            .frame(true)
                            .corner_radius(3.0)
                            .stroke(Stroke::new(1.0, Color32::WHITE))
                            .min_size(egui::vec2(width, 30.0));

                        if ui.add(button).clicked() {
                            action = btn_act.create_action(app_context);
                        }
                    }
                });
            });
        });

    action
}
