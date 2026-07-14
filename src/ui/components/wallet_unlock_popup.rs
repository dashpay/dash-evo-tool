use crate::backend_task::error::TaskError;
use crate::context::{AppContext, WalletUnlockRetention};
use crate::model::wallet::Wallet;
use crate::ui::components::passphrase_modal::{
    KEEP_UNLOCKED_LABEL, PassphraseModalConfig, PassphraseModalOutcome,
    clear_passphrase_modal_state, passphrase_modal,
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

/// Result of showing the migration-specific wallet unlock prompt.
#[derive(Debug, Clone, PartialEq)]
pub enum MigrationWalletUnlockResult {
    /// The prompt is still awaiting a choice.
    Pending,
    /// The wallet was successfully unlocked.
    Unlocked,
    /// The wallet was skipped for this migration run.
    Skipped,
}

enum UnlockInteraction {
    Pending,
    Unlocked,
    Cancelled,
    Skipped,
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
    /// Typed storage failure from the secret seam. Kept typed until render.
    storage_error: Option<TaskError>,
    /// Whether the user opted to keep the seed in the session cache after this
    /// unlock. The secure default is `false` — the seed is promoted to the
    /// session cache only when the user ticks the box; otherwise the next
    /// operation re-prompts.
    remember: bool,
    active_modal: Option<(egui::Context, egui::Id)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnlockMode {
    Standard,
    Migration,
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
            storage_error: None,
            remember: false,
            active_modal: None,
        }
    }

    /// Open the popup
    pub fn open(&mut self) {
        self.is_open = true;
        self.error = None;
        self.storage_error = None;
        self.remember = false;
    }

    /// Close the popup
    pub fn close(&mut self) {
        if let Some((ctx, state_id)) = self.active_modal.take() {
            clear_passphrase_modal_state(&ctx, state_id);
        }
        self.is_open = false;
        self.error = None;
        self.storage_error = None;
    }

    /// Check if the popup is currently open
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    fn activate_modal(&mut self, ctx: &egui::Context, modal_state_id: egui::Id) {
        if self
            .active_modal
            .as_ref()
            .is_some_and(|(_, active_id)| *active_id != modal_state_id)
            && let Some((old_ctx, old_id)) = self.active_modal.take()
        {
            clear_passphrase_modal_state(&old_ctx, old_id);
        }
        self.active_modal = Some((ctx.clone(), modal_state_id));
    }

    /// Show the popup and handle wallet unlock.
    /// Returns the result of the unlock attempt.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        wallet: &Arc<RwLock<Wallet>>,
        app_context: &Arc<AppContext>,
    ) -> WalletUnlockResult {
        match self.show_with_mode(ctx, wallet, app_context, UnlockMode::Standard) {
            UnlockInteraction::Pending | UnlockInteraction::Skipped => WalletUnlockResult::Pending,
            UnlockInteraction::Unlocked => WalletUnlockResult::Unlocked,
            UnlockInteraction::Cancelled => WalletUnlockResult::Cancelled,
        }
    }

    /// Show a non-dismissible unlock prompt required by wallet migration.
    pub fn show_for_migration(
        &mut self,
        ctx: &egui::Context,
        wallet: &Arc<RwLock<Wallet>>,
        app_context: &Arc<AppContext>,
    ) -> MigrationWalletUnlockResult {
        match self.show_with_mode(ctx, wallet, app_context, UnlockMode::Migration) {
            UnlockInteraction::Pending | UnlockInteraction::Cancelled => {
                MigrationWalletUnlockResult::Pending
            }
            UnlockInteraction::Unlocked => MigrationWalletUnlockResult::Unlocked,
            UnlockInteraction::Skipped => MigrationWalletUnlockResult::Skipped,
        }
    }

    fn show_with_mode(
        &mut self,
        ctx: &egui::Context,
        wallet: &Arc<RwLock<Wallet>>,
        app_context: &Arc<AppContext>,
        mode: UnlockMode,
    ) -> UnlockInteraction {
        if !self.is_open {
            return UnlockInteraction::Pending;
        }

        let (wallet_alias, seed_hash) = {
            let wallet = wallet.read_recover();
            (
                wallet.alias.clone().unwrap_or_else(|| "Wallet".to_string()),
                wallet.seed_hash(),
            )
        };
        let modal_state_id = egui::Id::new("wallet_unlock_passphrase").with(seed_hash);
        self.activate_modal(ctx, modal_state_id);

        let (window_title, body, submit_label, secondary_action_label, cancellable) = match mode {
            UnlockMode::Standard => (
                "Unlock Wallet",
                format!("Enter password to unlock \"{wallet_alias}\":"),
                "Unlock",
                None,
                true,
            ),
            UnlockMode::Migration => (
                "Continue the storage update",
                migration_prompt_body(&wallet_alias),
                "Continue",
                Some("Skip this wallet"),
                false,
            ),
        };

        let storage_error = self.storage_error.as_ref().map(ToString::to_string);
        let config = PassphraseModalConfig {
            state_id: modal_state_id,
            window_title,
            body: &body,
            hint: None,
            error: storage_error.as_deref().or(self.error.as_deref()),
            submit_label,
            secondary_action_label,
            input_placeholder: "Enter your password.",
            remember_label: None,
            cancellable,
        };

        let mut remember = self.remember;
        let outcome = passphrase_modal(ctx, &config, |ui| {
            if mode == UnlockMode::Standard {
                ui.checkbox(
                    &mut remember,
                    config.remember_label.unwrap_or(KEEP_UNLOCKED_LABEL),
                );
            } else {
                ui.label(migration_skip_body());
            }
        });
        self.remember = remember;

        match outcome {
            PassphraseModalOutcome::Pending => UnlockInteraction::Pending,
            PassphraseModalOutcome::Cancel => {
                if mode == UnlockMode::Migration {
                    return UnlockInteraction::Pending;
                }
                self.close();
                UnlockInteraction::Cancelled
            }
            PassphraseModalOutcome::SecondaryAction => {
                if mode != UnlockMode::Migration {
                    return UnlockInteraction::Pending;
                }
                self.close();
                UnlockInteraction::Skipped
            }
            PassphraseModalOutcome::Submit(text) => {
                let mut wallet_guard = wallet.write_recover();
                match wallet_guard.wallet_seed.open(&text) {
                    Ok(_) => {
                        drop(wallet_guard);
                        let retention = if self.remember || mode == UnlockMode::Migration {
                            WalletUnlockRetention::UntilAppClose
                        } else {
                            WalletUnlockRetention::OperationOnly
                        };
                        let passphrase = Zeroizing::new((*text).clone());
                        match app_context.handle_wallet_unlocked(wallet, &passphrase, retention) {
                            Ok(()) => {
                                self.close();
                                UnlockInteraction::Unlocked
                            }
                            Err(error) => {
                                self.error = None;
                                self.storage_error = Some(error);
                                UnlockInteraction::Pending
                            }
                        }
                    }
                    Err(_) => {
                        self.storage_error = None;
                        self.error = Some(match wallet_guard.password_hint() {
                            Some(hint) => format!(
                                "That password did not match. Check it and try again. Hint: {hint}"
                            ),
                            None => {
                                "That password did not match. Check it and try again.".to_string()
                            }
                        });
                        UnlockInteraction::Pending
                    }
                }
            }
        }
    }
}

