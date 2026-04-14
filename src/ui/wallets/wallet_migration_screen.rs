use crate::app::AppAction;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::database::Database;
use crate::database::LockedWalletInfo;
use crate::model::wallet::derive_wallet_id_from_seed;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::{BannerHandle, OptionBannerExt};
use crate::ui::{MessageType, ScreenLike};
use eframe::egui::Context;
use egui::{Grid, RichText, Ui};
use std::sync::Arc;

/// Typed errors from [`WalletMigrationEntry::try_unlock`].
///
/// User-facing messages live in the `#[error(...)]` attributes so the
/// UI can render them via `Display` at the call site — no user-facing
/// strings are stored in variant fields (per project convention).
#[cfg_attr(feature = "testing", allow(dead_code))]
#[derive(Debug, thiserror::Error)]
pub(crate) enum UnlockError {
    #[error("Enter the wallet password to continue.")]
    EmptyPassword,
    #[error("The password is incorrect. Check the password and try again.")]
    WrongPassword,
    #[error(
        "Could not derive a wallet key. The wallet may be corrupted — try re-importing your recovery phrase."
    )]
    DeriveFailed,
    #[error(
        "Could not derive a valid wallet key. The wallet may be corrupted — try re-importing your recovery phrase."
    )]
    ZeroWalletId,
    #[error("Could not save the wallet key. Please restart the application and try again.")]
    Persistence(#[source] rusqlite::Error),
}

/// Per-wallet entry state during migration. `Error` carries the
/// user-facing message so it stays paired with the status — callers
/// can't have `status == Error` with no message or vice versa.
#[cfg_attr(feature = "testing", allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum EntryStatus {
    /// Waiting for the user to enter a password.
    Idle,
    /// Unlock succeeded — wallet_id has been written.
    Done,
    /// Wrong password, derivation, or DB error. Message is
    /// user-ready (from `UnlockError::Display`).
    Error { message: String },
}

/// One row in the migration screen — one per locked wallet.
// INTENTIONAL: pub(crate) items are used in production (non-testing) code paths.
// The testing feature gates out the production constructor, causing false dead_code warnings.
#[cfg_attr(feature = "testing", allow(dead_code))]
pub(crate) struct WalletMigrationEntry {
    info: LockedWalletInfo,
    password_input: PasswordInput,
    status: EntryStatus,
}

#[cfg_attr(feature = "testing", allow(dead_code))]
impl WalletMigrationEntry {
    pub(crate) fn new(info: LockedWalletInfo) -> Self {
        let mut password_input = PasswordInput::new().with_hint_text("Wallet password");
        if let Some(hint) = &info.password_hint {
            password_input =
                PasswordInput::new().with_hint_text(format!("Password (hint: {hint})"));
        }
        Self {
            info,
            password_input,
            status: EntryStatus::Idle,
        }
    }

    /// Human-readable label for this wallet. Prefers the user-supplied alias;
    /// falls back to a short wallet fingerprint (hex prefix of the legacy seed_hash DB key).
    fn label(&self) -> String {
        if let Some(alias) = &self.info.alias
            && !alias.is_empty()
        {
            return alias.clone();
        }
        // Show first 8 hex chars as a wallet fingerprint derived from the DB key.
        let fp = hex::encode(&self.info.seed_hash[..4]);
        format!("Wallet …{fp}")
    }

    /// Set the password input text (used in tests to simulate user input).
    pub(crate) fn set_password(&mut self, password: &str) {
        self.password_input.set_text(password);
    }

    /// Attempt to decrypt the seed with the current password, derive wallet_id,
    /// and write it to the database. Returns `Ok(())` on success or a typed
    /// [`UnlockError`]; the caller translates to a user message via `Display`.
    pub(crate) fn try_unlock(&mut self, db: &Database) -> Result<(), UnlockError> {
        let password = self.password_input.text().to_string();
        if password.is_empty() {
            return Err(UnlockError::EmptyPassword);
        }

        // Decrypt the seed via the shared helper — no dummy
        // `ClosedKeyItem` with a fake `wallet_id` required.
        let seed = self
            .info
            .try_decrypt(&password)
            .map_err(|_| UnlockError::WrongPassword)?;

        let wallet_id = derive_wallet_id_from_seed(&seed, self.info.network)
            .ok_or(UnlockError::DeriveFailed)?;

        // Zero out seed immediately after use — we only need wallet_id.
        // derive_password_key and derive_wallet_id_from_seed both work on
        // local copies, so no further scrubbing is needed there. The seed
        // array is on the stack and will be zeroed here.
        let mut seed_zeroed = seed;
        zeroize::Zeroize::zeroize(&mut seed_zeroed);

        // A `wallet_id` of all-zero bytes is cryptographically
        // impossible from a real seed (would require a SHA-256
        // collision against the empty pre-image). Treat it as a
        // sentinel for derivation failure — refuse to write it,
        // since a NULL wallet_id surfaces in the migration screen
        // for retry while a zeroed wallet_id would silently bind
        // every future persister write to the wrong key.
        if wallet_id == [0u8; 32] {
            tracing::error!(
                db_seed_hash = %hex::encode(self.info.seed_hash),
                "WalletMigrationScreen: derived wallet_id is all-zero — refusing to write"
            );
            return Err(UnlockError::ZeroWalletId);
        }

        db.set_wallet_id_for_locked_wallet(&self.info.seed_hash, &wallet_id)
            .map_err(|e| {
                tracing::warn!(
                    db_seed_hash = %hex::encode(self.info.seed_hash),
                    error = %e,
                    "WalletMigrationScreen: failed to write wallet_id to database"
                );
                UnlockError::Persistence(e)
            })?;

        // Clear password from memory.
        self.password_input.clear();
        Ok(())
    }
}

