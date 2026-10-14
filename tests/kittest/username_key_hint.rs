//! What the Usernames card tells an identity that cannot register a username,
//! and what it lets the user do about it from where they are.

use crate::identity_usernames::seed_username_identity;
use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::legacy_recovery::{RecoveryItem, RecoveryItemDescriptor, RecoveryPlan};
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::ui::components::MessageBanner;
use dash_evo_tool::ui::identity::profile_cache::ProfileCache;
use dash_evo_tool::ui::identity::settings::SettingsTab;
use dash_evo_tool::ui::identity::{IdentityHubScreen, IdentityHubTab};
use dash_evo_tool::ui::{Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{Identity, Purpose};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

const ADD_A_KEY: &str = "Add a key to this identity to register usernames.";
const RESTORE_YOUR_KEYS: &str = "The key this identity needs to register usernames is not on this device yet. \
     Restore your keys to register usernames.";

/// A selected identity that publishes no key at all.
fn seed_identity_without_keys(app_context: &Arc<AppContext>, byte: u8) -> Identifier {
    let identity =
        Identity::create_basic_identity(Identifier::from([byte; 32]), PlatformVersion::latest())
            .expect("identity");
    let id = identity.id();
    let qualified = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some("Keyless".to_owned()),
        private_keys: KeyStorage::default(),
        dpns_names: Vec::new(),
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: app_context.network(),
    };
    app_context
        .insert_local_qualified_identity(&qualified, &None)
        .expect("seed identity");
    app_context.set_selected_identity(Some(id));
    id
}

/// The hub on its Settings tab, plus every action it returned.
type HubState = (IdentityHubScreen, Vec<AppAction>);

fn mount_hub_settings(app_context: &Arc<AppContext>) -> Harness<'static, HubState> {
    let mut hub = IdentityHubScreen::new(app_context);
    hub.select_tab(IdentityHubTab::Settings);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 1200.0))
        .build_ui_state(
            |ui, state: &mut HubState| {
                let action = state.0.ui(ui);
                if !matches!(action, AppAction::None) {
                    state.1.push(action);
                }
            },
            (hub, Vec::new()),
        );
    harness.run();
    harness
}

/// A plan restoring the HIGH authentication key `seed_username_identity`
/// publishes at key id 1.
fn plan_with_the_signing_key() -> RecoveryPlan {
    RecoveryPlan {
        items: vec![RecoveryItemDescriptor {
            item: RecoveryItem::Key {
                target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
                key_id: 1,
            },
            purpose: Some(Purpose::AUTHENTICATION),
        }],
        excluded: vec![],
    }
}

/// The restore task the hub dispatched, with the context it must be answered on.
fn dispatched_restore(
    harness: &Harness<'static, HubState>,
) -> Option<(Identifier, Vec<RecoveryItem>, BackendTaskContext)> {
    harness.state().1.iter().find_map(|action| match action {
        AppAction::BackendTaskWithContext {
            task:
                BackendTask::IdentityTask(IdentityTask::RecoverLegacyIdentityData {
                    identity_id,
                    approved,
                }),
            context,
        } => Some((*identity_id, approved.clone(), context.clone())),
        _ => None,
    })
}

/// An identity that publishes no key able to register a username really does
/// need a new key, so that advice stays.
#[test]
fn identity_without_a_suitable_key_is_told_to_add_one() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        seed_identity_without_keys(&app_context, 0x41);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1400.0, 1200.0))
            .build_ui_state(
                move |ui, tab: &mut SettingsTab| {
                    tab.render(
                        ui,
                        &app_context,
                        app_context.resolve_selected_identity().as_ref(),
                        &mut ProfileCache::default(),
                    );
                },
                SettingsTab::new(),
            );
        harness.run();
        assert!(harness.query_by_label(ADD_A_KEY).is_some());
        assert!(harness.query_by_label("Add a key").is_some());
        assert!(harness.query_by_label("Open keys").is_none());
    });
}

