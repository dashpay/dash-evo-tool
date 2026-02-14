use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::add_existing_identity_screen::AddExistingIdentityScreen;
use crate::ui::identities::add_new_identity_screen::AddNewIdentityScreen;
use crate::ui::{RootScreenType, Screen, ScreenLike};
use eframe::egui::Context;

use crate::model::wallet::encryption::{DASH_SECRET_MESSAGE, encrypt_message};
use crate::model::wallet::{ClosedKeyItem, OpenWalletSeed, Wallet, WalletSeed};
use crate::ui::theme::DashColors;
use crate::ui::wallets::add_new_wallet_screen::{
    DASH_BIP44_ACCOUNT_0_PATH_MAINNET, DASH_BIP44_ACCOUNT_0_PATH_TESTNET,
};
use bip39::Mnemonic;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::key_wallet::bip32::{ExtendedPrivKey, ExtendedPubKey};
use egui::{Color32, ComboBox, Grid, RichText, Ui, Vec2};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use zxcvbn::zxcvbn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportType {
    Mnemonic,
    PrivateKey,
}

pub struct ImportMnemonicScreen {
    // Common fields
    import_type: ImportType,
    password: String,
    alias_input: String,
    password_strength: f64,
    estimated_time_to_crack: String,
    error: Option<String>,
    pub app_context: Arc<AppContext>,
    use_password_for_app: bool,
    wallet_imported: bool,
    show_advanced_options: bool,

    // Mnemonic-specific fields
    seed_phrase_words: Vec<String>,
    selected_seed_phrase_length: usize,
    seed_phrase: Option<Mnemonic>,

    // Private key-specific fields
    private_key_input: String,
    parsed_single_key_wallet: Option<SingleKeyWallet>,

    // Identity discovery options
    identity_scan_count: u32,
}

