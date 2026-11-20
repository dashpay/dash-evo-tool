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
//! wallet seeds are never at risk. Every other boot failure is fatal and shows
//! the startup failure dialog instead of the app.
//!
//! Only the GUI binary boots through here; the headless MCP/CLI path keeps the
//! keyless open and surfaces the typed error instead of popping a dialog.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;
use platform_wallet_storage::secrets::{SecretStore, SecretString};

use crate::app::AppState;
use crate::context::AppContext;
use crate::database::Database;
use crate::model::error_report::StartupFailure;
use crate::ui::components::passphrase_modal::{
    PassphraseModalConfig, PassphraseModalOutcome, passphrase_modal,
};
use crate::ui::components::startup_error_dialog::{StartupErrorDialog, StartupErrorDialogAction};

type BootError = Box<dyn std::error::Error + Send + Sync>;

/// Sole owner of the production boot environment: resolve the app data
/// directory (creating it if needed), ensure the `.env` file exists, and
/// initialize logging. Returns the resolved data directory.
///
/// Called from `main` before the tokio runtime starts and again from
/// [`AppState::boot_inputs`](crate::app::AppState::boot_inputs) when
/// [`BootPreparation::run`] runs, still before the window opens; the logger's
/// `Once` guard makes the second call a no-op, so this stays the single place
/// the sequence is written.
pub fn prepare_environment() -> Result<PathBuf, PrepareEnvironmentError> {
    use crate::app_dir::{app_user_data_dir_path, ensure_data_dir_exists};
    let data_dir =
        app_user_data_dir_path().map_err(|source| PrepareEnvironmentError::Locate { source })?;
    ensure_data_dir_exists(&data_dir).map_err(|source| {
        PrepareEnvironmentError::CreateDataFolder {
            path: data_dir.clone(),
            source,
        }
    })?;
    // Logging first, so a failure to write the settings file is logged too.
    crate::logging::initialize_logger();
    ensure_settings_file(&data_dir)?;
    Ok(data_dir)
}

/// Create the `.env` settings file in `data_dir` if it is missing.
pub(crate) fn ensure_settings_file(
    data_dir: &std::path::Path,
) -> Result<(), PrepareEnvironmentError> {
    crate::app_dir::ensure_env_file(data_dir).map_err(|source| {
        PrepareEnvironmentError::WriteSettingsFile {
            path: data_dir.join(crate::app_dir::ENV_FILE),
            source,
        }
    })
}

/// The step of preparing the app data folder that failed, with the path it
/// worked on; the I/O error is the source.
#[derive(Debug, thiserror::Error)]
pub enum PrepareEnvironmentError {
    /// The app data folder's location could not be determined.
    #[error("The app data folder could not be found.")]
    Locate {
        #[source]
        source: std::io::Error,
    },
    /// The app data folder could not be created or secured.
    #[error("The app data folder {path} could not be prepared.", path = .path.display())]
    CreateDataFolder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The `.env` settings file could not be created.
    #[error("The settings file {path} could not be created.", path = .path.display())]
    WriteSettingsFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl PrepareEnvironmentError {
    /// The underlying I/O error.
    pub fn io_error(&self) -> &std::io::Error {
        match self {
            Self::Locate { source }
            | Self::CreateDataFolder { source, .. }
            | Self::WriteSettingsFile { source, .. } => source,
        }
    }
}

/// The eframe application during boot: still collecting the legacy vault
/// passphrase, the fully built [`AppState`], or a fatal startup failure.
pub enum BootApp {
    /// The vault is passphrase-protected; collecting the passphrase.
    Unlocking(UnlockState),
    /// The vault is open and the app is running normally.
    Running(Box<AppState>),
    /// Startup failed. Only the failure dialog is shown; no part of the app
    /// is built.
    Failed(StartupFailure),
}

impl BootApp {
    /// Run [`BootPreparation`] and build the boot app from it, as tests do;
    /// `main` prepares before the window opens and uses [`Startup::boot`].
    ///
    /// On success the full [`AppState`] is built immediately. If the keyless
    /// open fails *specifically* with a wrong vault passphrase, this returns
    /// [`BootApp::Unlocking`] so the frame loop can prompt for it. Any other
    /// failure (including app assembly) returns [`BootApp::Failed`].
    pub fn new(ctx: egui::Context, outcome: StartupOutcome) -> Self {
        Self::start(ctx, BootPreparation::run(), &outcome)
    }