/// The owner's case: the key is published, this device does not hold it, and
/// the previous version's saved data does. The card offers the restore in
/// place and dispatches exactly the listed items.
#[test]
fn card_offers_the_restore_when_the_previous_version_holds_the_key() {
    with_isolated_data_dir(|| {
        let (rt, app_context) = fresh_app_context();
        let _guard = rt.enter();
        let identity_id = seed_username_identity(&app_context, 0x42, "Upgraded", &[], 0, false);
        let mut harness = mount_hub_settings(&app_context);

        harness.state_mut().0.display_task_result(
            BackendTaskSuccessResult::LegacyRecoveryCandidates {
                identity_id,
                plan: plan_with_the_signing_key(),
            },
        );
        harness.run();

        assert!(harness.query_by_label(RESTORE_YOUR_KEYS).is_some());
        assert!(harness.query_by_label(ADD_A_KEY).is_none());
        harness.get_by_label("Restore keys").click();
        // The running restore shows a spinner, which repaints forever.
        harness.run_steps(3);

        let (restored_for, approved, context) =
            dispatched_restore(&harness).expect("the card dispatches the restore");
        assert_eq!(restored_for, identity_id);
        assert_eq!(
            approved,
            vec![RecoveryItem::Key {
                target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
                key_id: 1,
            }]
        );
        assert!(
            harness
                .state()
                .0
                .accepts_legacy_recovery_result(&context, true),
            "the hub must receive the answer to the restore it dispatched",
        );
        assert!(
            harness.query_by_label("Restore keys").is_none(),
            "a running restore must not be dispatched twice",
        );

        // A failed restore returns to the offer so it can be tried again.
        harness
            .state_mut()
            .0
            .display_backend_task_error(&context, &TaskError::LegacyRecoveryIdentityChanged);
        harness.run();
        assert!(harness.query_by_label("Restore keys").is_some());
    });
}

/// Whether the hub returned an action opening the "Get a username" screen.
fn opened_registration(harness: &Harness<'static, HubState>) -> bool {
    harness.state().1.iter().any(|action| {
        matches!(
            action,
            AppAction::AddScreen(Screen::RegisterDpnsNameScreen(_))
        )
    })
}

/// The promise the offer makes: once the restore lands, the hub answers it,
/// the card reads the restored key back, the guidance and the offer are gone,
/// and "Get a username" opens the registration screen.
#[test]
fn card_enables_registration_once_the_restore_completes() {
    with_isolated_data_dir(|| {
        let (rt, app_context) = fresh_app_context();
        let _guard = rt.enter();
        let identity_id = seed_username_identity(&app_context, 0x44, "Upgraded", &[], 0, false);
        let mut harness = mount_hub_settings(&app_context);

        harness.state_mut().0.display_task_result(
            BackendTaskSuccessResult::LegacyRecoveryCandidates {
                identity_id,
                plan: plan_with_the_signing_key(),
            },
        );
        harness.run();
        harness.get_by_label("Restore keys").click();
        // One frame handles the click; the spinner it starts repaints forever.
        harness.step();
        let (_, _, context) =
            dispatched_restore(&harness).expect("the card dispatches the restore");

        // While the restore runs the identity still cannot register.
        harness.get_by_label("Get a username").click();
        harness.step();
        assert!(!opened_registration(&harness));

        // What the restore does: the stored record now holds the key. Then the
        // completion is delivered the way `AppState` delivers it, on the
        // context the restore was dispatched with.
        seed_username_identity(&app_context, 0x44, "Upgraded", &[], 0, true);
        MessageBanner::clear_all_global(app_context.egui_ctx());
        assert!(
            harness
                .state()
                .0
                .accepts_legacy_recovery_result(&context, true)
        );
        harness.state_mut().0.display_backend_task_result(
            &context,
            BackendTaskSuccessResult::LegacyRecoveryCompleted {
                identity_id,
                applied: plan_with_the_signing_key().items,
                skipped_stale: vec![],
                excluded: vec![],
            },
        );
        harness.run();

        assert!(
            MessageBanner::has_global(app_context.egui_ctx()),
            "the finished restore is confirmed to the user",
        );
        for gone in [RESTORE_YOUR_KEYS, "Restore keys", "Restoring…", "Open keys"] {
            assert!(harness.query_by_label(gone).is_none(), "{gone} remains");
        }
        assert!(
            !harness
                .state()
                .0
                .accepts_legacy_recovery_result(&context, true),
            "a finished offer is retired, not left waiting for an answer",
        );

        assert!(
            !harness.state().1.iter().any(|action| matches!(
                action,
                AppAction::BackendTaskWithContext {
                    task: BackendTask::IdentityTask(IdentityTask::CheckLegacyRecovery { .. }),
                    ..
                }
            )),
            "an identity that can register asks nothing more of the previous version's data",
        );

        harness.get_by_label("Get a username").click();
        harness.run();
        assert!(opened_registration(&harness));
    });
}

/// A recovery answer for another identity must not put an offer on this card.
#[test]
fn card_ignores_a_restore_offer_for_another_identity() {
    with_isolated_data_dir(|| {
        let (rt, app_context) = fresh_app_context();
        let _guard = rt.enter();
        seed_username_identity(&app_context, 0x43, "Upgraded", &[], 0, false);
        let mut harness = mount_hub_settings(&app_context);

        harness.state_mut().0.display_task_result(
            BackendTaskSuccessResult::LegacyRecoveryCandidates {
                identity_id: Identifier::from([0x7d; 32]),
                plan: plan_with_the_signing_key(),
            },
        );
        harness.run();

        assert!(harness.query_by_label("Restore keys").is_none());
        assert!(harness.query_by_label("Open keys").is_some());
    });
}
