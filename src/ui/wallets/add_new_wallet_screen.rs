use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::model::wallet::alias::validate_optional_alias;
use crate::ui::components::entropy_grid::U256EntropyGrid;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::helpers::{ModalOpeningGuard, clicked_outside_window_after_open};
use crate::ui::identities::add_new_identity_screen::AddNewIdentityScreen;
use crate::ui::identities::funding_common::generate_qr_code_image;
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::ui::wallets::alias_input::render_optional_alias_input;
use crate::ui::{RootScreenType, Screen, ScreenLike};
use bip39::{Language, Mnemonic};
use dash_sdk::dpp::dashcore::Address;
use eframe::egui::{Context, TextureHandle, TextureOptions};
use eframe::emath::Align;
use egui::load::SizedTexture;
use egui::{ComboBox, Frame, Grid, Layout, Margin, RichText, Stroke, Ui, Vec2};
use std::sync::Arc;
use zxcvbn::zxcvbn;

/// Word count options for BIP39 mnemonic seed phrases
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordCount {
    Words12 = 12,
    Words15 = 15,
    Words18 = 18,
    Words21 = 21,
    Words24 = 24,
}

impl WordCount {
    /// Returns the number of entropy bytes required for this word count
    pub fn entropy_bytes(&self) -> usize {
        match self {
            WordCount::Words12 => 16, // 128 bits
            WordCount::Words15 => 20, // 160 bits
            WordCount::Words18 => 24, // 192 bits
            WordCount::Words21 => 28, // 224 bits
            WordCount::Words24 => 32, // 256 bits
        }
    }

    /// Returns the word count as a number
    pub fn count(&self) -> usize {
        *self as usize
    }
}

pub struct AddNewWalletScreen {
    seed_phrase: Option<Mnemonic>,
    password_input: PasswordInput,
    entropy_grid: U256EntropyGrid,
    selected_language: Language,
    selected_word_count: WordCount,
    alias_input: String,
    wrote_it_down: bool,
    password_strength: f64,
    estimated_time_to_crack: String,
    error: Option<String>,
    pub app_context: Arc<AppContext>,
    wallet_created: bool,
    // Success screen state
    created_wallet_seed_hash: Option<[u8; 32]>,
    receive_address: Option<Address>,
    receive_address_string: Option<String>,
    receive_qr_texture: Option<TextureHandle>,
    show_receive_popup: bool,
    receive_popup_opening_guard: ModalOpeningGuard,
    funds_received: bool,
}

