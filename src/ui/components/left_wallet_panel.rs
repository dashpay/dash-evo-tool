use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::RootScreenType;
use crate::ui::theme::DashColors;
use eframe::epaint::Margin;
use egui::{Context, Frame, ImageButton, SidePanel, TextureHandle};
use rust_embed::RustEmbed;
use std::sync::Arc;
use tracing::error;

#[derive(RustEmbed)]
#[folder = "icons/"] // Adjust the folder path if necessary
struct Assets;

// Function to load an icon as a texture using embedded assets
#[allow(dead_code)]
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
            error!("Failed to load image from embedded data at path: {}", path);
            None
        }
    } else {
        error!("Image not found in embedded assets at path: {}", path);
        None
    }
}

#[allow(dead_code)]
pub fn add_left_panel(
    ctx: &Context,
    _app_context: &Arc<AppContext>,
    selected_screen: RootScreenType,
) -> AppAction {
    let mut action = AppAction::None;

    // Define the button details directly in this function
    let buttons = [
        ("I", RootScreenType::RootScreenIdentities, "identity.png"),
        (
            "C",
            RootScreenType::RootScreenDPNSActiveContests,
            "voting.png",
        ),
        ("Q", RootScreenType::RootScreenDocumentQuery, "doc.png"),
        (
            "T",
            RootScreenType::RootScreenToolsTransitionVisualizerScreen,
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
                    if *screen_type == RootScreenType::RootScreenDocumentQuery {
                        continue; // Skip rendering the document button for now
                    }

                    let texture: Option<TextureHandle> = load_icon(ctx, icon_path);
                    let is_selected = selected_screen == *screen_type;
                    let button_color = if is_selected {
                        DashColors::ICON_SELECTED_BLUE
                    } else {
                        DashColors::ICON_UNSELECTED
                    };

                    // Add icon-based button if texture is loaded
                    if let Some(ref texture) = texture {
                        let button = ImageButton::new(texture)
                            .frame(false) // Remove button frame
                            .tint(button_color);

                        if ui.add(button).clicked() {
                            action = AppAction::SetMainScreen(*screen_type);
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
            });
        });

    action
}
