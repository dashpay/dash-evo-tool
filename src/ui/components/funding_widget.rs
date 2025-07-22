use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::funding_common::{copy_to_clipboard, generate_qr_code_image};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::dashcore::Transaction;
use dash_sdk::dpp::prelude::AssetLockProof;
use eframe::epaint::TextureHandle;
use egui::{Color32, ComboBox, InnerResponse, Ui, Widget};
use std::sync::{Arc, RwLock};

/// Response from the funding widget containing all state changes and actions
#[derive(Debug, Clone, Default)]
pub struct FundingWidgetResponse {
    /// Wallet selection changed
    pub wallet_changed: Option<Arc<RwLock<Wallet>>>,
    /// Funding method changed
    pub funding_method_changed: Option<FundingMethod>,
    /// Funding amount changed
    pub amount_changed: Option<String>,
    /// Address generated or changed
    pub address_changed: Option<Address>,
    /// Asset lock selected (Transaction, AssetLockProof, Address)
    pub asset_lock_selected: Option<(Transaction, AssetLockProof, Address)>,
    /// Max button was clicked
    pub max_button_clicked: bool,
    /// Copy button was clicked
    pub copy_button_clicked: bool,
    /// Error occurred
    pub error: Option<String>,
}

impl FundingWidgetResponse {
    /// Check if any changes occurred
    pub fn has_changes(&self) -> bool {
        self.wallet_changed.is_some()
            || self.funding_method_changed.is_some()
            || self.amount_changed.is_some()
            || self.address_changed.is_some()
            || self.asset_lock_selected.is_some()
            || self.max_button_clicked
            || self.copy_button_clicked
            || self.error.is_some()
    }
}

/// Funding widget state
pub struct FundingWidget {
    app_context: Arc<AppContext>,

    // Configuration
    predefined_wallet: Option<Arc<RwLock<Wallet>>>,
    predefined_address: Option<Address>,
    default_amount: String,
    show_max_button: bool,
    amount_label: String,
    show_qr_code: bool,
    show_copy_button: bool,

    // Current state
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    funding_method: FundingMethod,
    funding_amount: String,
    funding_address: Option<Address>,
    selected_asset_lock: Option<(Transaction, AssetLockProof, Address)>,
    core_has_funding_address: Option<bool>,
    copied_to_clipboard: Option<Option<String>>,
}

impl FundingWidget {
    /// Create a new FundingWidget with default configuration
    pub fn new(app_context: Arc<AppContext>) -> Self {
        let default_amount = "0.5".to_string();

        Self {
            app_context,
            predefined_wallet: None,
            predefined_address: None,
            default_amount: default_amount.clone(),
            show_max_button: true,
            amount_label: "Amount (DASH):".to_string(),
            show_qr_code: true,
            show_copy_button: true,
            selected_wallet: None,
            funding_method: FundingMethod::NoSelection,
            funding_amount: default_amount,
            funding_address: None,
            selected_asset_lock: None,
            core_has_funding_address: None,
            copied_to_clipboard: None,
        }
    }

    /// Set a predefined wallet to use. If not set, user can select from available wallets
    pub fn with_wallet(mut self, wallet: Arc<RwLock<Wallet>>) -> Self {
        self.predefined_wallet = Some(wallet.clone());
        self.selected_wallet = Some(wallet);
        self
    }

    /// Set a predefined address to use. If not set, a new address will be generated
    pub fn with_address(mut self, address: Address) -> Self {
        self.predefined_address = Some(address.clone());
        self.funding_address = Some(address);
        // When address is predefined, force QR code method
        self.funding_method = FundingMethod::AddressWithQRCode;
        self
    }

    /// Set the default funding amount
    pub fn with_default_amount<S: Into<String>>(mut self, amount: S) -> Self {
        let amount_str = amount.into();
        self.default_amount = amount_str.clone();
        self.funding_amount = amount_str;
        self
    }

    /// Set whether to show the "Max" button for wallet balance funding
    pub fn with_max_button(mut self, show: bool) -> Self {
        self.show_max_button = show;
        self
    }

    /// Set the label for the funding amount input
    pub fn with_amount_label<S: Into<String>>(mut self, label: S) -> Self {
        self.amount_label = label.into();
        self
    }

    /// Set whether to show QR code
    pub fn with_qr_code(mut self, show: bool) -> Self {
        self.show_qr_code = show;
        self
    }

    /// Set whether to show copy button
    pub fn with_copy_button(mut self, show: bool) -> Self {
        self.show_copy_button = show;
        self
    }

    /// Get the current selected wallet
    pub fn selected_wallet(&self) -> Option<&Arc<RwLock<Wallet>>> {
        self.selected_wallet.as_ref()
    }

    /// Get the current funding method
    pub fn funding_method(&self) -> FundingMethod {
        self.funding_method
    }

