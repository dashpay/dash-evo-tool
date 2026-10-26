//! W4 kittest — TokensScreen (Token Creator) obeys the app-scoped selected identity.
//!
//! B4 migration: `TokensScreen::new()` when built for `TokenCreator` must seed
//! `selected_identity` from the app-scoped selection (fallback: first loaded).
//!
//! B6 regression lock: the 6 N/A token recipient/target/member selectors (mint,
//! transfer, freeze, unfreeze, destroy-frozen-funds, groups) must NOT carry
//! `syncing_global`. The `default_selector_has_no_sync_target` unit test in
//! `identity_selector.rs` covers any freshly-built `IdentitySelector`; B6 adds
//! a structural note and a spot-check that `TokensScreen` (Token Creator) does
//! NOT accidentally apply syncing_global to the N/A sites when rendering.
//!
//! Write-back: `syncing_global` component-level write-back is verified by
//! `identity_selector::tests::syncing_global_writes_selection_to_app_context`.
//!
//! # TODO(WalletFixture / private-key fixture)
//! Add screen-level write-back assertions (ComboBox click → `resolve_selected_identity()`
//! moves) once an identity fixture with loaded AUTH HIGH/CRITICAL private keys exists.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_contract::QualifiedContract;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::tokens::tokens_screen::{
    IdentityTokenBalance, IdentityTokenInfo, TokensScreen, TokensSubscreen,
};
use dash_evo_tool::ui::{RootScreenType, Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::collections::BTreeMap;
use std::sync::Arc;

fn build_ctx() -> (
    Harness<'static, dash_evo_tool::app::AppState>,
    Arc<AppContext>,
) {
    let mut h = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("AppState builds")
            .with_animations(false)
    });
    let ctx = crate::support::wait_for_wallet_backend(&mut h);
    (h, ctx)
}

fn seed_token_identity(app_context: &Arc<AppContext>, byte: u8, alias: &str) -> Identifier {
    let pv = PlatformVersion::latest();
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), pv).expect("basic identity");
    let id = identity.id();
    let qi = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some(alias.to_string()),
        private_keys: KeyStorage::default(),
        dpns_names: vec![dash_evo_tool::model::qualified_identity::DPNSNameInfo {
            name: alias.to_string(),
            acquired_at: 0,
        }],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::PendingCreation,
        network: app_context.network(),
    };
    app_context
        .insert_local_qualified_identity(&qi, &None)
        .expect("seed token identity");
    id
}

/// W4 B4: `TokensScreen` in `TokenCreator` subscreen must seed `selected_identity`
/// from the app-scoped selection, not always the first loaded identity.
#[test]
fn token_creator_defaults_to_app_scoped_identity() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let (_h, ctx) = build_ctx();

        let _first = seed_token_identity(&ctx, 0xA1, "Token Alpha");
        let second = seed_token_identity(&ctx, 0xB2, "Token Beta");

        ctx.set_selected_identity(Some(second));

        let screen = TokensScreen::new(&ctx, TokensSubscreen::TokenCreator);

        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(second),
            "TokensScreen (TokenCreator) must default to the app-scoped selected identity"
        );
    });
}

/// A user identity with one CRITICAL authentication key — enough for the token
/// action scaffold to render its form instead of the "no keys" notice.
fn identity_with_critical_key(app_context: &Arc<AppContext>) -> QualifiedIdentity {
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
    use dash_sdk::dpp::platform_value::BinaryData;

    let mut identity =
        Identity::create_basic_identity(Identifier::from([0xC3; 32]), PlatformVersion::latest())
            .expect("basic identity");
    let key: dash_sdk::platform::IdentityPublicKey = IdentityPublicKeyV0 {
        id: 0,
        key_type: KeyType::ECDSA_HASH160,
        purpose: Purpose::AUTHENTICATION,
        security_level: SecurityLevel::CRITICAL,
        read_only: false,
        data: BinaryData::new(vec![1; 20]),
        disabled_at: None,
        contract_bounds: None,
    }
    .into();
    identity.add_public_key(key.clone());
    // Placeholder key material: nothing here signs, but screens such as the
    // Token Creator only offer identities that hold a private key.
    let private_keys = KeyStorage::from(BTreeMap::from([(
        (PrivateKeyTarget::PrivateKeyOnMainIdentity, 0),
        (
            QualifiedIdentityPublicKey::from(key),
            PrivateKeyData::Clear([7u8; 32]),
        ),
    )]));
    QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some("token-action-layout".to_string()),
        private_keys,
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: app_context.network(),
    }
}

