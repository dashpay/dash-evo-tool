use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::funding_common::{copy_to_clipboard, generate_qr_code_image};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::dashcore::{Address, OutPoint, TxOut};
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::Transaction;
use dash_sdk::dpp::prelude::AssetLockProof;
use eframe::epaint::TextureHandle;
use egui::{Color32, ComboBox, InnerResponse, Ui, Widget};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Funding method for the funding widget - doesn't require identity indices
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FundingWidgetMethod {
    UseAssetLock(Address, Box<AssetLockProof>, Box<Transaction>),
    FundWithUtxo(OutPoint, TxOut, Address),
    FundWithWallet(Duffs),
}

impl FundingWidgetMethod {
    /// Convert to RegisterIdentityFundingMethod with the provided identity_index
    pub fn to_register_identity_funding_method(
        self,
        identity_index: u32,
    ) -> crate::backend_task::identity::RegisterIdentityFundingMethod {
        use crate::backend_task::identity::RegisterIdentityFundingMethod;

        match self {
            FundingWidgetMethod::UseAssetLock(address, proof, transaction) => {
                RegisterIdentityFundingMethod::UseAssetLock(address, proof, transaction)
            }
            FundingWidgetMethod::FundWithUtxo(outpoint, tx_out, address) => {
                RegisterIdentityFundingMethod::FundWithUtxo(
                    outpoint,
                    tx_out,
                    address,
                    identity_index,
                )
            }
            FundingWidgetMethod::FundWithWallet(amount) => {
                RegisterIdentityFundingMethod::FundWithWallet(amount, identity_index)
            }
        }
    }
}