impl AddNewWalletScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            seed_phrase: None,
            password_input: PasswordInput::new().with_hint_text("Optional password"),
            entropy_grid: U256EntropyGrid::new(),
            selected_language: Language::English,
            selected_word_count: WordCount::Words24, // Default to 24 words for maximum security
            alias_input: String::new(),
            wrote_it_down: false,
            password_strength: 0.0,
            estimated_time_to_crack: "".to_string(),
            error: None,
            app_context: app_context.clone(),
            wallet_created: false,
            created_wallet_seed_hash: None,
            receive_address: None,
            receive_address_string: None,
            receive_qr_texture: None,
            show_receive_popup: false,
            receive_popup_opening_guard: ModalOpeningGuard::default(),
            funds_received: false,
        }
    }

    /// Generate a new seed phrase based on the selected language and word count
    fn generate_seed_phrase(&mut self) {
        let full_entropy = self.entropy_grid.random_number_with_user_input();
        let entropy_bytes = self.selected_word_count.entropy_bytes();

        // Use only the required number of bytes for the selected word count
        let mnemonic =
            Mnemonic::from_entropy_in(self.selected_language, &full_entropy[..entropy_bytes])
                .expect("Failed to generate mnemonic");
        self.seed_phrase = Some(mnemonic);
    }

    fn resolve_wallet_alias(&self) -> Result<String, String> {
        let existing_wallet_count = self
            .app_context
            .wallets
            .read()
            .map(|w| w.len())
            .unwrap_or(0);
        resolve_wallet_alias(&self.alias_input, existing_wallet_count)
    }

    fn save_wallet(&mut self) -> Result<AppAction, String> {
        if let Some(mnemonic) = &self.seed_phrase {
            let seed = mnemonic.to_seed("");

            let password = if self.password_input.is_empty() {
                None
            } else {
                Some(self.password_input.secret().clone())
            };

            // Generate default wallet name if none provided
            let wallet_alias = self.resolve_wallet_alias()?;

            let wallet = Wallet::new_from_seed(
                seed,
                self.app_context.network,
                Some(wallet_alias),
                password.as_ref(),
            )
            .map_err(|e| e.to_string())?;

            // Extract first receive address for display before registering
            if let Some((address, _)) = wallet.known_addresses.first_key_value() {
                self.receive_address_string = Some(address.to_string());
                self.receive_address = Some(address.clone());
            }

            let (new_wallet_seed_hash, _wallet_arc) = self
                .app_context
                .register_wallet(
                    wallet,
                    &seed,
                    crate::model::wallet::birth_height::WalletOrigin::Fresh,
                )
                .map_err(|e| e.to_string())?;

            // Set pending wallet selection so the wallet screen auto-selects this wallet
            if let Ok(mut pending) = self.app_context.pending_wallet_selection.lock() {
                *pending = Some(new_wallet_seed_hash);
            }

            self.created_wallet_seed_hash = Some(new_wallet_seed_hash);
            self.wallet_created = true;
            Ok(AppAction::None) // Show success screen instead of navigating away
        } else {
            Ok(AppAction::None) // No action if no seed phrase exists
        }
    }

    fn show_success(&mut self, ui: &mut Ui, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;

        // Check for incoming funds via the display-only WalletBackend snapshot.
        if !self.funds_received {
            if let Some(seed_hash) = &self.created_wallet_seed_hash
                && self.app_context.snapshot_balance(seed_hash).total > 0
            {
                self.funds_received = true;
                // Auto-close the popup when funds are received
                self.show_receive_popup = false;
            }

            // Request periodic repaint while waiting for funds
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(1));
        }

        ui.vertical_centered(|ui| {
            ui.add_space(50.0);
            ui.heading("🎉");
            if self.funds_received {
                ui.heading("Funds Received!");
            } else {
                ui.heading("Wallet Created Successfully!");
            }

            ui.add_space(30.0);

            // Recommended Next Steps section
            let description_width = 500.0_f32.min(ui.available_width() - 40.0);
            ui.allocate_ui_with_layout(
                Vec2::new(description_width, 0.0),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.label(
                        RichText::new("Recommended Next Steps:")
                            .size(16.0)
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(12.0);

                    // Step 1: Fund wallet
                    ui.horizontal(|ui| {
                        let step_color = if self.funds_received {
                            DashColors::success_color(dark_mode)
                        } else {
                            DashColors::text_secondary(dark_mode)
                        };
                        ui.label(
                            RichText::new("1.")
                                .size(14.0)
                                .strong()
                                .color(step_color),
                        );
                        let step_text = if self.funds_received {
                            "Fund your wallet with Dash (Done)"
                        } else {
                            "Fund your wallet with Dash"
                        };
                        ui.label(
                            RichText::new(step_text)
                                .size(14.0)
                                .color(step_color),
                        );
                    });
                    ui.add_space(4.0);

                    // Step 2: Create identity
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("2.")
                                .size(14.0)
                                .strong()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        ui.label(
                            RichText::new("Create a Platform Identity to register a username and interact with apps")
                                .size(14.0)
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    });
                },
            );

            ui.add_space(20.0);

            // Buttons
            if !self.funds_received {
                if ui.button("Fund Wallet").clicked() {
                    self.show_receive_popup = true;
                    self.receive_popup_opening_guard.arm();
                }
                ui.add_space(8.0);
            }

            if ui.button("Create Platform Identity").clicked() {
                action = AppAction::PopThenAddScreenToMainScreen(
                    RootScreenType::RootScreenIdentities,
                    Screen::AddNewIdentityScreen(AddNewIdentityScreen::new_with_wallet(
                        &self.app_context,
                        self.created_wallet_seed_hash,
                    )),
                );
            }

            ui.add_space(8.0);

            if ui.button("Go To Wallet Screen").clicked() {
                action = AppAction::GoToMainScreen;
            }

            ui.add_space(40.0);
        });

        // Render receive popup
        action |= self.render_receive_popup(ctx);

        action
    }

    fn render_receive_popup(&mut self, ctx: &Context) -> AppAction {
        if !self.show_receive_popup {
            return AppAction::None;
        }

        // Draw dark overlay behind the dialog
        let screen_rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("receive_funds_overlay"),
        ));
        painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());

        // Generate QR code if needed
        let mut qr_error: Option<String> = None;
        if let Some(address) = &self.receive_address_string
            && self.receive_qr_texture.is_none()
        {
            match generate_qr_code_image(address) {
                Ok(image) => {
                    let texture = ctx.load_texture(
                        format!("wallet_receive_{}", address),
                        image,
                        TextureOptions::LINEAR,
                    );
                    self.receive_qr_texture = Some(texture);
                }
                Err(e) => {
                    qr_error = Some(format!("QR error: {:?}", e));
                }
            }
        }

        let mut open = self.show_receive_popup;
        let window_response = egui::Window::new("Fund Wallet")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(texture) = &self.receive_qr_texture {
                        ui.image(SizedTexture::new(texture.id(), egui::vec2(220.0, 220.0)));
                    } else if let Some(err) = &qr_error {
                        ui.label(err);
                    } else if self.receive_address_string.is_none() {
                        ui.label("No receive address available");
                    } else {
                        ui.label("Generating QR code...");
                    }

                    ui.add_space(8.0);

                    if let Some(address) = &self.receive_address_string {
                        ui.label(address);
                        ui.add_space(4.0);
                        if ComponentStyles::add_primary_button(ui, "Copy Address").clicked()
                            && let Err(err) = crate::ui::helpers::copy_text_to_clipboard(address)
                        {
                            tracing::warn!("Failed to copy address: {}", err);
                        }
                    }

                    ui.add_space(8.0);

                    ui.label("Waiting for funds...");
                });
            });

        if let Some(ref resp) = window_response
            && clicked_outside_window_after_open(
                ctx,
                resp.response.rect,
                &mut self.receive_popup_opening_guard,
            )
        {
            open = false;
        }

        self.show_receive_popup = open;
        AppAction::None
    }

    fn render_seed_phrase_input(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        let surface = DashColors::surface(dark_mode);
        let border = DashColors::border(dark_mode);
        let text_primary = DashColors::text_primary(dark_mode);
        let text_secondary = DashColors::text_secondary(dark_mode);

        ui.add_space(15.0); // Add spacing from the top
        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
            ui.add_space(-6.0);
            // Language and word count selectors with generate button
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.add_space(7.0);
                    ui.label("Language:");
                });

                ui.vertical(|ui| {
                    ComboBox::from_id_salt("language_selector")
                        .selected_text(format!("{:?}", self.selected_language))
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.selected_language,
                                Language::English,
                                "English",
                            );
                            ui.selectable_value(
                                &mut self.selected_language,
                                Language::Spanish,
                                "Spanish",
                            );
                            ui.selectable_value(
                                &mut self.selected_language,
                                Language::French,
                                "French",
                            );
                            ui.selectable_value(
                                &mut self.selected_language,
                                Language::Italian,
                                "Italian",
                            );
                            ui.selectable_value(
                                &mut self.selected_language,
                                Language::Portuguese,
                                "Portuguese",
                            );
                        });
                });

                ui.add_space(10.0);

                ui.vertical(|ui| {
                    ui.add_space(7.0);
                    ui.label("Word Count:");
                });

                ui.vertical(|ui| {
                    ComboBox::from_id_salt("word_count_selector")
                        .selected_text(format!("{} words", self.selected_word_count.count()))
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.selected_word_count,
                                WordCount::Words12,
                                "12 words",
                            );
                            ui.selectable_value(
                                &mut self.selected_word_count,
                                WordCount::Words15,
                                "15 words",
                            );
                            ui.selectable_value(
                                &mut self.selected_word_count,
                                WordCount::Words18,
                                "18 words",
                            );
                            ui.selectable_value(
                                &mut self.selected_word_count,
                                WordCount::Words21,
                                "21 words",
                            );
                            ui.selectable_value(
                                &mut self.selected_word_count,
                                WordCount::Words24,
                                "24 words",
                            );
                        });
                });

                ui.add_space(10.0);

                if ComponentStyles::add_primary_button(ui, "Generate").clicked() {
                    self.generate_seed_phrase();
                }
            });

            // Only show the seed phrase box after generation
            if let Some(mnemonic) = &self.seed_phrase {
                ui.add_space(10.0);

                // Calculate grid dimensions based on word count
                let word_count = mnemonic.word_count();
                let columns = if word_count <= 12 { 3 } else { 4 };
                let rows = word_count.div_ceil(columns); // Ceiling division

                // Create a container with a fixed width (limited to 600px max to prevent overflow)
                let available_width = ui.available_width();
                let frame_width = (available_width * 0.65).min(600.0);
                let frame_height = (rows as f32 * 40.0).max(120.0); // Dynamic height based on rows

                ui.allocate_ui_with_layout(
                    Vec2::new(frame_width, frame_height + 20.0), // Set width and height of the container
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        Frame::new()
                            .fill(surface)
                            .stroke(Stroke::new(1.0, border))
                            .corner_radius(5.0)
                            .inner_margin(Margin::same(10))
                            .show(ui, |ui| {
                                // Calculate the size of each grid cell with padding
                                let column_width = (frame_width - 20.0) / columns as f32; // Account for inner margin
                                let row_height = frame_height / rows as f32;

                                Grid::new("seed_phrase_grid")
                                    .num_columns(columns)
                                    .spacing((0.0, 0.0))
                                    .min_col_width(column_width)
                                    .min_row_height(row_height)
                                    .show(ui, |ui| {
                                        for (i, word) in mnemonic.words().enumerate() {
                                            let number_text = RichText::new(format!("{} ", i + 1))
                                                .size(row_height * 0.3)
                                                .color(text_secondary);

                                            let word_text = RichText::new(word)
                                                .size(row_height * 0.5)
                                                .color(text_primary);

                                            ui.with_layout(
                                                Layout::left_to_right(Align::Min),
                                                |ui| {
                                                    ui.label(number_text); // Add the number with the vertical offset
                                                    ui.label(word_text); // Add the word
                                                },
                                            );

                                            if (i + 1) % columns == 0 {
                                                ui.end_row();
                                            }
                                        }
                                    });
                            });
                    },
                );
            }
        });
    }
}