    /// Get the current funding amount
    pub fn funding_amount(&self) -> &str {
        &self.funding_amount
    }

    /// Get the current funding address
    pub fn funding_address(&self) -> Option<&Address> {
        self.funding_address.as_ref()
    }

    /// Get the currently selected asset lock
    pub fn selected_asset_lock(&self) -> Option<&(Transaction, AssetLockProof, Address)> {
        self.selected_asset_lock.as_ref()
    }

    fn render_wallet_selection(
        &mut self,
        ui: &mut Ui,
        response: &mut FundingWidgetResponse,
    ) -> bool {
        // If wallet is predefined, don't show selection
        if self.predefined_wallet.is_some() {
            return false;
        }

        if !self
            .app_context
            .has_wallet
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            ui.label("No wallets available.");
            return false;
        }

        let wallets = self.app_context.wallets.read().unwrap();
        if wallets.len() <= 1 {
            // Auto-select the only wallet if available
            if let Some(wallet) = wallets.values().next() {
                if self.selected_wallet.is_none() {
                    let wallet_clone = wallet.clone();
                    // Drop wallets before modifying self
                    drop(wallets);
                    self.selected_wallet = Some(wallet_clone.clone());
                    self.funding_method = FundingMethod::NoSelection; // Reset funding method
                    self.selected_asset_lock = None; // Reset asset lock
                    response.wallet_changed = Some(wallet_clone);
                    response.funding_method_changed = Some(FundingMethod::NoSelection);
                }
            }
            return false;
        }

        // Multiple wallets - show selection
        let selected_wallet_alias = self
            .selected_wallet
            .as_ref()
            .and_then(|wallet| wallet.read().ok()?.alias.clone())
            .unwrap_or_else(|| "Select Wallet".to_string());

        ui.label("Select Wallet:");

        let mut wallet_selected = None;
        ComboBox::from_id_salt("funding_widget_wallet_selection")
            .selected_text(selected_wallet_alias)
            .show_ui(ui, |ui| {
                for wallet in wallets.values() {
                    let wallet_alias = wallet
                        .read()
                        .ok()
                        .and_then(|w| w.alias.clone())
                        .unwrap_or_else(|| "Unnamed Wallet".to_string());

                    let is_selected = self
                        .selected_wallet
                        .as_ref()
                        .is_some_and(|selected| Arc::ptr_eq(selected, wallet));

                    if ui.selectable_label(is_selected, wallet_alias).clicked() {
                        wallet_selected = Some(wallet.clone());
                    }
                }
            });
        // Drop wallets borrow before accessing self mutably
        drop(wallets);

        if let Some(wallet) = wallet_selected {
            self.selected_wallet = Some(wallet.clone());
            // Reset address when wallet changes
            if self.predefined_address.is_none() {
                self.funding_address = None;
            }
            // Reset funding method and asset lock when wallet changes
            self.funding_method = FundingMethod::NoSelection;
            self.selected_asset_lock = None;
            response.wallet_changed = Some(wallet);
            response.funding_method_changed = Some(FundingMethod::NoSelection);
        }

