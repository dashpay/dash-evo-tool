use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::user_role::UserRole;
use crate::ui::components::icons::load_svg_icon;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::components::styled::island_central_panel;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing, Typography};
use crate::ui::{MessageType, RootScreenType, ScreenType};
use egui::{Response, RichText, ScrollArea, Stroke, Vec2, WidgetInfo, WidgetType};
use std::sync::Arc;

const ROLE_OPTIONS: [UserRole; 3] = [UserRole::Everyday, UserRole::Power, UserRole::Developer];
const ONBOARDING_CARDS: [(OnboardingAction, &str, &str); 3] = [
    (
        OnboardingAction::CreateWallet,
        "Create Wallet",
        "Start fresh with a new HD wallet",
    ),
    (
        OnboardingAction::LoadWallet,
        "Import Wallet",
        "Load a wallet you already have",
    ),
    (
        OnboardingAction::JustBrowse,
        "Just Explore",
        "Explore without setting up",
    ),
];

const WELCOME_CARD_CONTENT_WIDTH: f32 = 170.0;
const ROLE_CARD_CONTENT_HEIGHT: f32 = 100.0;
const ACTION_CARD_CONTENT_HEIGHT: f32 = 60.0;

struct PreparedRoleCard {
    role: UserRole,
    frame: egui::containers::frame::Prepared,
    response: Response,
}

/// The action the user wants to take after onboarding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingAction {
    LoadWallet,
    CreateWallet,
    ImportIdentity,
    JustBrowse,
}

pub struct WelcomeScreen {
    pub app_context: Arc<AppContext>,
}

