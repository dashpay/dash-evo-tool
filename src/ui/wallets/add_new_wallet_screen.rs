use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::wallet::encryption::{DASH_SECRET_MESSAGE, encrypt_message};
use crate::model::wallet::{
    AddressInfo as WalletAddressInfo, ClosedKeyItem, DerivationPathReference, DerivationPathType,
    OpenWalletSeed, Wallet, WalletSeed,
};
use crate::ui::components::entropy_grid::U256EntropyGrid;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::add_new_identity_screen::AddNewIdentityScreen;
use crate::ui::identities::funding_common::generate_qr_code_image;
use crate::ui::theme::DashColors;
use crate::ui::{RootScreenType, Screen, ScreenLike};
use bip39::{Language, Mnemonic};
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use dash_sdk::dpp::key_wallet::bip32::{ExtendedPrivKey, ExtendedPubKey};
use eframe::egui::{Context, TextureHandle, TextureOptions};
use eframe::emath::Align;
use egui::load::SizedTexture;
use egui::{Color32, ComboBox, Frame, Grid, Layout, Margin, RichText, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use zxcvbn::zxcvbn;

// Constants for feature purposes and sub-features
pub const BIP44_PURPOSE: u32 = 44;
pub const DASH_COIN_TYPE: u32 = 5;
pub const DASH_TESTNET_COIN_TYPE: u32 = 1;
pub const DASH_BIP44_ACCOUNT_0_PATH_MAINNET: [ChildNumber; 3] = [
    ChildNumber::Hardened {
        index: BIP44_PURPOSE,
    },
    ChildNumber::Hardened {
        index: DASH_COIN_TYPE,
    },
    ChildNumber::Hardened { index: 0 },
];

pub const DASH_BIP44_ACCOUNT_0_PATH_TESTNET: [ChildNumber; 3] = [
    ChildNumber::Hardened {
        index: BIP44_PURPOSE,
    },
    ChildNumber::Hardened {
        index: DASH_TESTNET_COIN_TYPE,
    },
    ChildNumber::Hardened { index: 0 },
];

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
    password: String,
    show_password: bool,
    entropy_grid: U256EntropyGrid,
    selected_language: Language,
    selected_word_count: WordCount,
    alias_input: String,
    wrote_it_down: bool,
    password_strength: f64,
    estimated_time_to_crack: String,
    error: Option<String>,
    pub app_context: Arc<AppContext>,
    use_password_for_app: bool,
    wallet_created: bool,
    // Success screen state
    created_wallet_seed_hash: Option<[u8; 32]>,
    receive_address: Option<Address>,
    receive_address_string: Option<String>,
    receive_qr_texture: Option<TextureHandle>,
    show_receive_popup: bool,
    funds_received: bool,
}

