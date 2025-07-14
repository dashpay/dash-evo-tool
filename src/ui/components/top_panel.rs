use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::BackendTask;
use crate::backend_task::core::CoreTask;
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::context::AppContext;
use crate::ui::ScreenType;
use crate::ui::theme::{DashColors, Shadow, Shape};
use dash_sdk::dashcore_rpc::dashcore::Network;
use egui::{
    Align, Color32, Context, Frame, Margin, Popup, RichText, Stroke, TextureHandle, TopBottomPanel,
    Ui,
};
use rust_embed::RustEmbed;
use std::sync::Arc;

#[derive(RustEmbed)]
#[folder = "icons/"]
struct Assets;

// Function to load an icon as a texture using embedded assets
#[allow(dead_code)]
fn load_icon(ctx: &Context, path: &str) -> Option<TextureHandle> {
    // Use ctx.data_mut to check if texture is already cached
    ctx.data_mut(|d| d.get_temp::<TextureHandle>(egui::Id::new(path)))
        .or_else(|| {
            // Only do expensive operations if texture is not cached
            if let Some(content) = Assets::get(path) {
                // Load the image from the embedded bytes
                if let Ok(image) = image::load_from_memory(&content.data) {
                    let size = [image.width() as usize, image.height() as usize];
                    let rgba_image = image.into_rgba8();
                    let pixels = rgba_image.into_raw();

                    let texture = ctx.load_texture(
                        path,
                        egui::ColorImage::from_rgba_unmultiplied(size, &pixels),
                        Default::default(),
                    );

                    // Cache the texture
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new(path), texture.clone()));

                    Some(texture)
                } else {
                    eprintln!("Failed to load image from embedded data at path: {}", path);
                    None
                }
            } else {
                eprintln!("Image not found in embedded assets at path: {}", path);
                None
            }
        })
}

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