        true
    }

    fn render_funding_method_selection(
        &mut self,
        ui: &mut Ui,
        response: &mut FundingWidgetResponse,
    ) {
        let Some(selected_wallet) = self.selected_wallet.clone() else {
            return;
        };

        // If address is predefined, only show QR code method and don't allow changing
        if self.predefined_address.is_some() {
            ui.label("Funding Method:");
            ui.label("Address with QR Code (predefined address)");
            return;
        }

        ui.label("Funding Method:");

        let mut method_changed = None;
        ComboBox::from_id_salt("funding_widget_method_selection")
            .selected_text(format!("{}", self.funding_method))
            .show_ui(ui, |ui| {
                let mut temp_method = self.funding_method;
                if ui
                    .selectable_value(
                        &mut temp_method,
                        FundingMethod::NoSelection,
                        "Please select funding method",
                    )
                    .changed()
                {
                    method_changed = Some(temp_method);
                }

                let (has_unused_asset_lock, has_balance) = {
                    let wallet = selected_wallet.read().unwrap();
                    (wallet.has_unused_asset_lock(), wallet.has_balance())
                };

                // Use Unused Asset Lock option
                ui.add_enabled_ui(has_unused_asset_lock, |ui| {
                    let response = ui.selectable_value(
                        &mut temp_method,
                        FundingMethod::UseUnusedAssetLock,
                        "Use Unused Asset Lock (recommended)",
                    );
                    if response.changed() {
                        method_changed = Some(temp_method);
                    }
                    if !has_unused_asset_lock {
                        response.on_disabled_hover_text(
                            "This wallet has no unused asset locks available",
                        );
                    }
                });

                // Use Wallet Balance option
                ui.add_enabled_ui(has_balance, |ui| {
                    let response = ui.selectable_value(
                        &mut temp_method,
                        FundingMethod::UseWalletBalance,
                        "Use Wallet Balance",
                    );
                    if response.changed() {
                        method_changed = Some(temp_method);
                    }
                    if !has_balance {
                        response.on_disabled_hover_text("This wallet has no available balance");
                    }
                });

                // Address with QR Code option (always available)
                if ui
                    .selectable_value(
                        &mut temp_method,
                        FundingMethod::AddressWithQRCode,
                        "Address with QR Code",
                    )
                    .changed()
                {
                    method_changed = Some(temp_method);
                }
            });

        if let Some(new_method) = method_changed {
            self.funding_method = new_method;
            // Reset asset lock when method changes
            self.selected_asset_lock = None;
            // Only set max amount when switching to wallet balance if using default amount
            if new_method == FundingMethod::UseWalletBalance {
                if let Some(wallet) = &self.selected_wallet {
                    let wallet = wallet.read().unwrap();
                    let max_amount = wallet.max_balance();
                    self.funding_amount = format!("{:.4}", max_amount as f64 * 1e-8);
                }
            }
            response.funding_method_changed = Some(new_method);
        }
    }

    fn render_amount_input(&mut self, ui: &mut Ui, response: &mut FundingWidgetResponse) {
        ui.horizontal(|ui| {
            ui.label(&self.amount_label);

            let amount_input = ui
                .add(
                    egui::TextEdit::singleline(&mut self.funding_amount)
                        .hint_text("Enter amount (e.g., 0.1234)")
                        .desired_width(100.0),
                )
                .lost_focus();

            if amount_input {
                response.amount_changed = Some(self.funding_amount.clone());
            }

            // Show Max button if using wallet balance and it's enabled in config
            if self.show_max_button && self.funding_method == FundingMethod::UseWalletBalance {
                if let Some(wallet) = &self.selected_wallet {
                    let max_amount = {
                        let wallet = wallet.read().unwrap();
                        wallet.max_balance()
                    };
                    if ui.button("Max").clicked() {
                        self.funding_amount = format!("{:.4}", max_amount as f64 * 1e-8);
                        response.max_button_clicked = true;
                        response.amount_changed = Some(self.funding_amount.clone());
                    }
                }
            }

            // Validate amount
            if self.funding_amount.parse::<f64>().is_err()
                || self.funding_amount.parse::<f64>().unwrap_or_default() <= 0.0
            {
                ui.colored_label(Color32::DARK_RED, "Invalid amount");
            }
        });
    }

    fn ensure_funding_address(
        &mut self,
        response: &mut FundingWidgetResponse,
    ) -> Result<Address, String> {
        // Use predefined address if available
        if let Some(address) = &self.predefined_address {
            return Ok(address.clone());
        }

        // Generate new address if needed
        if self.funding_address.is_none() {
            let Some(wallet_guard) = self.selected_wallet.as_ref() else {
                return Err("No wallet selected".to_string());
            };

            let receive_address = {
                let mut wallet = wallet_guard.write().unwrap();
                wallet.receive_address(self.app_context.network, false, Some(&self.app_context))?
            };

            // Import address to Core if needed
            if let Some(has_address) = self.core_has_funding_address {
                if !has_address {
                    self.app_context
                        .core_client
                        .read()
                        .expect("Core client lock was poisoned")
                        .import_address(
                            &receive_address,
                            Some("Managed by Dash Evo Tool"),
                            Some(false),
                        )
                        .map_err(|e| e.to_string())?;
                }
            } else {
                let info = self
                    .app_context
                    .core_client
                    .read()
                    .expect("Core client lock was poisoned")
                    .get_address_info(&receive_address)
                    .map_err(|e| e.to_string())?;

                if !(info.is_watchonly || info.is_mine) {
                    self.app_context
                        .core_client
                        .read()
                        .expect("Core client lock was poisoned")
                        .import_address(
                            &receive_address,
                            Some("Managed by Dash Evo Tool"),
                            Some(false),
                        )
                        .map_err(|e| e.to_string())?;
                }
                self.core_has_funding_address = Some(true);
            }

            // Store the address and emit event
            self.funding_address = Some(receive_address.clone());
            response.address_changed = Some(receive_address);
        }

        Ok(self.funding_address.as_ref().unwrap().clone())
    }

    fn render_qr_code(
        &mut self,
        ui: &mut Ui,
        response: &mut FundingWidgetResponse,
    ) -> Result<(), String> {
        if !self.show_qr_code {
            return Ok(());
        }

        let Ok(amount) = self.funding_amount.parse::<f64>() else {
            return Err("Invalid amount".to_string());
        };

        if amount <= 0.0 {
            return Err("Amount must be greater than 0".to_string());
        }

        let address = self.ensure_funding_address(response)?;
        let pay_uri = format!("{}?amount={:.4}", address.to_qr_uri(), amount);

        // Generate the QR code image
        if let Ok(qr_image) = generate_qr_code_image(&pay_uri) {
            let texture: TextureHandle = ui.ctx().load_texture(
                "funding_widget_qr_code",
                qr_image,
                egui::TextureOptions::LINEAR,
            );
            ui.vertical_centered(|ui| {
                ui.image(&texture);
            });
        } else {
            ui.vertical_centered(|ui| {
                ui.label("Failed to generate QR code.");
            });
        }

        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            ui.label(&pay_uri);

            if self.show_copy_button {
                ui.add_space(5.0);
                if ui.button("Copy Address").clicked() {
                    self.copied_to_clipboard = Some(copy_to_clipboard(&pay_uri).err());
                    response.copy_button_clicked = true;
                }

                if let Some(error) = &self.copied_to_clipboard {
                    ui.add_space(5.0);
                    if let Some(error) = error {
                        ui.label(format!("Failed to copy to clipboard: {}", error));
                    } else {
                        ui.label("Address copied to clipboard.");
                    }
                }
            }
        });
        Ok(())
    }

    fn render_asset_lock_selection(
        &mut self,
        ui: &mut Ui,
        response: &mut FundingWidgetResponse,
    ) {
        let Some(selected_wallet) = self.selected_wallet.clone() else {
            ui.label("No wallet selected.");
            return;
        };

        let wallet = selected_wallet.read().unwrap();

        if wallet.unused_asset_locks.is_empty() {
            ui.label("No unused asset locks available.");
            return;
        }

        ui.heading("Select an unused asset lock:");

        // Track the index of the currently selected asset lock (if any)
        let selected_index = self.selected_asset_lock.as_ref().and_then(|(_, proof, _)| {
            wallet
                .unused_asset_locks
                .iter()
                .position(|(_, _, _, _, p)| p.as_ref() == Some(proof))
        });

        // Display the asset locks in a scrollable area
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, (tx, address, amount, islock, proof)) in
                wallet.unused_asset_locks.iter().enumerate()
            {
                ui.horizontal(|ui| {
                    let tx_id = tx.txid().to_string();
                    let lock_amount = *amount as f64 * 1e-8; // Convert to DASH
                    let is_locked = if islock.is_some() { "Yes" } else { "No" };

                    // Display asset lock information with "Selected" if this one is selected
                    let selected_text = if Some(index) == selected_index {
                        " (Selected)"
                    } else {
                        ""
                    };

                    ui.label(format!(
                        "TxID: {}, Address: {}, Amount: {:.8} DASH, InstantLock: {}{}",
                        tx_id, address, lock_amount, is_locked, selected_text
                    ));

                    // Button to select this asset lock
                    if ui.button("Select").clicked() {
                        // Update the selected asset lock
                        let selected_lock = (
                            tx.clone(),
                            proof.clone().expect("Asset lock proof is required"),
                            address.clone(),
                        );
                        self.selected_asset_lock = Some(selected_lock.clone());
                        response.asset_lock_selected = Some(selected_lock);
                    }
                });

                ui.add_space(5.0); // Add space between each entry
            }
        });
    }
}

