//! Kittest coverage for the blocking storage-preparation gate.
//!
//! The gate is the one place boot decides a network's storage is usable: wiring,
//! the schema ladder, hydration and the legacy drain run as one sequence, and
//! nothing — not a screen, not chain sync — is handed to the user until it
//! returns.
//!
//! The load-bearing test here is
//! [`gate_password_prompt_is_focusable_and_typeable`]. Storage preparation waits
//! for a migrated wallet's password, and that password can only arrive from the
//! frame loop the gate blocks: **the gate blocks the loop its own completion
//! depends on.** What breaks the cycle is `AppState::has_blocking_secret_prompt`,
//! which makes the frame loop skip the overlay's input claim and paint no card,
//! dimmer or focus trap while a prompt is up. A gate that raised its own surface
//! instead of going through `ProgressOverlay` would not feed that predicate, and
//! production would deadlock while every test that raises a block *directly*
//! still passed. So these tests raise the **gate**, never
//! `ProgressOverlay::set_global`.

#![cfg(feature = "testing")]

use std::cell::Cell as StdCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dash_evo_tool::app::{AppState, BootPhase, STORAGE_PREP_PASSWORD_DESCRIPTION};
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::context::migration_status::{MigrationState, MigrationStep};
use dash_evo_tool::model::secret::Secret;
use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::model::wallet::birth_height::WalletOrigin;
use dash_evo_tool::ui::RootScreenType;
use dash_evo_tool::ui::components::ProgressOverlay;
use dash_sdk::dpp::dashcore::Network;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// Frames a gate test will pump before declaring the app wedged. Large enough to
/// absorb suite-wide CPU contention, small enough that a regression fails CI in
/// seconds instead of hanging it.
const MAX_GATE_FRAMES: usize = 600;

/// Mount `AppState` with the storage-preparation gate held up by the test seam,
/// so the real frame loop runs against a gate that never completes.
fn mount_with_raised_gate() -> Harness<'static, AppState> {
    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        let mut app = AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false);
        app.show_welcome_screen = false;
        app.welcome_screen = None;
        app.test_raise_storage_prep_gate();
        app
    });
    harness.set_size(egui::vec2(800.0, 600.0));
    harness.run_steps(3);
    harness
}

/// Step until `predicate` holds, giving background work real time to run.
///
/// The frame-count-based [`step_until`] spins too fast to wait on anything that
/// is not frame-driven — building a network's `AppContext` is a backend task, so
/// its test has to wait on the clock instead.
fn poll_until(
    harness: &mut Harness<'static, AppState>,
    what: &str,
    mut predicate: impl FnMut(&AppState) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        harness.step();
        if predicate(harness.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what} did not happen in time (boot phase {:?}, network {:?})",
            harness.state().boot_phase(),
            harness.state().current_app_context().network(),
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Mount `AppState` and let its boot preparation finish, so the app is on a
/// screen with the gate released — the state every switch test starts from.
fn mount_prepared_app() -> Harness<'static, AppState> {
    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        let mut app = AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false);
        app.show_welcome_screen = false;
        app.welcome_screen = None;
        app
    });
    harness.set_size(egui::vec2(800.0, 600.0));
    crate::support::wait_for_screens(&mut harness);
    harness
}

/// Step until `predicate` holds, panicking with `what` after [`MAX_GATE_FRAMES`].
/// Bounded on purpose: an unbounded wait turns a gate regression from a red test
/// into a hung CI job.
fn step_until(
    harness: &mut Harness<'static, AppState>,
    what: &str,
    mut predicate: impl FnMut(&AppState) -> bool,
) {
    for _ in 0..MAX_GATE_FRAMES {
        if predicate(harness.state()) {
            return;
        }
        harness.step();
    }
    panic!("{what} did not happen within {MAX_GATE_FRAMES} frames");
}

// ── The password-prompt contract (the deadlock guard) ────────────────────────