fn add_connection_indicator(ui: &mut Ui, app_context: &Arc<AppContext>) -> AppAction {
    let mut action = AppAction::None;
    let connected = app_context
        .zmq_connection_status
        .lock()
        .map(|status| matches!(*status, ZMQConnectionEvent::Connected))
        .unwrap_or(false);

    // Get time for pulsating animation (only when connected)
    let pulse_scale = if connected {
        let time = ui.ctx().input(|i| i.time as f32);
        1.0 + 0.2 * (time * 2.0).sin() // Pulsate between 1.0 and 1.2
    } else {
        1.0 // No pulsation when disconnected
    };

    let dark_mode = ui.ctx().style().visuals.dark_mode;
    let circle_size = 14.0;
    let color = if connected {
        DashColors::success_color(dark_mode)
    } else {
        DashColors::error_color(dark_mode)
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
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(circle_size, circle_size),
                        egui::Sense::click(),
                    );
                    let center = rect.center();

                    // Draw the background circle with pulsating effect
                    let bg_radius = if connected {
                        (circle_size / 2.0 + 3.0) * pulse_scale
                    } else {
                        circle_size / 2.0 // Same size as main circle when disconnected
                    };
                    ui.painter()
                        .circle_filled(center, bg_radius, color.linear_multiply(0.3));

                    // Draw the main circle
                    ui.painter().circle_filled(center, circle_size / 2.0, color);

                    // Request repaint for animation (only when connected and pulsating)
                    if connected {
                        app_context.repaint_animation(ui.ctx());
                    }
                    let tip = if connected {
                        "Connected to Dash Core Wallet"
                    } else {
                        "Disconnected from Dash Core Wallet. Click to start it."
                    };
                    let resp = resp.on_hover_text(tip);

                    if resp.clicked() && !connected {
                        let settings = app_context.db.get_settings().ok().flatten();
                        let (custom_path, overwrite) = settings
                            .map(|(_, _, _, custom_path, overwrite, _)| (custom_path, overwrite))
                            .unwrap_or((None, true));
                        if let Some(dash_qt_path) = custom_path {
                            action |= AppAction::BackendTask(BackendTask::CoreTask(
                                CoreTask::StartDashQT(app_context.network, dash_qt_path, overwrite),
                            ));
                        } else {
                            tracing::debug!(
                                "Dash-Qt path not set in settings, not starting Dash-Qt from connection indicator."
                            );
                        }
                    }
                });
            },
        );
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
    let dark_mode = ctx.style().visuals.dark_mode;
    let network_accent = match app_context.network {
        Network::Dash => {
            if dark_mode {
                Color32::from_rgb(0, 113, 182) // Muted blue for dark mode (20% darker)
            } else {
                DashColors::DASH_BLUE // Original: rgb(0, 141, 228)
            }
        }
        Network::Testnet => {
            if dark_mode {
                Color32::from_rgb(204, 132, 0) // Muted orange for dark mode
            } else {
                Color32::from_rgb(255, 165, 0) // Original bright orange for light mode
            }
        }
        Network::Devnet => {
            if dark_mode {
                Color32::from_rgb(111, 0, 0) // Muted dark red for dark mode (20% darker)
            } else {
                Color32::DARK_RED // Original: rgb(139, 0, 0)
            }
        }
        Network::Regtest => {
            if dark_mode {
                Color32::from_rgb(111, 55, 15) // Muted brown for dark mode (20% darker)
            } else {
                Color32::from_rgb(139, 69, 19) // Original brown
            }
        }
        _ => {
            if dark_mode {
                Color32::from_rgb(0, 113, 182) // Muted blue for dark mode
            } else {
                DashColors::DASH_BLUE
            }
        }
    };

    TopBottomPanel::top("top_panel")
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin {
                    left: 10,
                    right: 10,
                    top: 10,
                    bottom: 10,
                }),
        )
        .exact_height(76.0)
        .show(ctx, |ui| {
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
                    // Load Dash logo
                    // let dash_logo_texture: Option<TextureHandle> = load_icon(ctx, "dash.png");

                    ui.columns(3, |columns| {
                        // Left column: connection indicator and location
                        columns[0].with_layout(
                            egui::Layout::left_to_right(egui::Align::Center)
                                .with_cross_align(Align::Center),
                            |ui| {
                                action |= add_connection_indicator(ui, app_context);
                                action |= add_location_view(ui, location, dark_mode);
                            },
                        );

                        // Center column: Placeholder for future logo placement
                        columns[1].with_layout(
                            egui::Layout::centered_and_justified(egui::Direction::TopDown),
                            |ui| {
                                // Placeholder - logo moved back to left panel for now
                                ui.label("");
                            },
                        );

                        // Right column: action buttons (right-aligned)
                        columns[2].with_layout(
                            egui::Layout::right_to_left(egui::Align::Center)
                                .with_cross_align(Align::Center),
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

                                    // give it the same style as your other buttons
                                    let docs_btn = egui::Button::new(
                                        RichText::new("Documents").color(Color32::WHITE),
                                    )
                                    .fill(network_accent)
                                    .frame(true)
                                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                                    .stroke(Stroke::NONE)
                                    .min_size(egui::vec2(100.0, 30.0));

                                    let resp = ui.add(docs_btn);
                                    let popup_id = ui.make_persistent_id("docs_popup");

                                    if resp.clicked() {
                                        Popup::toggle_id(ui.ctx(), popup_id);
                                    }

                                    // open the popup directly below the button
                                    egui::old_popup::popup_below_widget(
                                        ui,
                                        popup_id,
                                        &resp,
                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                        |ui| {
                                            ui.set_min_width(150.0);
                                            for (text, da) in doc_actions {
                                                if ui.button(text).clicked() {
                                                    action = da.create_action(app_context);
                                                    ui.close_kind(egui::UiKind::Menu);
                                                }
                                            }
                                        },
                                    );
                                }

                                // Grouped Contracts menu
                                if !contract_actions.is_empty() {
                                    ui.add_space(3.0);

                                    let contracts_btn = egui::Button::new(
                                        RichText::new("Contracts").color(Color32::WHITE),
                                    )
                                    .fill(network_accent)
                                    .frame(true)
                                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                                    .stroke(Stroke::NONE)
                                    .min_size(egui::vec2(100.0, 30.0));

                                    let popup_id = ui.auto_id_with("contracts_popup");
                                    let resp = ui.add(contracts_btn);
                                    if resp.clicked() {
                                        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
                                    }

                                    egui::old_popup::popup_below_widget(
                                        ui,
                                        popup_id,
                                        &resp,
                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                        |ui| {
                                            ui.set_min_width(150.0);
                                            for (text, ca) in contract_actions {
                                                if ui.button(text).clicked() {
                                                    action = ca.create_action(app_context);
                                                    ui.close_kind(egui::UiKind::Menu);
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
                                            f.layout_no_wrap(
                                                text.to_string(),
                                                font.clone(),
                                                Color32::WHITE,
                                            )
                                        })
                                        .size();
                                    let width = text_size.x + 12.0;

                                    let button = egui::Button::new(
                                        RichText::new(text).color(Color32::WHITE),
                                    )
                                    .fill(network_accent)
                                    .frame(true)
                                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                                    .stroke(Stroke::NONE)
                                    .min_size(egui::vec2(width, 30.0));

                                    if ui.add(button).clicked() {
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
