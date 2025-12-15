//! Import Private Key Screen - Import a private key directly

use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::ui::ScreenLike;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use eframe::egui::Context;
use egui::{Color32, Direction, Layout, RichText, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use zxcvbn::zxcvbn;

pub struct ImportPrivateKeyScreen {
    /// The private key input (hex or WIF format)
    private_key_input: String,
    /// Password for encrypting the key (optional)
    password: String,
    /// Alias for the wallet
    alias_input: String,
    /// Password strength indicator
    password_strength: f64,
    /// Estimated time to crack
    estimated_time_to_crack: String,
    /// Error message
    error: Option<String>,
    /// Parsed wallet (for preview)
    parsed_wallet: Option<SingleKeyWallet>,
    /// App context
    pub app_context: Arc<AppContext>,
}

impl ImportPrivateKeyScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            private_key_input: String::new(),
            password: String::new(),
            alias_input: String::new(),
            password_strength: 0.0,
            estimated_time_to_crack: String::new(),
            error: None,
            parsed_wallet: None,
            app_context: app_context.clone(),
        }
    }

    fn try_parse_key(&mut self) {
        let input = self.private_key_input.trim();
        if input.is_empty() {
            self.parsed_wallet = None;
            self.error = None;
            return;
        }

        // Try to parse as WIF first, then as hex
        let result = SingleKeyWallet::from_wif(input, None, None)
            .or_else(|_| SingleKeyWallet::from_hex(input, self.app_context.network, None, None));

        match result {
            Ok(wallet) => {
                self.parsed_wallet = Some(wallet);
                self.error = None;
            }
            Err(e) => {
                self.parsed_wallet = None;
                self.error = Some(format!("Invalid private key: {}", e));
            }
        }
    }

    fn save_wallet(&mut self) -> Result<AppAction, String> {
        let input = self.private_key_input.trim();
        if input.is_empty() {
            return Err("Please enter a private key".to_string());
        }

        // Parse the key with password and alias
        let password = if self.password.is_empty() {
            None
        } else {
            Some(self.password.as_str())
        };
        let alias = if self.alias_input.is_empty() {
            None
        } else {
            Some(self.alias_input.clone())
        };

        // Try WIF first, then hex
        let wallet = SingleKeyWallet::from_wif(input, password, alias.clone()).or_else(|_| {
            SingleKeyWallet::from_hex(input, self.app_context.network, password, alias)
        })?;

        let key_hash = wallet.key_hash();

        // Store in database
        self.app_context
            .db
            .store_single_key_wallet(&wallet, self.app_context.network)
            .map_err(|e| {
                if e.to_string().contains("UNIQUE constraint failed") {
                    "This key has already been imported.".to_string()
                } else {
                    e.to_string()
                }
            })?;

        // Add to app context
        let wallet_arc = Arc::new(RwLock::new(wallet));
        if let Ok(mut single_key_wallets) = self.app_context.single_key_wallets.write() {
            single_key_wallets.insert(key_hash, wallet_arc);
            self.app_context.has_wallet.store(true, Ordering::Relaxed);
        }

        Ok(AppAction::GoToMainScreen)
    }

    fn render_key_input(&mut self, ui: &mut Ui) {
        ui.add_space(15.0);

        ui.heading("1. Enter your private key (WIF or 64-character hex format)");
        ui.add_space(8.0);

        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let response = ui.add_sized(
            Vec2::new(ui.available_width() - 20.0, 40.0),
            egui::TextEdit::singleline(&mut self.private_key_input)
                .hint_text("Enter private key (WIF: 51-52 chars, or hex: 64 chars)")
                .text_color(crate::ui::theme::DashColors::text_primary(dark_mode))
                .background_color(crate::ui::theme::DashColors::input_background(dark_mode))
                .password(true),
        );

        if response.changed() {
            self.try_parse_key();
        }

        // Show parsed address preview
        if let Some(ref wallet) = self.parsed_wallet {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("Derived Address:");
                ui.label(
                    RichText::new(wallet.address.to_string())
                        .monospace()
                        .color(Color32::from_rgb(100, 200, 100)),
                );
            });
        }

        // Show error if any
        if let Some(ref err) = self.error {
            ui.add_space(5.0);
            ui.colored_label(Color32::from_rgb(255, 100, 100), err);
        }
    }

    fn render_alias_input(&mut self, ui: &mut Ui) {
        ui.add_space(20.0);
        ui.heading("2. Give this key a name (optional)");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("Alias:");
            ui.text_edit_singleline(&mut self.alias_input);
        });
    }

    fn render_password_input(&mut self, ui: &mut Ui) {
        ui.add_space(20.0);
        ui.heading("3. Add a password to encrypt this key (optional but recommended)");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("Password:");
            if ui.text_edit_singleline(&mut self.password).changed() {
                if !self.password.is_empty() {
                    let estimate = zxcvbn(&self.password, &[]);
                    let score_u8 = u8::from(estimate.score());
                    self.password_strength = score_u8 as f64 * 25.0;
                    let estimated_seconds =
                        estimate.crack_times().offline_slow_hashing_1e4_per_second();
                    self.estimated_time_to_crack = estimated_seconds.to_string();
                } else {
                    self.password_strength = 0.0;
                    self.estimated_time_to_crack = String::new();
                }
            }
        });

        if !self.password.is_empty() {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("Password Strength:");
                let strength_percentage = (self.password_strength / 100.0).min(1.0);
                let fill_color = match self.password_strength as i32 {
                    0..=25 => Color32::from_rgb(255, 182, 193),
                    26..=50 => Color32::from_rgb(255, 224, 130),
                    51..=75 => Color32::from_rgb(144, 238, 144),
                    _ => Color32::from_rgb(90, 200, 90),
                };
                ui.add(
                    egui::ProgressBar::new(strength_percentage as f32)
                        .desired_width(200.0)
                        .show_percentage()
                        .text(match self.password_strength as i32 {
                            0 => "None".to_string(),
                            1..=25 => "Very Weak".to_string(),
                            26..=50 => "Weak".to_string(),
                            51..=75 => "Strong".to_string(),
                            _ => "Very Strong".to_string(),
                        })
                        .fill(fill_color),
                );
            });

            ui.add_space(5.0);
            ui.label(format!(
                "Estimated time to crack: {}",
                self.estimated_time_to_crack
            ));
        }
    }
}