/// T1 — the storage update's password prompt renders, holds focus and accepts
/// typing **above the raised gate**, and the gate's own progress copy is not
/// painted over it.
///
/// This is the assertion that goes red if the gate stops feeding
/// `has_blocking_secret_prompt`. It raises the gate rather than the overlay
/// directly, which is the whole point: the previous version of this test raised
/// `ProgressOverlay::set_global`, so it would have passed against a gate that
/// deadlocks production.
#[test]
fn gate_password_prompt_is_focusable_and_typeable() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let seed_hash = Rc::new(StdCell::new([0; 32]));
        let seed_hash_for_app = Rc::clone(&seed_hash);
        let mut harness = Harness::builder()
            .with_max_steps(100)
            .build_eframe(move |ctx| {
                let mut app = AppState::new(ctx.egui_ctx.clone())
                    .expect("Failed to create AppState")
                    .with_animations(false);
                app.show_welcome_screen = false;
                app.welcome_screen = None;

                let password = Secret::new("correct password");
                let seed = [0xA7; 64];
                let wallet = Wallet::new_from_seed(
                    seed,
                    Network::Testnet,
                    Some("Savings".to_string()),
                    Some(&password),
                )
                .expect("build protected wallet");
                let (hash, wallet) = app
                    .current_app_context()
                    .register_wallet(wallet, &seed, WalletOrigin::Imported)
                    .expect("register protected wallet fixture");
                wallet.write().expect("wallet lock").wallet_seed.close();
                seed_hash_for_app.set(hash);
                app
            });
        harness.set_size(egui::vec2(800.0, 600.0));
        let app_context = crate::support::wait_for_wallet_backend(&mut harness);

        // Raise the GATE, then publish the state preparation would publish while
        // waiting on this wallet's password.
        harness.state_mut().test_raise_storage_prep_gate();
        app_context
            .migration_status()
            .set_state(MigrationState::AwaitingWalletPasswords {
                wallets: vec![seed_hash.get()],
            });
        harness.run_steps(5);

        assert!(
            ProgressOverlay::has_global(&harness.ctx),
            "the gate is still up while it waits for the password"
        );
        assert!(
            harness
                .query_by_label("Enter the password for \"Savings\" to update this wallet now.")
                .is_some(),
            "the password prompt renders above the gate",
        );
        assert!(
            harness
                .query_by_label(STORAGE_PREP_PASSWORD_DESCRIPTION)
                .is_none(),
            "the gate stays raised but must not paint ITS OWN card copy over the \
             prompt — this is the exact string the card would show in this state, so \
             a gate that skips the `has_blocking_secret_prompt` suppression fails here",
        );

        harness
            .input_mut()
            .events
            .push(egui::Event::Text("wrong password".to_string()));
        harness.run_steps(2);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label_contains("That password did not match")
                .is_some(),
            "the password field is focused and typeable while the gate is raised — \
             without this the gate deadlocks the loop its completion depends on",
        );
    });
}

/// T2 — "Skip this wallet" empties the pending list. Skipping is the only escape
/// from the password loop, and `pending_wallet_passwords` filters skipped wallets
/// out of the next batch, so skipping every wallet is what lets preparation
/// finish on an install whose passwords the user cannot supply.
///
/// Also proves the gate's pointer sink does not swallow the prompt's own
/// secondary action.
#[test]
fn skipping_a_wallet_empties_the_pending_list_through_the_gate() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let seed_hash = Rc::new(StdCell::new([0; 32]));
        let seed_hash_for_app = Rc::clone(&seed_hash);
        let mut harness = Harness::builder()
            .with_max_steps(100)
            .build_eframe(move |ctx| {
                let mut app = AppState::new(ctx.egui_ctx.clone())
                    .expect("Failed to create AppState")
                    .with_animations(false);
                app.show_welcome_screen = false;
                app.welcome_screen = None;

                let password = Secret::new("correct password");
                let seed = [0xA7; 64];
                let wallet = Wallet::new_from_seed(
                    seed,
                    Network::Testnet,
                    Some("Savings".to_string()),
                    Some(&password),
                )
                .expect("build protected wallet");
                let (hash, wallet) = app
                    .current_app_context()
                    .register_wallet(wallet, &seed, WalletOrigin::Imported)
                    .expect("register protected wallet fixture");
                wallet.write().expect("wallet lock").wallet_seed.close();
                seed_hash_for_app.set(hash);
                app
            });
        harness.set_size(egui::vec2(800.0, 600.0));
        let app_context = crate::support::wait_for_wallet_backend(&mut harness);

        harness.state_mut().test_raise_storage_prep_gate();
        app_context
            .migration_status()
            .set_state(MigrationState::AwaitingWalletPasswords {
                wallets: vec![seed_hash.get()],
            });
        harness.run_steps(5);

        harness.get_by_label("Skip this wallet").click();
        harness.run_steps(3);
        assert!(
            matches!(
                harness.state().current_app_context().migration_status().state().as_ref(),
                MigrationState::AwaitingWalletPasswords { wallets } if wallets.is_empty()
            ),
            "skipping empties the pending list — the gate's pointer sink must not \
             swallow the prompt's secondary action",
        );
    });
}