impl ImportMnemonicScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            // Common fields
            import_type: ImportType::Mnemonic,
            password: String::new(),
            alias_input: String::new(),
            password_strength: 0.0,
            estimated_time_to_crack: String::new(),
            error: None,
            app_context: app_context.clone(),
            use_password_for_app: true,
            wallet_imported: false,
            show_advanced_options: false,

            // Mnemonic-specific fields
            seed_phrase_words: vec!["".to_string(); 24],
            selected_seed_phrase_length: 12,
            seed_phrase: None,

            // Private key-specific fields
            private_key_input: String::new(),
            parsed_single_key_wallet: None,

            // Identity discovery options
            identity_scan_count: 5,
        }
    }

    /// Set the seed phrase length (for testing).
    /// Resizes the word vector to match.
    pub fn set_seed_phrase_length(&mut self, length: usize) {
        self.selected_seed_phrase_length = length;
        self.seed_phrase_words.resize(length, String::new());
    }

    /// Set all seed phrase words and parse the mnemonic (for testing).
    pub fn set_seed_phrase_words(&mut self, words: &[&str]) {
        self.selected_seed_phrase_length = words.len();
        self.seed_phrase_words = words.iter().map(|w| w.to_string()).collect();
        if let Ok(mnemonic) = Mnemonic::parse_normalized(words.join(" ").as_str()) {
            self.seed_phrase = Some(mnemonic);
            self.error = None;
        }
    }

    /// Set the wallet alias (for testing).
    pub fn set_alias(&mut self, alias: &str) {
        self.alias_input = alias.to_string();
    }

    /// Trigger wallet import — stores to DB and adds to AppContext (for testing).
    pub fn trigger_save(&mut self) -> Result<AppAction, String> {
        self.save_wallet()
    }

    fn try_parse_private_key(&mut self) {
        let input = self.private_key_input.trim();
        if input.is_empty() {
            self.parsed_single_key_wallet = None;
            self.error = None;
            return;
        }

        // Try to parse as WIF first, then as hex
        let result = SingleKeyWallet::from_wif(input, None, None)
            .or_else(|_| SingleKeyWallet::from_hex(input, self.app_context.network, None, None));

        match result {
            Ok(wallet) => {
                self.parsed_single_key_wallet = Some(wallet);
                self.error = None;
            }
            Err(e) => {
                self.parsed_single_key_wallet = None;
                self.error = Some(format!("Invalid private key: {}", e));
            }
        }
    }

    fn save_private_key_wallet(&mut self) -> Result<AppAction, String> {
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

        // Generate default wallet name if none provided
        let alias = if self.alias_input.trim().is_empty() {
            let existing_wallet_count = self
                .app_context
                .single_key_wallets
                .read()
                .map(|w| w.len())
                .unwrap_or(0);
            Some(format!("Key {}", existing_wallet_count + 1))
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

        self.wallet_imported = true;
        Ok(AppAction::None)
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

            // Generate default wallet name if none provided
            let wallet_alias = if self.alias_input.trim().is_empty() {
                let existing_wallet_count = self
                    .app_context
                    .wallets
                    .read()
                    .map(|w| w.len())
                    .unwrap_or(0);
                format!("Wallet {}", existing_wallet_count + 1)
            } else {
                self.alias_input.clone()
            };

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
                address_total_received: Default::default(),
                known_addresses: Default::default(),
                watched_addresses: Default::default(),
                unused_asset_locks: Default::default(),
                alias: Some(wallet_alias),
                identities: Default::default(),
                utxos: Default::default(),
                transactions: Vec::new(),
                is_main: true,
                confirmed_balance: 0,
                unconfirmed_balance: 0,
                total_balance: 0,
                platform_address_info: Default::default(),
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

            let wallet_arc = Arc::new(RwLock::new(wallet));
            let new_wallet_seed_hash = wallet_arc.read().unwrap().seed_hash();

            // Acquire a write lock and add the new wallet
            if let Ok(mut wallets) = self.app_context.wallets.write() {
                wallets.insert(new_wallet_seed_hash, wallet_arc.clone());
                self.app_context.has_wallet.store(true, Ordering::Relaxed);
            } else {
                tracing::error!("Failed to acquire write lock on wallets");
            }

            // Set pending wallet selection so the wallet screen auto-selects this wallet
            if let Ok(mut pending) = self.app_context.pending_wallet_selection.lock() {
                *pending = Some(new_wallet_seed_hash);
            }

            self.app_context.bootstrap_wallet_addresses(&wallet_arc);
            if self.app_context.core_backend_mode() == crate::spv::CoreBackendMode::Spv {
                self.app_context.handle_wallet_unlocked(&wallet_arc);
            }

            // Auto-discover identities derived from this wallet
            if self.identity_scan_count > 0 {
                self.app_context
                    .queue_wallet_identity_discovery(&wallet_arc, self.identity_scan_count - 1);
            }

            self.wallet_imported = true;
            Ok(AppAction::None) // Show success screen instead of navigating away
        } else {
            Ok(AppAction::None) // No action if no seed phrase exists
        }
    }

    fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        let title = match self.import_type {
            ImportType::Mnemonic => "Wallet Imported Successfully!",
            ImportType::PrivateKey => "Key Imported Successfully!",
        };

        let mut buttons = vec![("Go to Wallet Screen".to_string(), AppAction::GoToMainScreen)];

        // Only show identity options for HD wallets (mnemonic import)
        if self.import_type == ImportType::Mnemonic {
            buttons.push((
                "Create Identity".to_string(),
                AppAction::PopThenAddScreenToMainScreen(
                    RootScreenType::RootScreenIdentities,
                    Screen::AddNewIdentityScreen(AddNewIdentityScreen::new(&self.app_context)),
                ),
            ));
            buttons.push((
                "Load Existing Identity".to_string(),
                AppAction::PopThenAddScreenToMainScreen(
                    RootScreenType::RootScreenIdentities,
                    Screen::AddExistingIdentityScreen(AddExistingIdentityScreen::new(
                        &self.app_context,
                    )),
                ),
            ));
        }

        buttons.push((
            "Import Another Wallet".to_string(),
            AppAction::Custom("import_another_wallet".to_string()),
        ));

        let action = crate::ui::helpers::show_success_screen(ui, title.to_string(), buttons);

        // Handle the custom action to reset the form
        if let AppAction::Custom(ref s) = action
            && s == "import_another_wallet"
        {
            // Reset mnemonic fields
            self.seed_phrase_words = vec!["".to_string(); 24];
            self.selected_seed_phrase_length = 12;
            self.seed_phrase = None;

            // Reset private key fields
            self.private_key_input = String::new();
            self.parsed_single_key_wallet = None;

            // Reset common fields
            self.password = String::new();
            self.alias_input = String::new();
            self.password_strength = 0.0;
            self.estimated_time_to_crack = String::new();
            self.error = None;
            self.wallet_imported = false;
            self.identity_scan_count = 5;
            return AppAction::None;
        }

        action
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
                                    .hint_text(format!("Word {}", i + 1))
                                    .text_color(DashColors::text_primary(dark_mode))
                                    .background_color(DashColors::input_background(dark_mode)),
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

    fn render_private_key_input(&mut self, ui: &mut Ui, step: u32) {
        ui.heading(format!(
            "{}. Enter your private key (WIF or 64-character hex format)",
            step
        ));
        ui.add_space(8.0);

        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let response = ui.add_sized(
            Vec2::new(ui.available_width() - 20.0, 40.0),
            egui::TextEdit::singleline(&mut self.private_key_input)
                .hint_text("Enter private key (WIF: 51-52 chars, or hex: 64 chars)")
                .text_color(DashColors::text_primary(dark_mode))
                .background_color(DashColors::input_background(dark_mode))
                .password(true),
        );

        if response.changed() {
            self.try_parse_private_key();
        }

        // Show parsed address preview
        if let Some(ref wallet) = self.parsed_single_key_wallet {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("Derived Address:");
                ui.label(
                    RichText::new(wallet.address.to_string())
                        .monospace()
                        .color(DashColors::SUCCESS),
                );
            });
        }

        // Show error if any
        if let Some(ref err) = self.error {
            ui.add_space(5.0);
            ui.colored_label(DashColors::ERROR, err);
        }
    }

    fn render_import_type_selection(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Import Type:");
            ui.selectable_value(
                &mut self.import_type,
                ImportType::Mnemonic,
                "Seed Phrase (HD Wallet)",
            );
            ui.selectable_value(
                &mut self.import_type,
                ImportType::PrivateKey,
                "Private Key (Single Address)",
            );
        });
    }
}

