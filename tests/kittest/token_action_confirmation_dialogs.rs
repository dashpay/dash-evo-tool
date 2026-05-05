//! Regression coverage for the click → confirmation dialog wiring on the
//! token action screens (Freeze / Unfreeze / Mint).
//!
//! See dashpay/dash-evo-tool#852 — clicking the primary action button must
//! populate `confirmation_dialog` AND the screen must render it on the next
//! frame. These tests exercise the actual `ScreenLike::ui()` code path
//! through an `egui_kittest` harness so that removing either half of the
//! wiring will fail the test.

use std::collections::BTreeMap;
use std::sync::{Arc, Once};

use dash_evo_tool::context::AppContext;
use dash_evo_tool::database::Database;
use dash_evo_tool::model::qualified_contract::QualifiedContract;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::tokens::freeze_tokens_screen::FreezeTokensScreen;
use dash_evo_tool::ui::tokens::mint_tokens_screen::MintTokensScreen;
use dash_evo_tool::ui::tokens::tokens_screen::IdentityTokenInfo;
use dash_evo_tool::ui::tokens::unfreeze_tokens_screen::UnfreezeTokensScreen;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
use dash_sdk::dpp::data_contract::config::DataContractConfig;
use dash_sdk::dpp::data_contract::v1::DataContractV1;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::{Identifier, Identity, IdentityPublicKey};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// Set the LOCAL_* env vars that `Network::Regtest` consumes. Mirrors the
/// pattern used by `src/ui/tokens/tokens_screen/mod.rs::ensure_test_env`.
fn ensure_test_env() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Safety: set once, no other test mutates these.
        unsafe {
            std::env::set_var("LOCAL_dapi_addresses", "http://127.0.0.1:2443");
            std::env::set_var("LOCAL_core_host", "127.0.0.1");
            std::env::set_var("LOCAL_core_rpc_port", "20302");
            std::env::set_var("LOCAL_core_rpc_user", "dashmate");
            std::env::set_var("LOCAL_core_rpc_password", "password");
        }
    });
}

/// Build an `AppContext` for `Network::Regtest` backed by an in-memory DB
/// and a leaked tempdir. The leak is intentional: AppContext stores the
/// data_dir path and SPV lazily writes into it during the test lifetime.
fn build_test_app_context(egui_ctx: egui::Context) -> Arc<AppContext> {
    ensure_test_env();
    let temp = tempfile::tempdir().expect("create tempdir");
    let data_dir = temp.path().to_path_buf();
    // Keep the dir alive for the rest of the process; tests are short-lived.
    std::mem::forget(temp);

    let db = Arc::new(Database::new(":memory:").expect("open db"));
    db.initialize(&data_dir).expect("init db");

    let app_context = AppContext::new(
        data_dir,
        Network::Regtest,
        db,
        None,
        Default::default(),
        Default::default(),
        egui_ctx,
    )
    .expect("AppContext::new for Regtest");
    // Developer mode loosens the `has_keys` check inside the token action
    // screens so the test fixture only needs to attach a single random key
    // rather than wire a full critical-level authentication key + matching
    // private-key entry into `QualifiedIdentity::private_keys`.
    app_context.enable_developer_mode(true);
    app_context
}

/// Build a minimal `IdentityTokenInfo` whose identity has a CRITICAL-level
/// authentication key (so the action button is rendered) and whose token
/// configuration uses the SDK's "most restrictive" defaults (the resulting
/// in-screen banner is harmless for this test — we only need the button).
fn build_test_identity_token_info(app_context: &AppContext) -> IdentityTokenInfo {
    let pv = app_context.platform_version();

    // Identity with one public key. The screen's `has_keys` check passes
    // when developer mode is enabled (see `build_test_app_context` below).
    let identity_id = Identifier::random();
    let mut identity = Identity::create_basic_identity(identity_id, pv).expect("create identity");
    identity.add_public_key(IdentityPublicKey::random_key(0, Some(1), pv));

    let qualified_identity = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: None,
        private_keys: KeyStorage {
            private_keys: BTreeMap::new(),
        },
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: Network::Regtest,
    };

    // Minimal V1 contract carrying a single token configuration at position 0.
    let token_config = TokenConfiguration::V0(TokenConfigurationV0::default_most_restrictive());
    let mut tokens = BTreeMap::new();
    tokens.insert(0u16, token_config.clone());

    let contract = DataContract::V1(DataContractV1 {
        id: Identifier::random(),
        version: 1,
        owner_id: qualified_identity.identity.id(),
        document_types: BTreeMap::new(),
        config: DataContractConfig::default_for_version(pv).expect("default config"),
        schema_defs: None,
        created_at: None,
        updated_at: None,
        created_at_block_height: None,
        updated_at_block_height: None,
        created_at_epoch: None,
        updated_at_epoch: None,
        groups: BTreeMap::new(),
        tokens,
        keywords: vec![],
        description: None,
    });

    IdentityTokenInfo {
        token_id: Identifier::random(),
        token_alias: "TestToken".to_string(),
        identity: qualified_identity,
        data_contract: QualifiedContract {
            contract,
            alias: None,
        },
        token_config,
        token_position: 0,
    }
}