impl WelcomeScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = AppAction::None;
        let dark_mode = ctx.global_style().visuals.dark_mode;

        // Central panel with welcome content (using island style like other screens)
        island_central_panel(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(80.0);

                        // Logo
                        if let Some(logo) = load_svg_icon(ctx, "dashlogo.svg", 200, 80) {
                            ui.add(
                                egui::Image::new(&logo).fit_to_exact_size(Vec2::new(150.0, 60.0)),
                            );
                        }

                        ui.add_space(24.0);

                        // Title
                        ui.label(
                            RichText::new("Welcome to Dash Evo Tool")
                                .size(28.0)
                                .strong()
                                .color(DashColors::text_primary(dark_mode)),
                        );

                        ui.add_space(8.0);

                        // Subtitle
                        ui.label(
                            RichText::new("Your gateway to decentralized data")
                                .size(16.0)
                                .color(DashColors::text_secondary(dark_mode)),
                        );

                        ui.add_space(40.0);

                        // Experience-level selector — sets the app-global role
                        // before the user picks an onboarding path.
                        self.render_role_selector(ui, dark_mode);

                        ui.add_space(32.0);

                        // Instructional text
                        ui.label(
                            RichText::new("Select an option to get started:")
                                .size(14.0)
                                .color(DashColors::text_secondary(dark_mode)),
                        );

                        ui.add_space(16.0);

                        // Getting Started section - cards directly trigger navigation
                        action |= self.render_getting_started_section(ui, dark_mode);

                        ui.add_space(40.0);
                    });
                });
        });

        action
    }

    /// Experience-level selector shown during onboarding. Writes the app-global
    /// role (runtime cell + persisted `AppSettings.user_role`) the moment the
    /// user changes it; the same value backs the Settings selector.
    fn render_role_selector(&self, ui: &mut egui::Ui, dark_mode: bool) {
        ui.label(
            RichText::new("Choose your experience level:")
                .font(Typography::body_small())
                .color(DashColors::text_secondary(dark_mode)),
        );

        ui.add_space(Spacing::SM);

        let mut role = self.app_context.user_role();
        let previous = role;
        let mut cards = Vec::with_capacity(ROLE_OPTIONS.len());

        render_adaptive_card_rows(
            ui,
            &ROLE_OPTIONS,
            ROLE_CARD_CONTENT_HEIGHT,
            |ui, option, width| {
                cards.push(self.prepare_role_card(ui, dark_mode, option, width));
            },
        );

        if let Some(card) = cards.iter().find(|card| card.response.clicked()) {
            role = card.role;
        }

        for mut card in cards {
            let selected = card.role == role;
            card.frame.frame.fill = if selected {
                DashColors::selected(dark_mode)
            } else {
                DashColors::background(dark_mode)
            };
            let stroke = if card.response.has_focus() {
                welcome_card_focus_stroke(dark_mode)
            } else if selected {
                Stroke::new(Shape::BORDER_WIDTH_THICK, DashColors::DASH_BLUE)
            } else {
                Stroke::new(Shape::BORDER_WIDTH, DashColors::border_light(dark_mode))
            };
            paint_welcome_card(ui, &mut card.frame, stroke);

            if card.response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            card.response.widget_info(|| {
                WidgetInfo::selected(WidgetType::RadioButton, true, selected, card.role.label())
            });
        }

        if role != previous {
            match self.app_context.set_and_persist_user_role(role) {
                // `role` is re-read from the context every frame, so a failed
                // selection reverts on its own — only the banner is needed.
                Ok(()) => ui.ctx().request_repaint(),
                Err(e) => {
                    MessageBanner::set_global(
                        ui.ctx(),
                        "Could not save your experience level. Select it again, or restart the application if the problem continues.",
                        MessageType::Error,
                    )
                    .with_details(e);
                }
            }
        }

        ui.add_space(Spacing::SM);
        ui.label(
            RichText::new("You can change this later in Network Settings.")
                .font(Typography::caption())
                .color(DashColors::text_secondary(dark_mode)),
        );
    }

    fn prepare_role_card(
        &self,
        ui: &mut egui::Ui,
        dark_mode: bool,
        role: UserRole,
        content_width: f32,
    ) -> PreparedRoleCard {
        let mut frame = egui::Frame::new()
            .stroke(Stroke::new(
                Shape::BORDER_WIDTH_THICK,
                egui::Color32::TRANSPARENT,
            ))
            .corner_radius(Shape::RADIUS_MD)
            .shadow(Shadow::small())
            .inner_margin(Spacing::CARD_PADDING)
            .begin(ui);

        frame
            .content_ui
            .set_min_size(Vec2::new(content_width, ROLE_CARD_CONTENT_HEIGHT));
        frame
            .content_ui
            .set_max_size(Vec2::new(content_width, ROLE_CARD_CONTENT_HEIGHT));

        frame.content_ui.vertical_centered(|ui| {
            ui.label(RichText::new(role.role_icon()).font(Typography::heading_medium()));

            ui.add_space(Spacing::XS);

            ui.label(
                RichText::new(role.label())
                    .font(Typography::body_small())
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );

            ui.add_space(Spacing::SM);

            ui.label(
                RichText::new(role.description())
                    .font(Typography::caption())
                    .color(DashColors::text_secondary(dark_mode)),
            );
        });

        let response = frame.allocate_space(ui).interact(egui::Sense::click());
        PreparedRoleCard {
            role,
            frame,
            response,
        }
    }

    fn render_getting_started_section(&mut self, ui: &mut egui::Ui, dark_mode: bool) -> AppAction {
        let mut action = AppAction::None;
        render_adaptive_card_rows(
            ui,
            &ONBOARDING_CARDS,
            ACTION_CARD_CONTENT_HEIGHT,
            |ui, (onboarding_action, title, description), width| {
                action |= self.render_action_card(
                    ui,
                    dark_mode,
                    onboarding_action,
                    title,
                    description,
                    width,
                );
            },
        );

        action
    }

    fn render_action_card(
        &self,
        ui: &mut egui::Ui,
        dark_mode: bool,
        onboarding_action: OnboardingAction,
        title: &str,
        description: &str,
        content_width: f32,
    ) -> AppAction {
        let bg_color = DashColors::background(dark_mode);
        let border_color = DashColors::border_light(dark_mode);

        let mut frame = egui::Frame::new()
            .fill(bg_color)
            .stroke(Stroke::new(
                Shape::BORDER_WIDTH_THICK,
                egui::Color32::TRANSPARENT,
            ))
            .corner_radius(Shape::RADIUS_MD)
            .shadow(Shadow::small())
            .inner_margin(Spacing::CARD_PADDING)
            .begin(ui);

        frame
            .content_ui
            .set_min_size(Vec2::new(content_width, ACTION_CARD_CONTENT_HEIGHT));
        frame
            .content_ui
            .set_max_size(Vec2::new(content_width, ACTION_CARD_CONTENT_HEIGHT));

        frame.content_ui.vertical_centered(|ui| {
            ui.add_space(Spacing::XS);

            ui.label(
                RichText::new(title)
                    .font(Typography::body_small())
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );

            ui.add_space(Spacing::SM);

            ui.label(
                RichText::new(description)
                    .font(Typography::caption())
                    .color(DashColors::text_secondary(dark_mode)),
            );
        });

        let response = frame.allocate_space(ui).interact(egui::Sense::click());
        let stroke = if response.has_focus() {
            welcome_card_focus_stroke(dark_mode)
        } else {
            Stroke::new(Shape::BORDER_WIDTH, border_color)
        };
        paint_welcome_card(ui, &mut frame, stroke);

        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        if response.clicked() {
            // Save settings to database
            let _ = self.app_context.update_onboarding_completed(true);

            // Return OnboardingComplete with navigation based on selection
            let (main_screen, add_screen) = match onboarding_action {
                OnboardingAction::CreateWallet => (
                    RootScreenType::RootScreenWalletsBalances,
                    Some(Box::new(ScreenType::AddNewWallet)),
                ),
                OnboardingAction::LoadWallet => (
                    RootScreenType::RootScreenWalletsBalances,
                    Some(Box::new(ScreenType::ImportMnemonic)),
                ),
                OnboardingAction::ImportIdentity => (RootScreenType::RootScreenIdentities, None),
                OnboardingAction::JustBrowse => (RootScreenType::RootScreenIdentityHub, None),
            };

            return AppAction::OnboardingComplete {
                main_screen,
                add_screen,
            };
        }

        AppAction::None
    }
}