/// T4 — `has_blocking_secret_prompt` is true for `AwaitingWalletPasswords`, and
/// the gate consults it: the gate is raised, yet its description is not painted.
///
/// Deliberately names no button, so it survives any rewrite of the gate's or the
/// SPV block's action rows.
#[test]
fn gate_defers_its_card_to_a_blocking_password_prompt() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_with_raised_gate();
        let app_context = harness.state().current_app_context().clone();

        // Baseline: with no prompt, the gate's own copy IS painted.
        app_context
            .migration_status()
            .set_state(MigrationState::Running {
                step: MigrationStep::Wiring,
            });
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label_contains("The app is opening your saved data")
                .is_some(),
            "the gate paints its progress copy when nothing blocks above it",
        );

        // With a blocking prompt state published, the same gate paints nothing —
        // asserted against the copy this state would put ON the card, so the
        // assertion cannot pass merely because the text changed.
        app_context
            .migration_status()
            .set_state(MigrationState::AwaitingWalletPasswords {
                wallets: vec![[0x11; 32]],
            });
        harness.run_steps(3);
        assert!(
            ProgressOverlay::has_global(&harness.ctx),
            "the gate is still raised — it is suppressed, not lowered",
        );
        assert!(
            harness
                .query_by_label(STORAGE_PREP_PASSWORD_DESCRIPTION)
                .is_none(),
            "a blocking password prompt takes the surface from the gate's card",
        );
    });
}

// ── What the gate blocks, and for how long ───────────────────────────────────

/// T3 — a raised gate lifts when its preparation completes, and the app it hands
/// back is a working one.
///
/// The bound is part of the assertion: a gate that never lifts has no other
/// symptom, so a regression must fail CI rather than hang it. But the phase alone
/// proves too little — a boot whose preparation finished before the first frame
/// reports `Ready` without the driver having done anything. So the gate is held
/// up first, and the release is asserted through what it must produce: root
/// screens, which exist only once `finish_boot_phase` has built them.
#[test]
fn a_completed_preparation_lifts_the_gate_and_builds_the_screens() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_with_raised_gate();
        assert!(
            matches!(harness.state().boot_phase(), BootPhase::Preparing { .. }),
            "the gate is up, so what follows is a release and not a no-op",
        );
        assert_eq!(
            harness
                .state()
                .main_screens
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![RootScreenType::RootScreenNetworkChooser],
            "the chooser is the only screen that exists while the gate is up",
        );

        let started = Instant::now();
        harness.state_mut().test_complete_storage_prep_gate();
        step_until(
            &mut harness,
            "the storage-preparation gate lifting",
            |app| app.boot_phase() == BootPhase::Ready,
        );
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "a completed preparation must hand the app back promptly",
        );
        assert!(
            harness
                .state()
                .main_screens
                .contains_key(&RootScreenType::RootScreenWalletsBalances),
            "the release builds the root screens the gate was withholding",
        );
    });
}