impl ScreenLike for AddNewWalletScreen {
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let pending_action = AppAction::None;

        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Wallets", AppAction::GoToMainScreen),
                ("Create Wallet", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenWalletsBalances,
        );

        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;
            let ctx = ui.ctx().clone();

            // Show success screen if wallet was created
            if self.wallet_created {
                inner_action = self.show_success(ui, &ctx);
                return inner_action;
            }

            // Add the scroll area to make the content scrollable both vertically and horizontally
            egui::ScrollArea::both()
                .auto_shrink([false; 2]) // Prevent shrinking when content is less than the available area
                .show(ui, |ui| {
                    ui.add_space(10.0);
                    ui.heading("Follow these steps to create your wallet.");
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(5.0);

                    self.entropy_grid.ui(ui);

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(5.0);

                    ui.heading("2. Select your desired seed phrase language and word count and press \"Generate\".");
                    self.render_seed_phrase_input(ui);

                    if self.seed_phrase.is_none() {
                        return;
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading(
                        "3. Write down the passphrase on a piece of paper and put it somewhere secure.",
                    );

                    ui.add_space(10.0);

                    // Add "I wrote it down" checkbox
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.wrote_it_down, "I wrote it down");
                    });

                    if !self.wrote_it_down {
                        return;
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading("4. Enter a wallet name to remember it by. (This will not go on the blockchain)");

                    ui.add_space(8.0);

                    render_optional_alias_input(ui, "Wallet Name:", &mut self.alias_input);

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading("5. Add a password that must be used to unlock the wallet. (Optional but recommended)");

                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label("Optional Password:");
                        let pw_response = self.password_input.show(ui);
                        if pw_response.changed {
                            if !self.password_input.is_empty() {
                                let estimate = zxcvbn(self.password_input.text(), &[]);

                                // Convert Score to u8
                                let score_u8 = u8::from(estimate.score());

                                // Use the score to determine password strength percentage
                                self.password_strength = score_u8 as f64 * 25.0; // Since score ranges from 0 to 4

                                // Get the estimated crack time in seconds
                                let estimated_seconds = estimate.crack_times().offline_slow_hashing_1e4_per_second();

                                // Format the estimated time to a human-readable string
                                self.estimated_time_to_crack = estimated_seconds.to_string();
                            } else {
                                self.password_strength = 0.0;
                                self.estimated_time_to_crack = String::new();
                            }
                        }
                    });

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label("Password Strength:");

                        // Since score ranges from 0 to 4, adjust percentage accordingly
                        let strength_percentage = (self.password_strength / 100.0).min(1.0);
                        let fill_color = match self.password_strength as i32 {
                            0..=25 => DashColors::STRENGTH_WEAK,
                            26..=50 => DashColors::STRENGTH_FAIR,
                            51..=75 => DashColors::STRENGTH_GOOD,
                            _ => DashColors::STRENGTH_STRONG,
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

                    ui.add_space(10.0);
                    ui.label(format!(
                        "Estimated time to crack: {}",
                        self.estimated_time_to_crack
                    ));

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading("6. Save the wallet.");
                    ui.add_space(10.0);

                    // Save Wallet button styled like Load Identity button
                    let mut new_style = (**ui.style()).clone();
                    new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                    ui.set_style(new_style);
                    if ComponentStyles::add_primary_button(ui, "Save Wallet").clicked() {
                        match self.save_wallet() {
                            Ok(save_wallet_action) => {
                                inner_action = save_wallet_action;
                            }
                            Err(e) => {
                                self.error = Some(e)
                            }
                        }
                    }
                });

            inner_action
        });

        // Display error popup if there's an error
        if let Some(error_message) = self.error.as_ref() {
            let error_message = error_message.clone();
            egui::Window::new("Error")
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(error_message);
                    ui.add_space(10.0);
                    let dark_mode = ui.style().visuals.dark_mode;
                    if ComponentStyles::add_secondary_button(ui, "Close", dark_mode).clicked() {
                        self.error = None;
                    }
                });
        }

        action |= pending_action;
        action
    }
}

