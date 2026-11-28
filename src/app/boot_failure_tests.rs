use super::*;
use crate::boot::{BootApp, StartupOutcome};
use crate::model::error_report::GENERIC_STARTUP_FAILURE;
use crate::ui::components::startup_error_dialog::STARTUP_ERROR_TITLE;
use crate::wallet_backend::{RememberPolicy, SecretPlaintext, SecretScope};
use egui_kittest::{Harness, kittest::Queryable};

fn assert_first_screen_failure(test: &str, hook: fn(bool)) {
    assert_first_screen_failure_with_unlock(test, hook, false);
}

fn assert_first_screen_failure_with_unlock(test: &str, hook: fn(bool), unlock: bool) {
    let dir = tempfile::tempdir().expect("isolated profile");
    if crate::test_support::run_test_in_child(test, &[("DASH_EVO_DATA_DIR", dir.path())]) {
        return;
    }
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let _entered = runtime.enter();
    let outcome = StartupOutcome::default();
    let scope = SecretScope::HdSeed {
        seed_hash: [17; 32],
    };
    let passphrase = format!("{:?}", crate::test_support::keys::seed());
    if unlock {
        AppContext::open_secret_store_with_passphrase(
            &crate::app_dir::app_user_data_dir_path().unwrap(),
            platform_wallet_storage::secrets::SecretString::new(passphrase.clone()),
        )
        .expect("legacy vault");
    }
    let mut resources = None;
    let mut harness = Harness::builder().build_eframe(|cc| {
        let mut boot = BootApp::new(cc.egui_ctx.clone(), outcome.clone());
        if unlock {
            assert!(matches!(boot, BootApp::Unlocking(_)));
            boot.test_unlock(platform_wallet_storage::secrets::SecretString::new(
                passphrase.clone(),
            ));
        }
        let BootApp::Running(app) = &mut boot else {
            panic!("test profile must boot");
        };
        app.boot_transition_hook = Some(hook);
        let context = app.current_app_context().clone();
        runtime
            .block_on(context.ensure_wallet_backend(app.task_result_sender.clone()))
            .expect("wallet backend");
        let backend = context.wallet_backend().expect("wired backend");
        let access = backend.secret_access();
        let seed = zeroize::Zeroizing::new(crate::test_support::keys::seed());
        access.remember_session(
            &scope,
            SecretPlaintext::HdSeed(&seed),
            RememberPolicy::UntilAppClose,
        );
        let cancel = app.subtasks.cancellation_token.clone();
        assert!(
            app.subtasks
                .spawn_sync("boot-failure-test-worker", async move {
                    cancel.cancelled().await
                })
                .is_ok()
        );
        resources = Some((app.subtasks.clone(), access));
        boot
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    while matches!(harness.state(), BootApp::Running(_)) {
        assert!(
            Instant::now() < deadline,
            "storage-ready frame did not arrive"
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    let BootApp::Failed(failure) = harness.state() else {
        panic!("first screen panic must enter the failure state");
    };
    assert_eq!(failure.report().message(), GENERIC_STARTUP_FAILURE);
    assert!(failure.report().details().contains("redacted"));
    assert!(!failure.report().details().contains("private-screen-marker"));
    assert!(
        outcome.failed(),
        "main must receive the failure exit status"
    );
    let (tasks, secrets) = resources.expect("running resources");
    assert!(tasks.cancellation_token.is_cancelled());
    assert!(!tasks.is_active("boot-failure-test-worker"));
    assert!(tasks.spawn_sync("after-failure", async {}).is_err());
    assert!(
        !secrets.is_session_cached(&scope),
        "wallet teardown clears secrets"
    );
    drop(secrets);
    let data_dir = crate::app_dir::app_user_data_dir_path().unwrap();
    let reopened = if unlock {
        AppContext::open_secret_store_with_passphrase(
            &data_dir,
            platform_wallet_storage::secrets::SecretString::new(passphrase.clone()),
        )
    } else {
        AppContext::open_secret_store(&data_dir)
    };
    assert!(
        reopened.is_err(),
        "the failure window must retain the profile lock"
    );
    harness.run_steps(2);
    assert!(harness.query_by_label(STARTUP_ERROR_TITLE).is_some());
    assert!(harness.query_by_label("Quit").is_some());
    assert!(!eframe::App::persist_egui_memory(harness.state()));
}

#[test]
fn first_screen_construction_panic_shows_failure_and_stops_backend() {
    assert_first_screen_failure(
        "app::boot_failure_tests::first_screen_construction_panic_shows_failure_and_stops_backend",
        |refresh| {
            if !refresh {
                let private = String::from("private-screen-marker");
                panic!("screen build: {private}");
            }
        },
    );
}

#[test]
fn first_screen_refresh_panic_shows_failure_and_stops_backend() {
    assert_first_screen_failure(
        "app::boot_failure_tests::first_screen_refresh_panic_shows_failure_and_stops_backend",
        |refresh| {
            if refresh {
                let private = String::from("private-screen-marker");
                panic!("screen refresh: {private}");
            }
        },
    );
}

fn assert_post_startup_panic_propagates(test: &str, later_transition: bool) {
    let dir = tempfile::tempdir().expect("isolated profile");
    if crate::test_support::run_test_in_child(test, &[("DASH_EVO_DATA_DIR", dir.path())]) {
        return;
    }
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let _entered = runtime.enter();
    let outcome = StartupOutcome::default();
    let mut harness =
        Harness::builder().build_eframe(|cc| BootApp::new(cc.egui_ctx.clone(), outcome.clone()));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let BootApp::Running(app) = harness.state() else {
            panic!("healthy startup must keep running");
        };
        if app.main_screens.len() > 1 {
            break;
        }
        assert!(Instant::now() < deadline, "first screens did not arrive");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    let BootApp::Running(app) = harness.state_mut() else {
        unreachable!();
    };
    assert!(!app.subtasks.cancellation_token.is_cancelled());
    if later_transition {
        app.boot_transition_hook = Some(|_| panic!("later storage-ready transition"));
        app.test_raise_storage_prep_gate();
        app.test_complete_storage_prep_gate();
    } else {
        app.show_welcome_screen = false;
        app.main_screens.clear();
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| harness.step()));
    assert!(result.is_err(), "post-startup panic must propagate");
    assert!(!outcome.failed(), "this is not a startup failure");
    eframe::App::on_exit(harness.state_mut());
}

#[test]
fn later_storage_ready_transition_remains_outside_startup_guard() {
    assert_post_startup_panic_propagates(
        "app::boot_failure_tests::later_storage_ready_transition_remains_outside_startup_guard",
        true,
    );
}

#[test]
fn ordinary_frames_remain_outside_startup_guard() {
    assert_post_startup_panic_propagates(
        "app::boot_failure_tests::ordinary_frames_remain_outside_startup_guard",
        false,
    );
}

#[test]
fn direct_app_first_screen_panic_propagates() {
    let dir = tempfile::tempdir().expect("isolated profile");
    if crate::test_support::run_test_in_child(
        "app::boot_failure_tests::direct_app_first_screen_panic_propagates",
        &[("DASH_EVO_DATA_DIR", dir.path())],
    ) {
        return;
    }
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let _entered = runtime.enter();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut harness = Harness::builder().build_eframe(|cc| {
            let mut app = AppState::new(cc.egui_ctx.clone()).expect("test profile");
            app.boot_transition_hook = Some(|_| panic!("direct first screen"));
            app
        });
        let deadline = Instant::now() + Duration::from_secs(60);
        while harness.state().startup_panic.is_none() && harness.state().needs_first_screens() {
            assert!(
                Instant::now() < deadline,
                "storage-ready frame did not arrive"
            );
            harness.step();
            std::thread::sleep(Duration::from_millis(10));
        }
        eframe::App::on_exit(harness.state_mut());
    }));
    let payload = result.expect_err("direct AppState must propagate the startup panic");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"direct first screen"));
}

#[test]
fn legacy_unlock_first_screen_panic_sets_exit_status() {
    assert_first_screen_failure_with_unlock(
        "app::boot_failure_tests::legacy_unlock_first_screen_panic_sets_exit_status",
        |_| {
            let private = String::from("private-screen-marker");
            panic!("screen: {private}");
        },
        true,
    );
}