/// T5 — while preparing there is no background escape and no screen underneath:
/// the retired "Continue in the background" affordance is absent, and a click
/// where the screen would be is swallowed by the gate rather than reaching a
/// root screen (there is none built yet).
#[test]
fn preparing_offers_no_background_escape_and_swallows_screen_clicks() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let harness = mount_with_raised_gate();
        assert_eq!(
            harness.state().boot_phase(),
            BootPhase::Preparing {
                network: harness.state().chosen_network
            },
        );
        assert!(ProgressOverlay::has_global(&harness.ctx));
        assert!(
            harness
                .query_by_label("Continue in the background")
                .is_none(),
            "D2: the background escape is gone — preparation has no background",
        );
        assert!(
            harness.query_by_label("Cancel").is_none(),
            "the SPV block's Cancel belongs to sync, which has not started yet",
        );

        // Nothing from a root screen is reachable: none were built.
        assert!(
            harness.query_by_label("Wallets").is_none(),
            "no root screen renders behind the gate",
        );
    });
}

/// T6 — chain sync does not start behind the gate. This is D2's actual
/// guarantee: SPV starts as a *continuation* of preparation, never alongside it.
///
/// The gate is held up for the whole window and the window is measured in wall
/// time, not frames: a start dispatched from the frame loop lands on a tokio
/// task, so a frame-only loop can outrun it and see `Idle` for reasons that have
/// nothing to do with the guarantee. The closing phase assertion is what keeps
/// the loop honest — every iteration above it happened while the gate was up.
#[test]
fn spv_stays_idle_for_every_frame_of_preparation() {
    use dash_evo_tool::model::spv_status::SpvStatus;
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_with_raised_gate();
        assert!(
            matches!(harness.state().boot_phase(), BootPhase::Preparing { .. }),
            "the gate is up, so the frames below are preparing frames",
        );

        for _ in 0..25 {
            assert_eq!(
                harness
                    .state()
                    .current_app_context()
                    .connection_status()
                    .spv_status(),
                SpvStatus::Idle,
                "chain sync must not start while storage preparation is still running",
            );
            harness.step();
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(
            matches!(harness.state().boot_phase(), BootPhase::Preparing { .. }),
            "the gate never lifted, so no frame above was measured after the lift",
        );
    });
}

/// Task results arriving mid-preparation must be swallowed, not fatal, and the
/// user's persisted route must survive the wait.
///
/// The task-result poll loop calls `visible_screen_mut()` at ~25 sites and runs
/// BEFORE the gate driver on every frame — including the gate's own failure
/// routing. With root screens deferred, every one of those is a lookup into a
/// map holding nothing but the network chooser. Resolving to the chooser
/// *without* rewriting `selected_main_screen` is what keeps both properties:
/// no panic now, and the persisted route still there when screens are built.
#[test]
fn results_arriving_mid_preparation_are_swallowed_and_the_route_survives() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_with_raised_gate();
        // A route that is NOT the chooser, so a fallback that rewrote the
        // selection would be visible rather than coincidentally correct.
        harness.state_mut().selected_main_screen = RootScreenType::RootScreenWalletsBalances;
        harness.run_steps(3);

        // The two accessors every poll-loop site funnels through, called while
        // no root screen exists. A bare `.expect()` on the selected route — the
        // shape before the gate — panics on both.
        harness.state_mut().visible_screen_mut();
        harness.state_mut().active_root_screen_mut();
        harness.run_steps(3);

        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenWalletsBalances,
            "a result handled mid-preparation must not spend the user's persisted route",
        );

        // Same for the Masternodes live de-gating branch, which rewrites the
        // selection to a fallback that is itself not built yet.
        harness.state_mut().selected_main_screen = RootScreenType::RootScreenMasternodes;
        harness.state_mut().active_root_screen_mut();
        harness.run_steps(2);
        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenMasternodes,
            "live de-gating must not fire while the gate is up — its fallback screen \
             does not exist yet, so the rewrite would spend the route for nothing",
        );
        harness.state_mut().selected_main_screen = RootScreenType::RootScreenWalletsBalances;

        // Release the gate: the deferred screens are built and the route is honoured.
        harness.state_mut().test_complete_storage_prep_gate();
        step_until(&mut harness, "the gate lifting", |app| {
            app.boot_phase() == BootPhase::Ready
        });
        harness.run_steps(3);
        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenWalletsBalances,
            "the terminal transition must land on the route the user had saved",
        );
        assert!(
            harness
                .state()
                .main_screens
                .contains_key(&RootScreenType::RootScreenWalletsBalances),
            "the terminal transition builds the deferred root screens",
        );
    });
}