fn migration_prompt_body(wallet_alias: &str) -> String {
    format!("Enter the password for \"{wallet_alias}\" to update this wallet now.")
}

fn migration_skip_body() -> &'static str {
    "You can skip this wallet if you do not know its password. It will stay locked and will not be updated now. Its storage update will finish the next time you unlock it with its password. Your coins are not lost."
}

/// Helper function to check if a wallet needs unlocking
pub fn wallet_needs_unlock(wallet: &Arc<RwLock<Wallet>>) -> bool {
    let wallet_guard = wallet.read_recover();
    wallet_guard.requires_password_unlock()
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

    #[test]
    fn migration_prompt_explains_why_the_password_is_required_now() {
        assert_eq!(
            migration_prompt_body("Savings"),
            "Enter the password for \"Savings\" to update this wallet now.",
        );
        assert_eq!(
            migration_skip_body(),
            "You can skip this wallet if you do not know its password. It will stay locked and will not be updated now. Its storage update will finish the next time you unlock it with its password. Your coins are not lost.",
        );
    }

    #[test]
    fn switching_wallets_clears_the_previous_modal_state() {
        use crate::ui::components::passphrase_modal::passphrase_modal_state_exists;

        let ctx = egui::Context::default();
        let wallet_a = egui::Id::new("wallet_unlock_passphrase").with([0xA1u8; 32]);
        let wallet_b = egui::Id::new("wallet_unlock_passphrase").with([0xB2u8; 32]);
        let config = PassphraseModalConfig {
            state_id: wallet_a,
            window_title: "Continue the storage update",
            body: "Enter the password for this wallet.",
            hint: None,
            error: None,
            submit_label: "Continue",
            secondary_action_label: Some("Skip this wallet"),
            input_placeholder: "Enter your password.",
            remember_label: None,
            cancellable: false,
        };
        let _ = ctx.run_ui(Default::default(), |ui| {
            let _ = passphrase_modal(ui.ctx(), &config, |_| {});
        });
        assert!(passphrase_modal_state_exists(&ctx, wallet_a));

        let mut popup = WalletUnlockPopup::new();
        popup.active_modal = Some((ctx.clone(), wallet_a));
        popup.activate_modal(&ctx, wallet_b);

        assert!(
            !passphrase_modal_state_exists(&ctx, wallet_a),
            "switching wallets must clear wallet A's typed-buffer state",
        );
        assert_eq!(
            popup.active_modal.as_ref().map(|(_, id)| *id),
            Some(wallet_b)
        );
    }
}
