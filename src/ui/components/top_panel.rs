use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use dash_sdk::dashcore_rpc::dashcore::Network;
use egui::{Align, Color32, Context, Frame, Layout, Margin, RichText, Stroke, TopBottomPanel, Ui};
use std::sync::Arc;
use crate::components::core_zmq_listener::ZMQConnectionEvent;

fn add_location_view(ui: &mut Ui, location: Vec<(&str, AppAction)>) -> AppAction {
    let mut action = AppAction::None;
    let font_id = egui::FontId::proportional(22.0);

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

                let connection_status = {
                    if let Ok(status) = app_context.zmq_connection_status.lock() {
                        match *status {
                            ZMQConnectionEvent::Connected => "CONNECTED",
                            ZMQConnectionEvent::Disconnected => "DISCONNECTED",
                        }
                    } else {
                        "Lock Error"
                    }
                };
                ui.label(connection_status);

                // Left-aligned content with location view
                action = add_location_view(ui, location);

                // Right-aligned content with buttons
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {

                    //ui.label("takis the cat");
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