impl ScreenLike for ImportMnemonicScreen {
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

            // Show success screen if wallet was imported
            if self.wallet_imported {
                inner_action = self.show_success(ui);
                return inner_action;
            }

            // Add the scroll area to make the content scrollable both vertically and horizontally
            egui::ScrollArea::both()
                .auto_shrink([false; 2]) // Prevent shrinking when content is less than the available area
                .show(ui, |ui| {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.heading("Follow these steps to import your wallet.");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.checkbox(&mut self.show_advanced_options, "Show Advanced Options");
                        });
                    });
                    ui.add_space(10.0);

                    // Track step number based on whether advanced options are shown
                    let mut step = 1;

                    // Import type selection (only show when advanced options is checked)
                    if self.show_advanced_options {
                        ui.heading(format!("{}. Select what you want to import.", step));
                        ui.add_space(10.0);
                        self.render_import_type_selection(ui);
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                        step += 1;

                        // Identity scan count option (only for mnemonic/HD wallets)
                        if self.import_type == ImportType::Mnemonic {
                            ui.heading(format!("{}. Configure identity auto-discovery.", step));
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                ui.label("Identity indices to scan:");
                                ui.add(egui::DragValue::new(&mut self.identity_scan_count)
                                    .range(0..=20)
                                    .speed(0.1));
                                ui.label("(0 to disable)");
                            });
                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(10.0);
                            step += 1;
                        }
                    } else {
                        // Reset to mnemonic when advanced options is hidden
                        self.import_type = ImportType::Mnemonic;
                    }

                    // Different UI based on import type
                    match self.import_type {
                        ImportType::Mnemonic => {
                            ui.heading(format!("{}. Select the seed phrase length and enter all words.", step));
                            self.render_seed_phrase_input(ui);

                            // Check seed phrase validity whenever all words are filled
                            if self.seed_phrase_words.iter().all(|string| !string.is_empty()) {
                                match Mnemonic::parse_normalized(self.seed_phrase_words.join(" ").as_str()) {
                                    Ok(mnemonic) => {
                                        self.seed_phrase = Some(mnemonic);
                                        // Clear any existing seed phrase error
                                        if let Some(ref mut error) = self.error
                                            && error.contains("Invalid seed phrase") {
                                                self.error = None;
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
                                if let Some(ref mut error) = self.error
                                    && error.contains("Invalid seed phrase") {
                                        self.error = None;
                                    }
                            }

                            // Display error message if seed phrase is invalid
                            if let Some(ref error_msg) = self.error
                                && error_msg.contains("Invalid seed phrase") {
                                    ui.add_space(10.0);
                                    ui.colored_label(DashColors::ERROR, error_msg);
                                }

                            if self.seed_phrase.is_none() {
                                return;
                            }
                        }
                        ImportType::PrivateKey => {
                            self.render_private_key_input(ui, step);

                            if self.parsed_single_key_wallet.is_none() {
                                return;
                            }
                        }
                    }
                    step += 1;

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading(format!("{}. Enter a name to remember it by. (This will not go on the blockchain)", step));

                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.add(egui::TextEdit::singleline(&mut self.alias_input).hint_text("Wallet name"));
                    });

                    step += 1;

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading(format!("{}. Add a password to encrypt. (Optional but recommended)", step));

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

                    // if self.app_context.password_info.is_none() {
                    //     ui.add_space(10.0);
                    //     ui.checkbox(&mut self.use_password_for_app, "Use password for Dash Evo Tool loose keys (recommended)");
                    // }

                    step += 1;

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    let button_text = match self.import_type {
                        ImportType::Mnemonic => format!("{}. Save the wallet.", step),
                        ImportType::PrivateKey => format!("{}. Import the key.", step),
                    };
                    ui.heading(button_text);
                    ui.add_space(10.0);

                    // Save button
                    let mut new_style = (**ui.style()).clone();
                    new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                    ui.set_style(new_style);

                    let button_label = match self.import_type {
                        ImportType::Mnemonic => "Save Wallet",
                        ImportType::PrivateKey => "Import Key",
                    };
                    let save_button = egui::Button::new(
                        RichText::new(button_label).color(Color32::WHITE),
                    )
                        .fill(DashColors::ACTION_BUTTON_BLUE)
                        .frame(true)
                        .corner_radius(3.0);

                    if ui.add(save_button).clicked() {
                        let result = match self.import_type {
                            ImportType::Mnemonic => self.save_wallet(),
                            ImportType::PrivateKey => self.save_private_key_wallet(),
                        };
                        match result {
                            Ok(save_action) => {
                                inner_action = save_action;
                            }
                            Err(e) => {
                                self.error = Some(e)
                            }
                        }
                    }
                });

            inner_action
        });

        action
    }
}