/// Token info for a placeholder token; the layout under test does not depend
/// on which contract carries it.
fn layout_token_info(app_context: &Arc<AppContext>) -> IdentityTokenInfo {
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};

    let contract = load_system_data_contract(SystemDataContract::DPNS, PlatformVersion::latest())
        .expect("dpns contract");
    IdentityTokenInfo {
        token_id: Identifier::from([0xD4; 32]),
        token_alias: "layout-token".to_string(),
        identity: identity_with_critical_key(app_context),
        data_contract: QualifiedContract {
            contract,
            alias: None,
        },
        token_config: TokenConfigurationV0::default_most_restrictive().into(),
        token_position: 0,
    }
}

/// Mint, burn, freeze, unfreeze, destroy-frozen-funds, pause and resume share
/// one screen scaffold. Its "Advanced Options" checkbox must sit on the page
/// heading's row and the first form step must follow right below — not be
/// pushed to the bottom of the window by a checkbox centred in the remaining
/// height.
#[test]
fn token_action_forms_start_right_under_the_heading() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenMyTokenBalances);
        let app_context = harness.state().current_app_context().clone();
        // Developer role renders the form for any identity with public keys,
        // so no private key material is needed.
        app_context.set_user_role(UserRole::Developer);
        let info = layout_token_info(&app_context);

        let screens = [
            ("Mint Tokens", ScreenType::MintTokensScreen(info.clone())),
            ("Burn Tokens", ScreenType::BurnTokensScreen(info.clone())),
            (
                "Freeze Identity’s Tokens",
                ScreenType::FreezeTokensScreen(info.clone()),
            ),
            (
                "Unfreeze Identity’s Tokens",
                ScreenType::UnfreezeTokensScreen(info.clone()),
            ),
            (
                "Destroy Frozen Funds",
                ScreenType::DestroyFrozenFundsScreen(info.clone()),
            ),
            (
                "Pause Token Contract",
                ScreenType::PauseTokensScreen(info.clone()),
            ),
            (
                "Resume Token Contract",
                ScreenType::ResumeTokensScreen(info.clone()),
            ),
        ];

        for (heading, screen_type) in screens {
            let screen = screen_type.create_screen(&app_context);
            {
                let state = harness.state_mut();
                state.screen_stack.clear();
                state.screen_stack.push(screen);
            }
            harness.run_steps(5);

            let heading_rect = harness.get_by_label(heading).rect();
            let checkbox_rect = harness.get_by_label("Advanced Options").rect();
            let first_step_rect = harness.get_by_label_contains("1. ").rect();

            assert!(
                checkbox_rect.center().y >= heading_rect.top()
                    && checkbox_rect.center().y <= heading_rect.bottom(),
                "{heading}: the Advanced Options checkbox must sit on the heading row \
                 (heading {heading_rect:?}, checkbox {checkbox_rect:?})"
            );
            let gap = first_step_rect.top() - heading_rect.bottom();
            assert!(
                (0.0..=60.0).contains(&gap),
                "{heading}: the first form step must start right under the heading, \
                 found a {gap}px gap (heading {heading_rect:?}, step {first_step_rect:?})"
            );
        }
    });
}

