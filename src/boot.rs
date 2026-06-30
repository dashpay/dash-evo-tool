//! GUI boot wrapper that un-bricks a passphrase-protected legacy seed vault.
//!
//! The seed vault is normally opened keyless at boot (obfuscation, not
//! confidentiality — see [`open_secret_store`](crate::wallet_backend::single_key::open_secret_store)).
//! A vault an older build sealed with a real passphrase fails that keyless
//! open with `SecretStoreError::WrongPassphrase`, which previously propagated
//! out of `AppState::new` and aborted startup before any window appeared.
//!
//! [`BootApp`] wraps the eframe app: when the keyless open fails *specifically*
//! with a wrong-passphrase, it renders the shared
//! [`passphrase_modal`](crate::ui::components::passphrase_modal) (proper masked
//! input, zeroized) in the SAME event loop (a second `eframe::run_native` is
//! not portable — winit forbids a second event loop on macOS) and re-opens the
//! SAME vault in place with the supplied passphrase. The open is
//! non-destructive — the vault is never deleted, recreated, or rekeyed — so
//! wallet seeds are never at risk. Every other boot failure stays fatal exactly
//! as before.
//!
//! Only the GUI binary boots through here; the headless MCP/CLI path keeps the
//! keyless open and surfaces the typed error instead of popping a dialog.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use platform_wallet_storage::secrets::SecretString;

use crate::app::AppState;
use crate::context::AppContext;
use crate::database::Database;
use crate::ui::components::passphrase_modal::{
    PassphraseModalConfig, PassphraseModalOutcome, passphrase_modal,
};

type BootError = Box<dyn std::error::Error + Send + Sync>;

/// The eframe application during boot: either still collecting the legacy
/// vault passphrase, or the fully built [`AppState`].
pub enum BootApp {
    /// The vault is passphrase-protected; collecting the passphrase.
    Unlocking(UnlockState),
    /// The vault is open and the app is running normally.
    Running(Box<AppState>),
    /// Unlock succeeded but app assembly failed (fatal, e.g. no network could
    /// be initialized). Renders nothing; a viewport close has been requested.
    Failed,
}

impl BootApp {
    /// Build the boot app, opening the seed vault keyless first.
    ///
    /// On success the full [`AppState`] is built immediately. If the keyless
    /// open fails *specifically* with a wrong vault passphrase, this returns
    /// [`BootApp::Unlocking`] so the frame loop can prompt for it. Any other
    /// failure (including app assembly) propagates as an error, aborting boot
    /// exactly as before.
    pub fn new(ctx: egui::Context) -> Result<Self, BootError> {
        let (data_dir, db) = AppState::boot_inputs()?;
        match AppContext::open_secret_store(&data_dir) {
            Ok(store) => Ok(BootApp::Running(Box::new(AppState::new_inner(
                ctx, db, data_dir, store,
            )?))),
            Err(e) if e.is_secret_store_wrong_passphrase() => {
                tracing::warn!(
                    "Seed vault is protected by a passphrase from an earlier version; \
                     prompting to unlock instead of aborting boot"
                );
                Ok(BootApp::Unlocking(UnlockState::new(ctx, db, data_dir)))
            }
            Err(e) => Err(Box::new(e)),
        }
    }

