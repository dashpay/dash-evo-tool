use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::ScreenLike;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use eframe::egui::Context;

use crate::model::wallet::encryption::{DASH_SECRET_MESSAGE, encrypt_message};
use crate::model::wallet::{ClosedKeyItem, OpenWalletSeed, Wallet, WalletSeed};
use crate::ui::wallets::add_new_wallet_screen::{
    DASH_BIP44_ACCOUNT_0_PATH_MAINNET, DASH_BIP44_ACCOUNT_0_PATH_TESTNET,
};
use bip39::Mnemonic;
use dash_sdk::dashcore_rpc::dashcore::bip32::DerivationPath;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::bip32::{ExtendedPrivKey, ExtendedPubKey};
use egui::{Color32, ComboBox, Direction, Grid, Layout, RichText, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use zxcvbn::zxcvbn;

pub struct ImportWalletScreen {
    seed_phrase_words: Vec<String>,
    selected_seed_phrase_length: usize,
    seed_phrase: Option<Mnemonic>,
    password: String,
    alias_input: String,
    password_strength: f64,
    estimated_time_to_crack: String,
    error: Option<String>,
    pub app_context: Arc<AppContext>,
    use_password_for_app: bool,
}

impl ImportWalletScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            seed_phrase_words: vec!["".to_string(); 24],
            selected_seed_phrase_length: 12,
            seed_phrase: None,
            password: String::new(),
            alias_input: String::new(),
            password_strength: 0.0,
            estimated_time_to_crack: "".to_string(),
            error: None,
            app_context: app_context.clone(),
            use_password_for_app: true,
        }
    }
    fn save_wallet(&mut self) -> Result<AppAction, String> {
        if let Some(mnemonic) = &self.seed_phrase {
            let seed = mnemonic.to_seed("");

            let (encrypted_seed, salt, nonce, uses_password) = if self.password.is_empty() {
                (seed.to_vec(), vec![], vec![], false)
            } else {
                // Encrypt the seed to obtain encrypted_seed, salt, and nonce
                let (encrypted_seed, salt, nonce) =
                    ClosedKeyItem::encrypt_seed(&seed, self.password.as_str())?;
                if self.use_password_for_app {
                    let (encrypted_message, salt, nonce) =
                        encrypt_message(DASH_SECRET_MESSAGE, self.password.as_str())?;
                    self.app_context
                        .db
                        .update_main_password(&salt, &nonce, &encrypted_message)
                        .map_err(|e| e.to_string())?;
                }
                (encrypted_seed, salt, nonce, true)
            };

            // Generate master ECDSA extended private key
            let master_ecdsa_extended_private_key =
                ExtendedPrivKey::new_master(self.app_context.network, &seed)
                    .expect("Failed to create master ECDSA extended private key");
            let bip44_root_derivation_path: DerivationPath = match self.app_context.network {
                Network::Dash => DerivationPath::from(DASH_BIP44_ACCOUNT_0_PATH_MAINNET.as_slice()),
                _ => DerivationPath::from(DASH_BIP44_ACCOUNT_0_PATH_TESTNET.as_slice()),
            };
            let secp = Secp256k1::new();
            let master_bip44_ecdsa_extended_public_key = master_ecdsa_extended_private_key
                .derive_priv(&secp, &bip44_root_derivation_path)
                .map_err(|e| e.to_string())?;

            let master_bip44_ecdsa_extended_public_key =
                ExtendedPubKey::from_priv(&secp, &master_bip44_ecdsa_extended_public_key);

            // Compute the seed hash
            let seed_hash = ClosedKeyItem::compute_seed_hash(&seed);

            let wallet = Wallet {
                wallet_seed: WalletSeed::Open(OpenWalletSeed {
                    seed,
                    wallet_info: ClosedKeyItem {
                        seed_hash,
                        encrypted_seed,
                        salt,
                        nonce,
                        password_hint: None, // Set a password hint if needed
                    },
                }),
                uses_password,
                master_bip44_ecdsa_extended_public_key,
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
                .map_err(|e| {
                    if e.to_string().contains("UNIQUE constraint failed: wallet.seed_hash") {
                        "This wallet has already been imported for another network. Each wallet can only be imported once per network. If you want to use this wallet on a different network, please switch networks first.".to_string()
                    } else {
                        e.to_string()
                    }
                })?;

            // Acquire a write lock and add the new wallet
            if let Ok(mut wallets) = self.app_context.wallets.write() {
                wallets.insert(wallet.seed_hash(), Arc::new(RwLock::new(wallet)));
                self.app_context.has_wallet.store(true, Ordering::Relaxed);
            } else {
                eprintln!("Failed to acquire write lock on wallets");
            }

            Ok(AppAction::GoToMainScreen) // Navigate back to the main screen after saving
        } else {
            Ok(AppAction::None) // No action if no seed phrase exists
        }
    }

    fn render_seed_phrase_input(&mut self, ui: &mut Ui) {
        ui.add_space(15.0); // Add spacing from the top
        ui.vertical_centered(|ui| {
            // Select the seed phrase length
            ui.horizontal(|ui| {
                ui.label("Seed Phrase Length:");

                ComboBox::from_label("")
                    .selected_text(format!("{}", self.selected_seed_phrase_length))
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for &length in &[12, 15, 18, 21, 24] {
                            ui.selectable_value(
                                &mut self.selected_seed_phrase_length,
                                length,
                                format!("{}", length),
                            );
                        }
                    });
            });

            ui.add_space(10.0);

            // Ensure the seed_phrase_words vector matches the selected length
            self.seed_phrase_words
                .resize(self.selected_seed_phrase_length, "".to_string());

            // Seed phrase input grid with shorter inputs
            let columns = 4; // 4 columns
            let _rows = self.selected_seed_phrase_length.div_ceil(columns);
            let input_width = 120.0; // Fixed width for each input

            Grid::new("seed_phrase_input_grid")
                .num_columns(columns)
                .spacing((15.0, 10.0))
                .show(ui, |ui| {
                    for i in 0..self.selected_seed_phrase_length {
                        ui.horizontal(|ui| {
                            ui.label(format!("{:2}:", i + 1));

                            let mut word = self.seed_phrase_words[i].clone();

                            let dark_mode = ui.ctx().style().visuals.dark_mode;
                            let response = ui.add_sized(
                                Vec2::new(input_width, 20.0),
                                egui::TextEdit::singleline(&mut word)
                                    .text_color(crate::ui::theme::DashColors::text_primary(
                                        dark_mode,
                                    ))
                                    .background_color(
                                        crate::ui::theme::DashColors::input_background(dark_mode),
                                    ),
                            );

                            if response.changed() {
                                // Update the seed_phrase_words[i]
                                self.seed_phrase_words[i] = word.clone();

                                // Check if the input contains multiple words
                                let words: Vec<&str> = word.split_whitespace().collect();

                                if words.len() > 1 {
                                    // User pasted multiple words into this field
                                    // Let's distribute them into the seed_phrase_words vector
                                    let total_words = self.selected_seed_phrase_length;
                                    let mut idx = i;
                                    for word in words {
                                        if idx < total_words {
                                            self.seed_phrase_words[idx] = word.to_string();
                                            idx += 1;
                                        } else {
                                            break;
                                        }
                                    }
                                    // Since we've updated the seed_phrase_words, the UI will reflect changes on the next frame
                                }
                            }
                        });

                        if (i + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }
                });
        });
    }
}