fn resolve_wallet_alias(alias_input: &str, existing_wallet_count: usize) -> Result<String, String> {
    if let Some(alias) = validate_optional_alias(alias_input).map_err(|e| e.to_string())? {
        Ok(alias.to_string())
    } else {
        Ok(format!("Wallet {}", existing_wallet_count + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::{AddNewWalletScreen, resolve_wallet_alias};
    use crate::app_dir::ensure_env_file;
    use crate::context::AppContext;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::wallet::alias::MAX_CHARS;
    use crate::utils::tasks::TaskManager;
    use bip39::Mnemonic;
    use dash_sdk::dpp::dashcore::Network;
    use std::sync::Arc;

    fn offline_ctx() -> (Arc<AppContext>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext::new");
        (ctx, temp_dir)
    }

    fn test_mnemonic() -> Mnemonic {
        Mnemonic::parse_normalized(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .expect("mnemonic should parse")
    }

    #[test]
    fn resolve_wallet_alias_falls_back_for_whitespace_only_input() {
        let alias = resolve_wallet_alias("   \n\t  ", 2).expect("alias should resolve");
        assert_eq!(alias, "Wallet 3");
    }

    #[test]
    fn resolve_wallet_alias_preserves_invalid_input_on_validation_error() {
        let raw = "a".repeat(MAX_CHARS + 1);
        let err = resolve_wallet_alias(&raw, 0).expect_err("alias should be rejected");

        assert!(err.contains("Name is"));
        assert_eq!(raw.chars().count(), MAX_CHARS + 1);
    }

    #[tokio::test]
    async fn save_wallet_uses_default_name_for_whitespace_only_alias() {
        let (app_context, _tmp) = offline_ctx();
        let mnemonic = test_mnemonic();
        let expected_seed_hash = {
            use crate::model::wallet::ClosedKeyItem;
            ClosedKeyItem::compute_seed_hash(&mnemonic.to_seed(""))
        };
        let mut screen = AddNewWalletScreen::new(&app_context);
        screen.seed_phrase = Some(mnemonic);
        screen.alias_input = "  \n\t ".to_string();

        screen.save_wallet().expect("save should succeed");

        let expected_wallet = app_context
            .wallets
            .read()
            .unwrap()
            .values()
            .find(|wallet| wallet.read().unwrap().seed_hash() == expected_seed_hash)
            .cloned()
            .expect("wallet should be registered");
        assert_eq!(
            expected_wallet.read().unwrap().alias.as_deref(),
            Some("Wallet 1")
        );
    }

    #[test]
    fn save_wallet_rejects_over_limit_alias_without_registering_wallet() {
        let (app_context, _tmp) = offline_ctx();
        let mut screen = AddNewWalletScreen::new(&app_context);
        screen.seed_phrase = Some(test_mnemonic());
        screen.alias_input = "a".repeat(MAX_CHARS + 1);

        let err = screen
            .save_wallet()
            .expect_err("over-limit alias should be rejected");

        assert!(err.contains("Name is"));
        assert!(app_context.wallets.read().unwrap().is_empty());
    }
}