// ── Terminal failure surfaces (D5) ───────────────────────────────────────────

/// T8 — a version-window failure is terminal: the surface offers only "Close the
/// app", never "Try again", because a retry cannot turn a newer on-disk layout
/// into a readable one. The copy comes from the `TaskError` variant's
/// `#[error(..)]`, not from a callsite literal.
#[test]
fn saved_data_too_new_is_terminal_with_no_retry() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_with_raised_gate();
        let error = TaskError::SavedDataTooNew {
            source: std::sync::Arc::new(
                dash_evo_tool::backend_task::migration::MigrationError::LegacyDataTooNew {
                    found: 99,
                    maximum_supported: 38,
                },
            ),
        };
        let expected = error.to_string();
        harness.state_mut().test_fail_storage_prep_gate(error);
        harness.run_steps(5);

        assert!(
            harness.query_by_label(expected.as_str()).is_some(),
            "the terminal surface shows the variant's own user-facing copy",
        );
        assert!(
            harness.query_by_label("Try again").is_none(),
            "a newer on-disk layout cannot be retried into a readable one",
        );
        assert!(
            harness.query_by_label("Close the app").is_some(),
            "the one action that exists must be present — the gate has no other exit",
        );

        harness.get_by_label("Close the app").click();
        // The click lands as an event on the next frame; the frame after it is
        // where the gate drains the action.
        let requested = (0..3).any(|_| {
            harness.step();
            close_was_requested(&harness)
        });
        assert!(
            requested,
            "the button must actually ask the app to quit — on this surface it is \
             the only exit, so a wire that does nothing leaves the user stuck",
        );
        assert!(
            harness.state().boot_phase() != BootPhase::Ready,
            "closing must not silently release the app instead",
        );
    });
}

/// Whether the last frame asked the window to close. `send_viewport_cmd` is
/// delivered through the frame's viewport output, which is the only place a test
/// can see it: nothing in `AppState` records that it fired.
fn close_was_requested(harness: &Harness<'static, AppState>) -> bool {
    harness
        .output()
        .viewport_output
        .values()
        .flat_map(|viewport| viewport.commands.iter())
        .any(|command| matches!(command, egui::ViewportCommand::Close))
}

/// A retryable failure offers both actions, and "Try again" re-raises the gate
/// rather than releasing the app — the user's data is unchanged either way, so a
/// retry is safe and a silent release would not be.
#[test]
fn a_retryable_failure_offers_try_again_and_close() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_with_raised_gate();
        harness
            .state_mut()
            .test_fail_storage_prep_gate(TaskError::WalletStorageNotReady);
        harness.run_steps(5);

        assert!(
            harness
                .query_by_label_contains("Your wallets and keys are unchanged")
                .is_some(),
            "a retryable failure reassures the user their data is untouched",
        );
        assert!(harness.query_by_label("Try again").is_some());
        assert!(harness.query_by_label("Close the app").is_some());

        harness.get_by_label("Try again").click();
        harness.run_steps(3);
        assert!(
            harness.query_by_label("Try again").is_none(),
            "retrying replaces the failure surface with a fresh preparation",
        );
    });
}