/// Modal blocking screen shown on first launch after a v33→v34 database
/// upgrade when password-protected wallets exist.
///
/// Prompts the user to enter each wallet's password so the migration can
/// derive and persist `wallet_id`. Once all wallets are unlocked, signals
/// `completed = true` to `AppState`, which then retries `Database::initialize`.
///
/// This screen has no `AppContext` — it is constructed before the main
/// `AppContext` is available. It only uses `Arc<Database>`.
pub struct WalletMigrationScreen {
    db: Arc<Database>,
    wallets: Vec<WalletMigrationEntry>,
    /// Global banner used to signal overall progress or errors.
    refresh_banner: Option<BannerHandle>,
    /// Set to `true` once all wallets are marked Done.
    pub completed: bool,
}

#[cfg_attr(feature = "testing", allow(dead_code))]
impl WalletMigrationScreen {
    /// Create a new migration screen for the given locked wallets.
    pub fn new(db: Arc<Database>, locked: Vec<LockedWalletInfo>) -> Self {
        let wallets = locked.into_iter().map(WalletMigrationEntry::new).collect();
        Self {
            db,
            wallets,
            refresh_banner: None,
            completed: false,
        }
    }

    /// Render one row for a wallet that needs unlocking.
    fn show_wallet_row(entry: &mut WalletMigrationEntry, ui: &mut Ui, db: &Database) {
        let label = entry.label();

        ui.label(RichText::new(&label).strong());

        match &entry.status {
            EntryStatus::Done => {
                ui.label(RichText::new("Unlocked").color(crate::ui::theme::DashColors::SUCCESS));
                ui.end_row();
            }
            EntryStatus::Idle | EntryStatus::Error { .. } => {
                let resp = entry.password_input.show(ui);

                let unlock_clicked = ui.button("Unlock").clicked();
                let enter_pressed =
                    resp.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                if unlock_clicked || enter_pressed {
                    match entry.try_unlock(db) {
                        Ok(()) => {
                            entry.status = EntryStatus::Done;
                        }
                        Err(err) => {
                            let message = err.to_string();
                            entry.password_input.set_error(Some(message.clone()));
                            entry.status = EntryStatus::Error { message };
                        }
                    }
                } else if matches!(entry.status, EntryStatus::Error { .. }) {
                    // Keep the error visible until the user changes the password.
                } else {
                    entry.password_input.set_error(None);
                }

                ui.end_row();
            }
        }
    }

    /// Render the main content area.
    fn show_content(&mut self, ui: &mut Ui) -> AppAction {
        ui.add_space(20.0);
        ui.heading("Unlock password-protected wallets");
        ui.add_space(8.0);
        ui.label(
            "Your wallets need to be unlocked once so the app can complete a one-time upgrade. \
             After this, you will not be asked again.",
        );
        ui.add_space(16.0);

        let all_done = self.wallets.iter().all(|w| w.status == EntryStatus::Done);

        if all_done {
            ui.label(
                RichText::new("All wallets unlocked. You can continue.")
                    .color(crate::ui::theme::DashColors::SUCCESS),
            );
            ui.add_space(12.0);
            if ui.button("Continue").clicked() {
                self.completed = true;
            }
            return AppAction::None;
        }

        let db = self.db.clone();

        Grid::new("wallet_migration_grid")
            .num_columns(3)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                // Header row
                ui.label(RichText::new("Wallet").strong());
                ui.label(RichText::new("Password").strong());
                ui.label(RichText::new("Status").strong());
                ui.end_row();

                for entry in &mut self.wallets {
                    Self::show_wallet_row(entry, ui, &db);
                }
            });

        AppAction::None
    }
}