/// The "My Tokens" top bar offers "Create Token", which opens the Token Creator.
#[test]
fn my_tokens_create_token_button_opens_token_creator() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenMyTokenBalances);

        harness.get_by_label("Create Token").click();
        harness.run_steps(5);

        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenTokenCreator,
            "Create Token must switch to the Token Creator"
        );
        assert_eq!(
            harness
                .state()
                .current_app_context()
                .get_app_settings()
                .root_screen_type,
            RootScreenType::RootScreenTokenCreator,
            "the tokens subscreen chooser must highlight Token Creator"
        );
        let Screen::TokensScreen(screen) = harness.state_mut().visible_screen_mut() else {
            panic!("the visible screen must be the tokens screen");
        };
        assert!(
            screen.tokens_subscreen == TokensSubscreen::TokenCreator,
            "the visible tokens screen must show the Token Creator"
        );
    });
}

/// Name the smoke token carries in every locale-less lookup.
const SMOKE_TOKEN: &str = "Smoketoken";

/// Seeds a locally owned identity, a token contract it owns (so every
/// owner-only action is authorised) and a positive balance, then returns the
/// pieces the token screens are built from.
fn seed_owned_token(
    app_context: &Arc<AppContext>,
) -> (
    QualifiedIdentity,
    dash_sdk::platform::DataContract,
    Identifier,
) {
    use dash_evo_tool::model::qualified_contract::InsertTokensToo;
    use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::{
        TokenConfigurationPreset, TokenConfigurationPresetFeatures,
    };
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::v0::TokenConfigurationConventionV0;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_localization::{
        TokenConfigurationLocalization, v0::TokenConfigurationLocalizationV0,
    };
    use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
    use dash_sdk::dpp::data_contract::config::DataContractConfig;
    use dash_sdk::dpp::data_contract::v1::DataContractV1;

    let identity = identity_with_critical_key(app_context);
    app_context
        .insert_local_qualified_identity(&identity, &None)
        .expect("seed identity");

    let token_config = TokenConfigurationPreset {
        features: TokenConfigurationPresetFeatures::WithExtremeActions,
        action_taker: AuthorizedActionTakers::ContractOwner,
    }
    .token_configuration_v0(
        TokenConfigurationConvention::V0(TokenConfigurationConventionV0 {
            localizations: BTreeMap::from([(
                "en".to_string(),
                TokenConfigurationLocalization::V0(TokenConfigurationLocalizationV0 {
                    should_capitalize: true,
                    singular_form: SMOKE_TOKEN.to_string(),
                    plural_form: format!("{SMOKE_TOKEN}s"),
                }),
            )]),
            decimals: 8,
        }),
        100_000,
        None,
        true,
        true,
    );
    let pv = PlatformVersion::latest();
    let contract = dash_sdk::platform::DataContract::V1(DataContractV1 {
        id: Identifier::from([0xE5; 32]),
        version: 1,
        owner_id: identity.identity.id(),
        document_types: BTreeMap::new(),
        config: DataContractConfig::default_for_version(pv).expect("config"),
        schema_defs: None,
        groups: BTreeMap::new(),
        tokens: BTreeMap::from([(0, token_config.into())]),
        keywords: vec![],
        created_at: None,
        updated_at: None,
        created_at_block_height: None,
        updated_at_block_height: None,
        created_at_epoch: None,
        updated_at_epoch: None,
        description: None,
    });
    app_context
        .insert_contract_if_not_exists(
            &contract,
            Some("smoke-contract"),
            InsertTokensToo::AllTokensShouldBeAdded,
        )
        .expect("seed contract");
    let token_id = contract.token_id(0).expect("token id");
    app_context
        .insert_token_identity_balance(&token_id, &identity.identity.id(), 1_000_000_000)
        .expect("seed balance");
    (identity, contract, token_id)
}

/// The token screens' fixtures, built once per test on a mounted app.
struct TokenFixture {
    harness: Harness<'static, dash_evo_tool::app::AppState>,
    app_context: Arc<AppContext>,
    info: IdentityTokenInfo,
    balance: IdentityTokenBalance,
}