    /// Build the boot app from the steps [`BootPreparation`] already ran.
    pub fn start(
        ctx: egui::Context,
        preparation: BootPreparation,
        outcome: &StartupOutcome,
    ) -> Self {
        // Every boot outcome, the failure dialog included, keeps selections
        // out of PRIMARY.
        crate::ui::primary_selection_guard::PrimarySelectionGuard::install(&ctx);
        match preparation
            .0
            .and_then(|prepared| Self::open(ctx, prepared, outcome))
        {
            Ok(app) => app,
            Err(error) => Self::failed(&*error, outcome),
        }
    }

    fn open(
        ctx: egui::Context,
        Prepared {
            data_dir,
            db,
            decision,
        }: Prepared,
        outcome: &StartupOutcome,
    ) -> Result<Self, BootError> {
        match decision {
            BootDecision::Ready(store) => Ok(BootApp::Running(Box::new(AppState::new_inner(
                ctx, db, data_dir, store,
            )?))),
            BootDecision::Unlock => {
                tracing::warn!(
                    "Seed vault is protected by a passphrase from an earlier version; \
                     prompting to unlock instead of aborting boot"
                );
                Ok(BootApp::Unlocking(UnlockState::new(
                    ctx,
                    db,
                    data_dir,
                    outcome.clone(),
                )))
            }
        }
    }

    /// The fatal-failure state for `error`: logs it, records it in `outcome`
    /// and shows it in the startup failure dialog with this run's log file.
    pub fn failed(error: &(dyn std::error::Error + 'static), outcome: &StartupOutcome) -> Self {
        tracing::error!(error = ?error, "Startup failed; showing the startup failure dialog");
        outcome.mark_failed();
        BootApp::Failed(StartupFailure::from_error(
            error,
            crate::logging::log_file_path(),
        ))
    }

    /// Attempt to open the vault with the supplied passphrase and, on success,
    /// build the full app. A wrong passphrase re-arms the prompt; any other
    /// failure is fatal and shows the startup failure dialog.
    fn try_unlock(&mut self, passphrase: SecretString) {
        let BootApp::Unlocking(state) = self else {
            return;
        };
        match AppContext::open_secret_store_with_passphrase(&state.data_dir, passphrase) {
            Ok(store) => {
                *self = match AppState::new_inner(
                    state.ctx.clone(),
                    Arc::clone(&state.db),
                    state.data_dir.clone(),
                    store,
                ) {
                    Ok(app) => BootApp::Running(Box::new(app)),
                    Err(error) => Self::failed(&*error, &state.outcome),
                };
            }
            Err(e) if e.is_recoverable_secret_store_passphrase_error() => {
                state.error = Some(if e.is_secret_store_wrong_passphrase() {
                    UnlockError::WrongPassphrase
                } else {
                    UnlockError::TooLong
                });
            }
            // Any non-passphrase failure on the keyed open is genuinely fatal —
            // mirror BootApp::new's classification. Re-prompting can never
            // resolve it, so stop instead of trapping the user behind a futile
            // prompt.
            Err(e) => *self = Self::failed(&e, &state.outcome),
        }
    }
}

impl eframe::App for BootApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let state = match self {
            BootApp::Running(app) => {
                app.ui(ui, frame);
                return;
            }
            BootApp::Failed(failure) => {
                if StartupErrorDialog::new(failure).show(ui) == StartupErrorDialogAction::Quit {
                    tracing::info!("User closed the startup failure dialog");
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                return;
            }
            BootApp::Unlocking(state) => state,
        };
        let ctx = ui.ctx().clone();
        match state.show_modal(&ctx) {
            UnlockOutcome::Pending => {}
            UnlockOutcome::Cancel => {
                tracing::info!("User chose to quit at the legacy-vault unlock prompt");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            UnlockOutcome::Submit(passphrase) => self.try_unlock(passphrase),
        }
    }

    /// The failure dialog saves nothing: with no data folder, eframe's storage
    /// is the system default folder, outside the one the user chose.
    fn persist_egui_memory(&self) -> bool {
        !matches!(self, BootApp::Failed(_))
    }

