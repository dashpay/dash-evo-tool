use crate::support::with_isolated_data_dir;
use dash_evo_tool::SUPPORT_URL;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::boot::{BootApp, StartupOutcome};
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
        source: platform_wallet_storage::WalletStorageError::InsecureParentDir {
            ancestor: std::path::PathBuf::from("/shared"),
            reason: platform_wallet_storage::InsecureAncestor::WritableWithoutSticky {
                mode: 0o777,
            },
        },
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

fn open_url(harness: &Harness<'_, impl Sized>) -> Option<String> {
    harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        })
}

/// Copy question puts the same redacted question on the clipboard that Ask
/// ChatGPT sends, and Ask ChatGPT opens it prefilled in the browser.
#[test]
fn ask_ai_buttons_copy_and_open_the_question() {
    let failure = StartupFailure::from_error(&insecure_folder_error(), None);
    let message = failure.report().message().to_owned();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui(move |ui| {
            StartupErrorDialog::new(&failure).show(ui);
        });
    harness.run();

    harness.get_by_label("Copy question").click();
    harness.step();
    let question = copied_text(&harness).expect("question copied");
    assert!(question.contains(&message), "{question}");
    assert!(question.contains("WalletDataFolderInsecure"), "{question}");

    harness.run();
    harness.get_by_label("Ask ChatGPT").click();
    harness.step();
    let url = open_url(&harness).expect("assistant opened");
    let encoded = url
        .strip_prefix("https://chatgpt.com/?q=")
        .expect("ChatGPT URL");
    assert_eq!(urlencoding::decode(encoded).unwrap(), question);
}
