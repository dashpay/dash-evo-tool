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
use platform_wallet_storage::secrets::{SecretStore, SecretString};

use crate::app::AppState;
use crate::context::AppContext;
use crate::database::Database;
use crate::ui::components::passphrase_modal::{
    PassphraseModalConfig, PassphraseModalOutcome, passphrase_modal,
};

type BootError = Box<dyn std::error::Error + Send + Sync>;

/// Sole owner of the production boot environment: resolve the app data
/// directory (creating it if needed), ensure the `.env` file exists, and
/// initialize logging. Returns the resolved data directory.
///
/// Called from `main` before the tokio runtime starts and again from
/// [`AppState::boot_inputs`](crate::app::AppState::boot_inputs) inside the
/// eframe boot; the logger's `Once` guard makes the second call a no-op, so
/// this stays the single place the sequence is written.
pub fn prepare_environment() -> Result<PathBuf, std::io::Error> {
    use crate::app_dir::{app_user_data_dir_path, ensure_data_dir_exists, ensure_env_file};
    let data_dir = app_user_data_dir_path()?;
    ensure_data_dir_exists(&data_dir)?;
    ensure_env_file(&data_dir);
    crate::logging::initialize_logger();
    Ok(data_dir)
}

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
        match classify_open(&data_dir)? {
            BootDecision::Ready(store) => Ok(BootApp::Running(Box::new(AppState::new_inner(
                ctx, db, data_dir, store,
            )?))),
            BootDecision::Unlock => {
                tracing::warn!(
                    "Seed vault is protected by a passphrase from an earlier version; \
                     prompting to unlock instead of aborting boot"
                );
                Ok(BootApp::Unlocking(UnlockState::new(ctx, db, data_dir)))
            }
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
            // Any non-passphrase failure on the keyed open is genuinely fatal —
            // mirror BootApp::new's classification. Re-prompting can never
            // resolve it, so abort cleanly instead of trapping the user behind
            // a futile prompt (the inverse of the bug this fix addresses).
            Err(e) => {
                tracing::error!(error = ?e, "Vault open failed with a non-passphrase error; aborting");
                *self = BootApp::Failed;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
            state_id: egui::Id::new("boot_secret_store_passphrase"),
            window_title: "Unlock your saved keys",
            body: "Your saved keys are protected by a passphrase set in an earlier version. \
                   Enter it to open them. The app asks for this passphrase every time it starts.",
            hint: None,
            error: self.error.map(UnlockError::message),
            submit_label: "Unlock",
            secondary_action_label: None,
            input_placeholder: "Enter passphrase",
            remember_label: None,
            cancellable: true,
        };

        match passphrase_modal(ctx, &config, |_ui| {}) {
            PassphraseModalOutcome::Pending => UnlockOutcome::Pending,
            PassphraseModalOutcome::Cancel => UnlockOutcome::Cancel,
            PassphraseModalOutcome::SecondaryAction => UnlockOutcome::Pending,
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

/// User-facing reason the unlock prompt is re-shown. Only recoverable reasons
/// live here — a non-passphrase open failure aborts (see [`BootApp::try_unlock`])
/// rather than re-prompting, so it is never surfaced through this enum.
#[derive(Clone, Copy)]
enum UnlockError {
    WrongPassphrase,
    Blank,
}

impl UnlockError {
    fn message(self) -> &'static str {
        match self {
            UnlockError::WrongPassphrase => "That passphrase is not correct. Try again.",
            UnlockError::Blank => "Enter your passphrase to continue.",
        }
    }
}

/// The keyless-open classification at boot.
enum BootDecision {
    /// The vault opened keyless and is ready to use.
    Ready(Arc<SecretStore>),
    /// The vault is passphrase-protected (legacy); the user must unlock it.
    Unlock,
}

/// Open the seed vault keyless and classify the result.
///
/// A wrong vault passphrase (a legacy passphrase vault) is the ONLY recoverable
/// case and routes to [`BootDecision::Unlock`]; every other failure is fatal and
/// propagates, matching the original abort-on-error boot behavior. The same
/// predicate gates the unlock-retry loop in [`BootApp::try_unlock`], so both
/// open paths classify fatality identically.
fn classify_open(data_dir: &std::path::Path) -> Result<BootDecision, BootError> {
    match AppContext::open_secret_store(data_dir) {
        Ok(store) => Ok(BootDecision::Ready(store)),
        Err(e) if e.is_secret_store_wrong_passphrase() => Ok(BootDecision::Unlock),
        Err(e) => Err(Box::new(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::single_key::open_secret_store;

    /// A normal keyless vault classifies as ready — the happy path never gates.
    #[test]
    fn classify_open_ready_for_keyless_vault() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Seed a keyless vault at the data dir's secret path, then drop the
        // handle so its exclusive lock is released before re-opening.
        AppContext::open_secret_store(tmp.path()).expect("create keyless vault");

        let decision = classify_open(tmp.path()).expect("classify");
        assert!(
            matches!(decision, BootDecision::Ready(_)),
            "keyless vault must classify as Ready"
        );
    }

    /// A vault an older build sealed with a passphrase classifies as Unlock —
    /// the gate fires only here.
    #[test]
    fn classify_open_unlock_for_passphrase_vault() {
        let tmp = tempfile::tempdir().expect("tempdir");
        AppContext::open_secret_store_with_passphrase(tmp.path(), SecretString::new("legacy-pass"))
            .expect("create passphrase vault");

        let decision = classify_open(tmp.path()).expect("classify");
        assert!(
            matches!(decision, BootDecision::Unlock),
            "passphrase vault must classify as Unlock"
        );
    }

    /// A non-passphrase open failure (here a malformed vault) is FATAL — it
    /// propagates rather than gating to Unlock, mirroring try_unlock's policy.
    #[test]
    fn classify_open_propagates_non_passphrase_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let secrets = tmp.path().join("secrets");
        std::fs::create_dir_all(&secrets).expect("mkdir");
        std::fs::write(secrets.join("det-secrets.pwsvault"), b"not a vault")
            .expect("write garbage");

        assert!(
            classify_open(tmp.path()).is_err(),
            "a malformed vault must be fatal, never gated to Unlock"
        );
    }

    /// Sanity: the keyless open of a passphrase vault is precisely the failure
    /// the gate keys on — `open_secret_store` (keyless) fails it, while
    /// `open_secret_store_with_passphrase` recovers it non-destructively. (The
    /// data round-trip is pinned in single_key.rs; here we only assert the
    /// classification both helpers feed.)
    #[test]
    fn keyless_open_of_passphrase_vault_is_the_gated_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let vault = tmp.path().join("secrets").join("det-secrets.pwsvault");
        AppContext::open_secret_store_with_passphrase(tmp.path(), SecretString::new("legacy-pass"))
            .expect("create passphrase vault");

        let err = open_secret_store(&vault).expect_err("keyless open must fail");
        let task_err = crate::backend_task::error::TaskError::SecretStore {
            source: Box::new(err),
        };
        assert!(
            task_err.is_secret_store_wrong_passphrase(),
            "the keyless failure must be exactly the wrong-passphrase signal the gate keys on"
        );
    }
}