/// Response from the funding widget containing all state changes and actions
#[derive(Debug, Clone, Default)]
pub struct FundingWidgetResponse {
    /// Wallet selection changed
    pub wallet_changed: Option<Arc<RwLock<Wallet>>>,
    /// Funding method changed and has sufficient funds
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
    /// New address button was clicked
    pub new_address_clicked: bool,
    /// Error occurred
    pub error: Option<String>,
    /// Whether the currently selected funding method has sufficient funds
    pub funding_secured: Option<FundingWidgetMethod>,
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
            || self.new_address_clicked
            || self.error.is_some()
            || self.funding_secured.is_some()
    }

    /// Return true when the funding has been secured (funding method is selected and has sufficient funds)
    pub fn funded(&self) -> bool {
        self.funding_secured.is_some()
    }

    /// Call a function when the funding has been secured
    pub fn on_funded(self, mut f: impl FnMut(&FundingWidgetResponse)) -> Self {
        if self.funded() {
            f(&self)
        };
        self
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
    validation_hints: bool,
    ignore_existing_utxos: bool,

    // Current state
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    funding_method: FundingMethod,
    funding_amount: String,
    funding_address: Option<Address>,
    selected_asset_lock: Option<(Transaction, AssetLockProof, Address)>,
    core_has_funding_address: Option<bool>,
    copied_to_clipboard: Option<Option<String>>,
    /// Track existing UTXOs at the time the address was set up
    /// This is used when ignore_existing_utxos is true to filter out pre-existing UTXOs
    existing_utxos_snapshot: Option<HashMap<OutPoint, TxOut>>,
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
            validation_hints: true,
            ignore_existing_utxos: true,
            selected_wallet: None,
            funding_method: FundingMethod::NoSelection,
            funding_amount: default_amount,
            funding_address: None,
            selected_asset_lock: None,
            core_has_funding_address: None,
            copied_to_clipboard: None,
            existing_utxos_snapshot: None,
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

    /// Control whether to show validation hints to the user (like, not enough funds).
    /// Usually used to disable hints when an operation is in progress.
    ///
    /// Defaults to 'true'
    pub fn with_validation_hints(mut self, enabled: bool) -> Self {
        self.validation_hints = enabled;
        self
    }

    /// Control whether to show validation hints to the user (like, not enough funds).
    /// Usually used to disable hints when an operation is in progress.
    ///
    /// Defaults to 'true'.
    pub fn set_validation_hints(&mut self, enabled: bool) {
        self.validation_hints = enabled;
    }

    /// Set whether to ignore existing UTXOs when using QR code method.
    /// This is useful for wallet top-up scenarios where the user wants to send NEW funds
    /// even if there are already sufficient funds at the address.
    pub fn with_ignore_existing_utxos(mut self, ignore: bool) -> Self {
        self.ignore_existing_utxos = ignore;
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

    /// Reset wallet-dependent settings when wallet changes
    fn reset_wallet_dependent_settings(&mut self, response: &mut FundingWidgetResponse) {
        // Only reset if address is not predefined
        if self.predefined_address.is_none() {
            self.funding_address = None;
            response.address_changed = None; // Clear any previous address
        }

        self.funding_method = FundingMethod::NoSelection;
        self.selected_asset_lock = None;
        self.core_has_funding_address = None;
        self.copied_to_clipboard = None;
        self.existing_utxos_snapshot = None;

        // Set response fields to notify about the resets
        response.funding_method_changed = Some(FundingMethod::NoSelection);
        response.asset_lock_selected = None; // Clear any previous asset lock selection
        response.funding_secured = None; // Clear any previous funding method readiness
    }

    /// Check if the currently selected funding method has sufficient funds for the specified amount
    /// Returns Some(FundingWidgetMethod) if ready, None if not ready
    fn check_funding_method_readiness(&self) -> Option<FundingWidgetMethod> {
        let Some(wallet) = &self.selected_wallet else {
            return None;
        };

        let amount = self.funding_amount.parse::<f64>().unwrap_or(0.0);
        if amount <= 0.0 {
            return None;
        }

        match self.funding_method {
            FundingMethod::UseUnusedAssetLock => {
                let wallet = wallet.read().unwrap();
                if wallet.has_unused_asset_lock() && self.selected_asset_lock.is_some() {
                    if let Some((transaction, asset_lock_proof, address)) =
                        &self.selected_asset_lock
                    {
                        // Check if the selected asset lock has sufficient funds
                        if let Some((_, _, lock_amount, _, _)) = wallet
                            .unused_asset_locks
                            .iter()
                            .find(|(_, addr, _, _, _)| addr == address)
                        {
                            let available_amount_dash = *lock_amount as f64 * 1e-8;
                            if available_amount_dash >= amount {
                                Some(FundingWidgetMethod::UseAssetLock(
                                    address.clone(),
                                    Box::new(asset_lock_proof.clone()),
                                    Box::new(transaction.clone()),
                                ))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            FundingMethod::UseWalletBalance => {
                if self.check_wallet_balance_sufficient(amount) {
                    let amount_duffs = (amount * 1e8) as u64;
                    Some(FundingWidgetMethod::FundWithWallet(amount_duffs))
                } else {
                    None
                }
            }
            FundingMethod::AddressWithQRCode => {
                if self.funding_address.is_some() {
                    // Try to get UTXO information
                    if let Some((outpoint, tx_out, addr)) = self.get_funding_utxo() {
                        Some(FundingWidgetMethod::FundWithUtxo(outpoint, tx_out, addr))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            FundingMethod::NoSelection => None,
        }
    }

    /// Check if the wallet balance has an UTXO that is sufficient for the specified amount
    fn check_wallet_balance_sufficient(&self, required_amount_dash: f64) -> bool {
        let Some(wallet_guard) = &self.selected_wallet else {
            return false;
        };

        let wallet = wallet_guard.read().unwrap();
        let max_balance_duffs = wallet.max_balance();
        let required_amount_duffs = (required_amount_dash * 1e8) as u64;

        max_balance_duffs >= required_amount_duffs
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
                    self.reset_wallet_dependent_settings(response);
                    response.wallet_changed = Some(wallet_clone);
                }
            }
            return false;
        }

        // Multiple wallets - show selection
        let selected_wallet_alias = self
            .selected_wallet
            .as_ref()
            .and_then(|wallet| {
                let wallet_guard = wallet.read().ok()?;
                let alias = wallet_guard
                    .alias
                    .clone()
                    .unwrap_or_else(|| "Unnamed Wallet".to_string());
                let balance = wallet_guard.max_balance() as f64 * 1e-8; // Convert to DASH
                Some(format!("{} ({:.8} DASH)", alias, balance))
            })
            .unwrap_or_else(|| "Select Wallet".to_string());

        ui.label("Select Wallet:");

        let mut wallet_selected = None;
        ComboBox::from_id_salt("funding_widget_wallet_selection")
            .selected_text(selected_wallet_alias)
            .show_ui(ui, |ui| {
                for wallet in wallets.values() {
                    let wallet_guard = wallet.read().ok();
                    let (wallet_alias, wallet_balance_dash) =
                        if let Some(wallet_guard) = wallet_guard {
                            let alias = wallet_guard
                                .alias
                                .clone()
                                .unwrap_or_else(|| "Unnamed Wallet".to_string());
                            let balance_dash = wallet_guard.max_balance() as f64 * 1e-8;
                            (alias, balance_dash)
                        } else {
                            ("Unnamed Wallet".to_string(), 0.0)
                        };

                    let display_text =
                        format!("{} ({:.4} DASH)", wallet_alias, wallet_balance_dash);

                    let is_selected = self
                        .selected_wallet
                        .as_ref()
                        .is_some_and(|selected| Arc::ptr_eq(selected, wallet));

                    if ui.selectable_label(is_selected, display_text).clicked() {
                        wallet_selected = Some(wallet.clone());
                    }
                }
            });
        // Drop wallets borrow before accessing self mutably
        drop(wallets);

        if let Some(wallet) = wallet_selected {
            self.selected_wallet = Some(wallet.clone());
            self.reset_wallet_dependent_settings(response);
            response.wallet_changed = Some(wallet);
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

            // Clear existing UTXOs snapshot; even if we switch to QR code method,
            // we want to capture the current state of UTXOs for future checks
            self.existing_utxos_snapshot = None;

            // Only set max amount when switching to wallet balance if using default amount
            if new_method == FundingMethod::UseWalletBalance {
                if let Some(wallet) = &self.selected_wallet {
                    let wallet = wallet.read().unwrap();
                    let max_amount = wallet.max_balance();
                    self.funding_amount = format!("{:.4}", max_amount as f64 * 1e-8);
                }
            }
            // Always report the method change, readiness will be checked separately
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
        });
    }

    fn render_funding_status_hints(&self, ui: &mut Ui) {
        if !self.validation_hints {
            return;
        }
        if self.funding_method == FundingMethod::NoSelection {
            return;
        };

        // Only show hints if we have a valid amount and a funding method selected
        let Ok(amount) = self.funding_amount.parse::<f64>() else {
            // display error if amount is invalid
            ui.add_space(5.0);
            ui.colored_label(
                Color32::from_rgb(255, 165, 0), // Orange color
                format!(
                    "⚠ Invalid funding amount: '{}'. Please enter a valid number.",
                    self.funding_amount
                ),
            );
            return;
        };

        if amount <= 0.0 {
            ui.add_space(5.0);
            ui.colored_label(
                Color32::from_rgb(255, 165, 0), // Orange color
                format!(
                    "⚠ Funding amount must be greater than zero. Current value: '{}'",
                    self.funding_amount
                ),
            );
            return;
        }

        // Use the centralized readiness check to determine if we should show hints
        let is_ready = self.check_funding_method_readiness().is_some();

        if is_ready {
            // Show positive feedback when funding method is ready
            ui.add_space(5.0);
            ui.colored_label(
                Color32::from_rgb(34, 139, 34), // Forest green
                match self.funding_method {
                    FundingMethod::UseUnusedAssetLock => {
                        "✅ Selected asset lock has sufficient funds and is ready to use"
                    }
                    FundingMethod::UseWalletBalance => {
                        "✅ Wallet has sufficient balance for this transaction"
                    }
                    FundingMethod::AddressWithQRCode => {
                        "✅ Address has received sufficient funds and is ready to use"
                    }
                    FundingMethod::NoSelection => "", // This shouldn't happen when is_ready is true
                },
            );
        } else if !is_ready {
            match self.funding_method {
                FundingMethod::UseWalletBalance => {
                    if let Some(wallet) = &self.selected_wallet {
                        let wallet = wallet.read().unwrap();
                        let available_balance = wallet.max_balance() as f64 * 1e-8;
                        ui.add_space(5.0);
                        ui.colored_label(
                            Color32::from_rgb(255, 165, 0), // Orange color
                            format!(
                                "⚠ Insufficient wallet balance. Available: {:.8} DASH, Required: {:.8} DASH",
                                available_balance, amount
                            )
                        );
                    }
                }
                FundingMethod::UseUnusedAssetLock => {
                    if let Some(wallet) = &self.selected_wallet {
                        let wallet = wallet.read().unwrap();
                        if !wallet.has_unused_asset_lock() {
                            ui.add_space(5.0);
                            ui.colored_label(
                                Color32::from_rgb(255, 165, 0), // Orange color
                                "⚠ No unused asset locks available in this wallet",
                            );
                        } else if self.selected_asset_lock.is_none() {
                            ui.add_space(5.0);
                            ui.colored_label(
                                Color32::from_rgb(100, 149, 237), // Cornflower blue
                                "ℹ Please select an asset lock from the list",
                            );
                        } else {
                            // Asset lock is selected but funding method is not ready,
                            // so the selected asset lock must have insufficient funds
                            if let Some((transaction, _proof, address)) = &self.selected_asset_lock
                            {
                                if let Some((_, _, lock_amount, _, _)) = wallet
                                    .unused_asset_locks
                                    .iter()
                                    .find(|(tx, addr, _, _, _)| {
                                        addr == address && tx == transaction
                                    })
                                {
                                    let available_amount = *lock_amount as f64 * 1e-8;
                                    ui.add_space(5.0);
                                    ui.colored_label(
                                        Color32::from_rgb(255, 165, 0), // Orange color
                                        format!(
                                            "⚠ Selected asset lock has insufficient funds. Available: {:.8} DASH, Required: {:.8} DASH",
                                            available_amount, amount
                                        )
                                    );
                                }
                            }
                        }
                    }
                }
                FundingMethod::AddressWithQRCode => {
                    // Check if we have a UTXO for this address
                    let utxo = self.get_funding_utxo();

                    if utxo.is_none() {
                        ui.add_space(5.0);
                        ui.colored_label(
                            Color32::from_rgb(100, 149, 237), // Cornflower blue
                            format!(
                                "ℹ Waiting for {:.8} DASH to be sent to the address above",
                                amount
                            ),
                        );
                        ui.add_space(3.0);
                        ui.colored_label(
                            Color32::from_rgb(100, 149, 237), // Cornflower blue
                            "💡 Important: The funds must be sent in a single transaction",
                        );
                    }
                }
                FundingMethod::NoSelection => {}
            }
        }
    }

    fn ensure_funding_address(
        &mut self,
        response: &mut FundingWidgetResponse,
        force_new: bool,
    ) -> Result<Address, String> {
        // Use predefined address if available and not forcing new
        if let Some(address) = &self.predefined_address {
            if !force_new {
                return Ok(address.clone());
            }
        }

        // Generate new address if needed or forced
        if self.funding_address.is_none() || force_new {
            let Some(wallet_guard) = self.selected_wallet.as_ref() else {
                return Err("No wallet selected".to_string());
            };

            let receive_address = {
                let mut wallet = wallet_guard.write().map_err(|e| e.to_string())?;
                // Always generate a new address when force_new is true
                wallet.receive_address(
                    self.app_context.network,
                    force_new,
                    Some(&self.app_context),
                )?
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
            response.address_changed = Some(receive_address.clone());

            // If ignore_existing_utxos is enabled, capture current UTXOs as "existing"
            // so we can filter them out later when checking for new funds
            if self.ignore_existing_utxos {
                self.capture_existing_utxos(&receive_address);
            }
        }

        Ok(self
            .funding_address
            .as_ref()
            .ok_or_else(|| "No funding address available".to_string())?
            .clone())
    }

    /// Capture existing UTXOs for the given address to track what was already there
    /// before we started waiting for new funds
    fn capture_existing_utxos(&mut self, address: &Address) {
        if let Some(wallet_guard) = &self.selected_wallet {
            let wallet = wallet_guard.read().unwrap();
            if let Some(utxos) = wallet.utxos.get(address) {
                // Store a snapshot of existing UTXOs
                self.existing_utxos_snapshot = Some(utxos.clone());
            } else {
                // No existing UTXOs for this address
                self.existing_utxos_snapshot = Some(HashMap::new());
            }
        }
    }

    /// Ensure existing UTXOs are captured when we have both wallet and address available
    /// and ignore_existing_utxos is enabled
    fn ensure_existing_utxos_captured(&mut self) {
        if !self.ignore_existing_utxos
            || self.funding_method != FundingMethod::AddressWithQRCode
            || self.selected_wallet.is_none()
            || self.funding_address.is_none()
            || self.existing_utxos_snapshot.is_some()
        {
            return;
        }

        let address = self.funding_address.as_ref().unwrap().clone();
        self.capture_existing_utxos(&address);
    }

    fn render_qr_code(
        &mut self,
        ui: &mut Ui,
        response: &mut FundingWidgetResponse,
    ) -> Result<(), String> {
        if !self.show_qr_code {
            return Ok(());
        }

        // error displayed in `render_funding_status_hints`
        let amount = self.funding_amount.parse::<f64>().unwrap_or_default();
        if amount <= 0.0 {
            self.render_funding_status_hints(ui);
            return Ok(());
        }

        let address = self.ensure_funding_address(response, false)?;
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

            // Show buttons if needed
            let show_copy = self.show_copy_button;
            let show_new_address = self.predefined_address.is_none(); // Hide when address is predefined

            if show_copy || show_new_address {
                ui.add_space(5.0);

                // Use horizontal layout with manual centering for proper alignment
                ui.horizontal(|ui| {
                    // Calculate available width and center the buttons manually
                    let available_width = ui.available_width();

                    // Estimate button widths (approximate values for centering calculation)
                    let copy_button_width = if show_copy { 100.0 } else { 0.0 };
                    let new_address_button_width = if show_new_address { 100.0 } else { 0.0 };
                    let spacing_width = if show_copy && show_new_address {
                        10.0
                    } else {
                        0.0
                    };
                    let total_content_width =
                        copy_button_width + new_address_button_width + spacing_width;

                    // Add left padding to center the content
                    let left_padding = (available_width - total_content_width).max(0.0) / 2.0;
                    ui.add_space(left_padding);

                    if show_copy && ui.button("Copy Address").clicked() {
                        self.copied_to_clipboard = Some(copy_to_clipboard(&pay_uri).err());
                        response.copy_button_clicked = true;
                    }

                    if show_copy && show_new_address {
                        ui.add_space(10.0);
                    }

                    if show_new_address && ui.button("New Address").clicked() {
                        // Generate a new address
                        if let Ok(_new_address) = self.ensure_funding_address(response, true) {
                            // The address has been updated, no additional action needed
                            response.new_address_clicked = true;
                        } else {
                            response.error = Some("Failed to generate new address".to_string());
                        }
                    }
                });
            }

            if let Some(error) = &self.copied_to_clipboard {
                ui.add_space(5.0);
                if let Some(error) = error {
                    ui.label(format!("Failed to copy to clipboard: {}", error));
                } else {
                    ui.label("Address copied to clipboard.");
                }
            }

            // Show funding status for QR code method
            self.render_funding_status_hints(ui);
        });
        Ok(())
    }

    fn render_asset_lock_selection(&mut self, ui: &mut Ui, response: &mut FundingWidgetResponse) {
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
                        self.funding_amount = format!("{:.8}", lock_amount);
                        response.amount_changed = Some(self.funding_amount.clone());

                        // Update readiness status immediately after selection
                        response.funding_secured = self.check_funding_method_readiness();

                        // Request repaint to update hints immediately
                        ui.ctx().request_repaint();
                    }
                });

                ui.add_space(5.0); // Add space between each entry
            }
        });

        // Show validation message for selected asset lock
        self.render_funding_status_hints(ui);
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
                // Ensure existing UTXOs are captured when we have both wallet and address
                // and ignore_existing_utxos is enabled
                self.ensure_existing_utxos_captured();

                // Funding method selection
                self.render_funding_method_selection(ui, &mut response);
                ui.add_space(10.0);

                // Amount input
                if self.funding_method != FundingMethod::NoSelection {
                    // for asset locks, we just use asset lock amount
                    if self.funding_method != FundingMethod::UseUnusedAssetLock {
                        self.render_amount_input(ui, &mut response);
                        ui.add_space(10.0);
                    }

                    // Asset lock selection for UseUnusedAssetLock method
                    match self.funding_method {
                        FundingMethod::UseUnusedAssetLock => {
                            self.render_asset_lock_selection(ui, &mut response);
                            ui.add_space(10.0);
                        }

                        // QR code for address-based funding
                        FundingMethod::AddressWithQRCode => {
                            if let Err(e) = self.render_qr_code(ui, &mut response) {
                                response.error = Some(e);
                            }
                        }
                        FundingMethod::UseWalletBalance => {
                            // No additional UI for UseWalletBalance, just show hints
                            self.render_funding_status_hints(ui);
                        }
                        FundingMethod::NoSelection => {
                            ui.label("Please select a funding method.");
                        }
                    }
                }
            } else if self.predefined_wallet.is_none() {
                ui.label("Please select a wallet to continue.");
            }

            // Check if the current funding method is ready (has sufficient funds)
            response.funding_secured = self.check_funding_method_readiness();
        });

        InnerResponse::new(response, ui_response.response)
    }

    /// Get UTXO information for AddressWithQRCode funding method
    /// Returns (OutPoint, TxOut, Address) if a suitable UTXO is found
    ///
    /// Returns None if:
    /// - Funding method is not AddressWithQRCode
    /// - Funding address is not set
    /// - Funding amount is invalid or <= 0
    /// - No suitable UTXO found that meets the required amount
    /// - No wallet is selected
    /// - No UTXOs available for the funding address
    /// - Existing UTXOs snapshot contains the UTXO already
    pub fn get_funding_utxo(&self) -> Option<(OutPoint, TxOut, Address)> {
        if self.funding_method != FundingMethod::AddressWithQRCode {
            return None;
        }

        let funding_address = self.funding_address.as_ref()?;
        let amount_dash = self.funding_amount.parse::<f64>().ok()?;

        if amount_dash <= 0.0 {
            return None;
        }

        let wallet_guard = self.selected_wallet.as_ref()?;
        let wallet = wallet_guard.read().unwrap();
        let required_amount_duffs = (amount_dash * 1e8) as u64;

        // we don't use existing UTXOs snapshot if ignore_existing_utxos is enabled; when it's disabled, existing_utxos will be None
        let existing_utxos = self.existing_utxos_snapshot.as_ref();

        // Get UTXOs for the address
        if let Some(utxos) = wallet.utxos.get(funding_address) {
            utxos
                .iter()
                .find(|utxo| {
                    !existing_utxos.is_some_and(|snapshot| snapshot.contains_key(utxo.0))
                        && utxo.1.value >= required_amount_duffs
                })
                .map(|utxo| (*utxo.0, utxo.1.clone(), funding_address.clone()))
        } else {
            None
        }
    }
}

impl Widget for &mut FundingWidget {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        self.show(ui).response
    }
}