impl ScreenLike for ImportWalletScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Wallets", AppAction::GoToMainScreen),
                ("Import Wallet", AppAction::None),
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

            // Add the scroll area to make the content scrollable both vertically and horizontally
            egui::ScrollArea::both()
                .auto_shrink([false; 2]) // Prevent shrinking when content is less than the available area
                .show(ui, |ui| {
                    ui.add_space(10.0);
                    ui.heading("Follow these steps to import your wallet.");

                    ui.add_space(5.0);

                    ui.heading("1. Select the seed phrase length and enter all words.");
                    self.render_seed_phrase_input(ui);

                    // Check seed phrase validity whenever all words are filled
                    if self.seed_phrase_words.iter().all(|string| !string.is_empty()) {
                        match Mnemonic::parse_normalized(self.seed_phrase_words.join(" ").as_str()) {
                            Ok(mnemonic) => {
                                self.seed_phrase = Some(mnemonic);
                                // Clear any existing seed phrase error
                                if let Some(ref mut error) = self.error {
                                    if error.contains("Invalid seed phrase") {
                                        self.error = None;
                                    }
                                }
                            }
                            Err(_) => {
                                self.seed_phrase = None;
                                self.error = Some("Invalid seed phrase. Please check that all words are spelled correctly and are valid BIP39 words.".to_string());
                            }
                        }
                    } else {
                        // Clear seed phrase and error if not all words are filled
                        self.seed_phrase = None;
                        if let Some(ref mut error) = self.error {
                            if error.contains("Invalid seed phrase") {
                                self.error = None;
                            }
                        }
                    }

                    // Display error message if seed phrase is invalid
                    if let Some(ref error_msg) = self.error {
                        if error_msg.contains("Invalid seed phrase") {
                            ui.add_space(10.0);
                            ui.colored_label(Color32::from_rgb(255, 100, 100), error_msg);
                        }
                    }

                    if self.seed_phrase.is_none() {
                        return;
                    }

                    ui.add_space(20.0);

                    ui.heading("2. Select a wallet name to remember it. (This will not go to the blockchain)");

                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label("Wallet Name:");
                        ui.text_edit_singleline(&mut self.alias_input);
                    });

                    ui.add_space(20.0);

                    ui.heading("3. Add a password that must be used to unlock the wallet. (Optional but recommended)");

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
                        let fill_color = match self.password_strength as i32 {
                            0..=25 => Color32::from_rgb(255, 182, 193),    // Light pink
                            26..=50 => Color32::from_rgb(255, 224, 130),   // Light yellow
                            51..=75 => Color32::from_rgb(144, 238, 144),   // Light green
                            _ => Color32::from_rgb(90, 200, 90),           // Medium green
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

                    // if self.app_context.password_info.is_none() {
                    //     ui.add_space(10.0);
                    //     ui.checkbox(&mut self.use_password_for_app, "Use password for Dash Evo Tool loose keys (recommended)");
                    // }

                    ui.add_space(20.0);

                    ui.heading("4. Save the wallet.");
                    ui.add_space(5.0);

                    // Centered "Save Wallet" button at the bottom
                    ui.with_layout(Layout::centered_and_justified(Direction::TopDown), |ui| {
                        let save_button = egui::Button::new(
                            RichText::new("Save Wallet").strong().size(30.0),
                        )
                            .min_size(Vec2::new(300.0, 60.0))
                            .corner_radius(10.0)
                            .stroke(Stroke::new(1.5, Color32::WHITE))
                            .sense(if self.seed_phrase.is_some() {
                                egui::Sense::click()
                            } else {
                                egui::Sense::hover()
                            });

                        if ui.add(save_button).clicked() {
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
                });

            inner_action
        });

        action
    }
}