impl ScreenLike for ImportPrivateKeyScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Wallets", AppAction::GoToMainScreen),
                ("Import Private Key", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenWalletsBalances,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            egui::ScrollArea::both()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.add_space(10.0);
                    ui.heading("Import a single private key");
                    ui.add_space(5.0);
                    ui.label("This allows you to import a private key directly without using a HD wallet.");

                    self.render_key_input(ui);

                    if self.parsed_wallet.is_none() {
                        return;
                    }

                    self.render_alias_input(ui);
                    self.render_password_input(ui);

                    ui.add_space(30.0);
                    ui.heading("4. Save the key");
                    ui.add_space(10.0);

                    ui.with_layout(Layout::centered_and_justified(Direction::TopDown), |ui| {
                        let save_button = egui::Button::new(
                            RichText::new("Import Key").strong().size(30.0),
                        )
                        .min_size(Vec2::new(300.0, 60.0))
                        .corner_radius(10.0)
                        .stroke(Stroke::new(1.5, Color32::WHITE))
                        .sense(if self.parsed_wallet.is_some() {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        });

                        if ui.add(save_button).clicked() {
                            match self.save_wallet() {
                                Ok(save_action) => {
                                    inner_action = save_action;
                                }
                                Err(e) => {
                                    self.error = Some(e);
                                }
                            }
                        }
                    });
                });

            inner_action
        });

        action
    }
}