/// Mounts the app at the user's window size (Developer role, so every action
/// is offered) with an owned, fully permissive token seeded.
fn token_fixture() -> TokenFixture {
    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
    use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;

    let mut harness = mount_app(RootScreenType::RootScreenMyTokenBalances);
    harness.set_size(egui::vec2(1050.0, 700.0));
    let app_context = harness.state().current_app_context().clone();
    app_context.set_user_role(UserRole::Developer);
    let (identity, contract, token_id) = seed_owned_token(&app_context);
    let token_config = contract.tokens()[&0].clone();
    let info = IdentityTokenInfo {
        token_id,
        token_alias: SMOKE_TOKEN.to_string(),
        identity: identity.clone(),
        data_contract: QualifiedContract {
            contract: contract.clone(),
            alias: Some("smoke-contract".into()),
        },
        token_config: token_config.clone(),
        token_position: 0,
    };
    let balance = IdentityTokenBalance {
        token_id,
        token_alias: SMOKE_TOKEN.to_string(),
        token_config,
        identity_id: identity.identity.id(),
        balance: 1_000_000_000,
        estimated_unclaimed_rewards: None,
        data_contract_id: contract.id(),
        token_position: 0,
    };
    harness
        .state_mut()
        .active_root_screen_mut()
        .refresh_on_arrival();
    harness.run_steps(5);
    TokenFixture {
        harness,
        app_context,
        info,
        balance,
    }
}

impl TokenFixture {
    /// Shows `screen_type` as the only pushed screen.
    fn open(&mut self, screen_type: ScreenType) {
        let screen = screen_type.create_screen(&self.app_context);
        let state = self.harness.state_mut();
        state.screen_stack.clear();
        state.screen_stack.push(screen);
        self.harness.run_steps(5);
    }

    /// Clicks the top panel's breadcrumb `label` (the left navigation repeats
    /// some of these names lower down).
    fn click_breadcrumb(&mut self, label: &str) {
        self.harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, label)
            .find(|node| node.rect().top() < 60.0)
            .unwrap_or_else(|| panic!("no {label:?} breadcrumb"))
            .click();
        self.harness.run_steps(5);
    }
}

/// Fails when the central panel shows a blank band (content pushed down, as
/// in the Mint bug) or when two controls overlap.
fn assert_intact_layout(
    harness: &Harness<'static, dash_evo_tool::app::AppState>,
    screen: &str,
    content_left: f32,
) {
    use egui::accesskit::Role;
    const MAX_GAP: f32 = 120.0;
    let height = harness.ctx.content_rect().height();
    let mut nodes: Vec<(egui::Rect, Role)> = harness
        .query_all_by_label_contains("")
        .map(|node| (node.rect(), node.accesskit_node().role()))
        .filter(|(rect, _)| {
            rect.left() >= content_left && rect.top() >= 100.0 && rect.bottom() <= height
        })
        .collect();
    assert!(!nodes.is_empty(), "{screen}: the central panel is empty");
    nodes.sort_by(|a, b| a.0.top().total_cmp(&b.0.top()));
    let mut bottom = nodes[0].0.bottom();
    for (rect, _) in &nodes[1..] {
        let gap = rect.top() - bottom;
        assert!(
            gap <= MAX_GAP,
            "{screen}: {gap}px blank band above the control at {rect:?}"
        );
        bottom = bottom.max(rect.bottom());
    }
    let controls: Vec<egui::Rect> = nodes
        .iter()
        .filter(|(_, role)| {
            matches!(
                role,
                Role::Button | Role::CheckBox | Role::RadioButton | Role::ComboBox
            )
        })
        .map(|(rect, _)| *rect)
        .collect();
    for (i, a) in controls.iter().enumerate() {
        for b in &controls[i + 1..] {
            let overlap = a.intersect(*b);
            assert!(
                overlap.width() <= 2.0 || overlap.height() <= 2.0,
                "{screen}: controls overlap ({a:?} and {b:?})"
            );
        }
    }
}