    /// Attempt to open the vault with the supplied passphrase and, on success,
    /// build the full app. A wrong passphrase re-arms the prompt; a genuine
    /// assembly failure is fatal (the viewport is closed).
    fn try_unlock(&mut self, passphrase: SecretString, ctx: &egui::Context) {
        let data_dir = match self {
            BootApp::Unlocking(state) => state.data_dir.clone(),
            _ => return,
        };
        match AppContext::open_secret_store_with_passphrase(&data_dir, passphrase) {
            Ok(store) => {
                let BootApp::Unlocking(state) = std::mem::replace(self, BootApp::Failed) else {
                    return;
                };
                match AppState::new_inner(state.ctx, state.db, state.data_dir, store) {
                    Ok(app) => *self = BootApp::Running(Box::new(app)),
                    Err(e) => {
                        tracing::error!(error = ?e, "Could not start the app after unlocking the vault");
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
            Err(e) if e.is_secret_store_wrong_passphrase() => {
                if let BootApp::Unlocking(state) = self {
                    state.error = Some(UnlockError::WrongPassphrase);
                }
            }
            Err(e) => {
                tracing::warn!(error = ?e, "Vault unlock attempt failed");
                if let BootApp::Unlocking(state) = self {
                    state.error = Some(UnlockError::Storage);
                }
            }
        }
    }
}

impl eframe::App for BootApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let BootApp::Running(app) = self {
            app.ui(ui, frame);
            return;
        }
        if matches!(self, BootApp::Failed) {
            return;
        }
        // Unlocking: render the prompt, then act on the outcome. Split from the
        // transition so the prompt's borrow of `self` is released first.
        let ctx = ui.ctx().clone();
        let outcome = match self {
            BootApp::Unlocking(state) => state.show_modal(&ctx),
            _ => return,
        };
        match outcome {
            UnlockOutcome::Pending => {}
            UnlockOutcome::Cancel => {
                tracing::info!("User chose to quit at the legacy-vault unlock prompt");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            UnlockOutcome::Submit(passphrase) => self.try_unlock(passphrase, &ctx),
        }
    }

    fn on_exit(&mut self) {
        if let BootApp::Running(app) = self {
            app.on_exit();
        }
    }
}

/// State of the legacy-vault unlock prompt.
///
/// The masked input buffer and focus tracking live inside the reused
/// [`passphrase_modal`] (egui data cache), so this carries only the domain
/// state needed to re-open the vault and the re-prompt reason.
pub struct UnlockState {
    ctx: egui::Context,
    db: Arc<Database>,
    data_dir: PathBuf,
    error: Option<UnlockError>,
}

impl UnlockState {
    fn new(ctx: egui::Context, db: Arc<Database>, data_dir: PathBuf) -> Self {
        Self {
            ctx,
            db,
            data_dir,
            error: None,
        }
    }

    /// Render the shared masked unlock prompt for one frame and report the
    /// user's action. The passphrase is masked, zeroized, and extracted by
    /// [`passphrase_modal`]; a blank entry re-prompts rather than calling the
    /// vault.
    fn show_modal(&mut self, ctx: &egui::Context) -> UnlockOutcome {
        let config = PassphraseModalConfig {
            window_title: "Unlock your saved keys",
            body: "Your saved keys are protected by a passphrase set in an earlier version. \
                   Enter it to open them, or quit and reopen the app to try again later.",
            hint: None,
            error: self.error.map(UnlockError::message),
            submit_label: "Unlock",
            input_placeholder: "Enter passphrase",
            remember_label: None,
        };

        match passphrase_modal(ctx, &config, |_ui| {}) {
            PassphraseModalOutcome::Pending => UnlockOutcome::Pending,
            PassphraseModalOutcome::Cancel => UnlockOutcome::Cancel,
            PassphraseModalOutcome::Submit(text) => {
                let passphrase = SecretString::new(text.to_string());
                if passphrase.is_blank() {
                    self.error = Some(UnlockError::Blank);
                    UnlockOutcome::Pending
                } else {
                    self.error = None;
                    UnlockOutcome::Submit(passphrase)
                }
            }
        }
    }
}

/// One frame's outcome of the unlock prompt.
enum UnlockOutcome {
    /// No actionable input this frame.
    Pending,
    /// The user submitted a non-blank passphrase.
    Submit(SecretString),
    /// The user chose to quit.
    Cancel,
}

/// User-facing reason the unlock prompt is re-shown.
#[derive(Clone, Copy)]
enum UnlockError {
    WrongPassphrase,
    Blank,
    Storage,
}

impl UnlockError {
    fn message(self) -> &'static str {
        match self {
            UnlockError::WrongPassphrase => "That passphrase is not correct. Try again.",
            UnlockError::Blank => "Enter your passphrase to continue.",
            UnlockError::Storage => {
                "Your saved keys could not be opened. Make sure no other copy of Dash Evo Tool is running, then try again."
            }
        }
    }
}
