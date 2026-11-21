use crate::support::with_isolated_data_dir;
use dash_evo_tool::SUPPORT_URL;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::boot::{BootApp, Startup, StartupOutcome};
use dash_evo_tool::model::error_report::StartupFailure;
use dash_evo_tool::ui::components::startup_error_dialog::{
    STARTUP_ERROR_TITLE, StartupErrorDialog, StartupErrorDialogAction,
};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::cell::Cell;
use std::rc::Rc;

fn insecure_folder_error() -> TaskError {
    TaskError::WalletDataFolderInsecure {
        source: Box::new(
            platform_wallet_storage::WalletStorageError::InsecureParentDir {
                ancestor: std::path::PathBuf::from("/shared"),
                reason: platform_wallet_storage::InsecureAncestor::WritableWithoutSticky {
                    mode: 0o777,
                },
            },
        ),
    }
}

fn copied_text(harness: &Harness<'_, impl Sized>) -> Option<String> {
    harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
}

/// A boot that fails (here: an unreadable seed vault) opens a window with the
/// typed error's own message, the technical details and both actions, instead of
/// aborting before any window appears.
#[test]
fn failing_boot_renders_startup_error_dialog() {
    with_isolated_data_dir(|| {
        let data_dir = dash_evo_tool::app_dir::app_user_data_dir_path().expect("data dir");
        let secrets = data_dir.join("secrets");
        std::fs::create_dir_all(&secrets).expect("mkdir secrets");
        std::fs::write(secrets.join("det-secrets.pwsvault"), b"not a vault")
            .expect("write garbage vault");

        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();
        let outcome = StartupOutcome::default();
        let shared = outcome.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_eframe(move |cc| BootApp::new(cc.egui_ctx.clone(), shared));
        harness.run_steps(3);

        assert!(matches!(harness.state(), BootApp::Failed(_)));
        assert!(outcome.failed(), "main must learn that startup failed");
        assert!(harness.query_by_label(STARTUP_ERROR_TITLE).is_some());
        assert!(
            harness
                .query_by_label_contains("Your saved keys could not be opened")
                .is_some(),
            "the vault failure's own message must be shown"
        );
        assert!(harness.query_by_label("Copy details").is_some());
        assert!(harness.query_by_label("Quit").is_some());
    });
}

/// A data folder that is set up but cannot take the `.env` file shows the
/// startup failure dialog instead of panicking before any window appears. A
/// dangling symlink stands in for a full disk: the write itself fails.
#[cfg(unix)]
#[test]
fn unwritable_env_file_renders_startup_error_dialog() {
    with_isolated_data_dir(|| {
        let data_dir = dash_evo_tool::app_dir::app_user_data_dir_path().expect("data dir");
        std::fs::create_dir_all(&data_dir).expect("mkdir data dir");
        std::os::unix::fs::symlink(data_dir.join("missing").join("env"), data_dir.join(".env"))
            .expect("dangling .env symlink");

        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();
        let outcome = StartupOutcome::default();
        let shared = outcome.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_eframe(move |cc| BootApp::new(cc.egui_ctx.clone(), shared));
        harness.run_steps(3);

        let BootApp::Failed(failure) = harness.state() else {
            panic!("an unwritable .env must be fatal");
        };
        assert!(
            failure
                .report()
                .message()
                .starts_with("Dash Evo Tool could not create its settings file"),
            "the dialog must name the failed step: {}",
            failure.report().message()
        );
        assert!(
            failure.report().details().contains("WriteSettingsFile"),
            "the details must name the failed step: {}",
            failure.report().details()
        );
        assert!(outcome.failed(), "main must learn that startup failed");
        assert!(harness.query_by_label(STARTUP_ERROR_TITLE).is_some());
    });
}

/// Selecting the technical details of an early startup failure (the data
/// folder could not be prepared) never reaches the PRIMARY selection, while
/// Copy details still copies them.
#[test]
fn early_failure_details_stay_out_of_primary_selection() {
    const MARKER: &str = "details-marker-for-primary-selection";
    let outcome = StartupOutcome::default();
    let shared = outcome.clone();
    let mut startup = Startup::prepare(Err::<std::path::PathBuf, _>(std::io::Error::other(MARKER)));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_eframe(move |cc| startup.boot(cc.egui_ctx.clone(), &shared));
    harness.run();

    let rect = harness.get_by_label_contains(MARKER).rect();
    let (start, end) = (rect.left_center(), rect.right_center());
    let mut commands = Vec::new();
    for event in [
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerMoved(end),
        egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ] {
        harness.event(event);
        for _ in 0..2 {
            harness.step();
            commands.extend(harness.output().platform_output.commands.iter().cloned());
        }
    }
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, egui::OutputCommand::TextSelectionSettled(_))),
        "{commands:?}"
    );

    harness.get_by_label("Copy details").click();
    harness.step();
    assert!(copied_text(&harness).is_some_and(|text| text.contains(MARKER)));
    assert!(outcome.failed());
}

