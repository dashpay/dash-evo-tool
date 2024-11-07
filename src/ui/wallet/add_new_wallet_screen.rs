use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::ScreenLike;
use eframe::egui::Context;

use crate::model::wallet::{ClosedWalletSeed, OpenWalletSeed, Wallet, WalletSeed};
use crate::ui::components::entropy_grid::U256EntropyGrid;
use bip39::{Language, Mnemonic};
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::dashcore::bip32::{ExtendedPrivKey, ExtendedPubKey};
use egui::{
    Color32, ComboBox, Direction, FontId, Frame, Grid, Layout, Margin, RichText, Stroke, TextStyle,
    Ui, Vec2,
};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use zxcvbn::zxcvbn;

pub struct AddNewWalletScreen {
    seed_phrase: Option<Mnemonic>,
    password: String,
    entropy_grid: U256EntropyGrid,
    selected_language: Language,
    alias_input: String,
    wrote_it_down: bool,
    password_strength: f64,
    estimated_time_to_crack: String,
    pub app_context: Arc<AppContext>,
}

impl AddNewWalletScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            seed_phrase: None,
            password: String::new(),
            entropy_grid: U256EntropyGrid::new(),
            selected_language: Language::English,
            alias_input: String::new(),
            wrote_it_down: false,
            password_strength: 0.0,
            estimated_time_to_crack: "".to_string(),
            app_context: app_context.clone(),
        }
    }

    /// Generate a new seed phrase based on the selected language
    fn generate_seed_phrase(&mut self) {
        let mnemonic = Mnemonic::from_entropy_in(
            self.selected_language,
            &self.entropy_grid.random_number_with_user_input(),
        )
        .expect("Failed to generate mnemonic");
        self.seed_phrase = Some(mnemonic);
    }

    fn save_wallet(&mut self) -> AppAction {
        if let Some(mnemonic) = &self.seed_phrase {
            let seed = mnemonic.to_seed("");

            let (encrypted_seed, salt, nonce, uses_password) = if self.password.is_empty() {
                (seed.to_vec(), vec![], vec![], false)
            } else {
                // Encrypt the seed to obtain encrypted_seed, salt, and nonce
                let (encrypted_seed, salt, nonce) =
                    ClosedWalletSeed::encrypt_seed(&seed, self.password.as_str())
                        .expect("Encryption failed");
                (encrypted_seed, salt, nonce, true)
            };

            // Generate master ECDSA extended private key
            let master_ecdsa_extended_private_key =
                ExtendedPrivKey::new_master(self.app_context.network, &seed)
                    .expect("Failed to create master ECDSA extended private key");
            let secp = Secp256k1::new();
            let master_ecdsa_extended_public_key =
                ExtendedPubKey::from_priv(&secp, &master_ecdsa_extended_private_key);

            // Compute the seed hash
            let seed_hash = ClosedWalletSeed::compute_seed_hash(&seed);

            let wallet = Wallet {
                wallet_seed: WalletSeed::Open(OpenWalletSeed {
                    seed,
                    wallet_info: ClosedWalletSeed {
                        seed_hash,
                        encrypted_seed,
                        salt,
                        nonce,
                        password_hint: None, // Set a password hint if needed
                    },
                }),
                uses_password,
                master_ecdsa_extended_public_key,
                address_balances: Default::default(),
                known_addresses: Default::default(),
                watched_addresses: Default::default(),
                unused_asset_locks: Default::default(),
                alias: Some(self.alias_input.clone()),
                identities: Default::default(),
                utxos: Default::default(),
                is_main: true,
            };

            self.app_context
                .db
                .store_wallet(&wallet, &self.app_context.network)
                .ok();

            // Acquire a write lock and add the new wallet
            if let Ok(mut wallets) = self.app_context.wallets.write() {
                wallets.push(Arc::new(RwLock::new(wallet)));
                self.app_context.has_wallet.store(true, Ordering::Relaxed);
            } else {
                eprintln!("Failed to acquire write lock on wallets");
            }

            AppAction::GoToMainScreen // Navigate back to the main screen after saving
        } else {
            AppAction::None // No action if no seed phrase exists
        }
    }

    fn render_seed_phrase_input(&mut self, ui: &mut Ui) {
        ui.add_space(15.0); // Add spacing from the top
        ui.vertical(|ui| {
            // Allocate a full-width container to center align the elements
            let available_width = ui.available_width();

            ui.allocate_ui_with_layout(
                Vec2::new(available_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.horizontal(|ui| {
                        // Add spacing to align the combo box to the left of the center
                        let half_width = available_width / 2.0 - 400.0; // Adjust half-width with padding
                        ui.add_space(half_width);

                        let style = ui.style_mut();

                        // Customize text size for the ComboBox
                        style.text_styles.insert(
                            TextStyle::Button,          // Apply style to buttons (used in ComboBox entries)
                            FontId::proportional(24.0), // Set larger font size
                        );

                        ComboBox::from_label("")
                            .selected_text(format!("{:?}", self.selected_language))
                            .width(200.0)
                            .height(40.0)
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

                        // Add a spacer between the combo box and the generate button
                        ui.add_space(20.0); // Adjust the space between elements

                        let generate_button =
                            egui::Button::new(RichText::new("Generate").strong().size(24.0))
                                .min_size(Vec2::new(150.0, 30.0))
                                .rounding(5.0)
                                .stroke(Stroke::new(1.0, Color32::WHITE));

                        if ui.add(generate_button).clicked() {
                            self.generate_seed_phrase();
                        }
                    });
                },
            );

            ui.add_space(10.0);

            // Create a container with a fixed width (72% of the available width)
            let frame_width = available_width * 0.72;
            ui.allocate_ui_with_layout(
                Vec2::new(frame_width, 300.0), // Set width and height of the container
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    Frame::none()
                        .fill(Color32::WHITE)
                        .stroke(Stroke::new(1.0, Color32::BLACK))
                        .rounding(5.0)
                        .inner_margin(Margin::same(10.0))
                        .show(ui, |ui| {
                            let columns = 6;
                            let rows = 24 / columns;

                            // Calculate the size of each grid cell
                            let column_width = frame_width / columns as f32;
                            let row_height = 300.0 / rows as f32;

                            Grid::new("seed_phrase_grid")
                                .num_columns(columns)
                                .spacing((0.0, 0.0))
                                .min_col_width(column_width)
                                .min_row_height(row_height)
                                .show(ui, |ui| {
                                    if let Some(mnemonic) = &self.seed_phrase {
                                        for (i, word) in mnemonic.words().enumerate() {
                                            let word_text = RichText::new(word)
                                                .size(row_height * 0.5)
                                                .monospace();

                                            ui.with_layout(
                                                Layout::centered_and_justified(
                                                    Direction::LeftToRight,
                                                ),
                                                |ui| {
                                                    ui.label(word_text);
                                                },
                                            );

                                            if (i + 1) % columns == 0 {
                                                ui.end_row();
                                            }
                                        }
                                    } else {
                                        let word_text =
                                            RichText::new("Seed Phrase").size(40.0).monospace();

                                        ui.with_layout(
                                            Layout::centered_and_justified(Direction::LeftToRight),
                                            |ui| {
                                                ui.label(word_text);
                                            },
                                        );
                                    }
                                });
                        });
                },
            );
        });
    }
}

