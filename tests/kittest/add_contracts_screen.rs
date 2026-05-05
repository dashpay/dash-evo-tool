//! Locks in the early-return branches of
//! [`AddContractsScreen::add_contracts_clicked`]: parse error, duplicate
//! input, loaded-id lookup failure (with details), and already-loaded.
//!
//! Each branch must
//! - flip the screen into the error state,
//! - return [`AppAction::None`] (no `BackendTask` dispatched),
//! - and surface a global banner with the expected text. The lookup-failure
//!   branch additionally attaches details so the rendered banner has a
//!   "Show details" toggle.

use std::sync::{Arc, Once};

use dash_evo_tool::app::AppAction;
use dash_evo_tool::app_dir::ensure_env_file;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::database::Database;
use dash_evo_tool::database::test_helpers::create_test_database;
use dash_evo_tool::ui::components::MessageBanner;
use dash_evo_tool::ui::contracts_documents::add_contracts_screen::AddContractsScreen;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// Sets the minimum env vars required by `Config::load_from` so that
/// `AppContext::new` can construct an AppContext without touching the
/// user's real configuration. Mirrors the helper used by the
/// contract_token_db unit tests.
fn ensure_test_env() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Safety: tests set env vars once to ensure deterministic config;
        // no other test mutates these specific values.
        unsafe {
            std::env::set_var("MAINNET_dapi_addresses", "http://127.0.0.1:1443");
            std::env::set_var("MAINNET_core_host", "127.0.0.1");
            std::env::set_var("MAINNET_core_rpc_port", "9998");
            std::env::set_var("MAINNET_core_rpc_user", "dashrpc");
            std::env::set_var("MAINNET_core_rpc_password", "password");

            std::env::set_var("LOCAL_dapi_addresses", "http://127.0.0.1:2443");
            std::env::set_var("LOCAL_core_host", "127.0.0.1");
            std::env::set_var("LOCAL_core_rpc_port", "20302");
            std::env::set_var("LOCAL_core_rpc_user", "dashmate");
            std::env::set_var("LOCAL_core_rpc_password", "password");
        }
    });
}

fn make_app_context(db: Arc<Database>, ctx: egui::Context) -> Arc<AppContext> {
    ensure_test_env();
    let temp_dir = tempfile::tempdir().expect("create temp data dir");
    ensure_env_file(temp_dir.path());
    let app_context = AppContext::new(
        temp_dir.path().to_path_buf(),
        Network::Regtest,
        db,
        None,
        Default::default(),
        Default::default(),
        ctx,
    )
    .expect("construct AppContext for test");
    // Tests are short-lived and do not exercise file persistence; leak the
    // temp dir so it stays valid for the AppContext's lifetime.
    std::mem::forget(temp_dir);
    app_context
}

/// Builds a kittest harness whose closure renders any global banners on
/// `harness.ctx` every frame, so tests can assert on banner labels after
/// invoking screen behaviour.
fn build_banner_harness() -> Harness<'static> {
    Harness::builder()
        .with_size(egui::vec2(800.0, 400.0))
        .build_ui(|ui| {
            MessageBanner::show_global(ui);
        })
}

/// Parse-error branch: a contract id field that is neither valid hex nor
/// valid base58 must short-circuit before any backend dispatch and surface
/// a banner that points at the offending field number.
#[test]
fn add_contracts_clicked_parse_error_short_circuits() {
    let mut harness = build_banner_harness();
    let db = Arc::new(create_test_database().expect("create in-memory db"));
    let app_context = make_app_context(db, harness.ctx.clone());

    let mut screen = AddContractsScreen::new(&app_context);
    screen.set_contract_ids_inputs_for_test(vec!["not-a-real-id".to_string()]);

    let action = screen.click_add_contracts_for_test();

    assert_eq!(
        action,
        AppAction::None,
        "parse error must not dispatch a BackendTask",
    );
    assert!(
        screen.is_error_status_for_test(),
        "parse error must move the screen into the error status",
    );
    assert!(
        !screen.is_waiting_status_for_test(),
        "parse error must not move the screen into WaitingForResult",
    );
    assert!(
        MessageBanner::has_global(&harness.ctx),
        "parse error must surface a global banner",
    );

    harness.run();
    assert!(
        harness
            .query_by_label_contains("Invalid ID in field 1")
            .is_some(),
        "parse-error banner must call out the offending field",
    );
}

