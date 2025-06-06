use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::styled::GradientButton;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing};
use crate::ui::RootScreenType;
use dash_sdk::dpp::version::v9::PROTOCOL_VERSION_9;
use eframe::epaint::Margin;
use egui::{Color32, Context, Frame, ImageButton, SidePanel, TextureHandle};
use rust_embed::RustEmbed;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(RustEmbed)]
#[folder = "icons/"] // Adjust the folder path if necessary
struct Assets;

// Function to load an icon as a texture using embedded assets
fn load_icon(ctx: &Context, path: &str) -> Option<TextureHandle> {
    // Attempt to retrieve the embedded file
    if let Some(content) = Assets::get(path) {
        // Load the image from the embedded bytes
        if let Ok(image) = image::load_from_memory(&content.data) {
            let size = [image.width() as usize, image.height() as usize];
            let rgba_image = image.into_rgba8();
            let pixels = rgba_image.into_raw();

            Some(ctx.load_texture(
                path,
                egui::ColorImage::from_rgba_unmultiplied(size, &pixels),
                Default::default(),
            ))
        } else {
            eprintln!("Failed to load image from embedded data at path: {}", path);
            None
        }
    } else {
        eprintln!("Image not found in embedded assets at path: {}", path);
        None
    }
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

    SidePanel::left("left_panel")
        .default_width(panel_width + 20.0) // Add extra width for margins
        .frame(
            Frame::new()
                .fill(DashColors::BACKGROUND) // Light background instead of transparent
                .inner_margin(Margin::symmetric(10, 10)), // Add margins for island effect
        )
        .show(ctx, |ui| {
            // Create an island panel with rounded edges
            Frame::new()
                .fill(DashColors::SURFACE)
                .stroke(egui::Stroke::new(1.0, DashColors::BORDER_LIGHT))
                .inner_margin(Margin::same(Spacing::MD_I8))
                .rounding(egui::Rounding::same(Shape::RADIUS_LG))
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
                                DashColors::DASH_BLUE
                            } else {
                                DashColors::GRADIENT_ACCENT
                            };

                            // Add icon-based button if texture is loaded
                            if let Some(ref texture) = texture {
                                let button = ImageButton::new(texture)
                                    .frame(false) // Remove button frame
                                    .tint(button_color);

                                if ui.add(button).clicked() {
                                    action =
                                        AppAction::SetMainScreenThenGoToMainScreen(*screen_type);
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
                                        .fill(DashColors::glass_white())
                                        .stroke(egui::Stroke::new(1.0, DashColors::glass_border()))
                                        .rounding(egui::Rounding::same(Shape::RADIUS_MD))
                                        .min_size(egui::vec2(60.0, 60.0));

                                    if ui.add(button).clicked() {
                                        action = AppAction::SetMainScreen(*screen_type);
                                    }
                                }
                            }

                            ui.add_space(Spacing::MD); // Add some space between buttons
                        }

                        // Push content to the top and dev label to the bottom
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