/// The retry button's *behaviour*, not its label: clicking "Try again" must
/// clear the stale terminal status and dispatch a real preparation.
///
/// Both halves are load-bearing and neither is visible in a surface-swap
/// assertion. Without the status reset the fresh run reads as a second call on
/// an already prepared network and announces nothing, leaving the user watching
/// a silent gate; without the dispatch the failure surface simply disappears and
/// the app never leaves `Preparing`. The gate is held across the click so the
/// spawned run parks on its first line — what the retry published is then
/// observable before the run itself moves it on.
#[test]
fn try_again_reruns_the_preparation_it_advertises() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_with_raised_gate();
        let app_context = harness.state().current_app_context().clone();
        let network = app_context.network();
        // The terminal status a finished run leaves behind: `prepare_storage`
        // announces its own progress only from `Idle`, so a retry that does not
        // clear this one runs silent.
        app_context
            .migration_status()
            .set_state(MigrationState::Ready);

        harness
            .state_mut()
            .test_fail_storage_prep_gate(TaskError::WalletStorageNotReady);
        harness.run_steps(5);

        let gate = rt.block_on(app_context.test_hold_prepare_gate());
        harness.get_by_label("Try again").click();
        harness.run_steps(3);

        assert!(
            matches!(
                app_context.migration_status().state().as_ref(),
                MigrationState::Idle
            ),
            "the retry must clear the finished run's terminal status before \
             dispatching, or its own preparation announces no progress",
        );
        assert_eq!(
            harness.state().boot_phase(),
            BootPhase::Preparing { network },
            "a retry re-raises the gate rather than releasing the app",
        );

        drop(gate);
        step_until(
            &mut harness,
            "the retried preparation to release the app",
            |state| state.boot_phase() == BootPhase::Ready,
        );
        assert!(
            harness.query_by_label("Try again").is_none(),
            "the failure surface is gone because preparation succeeded, not \
             because the button repainted",
        );
    });
}

/// W7 — a network switch must hand the app back. The switch attaches a fresh
/// preparation for the incoming network and then drops the outgoing network's
/// surfaces; if the second step also drops the first step's work, the gate is
/// left `Preparing` with nothing to poll, no screen below it and no button on
/// it, and killing the process is the only way out.
///
/// Drives the real `change_network`, which is how both the network chooser and
/// the MCP switch path reach it — including on first launch, where confirming a
/// network on the chooser is a switch like any other.
#[test]
fn switching_networks_hands_the_app_back() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        // The app builds a context for exactly one network at boot; the switch
        // target's context is created by a backend task, which needs that
        // network configured. The shipped example config has every network.
        let data_dir = std::env::var("DASH_EVO_DATA_DIR").expect("the isolated data dir");
        std::fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/.env.example"),
            std::path::Path::new(&data_dir).join(".env"),
        )
        .expect("seed the isolated data dir with a full network config");

        let mut harness = mount_prepared_app();
        let first = harness.state().current_app_context().network();
        // Chain sync is not what this test is about, and the switch task awaits
        // its start — offline that never returns.
        harness
            .state()
            .current_app_context()
            .update_auto_start_spv(false)
            .expect("turn chain sync off for the switch");
        let second = if first == Network::Testnet {
            Network::Mainnet
        } else {
            Network::Testnet
        };

        harness.state_mut().change_network(second);
        // Deadline rather than a frame count: the incoming network's context is
        // built by a backend task, so this waits on wall-clock work, not on
        // frames.
        poll_until(
            &mut harness,
            "the switched-to network to take over and release the app",
            |state| {
                state.current_app_context().network() == second
                    && state.boot_phase() == BootPhase::Ready
            },
        );
        assert!(
            !ProgressOverlay::has_global(&harness.ctx),
            "a finished switch leaves no blocking overlay behind",
        );

        // Back to a network this process already prepared: the gate must not
        // re-raise over storage that is known good.
        harness.state_mut().change_network(first);
        harness.run_steps(3);
        assert_eq!(
            harness.state().boot_phase(),
            BootPhase::Ready,
            "returning to an already prepared network must not re-raise the gate",
        );
    });
}

/// The stuck-preparation watchdog measures unattended work, not the user.
///
/// `AwaitingWalletPasswords` is the one state designed to wait indefinitely: a
/// migrated wallet's password can only come from the person at the keyboard. A
/// watchdog that counts that wait tells a user whose preparation is progressing
/// perfectly that it is "taking longer than usual" and offers to close the app —
/// and because the flag latches, it keeps saying so for the rest of the run.
#[test]
fn a_password_prompt_does_not_trip_the_stuck_watchdog() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_with_raised_gate();
        let app_context = harness.state().current_app_context().clone();

        app_context
            .migration_status()
            .set_state(MigrationState::AwaitingWalletPasswords {
                wallets: vec![[0x11; 32]],
            });
        // A user who takes longer than the whole budget to find their password
        // manager. The gate paints nothing while the prompt owns the frame, so
        // the damage is only observable after the prompt clears.
        harness
            .state_mut()
            .test_backdate_storage_prep(Duration::from_secs(60));
        harness.run_steps(3);

        app_context
            .migration_status()
            .set_state(MigrationState::Running {
                step: MigrationStep::Wiring,
            });
        harness.run_steps(3);

        assert!(
            harness.query_by_label("Close the app").is_none(),
            "answering a prompt must not leave the gate offering to quit over a \
             preparation that is progressing",
        );
        assert!(
            harness
                .query_by_label_contains("The app is opening your saved data")
                .is_some(),
            "the gate goes back to its progress copy, not the stuck copy",
        );
    });
}