/// Every token screen opens without panicking at the user's window size,
/// keeps its form together from the top, and its first breadcrumb leads back.
#[test]
fn token_screens_open_with_an_intact_layout_and_lead_back() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();
        let mut f = token_fixture();
        let info = f.info.clone();
        let balance = f.balance.clone();

        // (name, screen, first breadcrumb, central panel's left edge)
        let pushed = [
            ("Import Token", ScreenType::AddTokenById, "Tokens", 360.0),
            (
                "Transfer",
                ScreenType::TransferTokensScreen(balance),
                "Tokens",
                360.0,
            ),
            (
                "Mint",
                ScreenType::MintTokensScreen(info.clone()),
                "Tokens",
                360.0,
            ),
            (
                "Burn",
                ScreenType::BurnTokensScreen(info.clone()),
                "Tokens",
                360.0,
            ),
            (
                "Freeze",
                ScreenType::FreezeTokensScreen(info.clone()),
                "Tokens",
                360.0,
            ),
            (
                "Unfreeze",
                ScreenType::UnfreezeTokensScreen(info.clone()),
                "Tokens",
                360.0,
            ),
            (
                "Destroy",
                ScreenType::DestroyFrozenFundsScreen(info.clone()),
                "Tokens",
                360.0,
            ),
            (
                "Pause",
                ScreenType::PauseTokensScreen(info.clone()),
                "Tokens",
                360.0,
            ),
            (
                "Resume",
                ScreenType::ResumeTokensScreen(info.clone()),
                "Tokens",
                360.0,
            ),
            (
                "Update config",
                ScreenType::UpdateTokenConfigScreen(info.clone()),
                "Tokens",
                360.0,
            ),
            (
                "Purchase",
                ScreenType::PurchaseTokenScreen(info.clone()),
                "Tokens",
                360.0,
            ),
            (
                "Set price",
                ScreenType::SetTokenPriceScreen(info.clone()),
                "Tokens",
                360.0,
            ),
            (
                "Group actions",
                ScreenType::GroupActions,
                "Contracts",
                160.0,
            ),
        ];
        for (name, screen_type, back, content_left) in pushed {
            f.open(screen_type);
            assert_intact_layout(&f.harness, name, content_left);
            f.click_breadcrumb(back);
            assert!(
                f.harness.state().screen_stack.is_empty(),
                "{name}: the {back:?} breadcrumb must lead back to the root screen"
            );
        }

        // Root views: My Tokens (with the token), its details, the More Info
        // popup, Search and the Token Creator.
        f.harness.state_mut().screen_stack.clear();
        f.harness.run_steps(5);
        assert_intact_layout(&f.harness, "My Tokens", 360.0);
        f.harness.get_by_label(SMOKE_TOKEN).click();
        f.harness.run_steps(5);
        assert_intact_layout(&f.harness, "Token details", 360.0);
        f.click_breadcrumb("Tokens");
        f.harness.get_by_label("More Info").click();
        f.harness.run_steps(5);
        f.harness.get_by_label("Close").click();
        f.harness.run_steps(5);
        f.harness.get_by_label(SMOKE_TOKEN);

        for (name, root) in [
            ("Search Tokens", RootScreenType::RootScreenTokenSearch),
            ("Token Creator", RootScreenType::RootScreenTokenCreator),
        ] {
            f.harness.get_by_label(name).click();
            f.harness.run_steps(5);
            assert_eq!(f.harness.state().selected_main_screen, root, "{name}");
            assert_intact_layout(&f.harness, name, 360.0);
        }
        assert!(
            f.harness.query_by_label("No Identities Loaded").is_none(),
            "the Token Creator must offer the seeded identity, so its form renders"
        );
    });
}