    fn on_exit(&mut self) {
        if let BootApp::Running(app) = self {
            app.on_exit();
        }
    }
}

/// The boot steps that need no window: data folder, database and the seed
/// vault open. `main` runs them before the window opens, so a failure here,
/// such as the vault held by another running instance, is known in time to
/// keep the window from saving into a profile that instance owns.
pub struct BootPreparation(Result<Prepared, BootError>);

struct Prepared {
    data_dir: PathBuf,
    db: Arc<Database>,
    decision: BootDecision,
}

impl BootPreparation {
    /// Prepare the data folder and database, and open the seed vault.
    pub fn run() -> Self {
        Self(AppState::boot_inputs().and_then(|(data_dir, db)| {
            let decision = classify_open(&data_dir)?;
            Ok(Prepared {
                data_dir,
                db,
                decision,
            })
        }))
    }

    /// Whether the window may save its state into the profile: only when
    /// these steps succeeded and this instance holds the vault. A legacy vault
    /// waiting for its passphrase is not held, so another launch can reach the
    /// same prompt.
    pub fn may_persist(&self) -> bool {
        matches!(
            self.0,
            Ok(Prepared {
                decision: BootDecision::Ready(_),
                ..
            })
        )
    }

    /// Why these steps failed, if they did.
    pub fn error(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.as_ref().err().map(|error| &**error as _)
    }
}

/// What `main` hands the window: the data folder preparation result and, when
/// it succeeded, the boot steps that need no window. Decides whether the
/// window may save into the profile, and keeps any failure reportable on the
/// terminal if eframe never creates the app.
pub enum Startup {
    /// The data folder could not be prepared; nothing else ran.
    EnvironmentFailed(BootError),
    /// The data folder at `data_dir` is ready and the boot steps ran.
    Prepared {
        data_dir: PathBuf,
        preparation: BootPreparation,
    },
    /// The prepared boot steps were handed to the app.
    Spent,
}

/// [`Startup::boot`] called again after the prepared steps were used.
#[derive(Debug, thiserror::Error)]
#[error("the startup steps were already used")]
struct StartupSpent;

impl Startup {
    /// Run the boot steps when `environment` holds the data folder.
    pub fn prepare<E>(environment: Result<PathBuf, E>) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        match environment {
            Ok(data_dir) => Self::Prepared {
                data_dir,
                preparation: BootPreparation::run(),
            },
            Err(error) => Self::EnvironmentFailed(Box::new(error)),
        }
    }

    /// Whether the window may save its geometry and state into the profile.
    pub fn persist_window(&self) -> bool {
        matches!(self, Self::Prepared { preparation, .. } if preparation.may_persist())
    }

    /// The profile file in the data folder, when there is one.
    pub fn persistence_path(&self) -> Option<PathBuf> {
        match self {
            Self::Prepared { data_dir, .. } => Some(data_dir.join("app.ron")),
            Self::EnvironmentFailed(_) | Self::Spent => None,
        }
    }

    /// Why startup failed before the app was built, if it did.
    pub fn error(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EnvironmentFailed(error) => Some(&**error as _),
            Self::Prepared { preparation, .. } => preparation.error(),
            Self::Spent => None,
        }
    }

    /// Build the boot app. Successful steps are handed over and the startup
    /// becomes [`Self::Spent`]; a failure stays here for the terminal notice.
    pub fn boot(&mut self, ctx: egui::Context, outcome: &StartupOutcome) -> BootApp {
        crate::ui::primary_selection_guard::PrimarySelectionGuard::install(&ctx);
        match std::mem::replace(self, Self::Spent) {
            Self::Prepared {
                preparation: preparation @ BootPreparation(Ok(_)),
                ..
            } => BootApp::start(ctx, preparation, outcome),
            Self::Spent => BootApp::failed(&StartupSpent, outcome),
            failed => {
                *self = failed;
                match self.error() {
                    Some(error) => BootApp::failed(error, outcome),
                    None => BootApp::failed(&StartupSpent, outcome),
                }
            }
        }
    }
}

/// Whether startup failed, shared with `main` so the process can exit with a
/// failure code after the user closes the failure dialog.
#[derive(Debug, Clone, Default)]
pub struct StartupOutcome(Arc<AtomicBool>);