impl FundingWidget {
    /// Render the funding widget and return response with all changes
    pub fn show(&mut self, ui: &mut Ui) -> InnerResponse<FundingWidgetResponse> {
        let mut response = FundingWidgetResponse::default();

        let ui_response = ui.vertical(|ui| {
            // Wallet selection (if not predefined)
            if self.render_wallet_selection(ui, &mut response) {
                ui.add_space(10.0);
            }

            // Only show funding options if wallet is selected
            if self.selected_wallet.is_some() {
                // Funding method selection
                self.render_funding_method_selection(ui, &mut response);
                ui.add_space(10.0);

                // Amount input
                if self.funding_method != FundingMethod::NoSelection {
                    self.render_amount_input(ui, &mut response);
                    ui.add_space(10.0);

                    // Asset lock selection for UseUnusedAssetLock method
                    if self.funding_method == FundingMethod::UseUnusedAssetLock {
                        self.render_asset_lock_selection(ui, &mut response);
                        ui.add_space(10.0);
                    }

                    // QR code for address-based funding
                    if self.funding_method == FundingMethod::AddressWithQRCode {
                        if let Err(e) = self.render_qr_code(ui, &mut response) {
                            response.error = Some(e);
                        }
                    }
                }
            } else if self.predefined_wallet.is_none() {
                ui.label("Please select a wallet to continue.");
            }
        });

        InnerResponse::new(response, ui_response.response)
    }
}

impl Widget for &mut FundingWidget {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        self.show(ui).response
    }
}