/// Duplicate-input branch: entering the same valid id twice must reject
/// the click before dispatch and tell the user which id is duplicated.
#[test]
fn add_contracts_clicked_duplicate_input_short_circuits() {
    let mut harness = build_banner_harness();
    let db = Arc::new(create_test_database().expect("create in-memory db"));
    let app_context = make_app_context(db, harness.ctx.clone());

    // Valid 32-byte id encoded as hex (64 hex chars) — round-trips through
    // hex::decode + Identifier::from_bytes.
    let dup = hex::encode([0xAAu8; 32]);
    let mut screen = AddContractsScreen::new(&app_context);
    screen.set_contract_ids_inputs_for_test(vec![dup.clone(), dup]);

    let action = screen.click_add_contracts_for_test();

    assert_eq!(
        action,
        AppAction::None,
        "duplicate input must not dispatch a BackendTask",
    );
    assert!(
        screen.is_error_status_for_test(),
        "duplicate input must move the screen into the error status",
    );

    harness.run();
    assert!(
        harness
            .query_by_label_contains("entered more than once")
            .is_some(),
        "duplicate-input banner must explain the rejection",
    );
}

/// Already-loaded branch: requesting a contract whose id is already cached
/// (e.g. a system contract) must reject the click before dispatch and name
/// the offending id.
#[test]
fn add_contracts_clicked_already_loaded_short_circuits() {
    let mut harness = build_banner_harness();
    let db = Arc::new(create_test_database().expect("create in-memory db"));
    let app_context = make_app_context(db, harness.ctx.clone());

    // Use the first id reported by `loaded_contract_ids` — system contracts
    // are always present, so that id is guaranteed to be in the loaded set.
    let already = *app_context
        .loaded_contract_ids()
        .expect("loaded_contract_ids succeeds")
        .first()
        .expect("at least the system contracts are loaded");
    let mut screen = AddContractsScreen::new(&app_context);
    screen.set_contract_ids_inputs_for_test(vec![already.to_string(Encoding::Base58)]);

    let action = screen.click_add_contracts_for_test();

    assert_eq!(
        action,
        AppAction::None,
        "already-loaded id must not dispatch a BackendTask",
    );
    assert!(
        screen.is_error_status_for_test(),
        "already-loaded id must move the screen into the error status",
    );

    harness.run();
    assert!(
        harness
            .query_by_label_contains("is already loaded")
            .is_some(),
        "already-loaded banner must explain the rejection",
    );
}

/// Lookup-failure branch: when `loaded_contract_ids()` fails (e.g. the
/// `contract` table is missing), the screen must reject the click, set the
/// error status, surface a user-facing banner, and attach details so the
/// rendered banner exposes a "Show details" toggle.
#[test]
fn add_contracts_clicked_lookup_failure_attaches_details() {
    let mut harness = build_banner_harness();
    let db = Arc::new(create_test_database().expect("create in-memory db"));
    let app_context = make_app_context(Arc::clone(&db), harness.ctx.clone());

    // Force `loaded_contract_ids()` to fail by dropping the underlying
    // table after AppContext construction. SQL prepare will then return
    // a rusqlite error for the SELECT.
    db.execute("DROP TABLE contract", [])
        .expect("drop contract table");

    // A syntactically valid id is required to reach the lookup branch.
    let id_hex = hex::encode([0xBBu8; 32]);
    let mut screen = AddContractsScreen::new(&app_context);
    screen.set_contract_ids_inputs_for_test(vec![id_hex]);

    let action = screen.click_add_contracts_for_test();

    assert_eq!(
        action,
        AppAction::None,
        "lookup failure must not dispatch a BackendTask",
    );
    assert!(
        screen.is_error_status_for_test(),
        "lookup failure must move the screen into the error status",
    );

    harness.run();
    assert!(
        harness
            .query_by_label_contains("Unable to check whether those contracts are already loaded")
            .is_some(),
        "lookup-failure banner must surface the user-facing message",
    );
    // Details toggle proves with_details(e) actually attached the technical
    // error context to the banner — without it the toggle would not render.
    assert!(
        harness.query_by_label("Show details").is_some(),
        "lookup-failure banner must expose a Show details toggle",
    );

    // Expanding the toggle must reveal the rendered details body.
    harness.get_by_label("Show details").click();
    harness.run();
    assert!(
        harness.query_by_label("Hide details").is_some(),
        "expanded banner must offer Hide details",
    );
}
