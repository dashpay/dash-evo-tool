use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::passphrase_modal::{
    PassphraseModalConfig, PassphraseModalOutcome, passphrase_modal,
};
use crate::ui::components::password_input::PasswordInput;
use egui;
use std::sync::{Arc, RwLock};

/// Result of showing the wallet unlock popup
#[derive(Debug, Clone, PartialEq)]
pub enum WalletUnlockResult {
    /// Popup is still open, no action taken yet
    Pending,
    /// User successfully unlocked the wallet
    Unlocked,
    /// User cancelled the unlock
    Cancelled,
}

/// A popup dialog for unlocking a wallet with password
/// Similar to InfoPopup and ConfirmationDialog but specialized for wallet unlock flow
pub struct WalletUnlockPopup {
    is_open: bool,
    password_input: PasswordInput,
    error_message: Option<String>,
    focus_requested: bool,
}

impl Default for WalletUnlockPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl WalletUnlockPopup {
    /// Create a new wallet unlock popup
    pub fn new() -> Self {
        Self {
            is_open: false,
            password_input: PasswordInput::new().with_hint_text("Enter password"),
            error_message: None,
            focus_requested: false,
        }
    }

    /// Open the popup
    pub fn open(&mut self) {
        self.is_open = true;
        self.password_input.clear();
        self.error_message = None;
        self.focus_requested = false;
    }

    /// Close the popup
    pub fn close(&mut self) {
        self.is_open = false;
        self.password_input.clear();
        self.error_message = None;
    }

    /// Check if the popup is currently open
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Show the popup and handle wallet unlock
    /// Returns the result of the unlock attempt
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        wallet: &Arc<RwLock<Wallet>>,
        app_context: &Arc<AppContext>,
    ) -> WalletUnlockResult {
        if !self.is_open {
            return WalletUnlockResult::Pending;
        }

        let wallet_alias = wallet
            .read()
            .ok()
            .and_then(|w| w.alias.clone())
            .unwrap_or_else(|| "Wallet".to_string());

        let config = PassphraseModalConfig {
            window_title: "Unlock Wallet",
            body: &format!("Enter password to unlock \"{wallet_alias}\":"),
            hint: None,
            error: self.error_message.as_deref(),
            submit_label: "Unlock",
        };

        let outcome = passphrase_modal(
            ctx,
            &config,
            &mut self.password_input,
            &mut self.focus_requested,
            |_ui| {},
        );

        match outcome {
            PassphraseModalOutcome::Pending => WalletUnlockResult::Pending,
            PassphraseModalOutcome::Cancel => {
                self.close();
                WalletUnlockResult::Cancelled
            }
            PassphraseModalOutcome::Submit => {
                let mut wallet_guard = wallet.write().unwrap();
                match wallet_guard.wallet_seed.open(self.password_input.text()) {
                    Ok(_) => {
                        drop(wallet_guard);
                        // Promote the just-verified seed into the JIT session
                        // cache using the passphrase the user just entered, so
                        // the rest of the session does not re-prompt.
                        app_context
                            .handle_wallet_unlocked(wallet, Some(self.password_input.text()));
                        self.close();
                        WalletUnlockResult::Unlocked
                    }
                    Err(_) => {
                        if let Some(hint) = wallet_guard.password_hint() {
                            self.error_message =
                                Some(format!("Incorrect password. Hint: {}", hint));
                        } else {
                            self.error_message = Some("Incorrect password".to_string());
                        }
                        self.password_input.clear();
                        self.focus_requested = false;
                        WalletUnlockResult::Pending
                    }
                }
            }
        }
    }
}

/// Helper function to check if a wallet needs unlocking
pub fn wallet_needs_unlock(wallet: &Arc<RwLock<Wallet>>) -> bool {
    let wallet_guard = wallet.read().unwrap();
    wallet_guard.uses_password && !wallet_guard.is_open()
}

/// Open a no-password wallet and route it through the unlock chokepoint.
///
/// For wallets that do not use a password this flips the in-memory seed to
/// `Open` for display and then notifies [`AppContext::handle_wallet_unlocked`].
/// Signing pulls the seed just-in-time from the encrypted vault — a
/// no-password wallet signs even without this call (the chokepoint's
/// unprotected fast-path), so this is a UX convenience, not a correctness
/// gate; no passphrase is passed because there is none. Password wallets are a
/// no-op here — they unlock through the password popup, which promotes the
/// seed with the entered passphrase.
pub fn try_open_wallet_no_password(
    app_context: &Arc<AppContext>,
    wallet: &Arc<RwLock<Wallet>>,
) -> Result<(), String> {
    {
        let mut wallet_guard = wallet.write().unwrap();
        if wallet_guard.uses_password {
            return Ok(());
        }
        wallet_guard.wallet_seed.open_no_password()?;
    }
    // The write guard is dropped above; notify only after releasing it.
    app_context.handle_wallet_unlocked(wallet, None);
    Ok(())
}