impl StartupOutcome {
    fn mark_failed(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether the app entered [`BootApp::Failed`].
    pub fn failed(&self) -> bool {
        self.0.load(Ordering::Relaxed)
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
    outcome: StartupOutcome,
    error: Option<UnlockError>,
}

impl UnlockState {
    fn new(
        ctx: egui::Context,
        db: Arc<Database>,
        data_dir: PathBuf,
        outcome: StartupOutcome,
    ) -> Self {
        Self {
            ctx,
            db,
            data_dir,
            outcome,
            error: None,
        }
    }

    /// Render the shared masked unlock prompt for one frame and report the
    /// user's action. The passphrase is masked, zeroized, and extracted by
    /// [`passphrase_modal`]; a blank entry re-prompts rather than calling the
    /// vault.
    fn show_modal(&mut self, ctx: &egui::Context) -> UnlockOutcome {
        let config = PassphraseModalConfig {
            state_id: egui::Id::unique("boot_secret_store_passphrase"),
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
    TooLong,
    Blank,
}

impl UnlockError {
    fn message(self) -> &'static str {
        match self {
            UnlockError::WrongPassphrase => "That passphrase is not correct. Try again.",
            UnlockError::TooLong => {
                "This passphrase is too long for this version of the app. Try a shorter passphrase, or reopen the previous version and change it."
            }
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
    use crate::model::error_report::GENERIC_STARTUP_FAILURE;
    use crate::wallet_backend::single_key::open_secret_store;

    /// A non-passphrase failure while unlocking shows the dialog instead of
    /// closing the window.
    #[test]
    fn unlock_with_broken_vault_enters_failed_state() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let secrets = tmp.path().join("secrets");
        std::fs::create_dir_all(&secrets).expect("mkdir");
        std::fs::write(secrets.join("det-secrets.pwsvault"), b"not a vault")
            .expect("write garbage");
        let db = Arc::new(crate::database::test_helpers::create_test_database().expect("db"));
        let outcome = StartupOutcome::default();
        let mut app = BootApp::Unlocking(UnlockState::new(
            egui::Context::default(),
            db,
            tmp.path().to_path_buf(),
            outcome.clone(),
        ));

        app.try_unlock(SecretString::new("anything"));

        let BootApp::Failed(failure) = &app else {
            panic!("a broken vault must be fatal");
        };
        assert_ne!(failure.report().message(), GENERIC_STARTUP_FAILURE);
        assert!(outcome.failed());
    }

    /// The failure dialog keeps no egui memory, so closing it writes nothing to
    /// eframe's storage, which falls back to the system default folder when the
    /// data folder could not be prepared.
    #[test]
    fn failed_boot_does_not_persist_egui_memory() {
        let outcome = StartupOutcome::default();
        let app = BootApp::failed(&std::io::Error::other("no data folder"), &outcome);
        assert!(!eframe::App::persist_egui_memory(&app));
    }

    /// The real preparation path returns an error, not a panic, when the
    /// `.env` write fails after the data folder is set up (a dangling symlink
    /// stands in for a full disk). Runs in a child process, because preparation
    /// installs the process-wide logger.
    #[cfg(unix)]
    #[test]
    fn prepare_environment_reports_a_failed_env_write() {
        const CHILD_ENV: &str = "DET_PREPARE_ENVIRONMENT_CHILD";
        if std::env::var_os(CHILD_ENV).is_some() {
            let error = prepare_environment().expect_err(".env cannot be written");
            assert!(
                matches!(
                    &error,
                    PrepareEnvironmentError::WriteSettingsFile { path, source }
                        if path.ends_with(".env") && source.kind() == std::io::ErrorKind::NotFound
                ),
                "{error:?}"
            );
            let details = StartupFailure::from_error(&error, None)
                .report()
                .details()
                .to_owned();
            assert!(details.contains("WriteSettingsFile"), "{details}");
            assert!(details.contains(".env"), "{details}");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink(
            dir.path().join("missing").join("env"),
            dir.path().join(".env"),
        )
        .expect("dangling .env symlink");
        let test_name = std::thread::current().name().expect("test name").to_owned();
        let status = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args(["--exact", &test_name, "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, "1")
            .env("DASH_EVO_DATA_DIR", dir.path())
            .status()
            .expect("run child");
        assert!(status.success(), "child failed: {status}");
    }

    /// Runs `f` with `DASH_EVO_DATA_DIR` set to `dir`, serialized with every
    /// other test that changes it.
    fn with_data_dir<R>(dir: &std::path::Path, f: impl FnOnce() -> R) -> R {
        let _lock = crate::test_support::DASH_EVO_DATA_DIR_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let prior = std::env::var_os("DASH_EVO_DATA_DIR");
        // SAFETY: serialized by DASH_EVO_DATA_DIR_LOCK; restored below.
        unsafe { std::env::set_var("DASH_EVO_DATA_DIR", dir) };
        let result = f();
        // SAFETY: serialized by DASH_EVO_DATA_DIR_LOCK.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("DASH_EVO_DATA_DIR", value),
                None => std::env::remove_var("DASH_EVO_DATA_DIR"),
            }
        }
        result
    }

    /// A healthy launch lets the window save into the profile in the data
    /// folder.
    #[test]
    fn healthy_startup_persists_into_the_data_folder() {
        let tmp = tempfile::tempdir().expect("tempdir");
        AppContext::open_secret_store(tmp.path()).expect("create keyless vault");
        let startup = with_data_dir(tmp.path(), || {
            Startup::prepare(Ok::<_, std::io::Error>(tmp.path().to_path_buf()))
        });
        assert!(startup.persist_window());
        assert_eq!(startup.persistence_path(), Some(tmp.path().join("app.ron")));
        assert!(startup.error().is_none());
    }

    /// A second launch whose vault another instance holds fails before the
    /// window opens: no profile writes, the failure stays reportable without a
    /// window, and the boot shows the failure dialog with the selection guard.
    #[test]
    fn locked_vault_fails_preparation_without_profile_writes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let first_instance = AppContext::open_secret_store(tmp.path()).expect("first instance");
        let mut startup = with_data_dir(tmp.path(), || {
            Startup::prepare(Ok::<_, std::io::Error>(tmp.path().to_path_buf()))
        });
        drop(first_instance);

        assert!(
            !startup.persist_window(),
            "a failed preparation must not let the window save"
        );
        assert!(
            startup.error().is_some(),
            "main must be able to report it without a window"
        );
        let ctx = egui::Context::default();
        let outcome = StartupOutcome::default();
        let app = startup.boot(ctx.clone(), &outcome);
        assert!(matches!(app, BootApp::Failed(_)));
        assert!(outcome.failed());
        assert!(
            ctx.plugin_opt::<crate::ui::primary_selection_guard::PrimarySelectionGuard>()
                .is_some(),
            "the failure dialog must not leak selections to PRIMARY"
        );
        assert!(
            startup.error().is_some(),
            "the error stays for the terminal notice"
        );
    }

    /// A data folder that could not be prepared shows the failure dialog and
    /// stays reportable; nothing is saved.
    #[test]
    fn environment_failure_shows_the_dialog_without_profile_writes() {
        let mut startup = Startup::prepare(Err::<PathBuf, _>(std::io::Error::other("no folder")));
        assert!(!startup.persist_window());
        assert_eq!(startup.persistence_path(), None);
        let outcome = StartupOutcome::default();
        assert!(matches!(
            startup.boot(egui::Context::default(), &outcome),
            BootApp::Failed(_)
        ));
        assert!(startup.error().is_some());
    }

    /// A legacy passphrase vault holds no lock at the unlock prompt, so two
    /// launches could both reach it: the window must not save in that state.
    #[test]
    fn legacy_vault_prompt_does_not_persist() {
        let tmp = tempfile::tempdir().expect("tempdir");
        AppContext::open_secret_store_with_passphrase(tmp.path(), SecretString::new("legacy-pass"))
            .expect("create passphrase vault");
        let startup = with_data_dir(tmp.path(), || {
            Startup::prepare(Ok::<_, std::io::Error>(tmp.path().to_path_buf()))
        });
        assert!(startup.error().is_none());
        assert!(!startup.persist_window());
    }

    /// Booting the prepared steps a second time never runs them again: it
    /// shows the failure dialog instead.
    #[test]
    fn spent_startup_does_not_prepare_again() {
        let outcome = StartupOutcome::default();
        assert!(matches!(
            Startup::Spent.boot(egui::Context::default(), &outcome),
            BootApp::Failed(_)
        ));
        assert!(outcome.failed());
    }

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
