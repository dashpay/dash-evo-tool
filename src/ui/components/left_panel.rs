use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::styled::GradientButton;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing};
use crate::ui::RootScreenType;
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dpp::version::v9::PROTOCOL_VERSION_9;
use eframe::epaint::Margin;
use egui::{Color32, Context, Frame, ImageButton, RichText, SidePanel, TextureHandle};
use rust_embed::RustEmbed;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(RustEmbed)]
#[folder = "icons/"] // Adjust the folder path if necessary
struct Assets;

// Function to load an icon as a texture using embedded assets
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
                        egui::TextureOptions::LINEAR, // Use linear filtering for smoother scaling
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

pub fn add_left_panel(
    ctx: &Context,
    app_context: &Arc<AppContext>,
    selected_screen: RootScreenType,
) -> AppAction {
    let mut action = AppAction::None;

    // Define the button details directly in this function
    let buttons = [
        ("I", RootScreenType::RootScreenIdentities, "identity.png"),
        ("Q", RootScreenType::RootScreenDocumentQuery, "doc.png"),
        ("O", RootScreenType::RootScreenMyTokenBalances, "tokens.png"),
        (
            "C",
            RootScreenType::RootScreenDPNSActiveContests,
            "voting.png",
        ),
        ("W", RootScreenType::RootScreenWalletsBalances, "wallet.png"),
        (
            "T",
            RootScreenType::RootScreenToolsProofLogScreen,
            "tools.png",
        ),
        ("N", RootScreenType::RootScreenNetworkChooser, "config.png"),
    ];

    let panel_width = 60.0 + (Spacing::MD * 2.0); // Button width + margins

    let dark_mode = ctx.style().visuals.dark_mode;

    SidePanel::left("left_panel")
        .default_width(panel_width + 20.0) // Add extra width for margins
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::symmetric(10, 10)), // Add margins for island effect
        )
        .show(ctx, |ui| {
            // Create an island panel with rounded edges
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .inner_margin(Margin::same(Spacing::MD_I8))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        for (label, screen_type, icon_path) in buttons.iter() {
                            if !check_root_screen_access(app_context, screen_type) {
                                continue; // Skip this button if access is denied
                            }
                            let texture: Option<TextureHandle> = load_icon(ctx, icon_path);
                            let is_selected = selected_screen == *screen_type;

                            let button_color = if is_selected {
                                Color32::WHITE // Bright white for selected
                            } else {
                                if dark_mode {
                                    Color32::from_rgb(180, 180, 180) // Bright gray for visibility in dark mode
                                } else {
                                    Color32::from_rgb(160, 160, 160) // Medium gray for contrast in light mode
                                }
                            };

                            // Add icon-based button if texture is loaded
                            if let Some(ref texture) = texture {
                                let button = ImageButton::new(texture)
                                    .frame(false)
                                    .tint(button_color);

                                if ui.add(button).clicked() {
                                    action = AppAction::SetMainScreenThenGoToMainScreen(*screen_type);
                                }
                            } else {
                                // Fallback to a modern gradient button if texture loading fails
                                if is_selected {
                                    if GradientButton::new(*label)
                                        .min_width(60.0)
                                        .glow()
                                        .show(ui)
                                        .clicked()
                                    {
                                        action = AppAction::SetMainScreen(*screen_type);
                                    }
                                } else {
                                    let button = egui::Button::new(*label)
                                        .fill(DashColors::glass_white(dark_mode))
                                        .stroke(egui::Stroke::new(
                                            1.0,
                                            DashColors::glass_border(dark_mode),
                                        ))
                                        .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                                        .min_size(egui::vec2(60.0, 60.0));

                                    if ui.add(button).clicked() {
                                        action = AppAction::SetMainScreen(*screen_type);
                                    }
                                }
                            }

                            ui.add_space(Spacing::MD); // Add some space between buttons
                        }

                        // Push content to the top and dev label + logo to the bottom
                        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                            if app_context.developer_mode.load(Ordering::Relaxed) {
                                ui.add_space(Spacing::MD);
                                let dev_label = egui::RichText::new("🔧 Dev mode")
                                    .color(DashColors::GRADIENT_PURPLE)
                                    .size(12.0);
                                if ui.label(dev_label).clicked() {
                                    action = AppAction::SetMainScreenThenGoToMainScreen(
                                        RootScreenType::RootScreenNetworkChooser,
                                    );
                                };
                            }

                            // Show network name if not on main Dash network
                            if app_context.network != Network::Dash {
                                let (network_name, network_color) = match app_context.network {
                                    Network::Testnet => ("Testnet", Color32::from_rgb(255, 165, 0)),
                                    Network::Devnet => ("Devnet", Color32::DARK_RED),
                                    Network::Regtest => {
                                        ("Local Network", Color32::from_rgb(139, 69, 19))
                                    }
                                    _ => ("Unknown", DashColors::DASH_BLUE),
                                };

                                ui.label(
                                    RichText::new(network_name)
                                        .color(network_color)
                                        .size(12.0)
                                        .strong(),
                                );
                                ui.add_space(2.0);
                            }

                            // Add Dash logo at the bottom
                            if let Some(dash_texture) = load_icon(ctx, "dash.png") {
                                if app_context.network == Network::Dash {
                                    ui.add_space(Spacing::SM);
                                }
                                let logo_size = egui::vec2(50.0, 20.0); // Even smaller size, same aspect ratio
                                let logo_response = ui.add(
                                    egui::Image::new(&dash_texture)
                                        .fit_to_exact_size(logo_size)
                                        .texture_options(egui::TextureOptions::LINEAR) // Smooth interpolation to reduce pixelation
                                        .sense(egui::Sense::click()),
                                );

                                if logo_response.clicked() {
                                    ui.ctx()
                                        .open_url(egui::OpenUrl::new_tab("https://dash.org"));
                                }

                                if logo_response.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                            }
                        });
                    });
                }); // Close the island frame
        });

    action
}

/// Checks if the user has access to the button based on the screen type.
fn check_root_screen_access(app_context: &Arc<AppContext>, screen_type: &RootScreenType) -> bool {
    let protocol_version = app_context.platform_version().protocol_version;

    // For RootScreenMyTokenBalances
    !matches!(screen_type, RootScreenType::RootScreenMyTokenBalances if protocol_version < PROTOCOL_VERSION_9)
}
