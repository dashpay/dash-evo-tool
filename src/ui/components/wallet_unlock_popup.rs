use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::passphrase_modal::{
    KEEP_UNLOCKED_LABEL, PassphraseModalConfig, PassphraseModalOutcome, passphrase_modal,
};
use crate::wallet_backend::poison::RwLockRecover;
use egui;
use std::sync::{Arc, RwLock};
use zeroize::Zeroizing;

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

/// A popup dialog for unlocking a wallet with password.
///
/// Thin wrapper around [`passphrase_modal`]: it stores only the two domain
/// fields (`remember`, `error`) plus the open/closed flag. All chrome —
/// overlay, window, `PasswordInput`, focus tracking, dismiss handling — lives
/// in `passphrase_modal`'s egui data-cache state.
pub struct WalletUnlockPopup {
    is_open: bool,
    /// Optional wrong-password message forwarded to `passphrase_modal`'s error
    /// line. Reset on open; set on a failed unlock attempt.
    error: Option<String>,
    /// Whether the user opted to keep the seed in the session cache after this
    /// unlock. The secure default is `false` — the seed is promoted to the
    /// session cache only when the user ticks the box; otherwise the next
    /// operation re-prompts.
    remember: bool,
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
            error: None,
            remember: false,
        }
    }

    /// Open the popup
    pub fn open(&mut self) {
        self.is_open = true;
        self.error = None;
        self.remember = false;
    }

    /// Close the popup
    pub fn close(&mut self) {
        self.is_open = false;
        self.error = None;
    }

    /// Check if the popup is currently open
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Show the popup and handle wallet unlock.
    /// Returns the result of the unlock attempt.
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
            error: self.error.as_deref(),
            submit_label: "Unlock",
            input_placeholder: "Enter password",
            remember_label: None,
        };

        let mut remember = self.remember;
        let outcome = passphrase_modal(ctx, &config, |ui| {
            ui.checkbox(
                &mut remember,
                config.remember_label.unwrap_or(KEEP_UNLOCKED_LABEL),
            );
        });
        self.remember = remember;

        match outcome {
            PassphraseModalOutcome::Pending => WalletUnlockResult::Pending,
            PassphraseModalOutcome::Cancel => {
                self.close();
                WalletUnlockResult::Cancelled
            }
            PassphraseModalOutcome::Submit(text) => {
                let mut wallet_guard = wallet.write_recover();
                match wallet_guard.wallet_seed.open(&text) {
                    Ok(_) => {
                        drop(wallet_guard);
                        // The wallet is already flipped open for display. Promote
                        // the just-verified seed into the session cache only when
                        // the user opted to keep it unlocked; the copy is zeroized
                        // on drop.
                        if self.remember {
                            let passphrase = Zeroizing::new((*text).clone());
                            app_context.handle_wallet_unlocked(wallet, &passphrase);
                        } else {
                            // Non-remember unlock: nothing to promote — the next
                            // operation re-prompts (secure default).
                            //
                            // TODO(det): a non-remember unlock (this branch, no
                            // passphrase handed to handle_wallet_unlocked) skips
                            // drive_unlock_registration, so the wallet is not
                            // re-registered with the upstream SPV backend until the
                            // next launch. Deferred 2026-07-08 pending a decision on
                            // whether this path should re-drive registration using
                            // the passphrase already verified by the unlock gesture
                            // itself (see the recorded wallet-unlock-registration
                            // gap in project memory).
                        }
                        self.close();
                        WalletUnlockResult::Unlocked
                    }
                    Err(_) => {
                        self.error = Some(match wallet_guard.password_hint() {
                            Some(hint) => format!(
                                "That password did not match. Check it and try again. Hint: {hint}"
                            ),
                            None => {
                                "That password did not match. Check it and try again.".to_string()
                            }
                        });
                        WalletUnlockResult::Pending
                    }
                }
            }
        }
    }
}

/// Helper function to check if a wallet needs unlocking
pub fn wallet_needs_unlock(wallet: &Arc<RwLock<Wallet>>) -> bool {
    let wallet_guard = wallet.read_recover();
    wallet_guard.uses_password && !wallet_guard.is_open()
}

/// Open a no-password wallet for display.
///
/// Flips the in-memory seed to `Open`. Signing pulls the seed just-in-time from
/// the encrypted vault — a no-password wallet signs even without this call (the
/// chokepoint's unprotected fast-path), so this is a UX convenience, not a
/// correctness gate. Password wallets are a no-op here — they unlock through the
/// password popup, which promotes the seed only when the user opts to keep the
/// wallet unlocked.
// TODO(cleanup): dead `_app_context` param — drop it and update the ~40 UI callsites.
pub fn try_open_wallet_no_password(
    _app_context: &Arc<AppContext>,
    wallet: &Arc<RwLock<Wallet>>,
) -> Result<(), String> {
    let mut wallet_guard = wallet.write_recover();
    if wallet_guard.uses_password {
        return Ok(());
    }
    if let Err(detail) = wallet_guard.wallet_seed.open_no_password() {
        // The raw error is a length-mismatch diagnostic (jargon). Log it
        // and return a calm, jargon-free message the callsite can show.
        tracing::error!(error = %detail, "Failed to open no-password wallet");
        return Err(
            "This wallet's saved data looks damaged and could not be opened. \
             Re-add it from its recovery phrase to restore it."
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_defaults_to_off() {
        let popup = WalletUnlockPopup::new();
        assert!(
            !popup.remember,
            "the keep-unlocked checkbox must default to off (secure default)"
        );
    }

    #[test]
    fn open_resets_remember_to_off() {
        let mut popup = WalletUnlockPopup::new();
        popup.remember = true;
        popup.open();
        assert!(
            !popup.remember,
            "reopening the popup must reset the keep-unlocked choice to off"
        );
    }
}
