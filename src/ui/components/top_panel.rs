use std::env;
use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::core::CoreTask;
use crate::backend_task::BackendTask;
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::context::AppContext;
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
            for (index, (text, location_action)) in location.into_iter().enumerate() {
                if ui
                    .button(
                        RichText::new(text)
                            .font(font_id.clone())
                            .color(Color32::WHITE),
                    )
                    .clicked()
                {
                    action = location_action;
                }

                // Add a separator (e.g., '>' symbol) between buttons, except for the last one
                if index < len - 1 {
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

fn show_app_version(ui: &mut Ui) {
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "".to_string());
    ui.label(version);
}

fn add_connection_indicator(ui: &mut Ui, app_context: &Arc<AppContext>) -> AppAction {
    let mut action = AppAction::None;

    // Determine connection status
    let connected = {
        if let Ok(status) = app_context.zmq_connection_status.lock() {
            matches!(*status, ZMQConnectionEvent::Connected)
        } else {
            false
        }
    };

    // Define circle properties
    let circle_size = 14.0; // Increase size slightly for visibility
    let color = if connected {
        Color32::DARK_GREEN
    } else {
        Color32::DARK_RED
    };

    // Allocate space for the circle with some padding
    ui.horizontal(|ui| {
        ui.add_space(8.0); // Add padding before the circle for visibility

        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(circle_size, circle_size), egui::Sense::click());

        // Offset the circle's center by 5 pixels down
        let circle_center = rect.center() + egui::vec2(0.0, 5.0);

        // Draw an outer glow
        ui.painter().circle_filled(
            circle_center,
            circle_size / 2.0 + 3.0, // Adjust glow size
            color.linear_multiply(0.3),
        );

        // Draw the main circle (the "light")
        ui.painter()
            .circle_filled(circle_center, circle_size / 2.0, color);

        // Tooltip text
        let tooltip_text = if connected {
            "Connected to Dash Core Wallet"
        } else {
            "Disconnected from Dash Core Wallet. Click to start it."
        };
        response.clone().on_hover_text(tooltip_text);

        let settings = app_context
            .db
            .get_settings()
            .expect("Failed to db get settings");
        let (custom_dash_qt_path, overwrite_dash_conf) = match settings {
            Some((.., db_custom_dash_qt_path, db_overwrite_dash_qt)) => {
                (db_custom_dash_qt_path, db_overwrite_dash_qt)
            }
            _ => {
                // Default values: Use system default path and overwrite conf
                (None, true)
            }
        };

        // Handle click to start DashQT if disconnected
        if response.clicked() && !connected {
            let network = app_context.network;
            action |= AppAction::BackendTask(BackendTask::CoreTask(CoreTask::StartDashQT(
                network,
                custom_dash_qt_path,
                overwrite_dash_conf,
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
        Network::Dash => Color32::from_rgb(21, 101, 192), // A blue color for mainnet
        Network::Testnet => Color32::from_rgb(255, 165, 0), // Orange for testnet
        Network::Devnet => Color32::from_rgb(255, 0, 0),  // Red for devnet
        Network::Regtest => Color32::from_rgb(139, 69, 19), // Brownish for regtest
        _ => Color32::BLACK,
    };

    TopBottomPanel::top("top_panel")
        .frame(
            Frame::none()
                .fill(color)
                .inner_margin(Margin::symmetric(10.0, 10.0)),
        )
        .exact_height(50.0)
        .show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.horizontal(|ui| {
                    action |= add_connection_indicator(ui, app_context);
                    show_app_version(ui);
                });
                // Left-aligned content with location view
                action |= add_location_view(ui, location);

                // Right-aligned content with buttons
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for (text, right_button_action) in right_buttons.into_iter().rev() {
                        ui.add_space(8.0);

                        let font_id = egui::FontId::proportional(16.0); // Adjust font size as needed
                        let color = Color32::WHITE;

                        // Calculate text size using layout method
                        let button_text = text.to_string();
                        let text_size = ui
                            .fonts(|fonts| {
                                fonts.layout_no_wrap(button_text.clone(), font_id.clone(), color)
                            })
                            .size();

                        let button_width = text_size.x + 12.0; // Add padding for the button

                        let button = egui::Button::new(RichText::new(text).color(Color32::WHITE))
                            .fill(Color32::from_rgb(0, 128, 255)) // Button background color
                            .frame(true) // Button frame
                            .rounding(3.0) // Rounded corners
                            .stroke(Stroke::new(1.0, Color32::WHITE)) // Border stroke
                            .min_size(egui::vec2(button_width, 30.0));

                        if ui.add(button).clicked() {
                            action = right_button_action.create_action(app_context);
                        }
                    }
                });
            });
        });

    action
}