/// Wraps a single `ScreenLike` so it can drive an `eframe`-style harness.
/// Using a dyn trait object keeps the type identical across the three test
/// cases without a bunch of generic plumbing.
struct ScreenApp {
    screen: Box<dyn ScreenLike>,
}

impl eframe::App for ScreenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let _ = self.screen.ui(ctx);
    }
}

/// Drive the harness through one click of `button_label` and verify the
/// confirmation dialog identified by `dialog_title` is then on screen.
fn assert_clicking_button_opens_dialog(
    mut harness: Harness<'_, ScreenApp>,
    button_label: &str,
    dialog_title: &str,
) {
    harness.set_size(egui::vec2(1280.0, 900.0));
    // Allow the screen to fully lay out, including any one-shot init that
    // happens on the first frame (banner setup, identity load, etc.).
    harness.run_steps(3);

    assert!(
        harness.query_by_label(dialog_title).is_none(),
        "{dialog_title} should not be visible before clicking {button_label}"
    );

    harness.get_by_label(button_label).click();
    harness.run_steps(3);

    assert!(
        harness.query_by_label(dialog_title).is_some(),
        "{dialog_title} dialog must be visible after clicking {button_label}",
    );
}

/// Setting `group_action_id` shifts the central-panel button label from
/// `"Freeze"` / `"Unfreeze"` / `"Mint"` to `"Sign Freeze"` / `"Sign Unfreeze"` /
/// `"Sign Mint"`. Doing so lets `get_by_label` uniquely target the action
/// button rather than the breadcrumb that shares the bare action name.
#[test]
fn freeze_screen_click_opens_confirm_freeze_dialog() {
    let harness = Harness::builder().build_eframe(|cc| {
        let app_context = build_test_app_context(cc.egui_ctx.clone());
        let info = build_test_identity_token_info(&app_context);
        let mut screen = FreezeTokensScreen::new(info, &app_context);
        screen.group_action_id = Some(Identifier::random());
        ScreenApp {
            screen: Box::new(screen),
        }
    });
    assert_clicking_button_opens_dialog(harness, "Sign Freeze", "Confirm Freeze");
}

#[test]
fn unfreeze_screen_click_opens_confirm_unfreeze_dialog() {
    let harness = Harness::builder().build_eframe(|cc| {
        let app_context = build_test_app_context(cc.egui_ctx.clone());
        let info = build_test_identity_token_info(&app_context);
        let mut screen = UnfreezeTokensScreen::new(info, &app_context);
        screen.group_action_id = Some(Identifier::random());
        ScreenApp {
            screen: Box::new(screen),
        }
    });
    assert_clicking_button_opens_dialog(harness, "Sign Unfreeze", "Confirm Unfreeze");
}

#[test]
fn mint_screen_click_opens_confirm_mint_dialog() {
    let harness = Harness::builder().build_eframe(|cc| {
        let app_context = build_test_app_context(cc.egui_ctx.clone());
        let info = build_test_identity_token_info(&app_context);
        let mut screen = MintTokensScreen::new(info, &app_context);
        screen.group_action_id = Some(Identifier::random());
        ScreenApp {
            screen: Box::new(screen),
        }
    });
    assert_clicking_button_opens_dialog(harness, "Sign Mint", "Confirm Mint");
}
