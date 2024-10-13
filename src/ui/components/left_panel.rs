use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::RootScreenType;
use eframe::epaint::{Color32, Margin};
use egui::{Context, Frame, ImageButton, SidePanel, TextureHandle};
use std::sync::Arc;

// Function to load an icon as a texture
fn load_icon(ctx: &Context, path: &str) -> Option<TextureHandle> {
    if let Ok(image) = image::open(path) {
        let size = [image.width() as usize, image.height() as usize];
        let rgba_image = image.into_rgba8();
        let pixels = rgba_image.into_raw();

        Some(ctx.load_texture(
            path,
            egui::ColorImage::from_rgba_unmultiplied(size, &pixels),
            Default::default(),
        ))
    } else {
        eprintln!("Failed to load icon at path: {}", path);
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
        (
            "I",
            RootScreenType::RootScreenIdentities,
            "icons/identity.png",
        ),
        (
            "C",
            RootScreenType::RootScreenDPNSContestedNames,
            "icons/voting.png",
        ),
        (
            "Q",
            RootScreenType::RootScreenDocumentQuery,
            "icons/tools.png",
        ),
        (
            "T",
            RootScreenType::RootScreenTransitionVisualizerScreen,
            "icons/tools.png",
        ),
        (
            "N",
            RootScreenType::RootScreenNetworkChooser,
            "icons/config.png",
        ),
    ];

    let panel_width = 50.0 + 20.0; // Button width (50) + 10px margin on each side (20 total)

    SidePanel::left("left_panel")
        .default_width(panel_width)
        .frame(
            Frame::none()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(Margin {
                    left: 10.0,
                    right: 10.0,
                    top: 10.0,
                    bottom: 0.0,
                }),
        )
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                for (label, screen_type, icon_path) in buttons.iter() {
                    let texture: Option<TextureHandle> = load_icon(ctx, icon_path);
                    let is_selected = selected_screen == *screen_type;
                    let button_color = if is_selected {
                        Color32::from_rgb(100, 149, 237) // Highlighted blue color for selected
                    } else {
                        Color32::from_rgb(169, 169, 169) // Default gray color for unselected
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
