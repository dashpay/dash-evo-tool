use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::RootScreenType;
use dash_sdk::dpp::version::v9::PROTOCOL_VERSION_9;
use eframe::epaint::{Color32, Margin};
use egui::{Context, Frame, ImageButton, SidePanel, TextureHandle};
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

    let panel_width = 50.0 + 20.0; // Button width (50) + 10px margin on each side (20 total)

    SidePanel::left("left_panel")
        .default_width(panel_width)
        .frame(
            Frame::new()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(Margin {
                    left: 10,
                    right: 10,
                    top: 10,
                    bottom: 0,
                }),
        )
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                for (label, screen_type, icon_path) in buttons.iter() {
                    if !check_root_screen_access(app_context, screen_type) {
                        continue; // Skip this button if access is denied
                    }
                    let texture: Option<TextureHandle> = load_icon(ctx, icon_path);
                    let is_selected = selected_screen == *screen_type;

                    let button_color = if is_selected {
                        Color32::from_rgb(100, 149, 237) // Highlighted blue color for selected
                    } else {
                        Color32::from_rgb(169, 169, 169) // Default grayish blue color for unselected
                    };

                    // Add icon-based button if texture is loaded
                    if let Some(ref texture) = texture {
                        let button = ImageButton::new(texture)
                            .frame(false) // Remove button frame
                            .tint(button_color);

                        if ui.add(button).clicked() {
                            action = AppAction::SetMainScreenThenGoToMainScreen(*screen_type);
                        }
                    } else {
                        // Fallback to a simple text button if texture loading fails
                        let button = egui::Button::new(*label)
                            .fill(button_color)
                            .min_size(egui::vec2(50.0, 50.0));

                        if ui.add(button).clicked() {
                            action = AppAction::SetMainScreen(*screen_type);
                        }
                    }

                    ui.add_space(10.0); // Add some space between buttons
                }

                // Push content to the top and dev label to the bottom
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if app_context.developer_mode.load(Ordering::Relaxed) {
                        ui.add_space(10.0);
                        if ui.label("Dev mode").clicked() {
                            action = AppAction::SetMainScreenThenGoToMainScreen(
                                RootScreenType::RootScreenNetworkChooser,
                            );
                        };
                    }
                });
            });
        });

    action
}

/// Checks if the user has access to the button based on the screen type.
fn check_root_screen_access(app_context: &Arc<AppContext>, screen_type: &RootScreenType) -> bool {
    let protocol_version = app_context.platform_version().protocol_version;

    // For RootScreenMyTokenBalances
    !matches!(screen_type, RootScreenType::RootScreenMyTokenBalances if protocol_version < PROTOCOL_VERSION_9)
}