fn render_adaptive_card_rows<T: Copy>(
    ui: &mut egui::Ui,
    cards: &[T],
    content_height: f32,
    mut render_card: impl FnMut(&mut egui::Ui, T, f32),
) {
    if cards.is_empty() {
        return;
    }

    let border_and_padding = Shape::BORDER_WIDTH_THICK + Spacing::CARD_PADDING;
    let standard_outer_width = WELCOME_CARD_CONTENT_WIDTH + (border_and_padding * 2.0);
    let available_width = ui.available_width().max(0.0);
    let columns = (((available_width + Spacing::MD) / (standard_outer_width + Spacing::MD)).floor()
        as usize)
        .clamp(1, cards.len());
    let content_width = if columns == 1 {
        WELCOME_CARD_CONTENT_WIDTH.min((available_width - (border_and_padding * 2.0)).max(0.0))
    } else {
        WELCOME_CARD_CONTENT_WIDTH
    };
    let outer_size = Vec2::new(
        content_width + (border_and_padding * 2.0),
        content_height + (border_and_padding * 2.0),
    );
    let row_count = cards.len().div_ceil(columns);

    for (row_index, row) in cards.chunks(columns).enumerate() {
        let row_width =
            (outer_size.x * row.len() as f32) + (Spacing::MD * row.len().saturating_sub(1) as f32);
        ui.allocate_ui(Vec2::new(row_width, outer_size.y), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = Spacing::MD;
                for card in row {
                    render_card(ui, *card, content_width);
                }
            });
        });

        if row_index + 1 < row_count {
            ui.add_space(Spacing::MD);
        }
    }
}

fn paint_welcome_card(
    ui: &egui::Ui,
    frame: &mut egui::containers::frame::Prepared,
    stroke: Stroke,
) {
    let rect = frame.frame.widget_rect(frame.content_ui.min_rect());
    frame.paint(ui);
    ui.painter()
        .rect_stroke(rect, Shape::RADIUS_MD, stroke, egui::StrokeKind::Inside);
}

fn welcome_card_focus_stroke(dark_mode: bool) -> Stroke {
    let color = if dark_mode {
        DashColors::WHITE
    } else {
        DashColors::DEEP_BLUE
    };
    Stroke::new(Shape::BORDER_WIDTH_THICK, color)
}