impl AddNewWalletScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            seed_phrase: None,
            password: String::new(),
            show_password: false,
            entropy_grid: U256EntropyGrid::new(),
            selected_language: Language::English,
            selected_word_count: WordCount::Words24, // Default to 24 words for maximum security
            alias_input: String::new(),
            wrote_it_down: false,
            password_strength: 0.0,
            estimated_time_to_crack: "".to_string(),
            error: None,
            app_context: app_context.clone(),
            use_password_for_app: true,
            wallet_created: false,
            created_wallet_seed_hash: None,
            receive_address: None,
            receive_address_string: None,
            receive_qr_texture: None,
            show_receive_popup: false,
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

            // Generate the first receive address BEFORE creating wallet (no locks needed)
            let address_path_extension = DerivationPath::from(
                [
                    ChildNumber::Normal { index: 0 }, // receive (not change)
                    ChildNumber::Normal { index: 0 }, // first address
                ]
                .as_slice(),
            );
            let first_address = master_bip44_ecdsa_extended_public_key
                .derive_pub(&secp, &address_path_extension)
                .ok()
                .map(|pk| Address::p2pkh(&pk.to_pub(), self.app_context.network));

            // Build known_addresses and watched_addresses with the first address
            let mut known_addresses = std::collections::BTreeMap::new();
            let mut watched_addresses = std::collections::BTreeMap::new();

            if let Some(ref address) = first_address {
                let full_derivation_path = DerivationPath::from(match self.app_context.network {
                    Network::Dash => [
                        DASH_BIP44_ACCOUNT_0_PATH_MAINNET[0],
                        DASH_BIP44_ACCOUNT_0_PATH_MAINNET[1],
                        DASH_BIP44_ACCOUNT_0_PATH_MAINNET[2],
                        ChildNumber::Normal { index: 0 },
                        ChildNumber::Normal { index: 0 },
                    ]
                    .as_slice(),
                    _ => [
                        DASH_BIP44_ACCOUNT_0_PATH_TESTNET[0],
                        DASH_BIP44_ACCOUNT_0_PATH_TESTNET[1],
                        DASH_BIP44_ACCOUNT_0_PATH_TESTNET[2],
                        ChildNumber::Normal { index: 0 },
                        ChildNumber::Normal { index: 0 },
                    ]
                    .as_slice(),
                });
                known_addresses.insert(address.clone(), full_derivation_path.clone());
                watched_addresses.insert(
                    full_derivation_path,
                    WalletAddressInfo {
                        address: address.clone(),
                        path_type: DerivationPathType::CLEAR_FUNDS,
                        path_reference: DerivationPathReference::BIP44,
                    },
                );

                self.receive_address_string = Some(address.to_string());
                self.receive_address = Some(address.clone());
            }

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
                        password_hint: None,
                    },
                }),
                uses_password,
                master_bip44_ecdsa_extended_public_key,
                address_balances: Default::default(),
                address_total_received: Default::default(),
                known_addresses,
                watched_addresses,
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
                .map_err(|e| e.to_string())?;

            let new_wallet_seed_hash = wallet.seed_hash();
            let wallet_arc = Arc::new(RwLock::new(wallet));

            // Acquire a write lock and add the new wallet
            if let Ok(mut wallets) = self.app_context.wallets.write() {
                wallets.insert(new_wallet_seed_hash, wallet_arc.clone());
                self.app_context.has_wallet.store(true, Ordering::Relaxed);
            } else {
                eprintln!("Failed to acquire write lock on wallets");
            }

            // Set pending wallet selection so the wallet screen auto-selects this wallet
            if let Ok(mut pending) = self.app_context.pending_wallet_selection.lock() {
                *pending = Some(new_wallet_seed_hash);
            }

            // Save the first address to database
            if let Some(ref address) = first_address {
                let full_derivation_path = DerivationPath::from(match self.app_context.network {
                    Network::Dash => [
                        DASH_BIP44_ACCOUNT_0_PATH_MAINNET[0],
                        DASH_BIP44_ACCOUNT_0_PATH_MAINNET[1],
                        DASH_BIP44_ACCOUNT_0_PATH_MAINNET[2],
                        ChildNumber::Normal { index: 0 },
                        ChildNumber::Normal { index: 0 },
                    ]
                    .as_slice(),
                    _ => [
                        DASH_BIP44_ACCOUNT_0_PATH_TESTNET[0],
                        DASH_BIP44_ACCOUNT_0_PATH_TESTNET[1],
                        DASH_BIP44_ACCOUNT_0_PATH_TESTNET[2],
                        ChildNumber::Normal { index: 0 },
                        ChildNumber::Normal { index: 0 },
                    ]
                    .as_slice(),
                });
                let _ = self.app_context.db.add_address_if_not_exists(
                    &new_wallet_seed_hash,
                    address,
                    &self.app_context.network,
                    &full_derivation_path,
                    DerivationPathReference::BIP44,
                    DerivationPathType::CLEAR_FUNDS,
                    None,
                );
            }

            // Load SPV wallet in background
            if self.app_context.core_backend_mode() == crate::spv::CoreBackendMode::Spv {
                self.app_context.handle_wallet_unlocked(&wallet_arc);
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
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Check for incoming funds by looking at wallet balance
        // Use total_balance_duffs() which falls back to max_balance() (from UTXOs) if SPV balance not set
        if !self.funds_received {
            if let Some(seed_hash) = &self.created_wallet_seed_hash
                && let Ok(wallets) = self.app_context.wallets.read()
                && let Some(wallet) = wallets.get(seed_hash)
                && let Ok(wallet_guard) = wallet.read()
                && wallet_guard.total_balance_duffs() > 0
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
                            .color(crate::ui::theme::DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(12.0);

                    // Step 1: Fund wallet
                    ui.horizontal(|ui| {
                        let step_color = if self.funds_received {
                            crate::ui::theme::DashColors::success_color(dark_mode)
                        } else {
                            crate::ui::theme::DashColors::text_secondary(dark_mode)
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
                                .color(crate::ui::theme::DashColors::text_secondary(dark_mode)),
                        );
                        ui.label(
                            RichText::new("Create a Platform Identity to register a username and interact with apps")
                                .size(14.0)
                                .color(crate::ui::theme::DashColors::text_secondary(dark_mode)),
                        );
                    });
                },
            );

            ui.add_space(20.0);

            // Buttons
            if !self.funds_received {
                if ui.button("Fund Wallet").clicked() {
                    self.show_receive_popup = true;
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
        let screen_rect = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("receive_funds_overlay"),
        ));
        painter.rect_filled(
            screen_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120),
        );

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
        egui::Window::new("Fund Wallet")
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
                        if ui.button("Copy Address").clicked()
                            && let Err(err) = crate::ui::helpers::copy_text_to_clipboard(address)
                        {
                            tracing::warn!("Failed to copy address: {}", err);
                        }
                    }

                    ui.add_space(8.0);

                    ui.label("Waiting for funds...");
                });
            });

        self.show_receive_popup = open;
        AppAction::None
    }

    fn render_seed_phrase_input(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
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

                let generate_button = egui::Button::new(
                    RichText::new("Generate")
                        .strong()
                        .size(12.0)
                        .color(Color32::WHITE),
                )
                .min_size(Vec2::new(100.0, 20.0))
                .fill(Color32::from_rgb(0, 128, 255)) // Blue background
                .corner_radius(5.0);

                if ui.add(generate_button).clicked() {
                    self.generate_seed_phrase();
                    self.entropy_grid.freeze();
                }
            });

            // Only show the seed phrase box after generation
            if let Some(mnemonic) = &self.seed_phrase {
                ui.add_space(10.0);

                // Calculate grid dimensions based on word count and available width
                let word_count = mnemonic.word_count();
                let available_width = ui.available_width();
                // Use more width on small screens, cap at 600px
                let frame_width = (available_width - 40.0).clamp(200.0, 600.0);
                // Adapt columns to available width: use fewer columns on narrow screens
                let columns = if frame_width < 300.0 {
                    2
                } else if word_count <= 12 {
                    3
                } else {
                    4
                };
                let rows = word_count.div_ceil(columns); // Ceiling division
                let row_height = 36.0_f32;
                let frame_height = (rows as f32 * row_height).max(120.0);

                ui.allocate_ui_with_layout(
                    Vec2::new(frame_width, frame_height + 20.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        Frame::new()
                            .fill(surface)
                            .stroke(Stroke::new(1.0, border))
                            .corner_radius(5.0)
                            .inner_margin(Margin::same(10))
                            .show(ui, |ui| {
                                let column_width = (frame_width - 20.0) / columns as f32;

                                Grid::new("seed_phrase_grid")
                                    .num_columns(columns)
                                    .spacing((0.0, 4.0))
                                    .min_col_width(column_width)
                                    .min_row_height(row_height)
                                    .show(ui, |ui| {
                                        for (i, word) in mnemonic.words().enumerate() {
                                            let number_text = RichText::new(format!("{} ", i + 1))
                                                .size(11.0)
                                                .color(text_secondary);

                                            let word_text =
                                                RichText::new(word).size(14.0).color(text_primary);

                                            ui.with_layout(
                                                Layout::left_to_right(Align::Min),
                                                |ui| {
                                                    ui.label(number_text);
                                                    ui.label(word_text);
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
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Wallets", AppAction::GoToMainScreen),
                ("Create Wallet", AppAction::None),
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

                    ui.horizontal(|ui| {
                        ui.label("Wallet Name:");
                        ui.text_edit_singleline(&mut self.alias_input);
                    });
                    ui.label(
                        RichText::new("This can be edited later and is not recorded publicly.")
                            .weak()
                            .italics()
                            .size(12.0),
                    );

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading("5. Add a password that must be used to unlock the wallet. (Optional but recommended)");

                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label("Optional Password:");
                        let password_edit = egui::TextEdit::singleline(&mut self.password)
                            .password(!self.show_password);
                        if ui.add(password_edit).changed() {
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
                        let toggle_text = if self.show_password { "Hide" } else { "Show" };
                        if ui.small_button(toggle_text).clicked() {
                            self.show_password = !self.show_password;
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
                        let strength_label = match self.password_strength as i32 {
                            0 => "None",
                            1..=25 => "Very Weak",
                            26..=50 => "Weak",
                            51..=75 => "Strong",
                            _ => "Very Strong",
                        };
                        ui.add(
                            egui::ProgressBar::new(strength_percentage as f32)
                                .desired_width(250.0)
                                .show_percentage()
                                .text(strength_label)
                                .fill(fill_color),
                        );
                    });

                    ui.add_space(10.0);
                    let crack_time = if self.estimated_time_to_crack == "less than a second" {
                        "<1 second"
                    } else {
                        &self.estimated_time_to_crack
                    };
                    ui.label(format!(
                        "Estimated time to crack: {}",
                        crack_time
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
                    let save_button = egui::Button::new(
                        RichText::new("Save Wallet").color(Color32::WHITE),
                    )
                        .fill(Color32::from_rgb(0, 128, 255))
                        .frame(true)
                        .corner_radius(3.0);

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
                    ui.add(egui::Label::new(error_message).wrap());
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        self.error = None; // Clear the error to close the popup
                    }
                });
        }

        action
    }
}
