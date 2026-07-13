use crate::app::AppAction;
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::user_role::UserRole;
use crate::ui::RootScreenType;
use crate::ui::components::icons::{load_icon, load_svg_icon};
use crate::ui::components::styled::GradientButton;
use crate::ui::theme::{DashColors, ResponseExt, Shadow, Shape, Spacing};
use dash_sdk::dashcore_rpc::dashcore::Network;
use eframe::epaint::Margin;
use egui::{Frame, Image, Panel, RichText, TextureHandle, Ui};
use egui_extras::{Size, StripBuilder};
use std::sync::Arc;

pub fn add_left_panel(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    selected_screen: RootScreenType,
) -> AppAction {
    let ctx = ui.ctx().clone();
    let ctx = &ctx;
    let mut action = AppAction::None;

    // Define the button details directly in this function.
    // The optional FeatureGate controls visibility — entries where the gate
    // evaluates to false are filtered out before rendering.
    // The button ordering below stays stable for existing users. The
    // Identities hub entry (design-spec §A.1) is inserted after the legacy
    // `Identities` entry below. Both legacy entries (Dashpay, Identities)
    // remain visible during the coexistence period.
    let legacy_buttons: &[(&str, RootScreenType, &str, Option<FeatureGate>)] = &[
        (
            "Dashpay",
            RootScreenType::RootScreenDashPayProfile,
            "dashpay.png",
            Some(FeatureGate::DashPay),
        ),
        (
            "Identities",
            RootScreenType::RootScreenIdentities,
            "identity.png",
            None,
        ),
        (
            "Contracts",
            RootScreenType::RootScreenDocumentQuery,
            "doc.png",
            None,
        ),
        (
            "Tokens",
            RootScreenType::RootScreenMyTokenBalances,
            "tokens.png",
            None,
        ),
        (
            "Wallets",
            RootScreenType::RootScreenWalletsBalances,
            "wallet.png",
            None,
        ),
        (
            "Tools",
            RootScreenType::RootScreenToolsPlatformInfoScreen,
            "tools.png",
            None,
        ),
        (
            "Settings",
            RootScreenType::RootScreenNetworkChooser,
            "config.png",
            None,
        ),
    ];

    // Build the final button list. The hub and Masternodes entries are inserted
    // directly after the legacy `Identities` entry so the identity-related
    // entries cluster together while the old ones stay clickable.
    let mut buttons: Vec<(&str, RootScreenType, &str, Option<FeatureGate>)> =
        Vec::with_capacity(legacy_buttons.len() + 2);
    for entry in legacy_buttons.iter() {
        buttons.push(*entry);
        if entry.1 == RootScreenType::RootScreenIdentities {
            buttons.push((
                "Identity Hub",
                RootScreenType::RootScreenIdentityHub,
                "identity.png",
                None,
            ));
            // Masternodes sits directly below the identity cluster (locked
            // decision #3), gated at the Power role — masternode operation is a
            // Power User activity. The nav item and route are both absent below
            // Power (the gate skip below drops the entry).
            // TODO: swap `voting.png` for a dedicated node/server glyph when one
            // is added to `icons/` (distinct from `identity.png`).
            buttons.push((
                "Masternodes",
                RootScreenType::RootScreenMasternodes,
                "voting.png",
                Some(FeatureGate::Masternodes),
            ));
        }
    }

    let dark_mode = ctx.global_style().visuals.dark_mode;

    Panel::left("left_panel")
        .min_size(140.0)
        .max_size(140.0)
        .resizable(false)
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::symmetric(10, 10)), // Add margins for island effect
        )
        .show(ui, |ui| {
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
                    if app_context.user_role().at_least(UserRole::Power) {
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
                                            for (label, screen_type, icon_path, gate) in buttons.iter() {
                                                // Skip entries whose feature gate is not available
                                                if let Some(gate) = gate
                                                    && !gate.is_available(app_context)
                                                {
                                                    continue;
                                                }
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
                                                            | RootScreenType::RootScreenToolsTransitionVisualizerScreen
                                                            | RootScreenType::RootScreenToolsDocumentVisualizerScreen
                                                            | RootScreenType::RootScreenToolsProofVisualizerScreen
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

                                        // Expert label (below network label if present)
                                        if app_context.user_role().at_least(UserRole::Power) {
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