fn format_time(seconds: f64) -> String {
    let minute = 60.0;
    let hour = 60.0 * minute;
    let day = 24.0 * hour;
    let year = 365.25 * day;
    let century = 100.0 * year;
    let millennium = 10.0 * century;

    if seconds < minute {
        format!("{:.2} seconds", seconds)
    } else if seconds < hour {
        format!("{:.2} minutes", seconds / minute)
    } else if seconds < day {
        format!("{:.2} hours", seconds / hour)
    } else if seconds < year {
        format!("{:.2} days", seconds / day)
    } else if seconds < century {
        format!("{:.2} years", seconds / year)
    } else if seconds < millennium {
        format!("{:.2} centuries", seconds / century)
    } else {
        format!("More than a millennium")
    }
}

impl ScreenLike for AddNewWalletScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Create Wallet", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            // Add the scroll area to make the content scrollable
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2]) // Prevent shrinking when content is less than the available area
                .show(ui, |ui| {
                    ui.add_space(10.0);
                    ui.heading("Follow these steps to create your wallet!");
                    ui.add_space(5.0);

                    self.entropy_grid.ui(ui);

                    ui.add_space(5.0);

                    ui.heading("2. Select your desired seed phrase language and press \"Generate\".");
                    self.render_seed_phrase_input(ui);

                    ui.add_space(10.0);

                    ui.heading(
                        "3. Write down the passphrase on a piece of paper and put it somewhere secure.",
                    );

                    ui.add_space(10.0);

                    // Add "I wrote it down" checkbox
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.wrote_it_down, "I wrote it down");
                    });

                    ui.add_space(20.0);

                    ui.heading("4. Add a password that must be used to unlock the wallet. (Optional but Recommended)");

                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label("Optional Password:");
                        if ui.text_edit_singleline(&mut self.password).changed() {
                            if !self.password.is_empty() {
                                let estimate = zxcvbn(&self.password, &[]);

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
                        let color = match self.password_strength as i32 {
                            0..=25 => Color32::RED,
                            26..=50 => Color32::YELLOW,
                            51..=75 => Color32::LIGHT_GREEN,
                            _ => Color32::GREEN,
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
                                .fill(color),
                        );
                    });

                    ui.add_space(10.0);
                    ui.label(format!(
                        "Estimated time to crack: {}",
                        self.estimated_time_to_crack
                    ));

                    ui.add_space(20.0);

                    ui.heading("5. Save the wallet.");
                    ui.add_space(5.0);

                    // Centered "Save Wallet" button at the bottom
                    ui.with_layout(Layout::centered_and_justified(Direction::TopDown), |ui| {
                        let save_button = egui::Button::new(
                            RichText::new("Save Wallet").strong().size(30.0),
                        )
                            .min_size(Vec2::new(300.0, 60.0))
                            .rounding(10.0)
                            .stroke(Stroke::new(1.5, Color32::WHITE))
                            .sense(if self.wrote_it_down {
                                egui::Sense::click()
                            } else {
                                egui::Sense::hover()
                            });

                        if ui.add(save_button).clicked() {
                            action = self.save_wallet(); // Trigger the save action
                        }
                    });
                });
        });

        action
    }
}