impl ScreenLike for WalletMigrationScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        island_central_panel(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| self.show_content(ui))
                .inner
        })
    }

    fn display_task_result(&mut self, _result: BackendTaskSuccessResult) {}

    fn display_message(&mut self, _msg: &str, _message_type: MessageType) {
        self.refresh_banner.take_and_clear();
    }

    fn display_task_error(&mut self, _error: &TaskError) -> bool {
        self.refresh_banner.take_and_clear();
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::create_test_database;
    use crate::model::wallet::encryption::encrypt_message;
    use dash_sdk::dpp::dashcore::Network;

    /// Build a `LockedWalletInfo` for an encrypted seed on testnet.
    /// Used by tests that don't need raw DB access.
    pub(crate) fn make_locked_wallet_info(seed: &[u8; 64], password: &str) -> LockedWalletInfo {
        let (encrypted_seed, salt, nonce) = encrypt_message(seed, password).expect("encrypt");
        let seed_hash = crate::model::wallet::ClosedKeyItem::compute_seed_hash(seed);
        LockedWalletInfo {
            seed_hash,
            alias: Some("Test Wallet".to_string()),
            password_hint: None,
            encrypted_seed,
            salt,
            nonce,
            network: Network::Testnet,
        }
    }

    // -------------------------------------------------------------------------
    // WalletMigrationEntry::label
    // -------------------------------------------------------------------------

    #[test]
    fn test_entry_label_uses_alias_when_present() {
        let seed = [1u8; 64];
        let mut info = make_locked_wallet_info(&seed, "pw");
        info.alias = Some("My Wallet".to_string());
        let entry = WalletMigrationEntry::new(info);
        assert_eq!(entry.label(), "My Wallet");
    }

    #[test]
    fn test_entry_label_falls_back_to_fingerprint() {
        let seed = [2u8; 64];
        let mut info = make_locked_wallet_info(&seed, "pw");
        info.alias = None;
        let entry = WalletMigrationEntry::new(info);
        let label = entry.label();
        // Should contain "Wallet …" prefix followed by hex chars.
        assert!(label.starts_with("Wallet \u{2026}"), "label was: {label}");
    }

    // -------------------------------------------------------------------------
    // WalletMigrationEntry::try_unlock — empty password guard (no DB needed)
    // -------------------------------------------------------------------------

    #[test]
    fn test_try_unlock_empty_password_returns_error() {
        let seed = [5u8; 64];
        let info = make_locked_wallet_info(&seed, "any");
        // password_input is empty by default
        let db = create_test_database().unwrap();
        let mut entry = WalletMigrationEntry::new(info);
        // Password not set — should return typed EmptyPassword variant.
        let err = entry.try_unlock(&db).expect_err("must error on empty password");
        assert!(
            matches!(err, UnlockError::EmptyPassword),
            "expected UnlockError::EmptyPassword, got: {err:?}"
        );
    }

    // -------------------------------------------------------------------------
    // WalletMigrationScreen struct / completed flag
    // -------------------------------------------------------------------------

    #[test]
    fn test_wallet_migration_screen_compiles_and_has_correct_initial_state() {
        let db = Arc::new(create_test_database().unwrap());
        let screen = WalletMigrationScreen::new(db, vec![]);
        assert!(!screen.completed, "should not be completed on creation");
    }

    /// TC-C3: If the DB `UPDATE wallet SET wallet_id = ?` fails
    /// (e.g. the row was concurrently deleted, or the `wallet`
    /// table is missing), `try_unlock` must return a typed
    /// `UnlockError::Persistence` variant. The caller relies on
    /// this to keep the entry in `EntryStatus::Error` rather than
    /// flipping it to `Done` — a silent success on a failed DB
    /// write would leave the user thinking the wallet was
    /// migrated while it remained at `wallet_id IS NULL`, trapping
    /// them in the migration screen forever on the next launch.
    #[test]
    fn test_try_unlock_db_error_returns_persistence_variant() {
        let seed = [0x42u8; 64];
        let password = "correct_password";
        let info = make_locked_wallet_info(&seed, password);

        // Build the DB, then DROP the wallet table so the UPDATE
        // inside set_wallet_id_for_locked_wallet will fail.
        let db = create_test_database().unwrap();
        db.execute("DROP TABLE wallet", []).unwrap();

        let mut entry = WalletMigrationEntry::new(info);
        entry.set_password(password);

        let err = entry
            .try_unlock(&db)
            .expect_err("try_unlock must fail when the wallet table is gone");
        assert!(
            matches!(err, UnlockError::Persistence(_)),
            "expected UnlockError::Persistence, got: {err:?}"
        );
    }
}
