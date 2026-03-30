use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::RootScreenType;
use crate::ui::components::styled::GradientButton;
use crate::ui::theme::{DashColors, ResponseExt, Shadow, Shape, Spacing};
use dash_sdk::dashcore_rpc::dashcore::Network;
use eframe::epaint::Margin;
use egui::{Context, Frame, Image, RichText, SidePanel, TextureHandle};
use egui_extras::{Size, StripBuilder};
use rust_embed::RustEmbed;
use std::sync::Arc;
use tracing::error;

#[derive(RustEmbed)]
#[folder = "icons/"] // Folder containing embedded assets
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
                    error!("Failed to load image from embedded data at path: {}", path);
                    None
                }
            } else {
                error!("Image not found in embedded assets at path: {}", path);
                None
            }
        })
}

// Function to load an SVG as a texture with specified dimensions
pub fn load_svg_icon(ctx: &Context, path: &str, width: u32, height: u32) -> Option<TextureHandle> {
    let cache_key = format!("{}_{}_{}", path, width, height);
    // Use ctx.data_mut to check if texture is already cached
    ctx.data_mut(|d| d.get_temp::<TextureHandle>(egui::Id::new(&cache_key)))
        .or_else(|| {
            // Only do expensive operations if texture is not cached
            if let Some(content) = Assets::get(path) {
                // Parse SVG
                let options = resvg::usvg::Options::default();
                let tree = match resvg::usvg::Tree::from_data(&content.data, &options) {
                    Ok(tree) => tree,
                    Err(e) => {
                        error!("Failed to parse SVG at {}: {}", path, e);
                        return None;
                    }
                };

                // Create a pixmap to render into
                let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;

                // Calculate scale to fit the SVG into the desired dimensions
                let svg_size = tree.size();
                let scale_x = width as f32 / svg_size.width();
                let scale_y = height as f32 / svg_size.height();
                let scale = scale_x.min(scale_y);

                // Center the SVG
                let offset_x = (width as f32 - svg_size.width() * scale) / 2.0;
                let offset_y = (height as f32 - svg_size.height() * scale) / 2.0;

                let transform = resvg::tiny_skia::Transform::from_scale(scale, scale)
                    .post_translate(offset_x, offset_y);

                // Render the SVG
                resvg::render(&tree, transform, &mut pixmap.as_mut());

                // Convert to egui texture
                let pixels = pixmap.data().to_vec();
                let texture = ctx.load_texture(
                    &cache_key,
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &pixels,
                    ),
                    egui::TextureOptions::LINEAR,
                );

                // Cache the texture
                ctx.data_mut(|d| d.insert_temp(egui::Id::new(&cache_key), texture.clone()));

                Some(texture)
            } else {
                error!("SVG not found in embedded assets at path: {}", path);
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
        (
            "Dashpay",
            RootScreenType::RootScreenDashPayProfile,
            "dashpay.png",
        ),
        (
            "Identities",
            RootScreenType::RootScreenIdentities,
            "identity.png",
        ),
        (
            "Contracts",
            RootScreenType::RootScreenDocumentQuery,
            "doc.png",
        ),
        (
            "Tokens",
            RootScreenType::RootScreenMyTokenBalances,
            "tokens.png",
        ),
        (
            "Wallets",
            RootScreenType::RootScreenWalletsBalances,
            "wallet.png",
        ),
        (
            "Tools",
            RootScreenType::RootScreenToolsPlatformInfoScreen,
            "tools.png",
        ),
        (
            "Settings",
            RootScreenType::RootScreenNetworkChooser,
            "config.png",
        ),
    ];

    let dark_mode = ctx.style().visuals.dark_mode;

    SidePanel::left("left_panel")
        .min_width(140.0)
        .max_width(140.0)
        .resizable(false)
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
                    // Reserve a fixed area at the bottom for the logo and labels,
                    // and make the button list above it vertically scrollable.
                    let mut bottom_reserved = Spacing::SM + 20.0; // spacing + logo height
                    if app_context.network != Network::Mainnet {
                        bottom_reserved += 22.0; // network label + spacing
                    }
                    if app_context.is_developer_mode() {
                        bottom_reserved += 2.0 + 16.0; // dev label area (spacing + label height)
                    }

                    StripBuilder::new(ui)
                        .size(Size::remainder()) // top: fills remaining height
                        .size(Size::exact(bottom_reserved.max(40.0))) // bottom: reserved area
                        .vertical(|mut strip| {
                            // Top cell: scrollable list of buttons
                            strip.cell(|ui| {
                                egui::ScrollArea::vertical()
                                    .id_salt("left_panel_buttons_scroll")
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        ui.vertical_centered(|ui| {
                                            for (label, screen_type, icon_path) in buttons.iter() {
                                                let texture: Option<TextureHandle> = load_icon(ctx, icon_path);
                                                // Check if this button's category is selected
                                                let is_selected = match *screen_type {
                                                    // DashPay: check if any DashPay subscreen is selected
                                                    RootScreenType::RootScreenDashPayProfile => matches!(
                                                        selected_screen,
                                                        RootScreenType::RootScreenDashpay
                                                            | RootScreenType::RootScreenDashPayProfile
                                                            | RootScreenType::RootScreenDashPayContacts
                                                            | RootScreenType::RootScreenDashPayPayments
                                                            | RootScreenType::RootScreenDashPayProfileSearch
                                                    ),
                                                    // Tokens: check if any Tokens subscreen is selected
                                                    RootScreenType::RootScreenMyTokenBalances => matches!(
                                                        selected_screen,
                                                        RootScreenType::RootScreenMyTokenBalances
                                                            | RootScreenType::RootScreenTokenSearch
                                                            | RootScreenType::RootScreenTokenCreator
                                                    ),
                                                    // Tools: check if any Tools subscreen is selected
                                                    RootScreenType::RootScreenToolsPlatformInfoScreen => matches!(
                                                        selected_screen,
                                                        RootScreenType::RootScreenToolsPlatformInfoScreen
                                                            | RootScreenType::RootScreenToolsProofLogScreen
                                                            | RootScreenType::RootScreenToolsTransitionVisualizerScreen
                                                            | RootScreenType::RootScreenToolsDocumentVisualizerScreen
                                                            | RootScreenType::RootScreenToolsProofVisualizerScreen
                                                            | RootScreenType::RootScreenToolsMasternodeListDiffScreen
                                                            | RootScreenType::RootScreenToolsContractVisualizerScreen
                                                            | RootScreenType::RootScreenToolsGroveSTARKScreen
                                                            | RootScreenType::RootScreenToolsAddressBalanceScreen
                                                    ),
                                                    // Contracts: check if any Contracts/DPNS subscreen is selected
                                                    RootScreenType::RootScreenDocumentQuery => matches!(
                                                        selected_screen,
                                                        RootScreenType::RootScreenDocumentQuery
                                                            | RootScreenType::RootScreenDPNSActiveContests
                                                            | RootScreenType::RootScreenDPNSPastContests
                                                            | RootScreenType::RootScreenDPNSOwnedNames
                                                            | RootScreenType::RootScreenDPNSScheduledVotes
                                                    ),
                                                    // All other screens: exact match
                                                    _ => selected_screen == *screen_type,
                                                };

                                                let button_color = DashColors::icon_tint(is_selected, dark_mode);

                                                if let Some(ref texture) = texture {
                                                    let button = egui::Button::image(
                                                        Image::new(texture).tint(button_color),
                                                    )
                                                    .frame(false);

                                                    let added = ui.add(button);
                                                    // Provide an accessible name for the image-only button
                                                    added.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, is_selected, *label));
                                                    if added.clicked() {
                                                        action = AppAction::SetMainScreenThenGoToMainScreen(
                                                            *screen_type,
                                                        );
                                                    } else if added.hovered() {
                                                        ui.ctx().set_cursor_icon(
                                                            egui::CursorIcon::PointingHand,
                                                        );
                                                    }
                                                    // Put the label beneath the icon
                                                    let color = if is_selected {
                                                        DashColors::DASH_BLUE
                                                    } else {
                                                        DashColors::text_primary(dark_mode)
                                                    };
                                                    let label_text =
                                                        RichText::new(*label).color(color).size(13.0);
                                                    ui.label(label_text);
                                                } else {
                                                    // Fallback button if texture not available
                                                    if is_selected {
                                                        let response = GradientButton::new(*label, app_context)
                                                            .min_width(60.0)
                                                            .glow()
                                                            .show(ui);
                                                        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, is_selected, *label));
                                                        if response.clicked() {
                                                            action = AppAction::SetMainScreen(*screen_type);
                                                        }
                                                    } else {
                                                        let button = egui::Button::new(*label)
                                                            .fill(DashColors::glass_white(dark_mode))
                                                            .stroke(egui::Stroke::new(
                                                                1.0,
                                                                DashColors::glass_border(dark_mode),
                                                            ))
                                                            .corner_radius(egui::CornerRadius::same(
                                                                Shape::RADIUS_MD,
                                                            ))
                                                            .min_size(egui::vec2(60.0, 60.0));

                                                        let response = ui.add(button);
                                                        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, is_selected, *label));
                                                        if response.clicked() {
                                                            action = AppAction::SetMainScreen(*screen_type);
                                                        }
                                                    }
                                                }

                                                ui.add_space(Spacing::MD);
                                            }
                                        });
                                    });
                            });

                            // Bottom cell: always visible logo and labels
                            strip.cell(|ui| {
                                ui.with_layout(
                                    egui::Layout::bottom_up(egui::Align::Center),
                                    |ui| {
                                        // Dash logo at the very bottom
                                        // Use 100x40 for rendering (2x for crisp display), then scale down
                                        if let Some(dash_texture) = load_svg_icon(ctx, "dashlogo.svg", 100, 40) {
                                            if app_context.network == Network::Mainnet {
                                                ui.add_space(Spacing::SM);
                                            }
                                            let logo_size = egui::vec2(50.0, 20.0);
                                            let logo_response = ui.add(
                                                egui::Image::new(&dash_texture)
                                                    .fit_to_exact_size(logo_size)
                                                    .texture_options(egui::TextureOptions::LINEAR)
                                                    .sense(egui::Sense::click()),
                                            );

                                            if logo_response.clicked() {
                                                ui.ctx()
                                                    .open_url(egui::OpenUrl::new_tab("https://dash.org"));
                                            }

                                            if logo_response.hovered() {
                                                ui.ctx()
                                                    .set_cursor_icon(egui::CursorIcon::PointingHand);
                                            }
                                        }

                                        // Network label (if not on mainnet)
                                        if app_context.network != Network::Mainnet {
                                            let (network_name, network_color) = match app_context.network {
                                                Network::Testnet => (
                                                    "Testnet",
                                                    DashColors::TESTNET_ORANGE,
                                                ),
                                                Network::Devnet => (
                                                    "Devnet",
                                                    DashColors::DEVNET_RED,
                                                ),
                                                Network::Regtest => (
                                                    "Local Network",
                                                    DashColors::REGTEST_BROWN,
                                                ),
                                                _ => ("Unknown", DashColors::DASH_BLUE),
                                            };

                                            ui.add_space(2.0);
                                            ui.label(
                                                RichText::new(network_name)
                                                    .color(network_color)
                                                    .size(12.0)
                                                    .strong(),
                                            );
                                        }

                                        // Dev mode label (below network label if present)
                                        if app_context.is_developer_mode() {
                                            ui.add_space(5.0);
                                            let dev_label = egui::RichText::new("🔧 Expert")
                                                .color(DashColors::GRADIENT_PURPLE)
                                                .size(12.0);
                                            if ui.label(dev_label).clickable_tooltip("Expert mode is enabled — shows advanced options").clicked() {
                                                action = AppAction::SetMainScreenThenGoToMainScreen(
                                                    RootScreenType::RootScreenNetworkChooser,
                                                );
                                            }
                                        }
                                    },
                                );
                            });
                        });
                }); // Close the island frame
        });

    action
}