/// A typed `TaskError` shows its own `Display`, so every typed startup error gets
/// its own message with no dialog changes.
#[test]
fn startup_error_dialog_shows_typed_error_message_and_details() {
    let error = insecure_folder_error();
    let expected = error.to_string();
    let failure = StartupFailure::from_error(&error, None);
    assert_eq!(failure.report().message(), expected);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui(move |ui| {
            StartupErrorDialog::new(&failure).show(ui);
        });
    harness.run();

    assert!(harness.query_by_label(&expected).is_some());
    assert!(harness.query_by_label_contains(SUPPORT_URL).is_some());
    assert!(
        harness
            .query_by_label_contains("The issue page is public")
            .is_some(),
        "the user must be told to review the details before posting them"
    );
    assert!(
        harness
            .query_by_label_contains("WalletDataFolderInsecure")
            .is_some(),
        "technical details must be shown"
    );
}

/// "Copy details" puts the technical details on the clipboard.
#[test]
fn copy_details_copies_technical_details() {
    let failure = StartupFailure::from_error(&insecure_folder_error(), None);
    let details = failure.report().details().to_owned();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui(move |ui| {
            StartupErrorDialog::new(&failure).show(ui);
        });
    harness.run();
    harness.get_by_label("Copy details").click();
    harness.step();

    let copied = copied_text(&harness).expect("details copied");
    assert!(copied.contains(&details), "{copied}");
    assert!(copied.contains(dash_evo_tool::VERSION), "{copied}");
}

/// In a small window the dialog stays inside it: Quit is visible without
/// scrolling, and Copy details can be scrolled into view and clicked.
#[test]
fn small_window_keeps_both_actions_reachable() {
    let failure = StartupFailure::from_error(&insecure_folder_error(), None);
    let quit = Rc::new(Cell::new(false));
    let seen = Rc::clone(&quit);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
    let mut harness = Harness::builder()
        .with_size(screen.size())
        .build_ui(move |ui| {
            if StartupErrorDialog::new(&failure).show(ui) == StartupErrorDialogAction::Quit {
                seen.set(true);
            }
        });
    harness.run();

    let copy = harness.get_by_label("Copy details");
    copy.scroll_to_me();
    harness.run();
    let copy = harness.get_by_label("Copy details");
    assert!(
        screen.contains_rect(copy.rect()),
        "Copy details at {:?}",
        copy.rect()
    );
    copy.click();
    harness.step();
    assert!(copied_text(&harness).is_some(), "details copied");

    let quit_button = harness.get_by_label("Quit");
    assert!(
        screen.contains_rect(quit_button.rect()),
        "Quit at {:?}",
        quit_button.rect()
    );
    quit_button.click();
    harness.run();
    assert!(quit.get());
}

/// "Quit" reports the request to the owner, which closes the app.
#[test]
fn quit_button_requests_quit() {
    let failure = StartupFailure::from_error(&insecure_folder_error(), None);
    let quit = Rc::new(Cell::new(false));
    let seen = Rc::clone(&quit);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui(move |ui| {
            if StartupErrorDialog::new(&failure).show(ui) == StartupErrorDialogAction::Quit {
                seen.set(true);
            }
        });
    harness.run();
    assert!(!quit.get());
    harness.get_by_label("Quit").click();
    harness.run();

    assert!(quit.get());
}

/// An app database only an earlier release can upgrade is refused at boot, and
/// the startup dialog shows that error's own message, which names the release.
#[test]
fn pinned_pr_app_database_shows_upgrade_release_message() {
    with_isolated_data_dir(|| {
        let data_dir = dash_evo_tool::app_dir::app_user_data_dir_path().expect("data dir");
        dash_evo_tool::app_dir::ensure_data_dir_exists(&data_dir).expect("data dir");
        let app_db = data_dir.join("det-app.sqlite");
        dash_evo_tool::wallet_backend::platform_compatibility::seed_pinned_pr_history(&app_db)
            .expect("seed pinned-PR history");
        let before = std::fs::read(&app_db).expect("read app database");

        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_eframe(|cc| BootApp::new(cc.egui_ctx.clone(), StartupOutcome::default()));
        harness.run_steps(3);

        let BootApp::Failed(failure) = harness.state() else {
            panic!("a pinned-PR app database must fail the boot");
        };
        let expected = TaskError::WalletDataNeedsUpgradeRelease {
            database: app_db.clone(),
        }
        .to_string();
        assert_eq!(failure.report().message(), expected);
        assert!(harness.query_by_label(&expected).is_some());
        assert_eq!(
            std::fs::read(&app_db).expect("read app database"),
            before,
            "the refused database must be left unchanged"
        );
    });
}