/// Belt and braces for the failure mode with no way out: a gate raised over
/// nothing must still offer an exit.
///
/// No code path may produce this state — every reset of the gate is followed by
/// an attach or by `Ready`. But it is the one gate bug a user cannot work
/// around: no screen renders beneath the overlay, and a wedge that predates this
/// guard left them with a buttonless card and no option but to kill the process.
/// So the invariant is enforced where the user is, not only where the bug was.
#[test]
fn a_gate_raised_over_nothing_still_offers_an_exit() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_with_raised_gate();
        harness.state_mut().test_orphan_storage_prep_gate();
        harness.run_steps(3);

        assert!(
            matches!(harness.state().boot_phase(), BootPhase::Preparing { .. }),
            "the gate is still up — this is the trapped state, not a released one",
        );
        assert!(
            harness.query_by_label("Close the app").is_some(),
            "a gate with nothing to wait for must still let the user out",
        );
    });
}

/// A retry withdraws its network's claim to being prepared.
///
/// "Try again" re-runs the FULL sequence, so the network it targets is no
/// longer known-good storage until that run finishes. Without the withdrawal a
/// later switch back consults a stale claim, skips the gate entirely, and hands
/// the user an app running on storage whose preparation never completed — a
/// silent failure with no surface at all, which is why the line needs a test
/// rather than a comment.
#[test]
fn a_retry_withdraws_the_networks_prepared_claim() {
    crate::support::with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let data_dir = std::env::var("DASH_EVO_DATA_DIR").expect("the isolated data dir");
        std::fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/.env.example"),
            std::path::Path::new(&data_dir).join(".env"),
        )
        .expect("seed the isolated data dir with a full network config");

        let mut harness = mount_with_raised_gate();
        let app_context = harness.state().current_app_context().clone();
        let first = app_context.network();
        let second = if first == Network::Testnet {
            Network::Mainnet
        } else {
            Network::Testnet
        };
        app_context
            .update_auto_start_spv(false)
            .expect("turn chain sync off for the switch");

        // Prepare `first`, so it holds a claim for the retry to withdraw.
        harness.state_mut().test_complete_storage_prep_gate();
        step_until(&mut harness, "the first preparation to finish", |state| {
            state.boot_phase() == BootPhase::Ready
        });

        // Fail it again, then park the retry's real preparation on the gate: a
        // retry that runs to completion would re-file the claim it just
        // withdrew, and prove nothing.
        harness.state_mut().test_raise_storage_prep_gate();
        harness
            .state_mut()
            .test_fail_storage_prep_gate(TaskError::WalletStorageNotReady);
        harness.run_steps(3);
        let parked = rt.block_on(app_context.test_hold_prepare_gate());
        harness.get_by_label("Try again").click();
        harness.run_steps(3);

        // Leave and come back. The gate must re-raise: this network's storage
        // has an unfinished preparation behind it.
        harness.state_mut().change_network(second);
        poll_until(&mut harness, "the switch away to complete", |state| {
            state.current_app_context().network() == second
                && state.boot_phase() == BootPhase::Ready
        });
        harness.state_mut().change_network(first);
        harness.run_steps(3);

        assert!(
            matches!(harness.state().boot_phase(), BootPhase::Preparing { .. }),
            "returning to a network whose retry never finished must re-raise the \
             gate, not hand back an app running on unprepared storage",
        );

        drop(parked);
        step_until(&mut harness, "the parked preparation to finish", |state| {
            state.boot_phase() == BootPhase::Ready
        });
    });
}
