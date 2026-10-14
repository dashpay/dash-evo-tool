use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::Screen;
use dash_evo_tool::ui::identity::home::{self, HomeOutcome, HomeState};
use dash_evo_tool::ui::identity::profile_cache::{ProfileCache, ProfileFields};
use dash_evo_tool::ui::wallets::send_screen::IdentitySendMode;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::identity::{Identity, KeyID, Purpose, SecurityLevel};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) fn key(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
    let mut key = IdentityPublicKey::random_key(id, Some(id as u64), PlatformVersion::latest());
    key.set_id(id);
    key.set_purpose(purpose);
    key.set_security_level(SecurityLevel::CRITICAL);
    key
}

pub(crate) fn seed_identity(
    app_context: &Arc<AppContext>,
    on_chain: Vec<IdentityPublicKey>,
    with_private: Vec<IdentityPublicKey>,
) -> Identifier {
    seed_identity_with_id(app_context, 0xA5, on_chain, with_private)
}

/// Seeds an identity whose ID repeats `id_byte` and selects it.
pub(crate) fn seed_identity_with_id(
    app_context: &Arc<AppContext>,
    id_byte: u8,
    on_chain: Vec<IdentityPublicKey>,
    with_private: Vec<IdentityPublicKey>,
) -> Identifier {
    let public_keys = on_chain.into_iter().map(|key| (key.id(), key)).collect();
    let identity = Identity::new_with_id_and_keys(
        Identifier::from([id_byte; 32]),
        public_keys,
        PlatformVersion::latest(),
    )
    .expect("identity");
    let identity_id = identity.id();

    let mut private_keys = BTreeMap::new();
    for key in with_private {
        private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, key.id()),
            (
                QualifiedIdentityPublicKey::from(key),
                PrivateKeyData::InVault,
            ),
        );
    }

    let qualified_identity = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some("Home test identity".to_string()),
        private_keys: KeyStorage::from(private_keys),
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: app_context.network(),
    };

    app_context
        .insert_local_qualified_identity(&qualified_identity, &None)
        .expect("seed identity");
    app_context.set_selected_identity(Some(identity_id));
    identity_id
}

/// Harness state for `home::render`: its own mutable inputs (`HomeState`,
/// `ProfileCache`) plus the `(AppAction, HomeOutcome)` pair the most recent
/// render produced. Capturing the result (rather than discarding it, as a
/// prior version of this harness did) is what lets tests assert a clicked
/// button actually routes to its intended destination, not merely that the
/// click was accepted.
type HomeHarnessState = (
    HomeState,
    ProfileCache,
    Option<(AppAction, HomeOutcome)>,
    dash_evo_tool::ui::state::AvatarCache,
    dash_evo_tool::ui::state::contacts_view::ContactsState,
);

fn mount_home(app_context: Arc<AppContext>) -> Harness<'static, HomeHarnessState> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(
            move |ui, state: &mut HomeHarnessState| {
                let mut pending = Vec::new();
                let (header_action, header_outcome) =
                    home::render_header(ui, &app_context, &mut state.1, &mut state.3, &mut pending);
                let mut result =
                    home::render(ui, &app_context, &mut state.0, &mut state.1, &mut state.4);
                result.0 |= header_action;
                if header_outcome != HomeOutcome::None {
                    result.1 = header_outcome;
                }
                if let Some(task) = pending.pop() {
                    result.0 = AppAction::BackendTask(task);
                }
                // As in the hub, a load goes out only on a frame nothing else took.
                if matches!(result.0, AppAction::None) {
                    result.0 = home::load_action(&app_context, &mut state.0, &mut state.4);
                }
                // `Harness::run`/`step` internally renders extra idle frames
                // (it loops until no immediate repaint is pending), and
                // `home::render` resets its `(AppAction, HomeOutcome)` pair
                // to the no-op value on every call. Only overwrite the
                // captured result on a real (non-no-op) frame, so a click's
                // outcome survives whatever idle re-renders follow it in the
                // same `run()`/`step()` call.
                if !matches!(result.0, AppAction::None) || result.1 != HomeOutcome::None {
                    state.2 = Some(result);
                }
            },
            (
                HomeState::default(),
                ProfileCache::default(),
                None,
                Default::default(),
                Default::default(),
            ),
        );
    harness.run();
    harness
}

/// The `(AppAction, HomeOutcome)` produced by the most recent render — the
/// frame's result of processing whichever button was clicked before
/// `harness.run()`.
fn last_result<'a>(
    harness: &'a Harness<'static, HomeHarnessState>,
) -> &'a (AppAction, HomeOutcome) {
    harness
        .state()
        .2
        .as_ref()
        .expect("home::render must run at least once before a result is available")
}

#[test]
fn home_contacts_start_beside_actions_and_username_notice_stays_with_history() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let id = seed_identity(&app_context, vec![], vec![]);
        let request = dash_evo_tool::model::dpns_usernames::UsernameRequest::submitted(
            "layout-name",
            dash_evo_tool::utils::time::now_ms(),
            dash_evo_tool::model::dpns::ContestDurations {
                total: std::time::Duration::from_secs(14 * 86_400),
                join: std::time::Duration::from_secs(7 * 86_400),
            },
            None,
        );
        app_context
            .store_username_requests(&id, vec![request])
            .expect("request");
        let mut harness = mount_home(app_context);
        harness.state_mut().0.dismissed_checklist = true;
        harness.state_mut().0.payments = vec![(
            String::new(),
            "History contact".into(),
            100,
            true,
            String::new(),
        )];
        harness.run();

        let send = harness.get_by_label("Send").rect();
        let contacts = harness.get_by_label("Contacts").rect();
        let status = harness.get_by_label("View status").rect();
        let history = harness.get_by_label("Recent payments").rect();
        assert!(
            contacts.top() < send.bottom(),
            "Contacts must start beside the actions: {contacts:?} vs {send:?}"
        );
        assert!(
            status.right() < contacts.left(),
            "The username notice belongs in the left column"
        );
        assert!(status.top() > send.bottom());
        assert!(history.top() > status.bottom());
        assert!(
            (status.left() - history.left()).abs() < 8.0,
            "Notices and history must share the left alignment"
        );
    });
}

#[test]
fn home_send_to_wallet_disabled_for_everyday_without_local_withdrawal_key() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let mut harness = mount_home(app_context);
        let button = harness.get_by_label("Move to wallet");
        assert!(button.accesskit_node().is_disabled());

        button.hover();
        harness.run();
        assert!(
            harness
                .query_by_label_contains(
                    "no key available for withdrawal in the current interface mode"
                )
                .is_some(),
            "the disabled tooltip must explain the role-aware capability requirement"
        );
    });
}

/// An on-chain-only key (no locally held private material) can never be
/// used to sign a withdrawal — `resolve_withdrawal_signing_key` enforces
/// this for every role, Developer included, since no signing override is
/// wired into the withdrawal backend task. Enabling the button here used to
/// route Developer-role users into a `WithdrawalScreen` that rendered no
/// form at all (`selected_key` stayed `None`); it must stay disabled instead.
#[test]
fn home_send_to_wallet_disabled_for_developer_with_on_chain_key_only() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Developer);
        seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let harness = mount_home(app_context);
        let button = harness.get_by_label("Move to wallet");
        assert!(button.accesskit_node().is_disabled());
    });
}

#[test]
fn home_send_to_wallet_enabled_for_everyday_with_local_transfer_key() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        let transfer_key = key(1, Purpose::TRANSFER);
        seed_identity(&app_context, vec![transfer_key.clone()], vec![transfer_key]);

        let harness = mount_home(app_context);
        let button = harness.get_by_label("Move to wallet");
        assert!(!button.accesskit_node().is_disabled());
    });
}

/// The prior version of this suite queried only `Send to wallet`, so `render`
/// could omit `Add funds` / `Send to another identity` / `Add contact`, or
/// render one of them twice, without failing anything here. Assert the full
/// four-button row is present, each exactly once.
#[test]
fn home_renders_shared_send_actions_and_contact_entry() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        let transfer_key = key(1, Purpose::TRANSFER);
        seed_identity(&app_context, vec![transfer_key.clone()], vec![transfer_key]);

        let harness = mount_home(app_context);
        for label in ["Top up", "Move to wallet", "Send", "Add contact"] {
            assert_eq!(
                harness.query_all_by_label(label).count(),
                1,
                "expected exactly one {label:?} button in the action row"
            );
        }
    });
}

/// Clicking `Add funds` must open the top-up screen — not merely be
/// clickable. `mount_home` used to discard `home::render`'s returned
/// `(AppAction, HomeOutcome)`, so a button wired to the wrong destination
/// (or to none at all) would have passed every test in this file.
#[test]
fn home_top_up_opens_shared_send_with_fixed_recipient() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let mut harness = mount_home(app_context);
        harness.get_by_label("Top up").click();
        // A single settle pass: `home::render` resets `action` to `None` at
        // the top of every call, so an extra idle frame here (no queued
        // input) would overwrite this frame's real result with a no-op one.
        harness.run();

        let (action, _outcome) = last_result(&harness);
        assert!(
            matches!(action, AppAction::AddScreen(Screen::WalletSendScreen(screen)) if screen.identity_flow().is_some_and(|(identity, mode)| *mode == IdentitySendMode::TopUp && identity.identity.id() == Identifier::from([0xA5; 32]))),
            "expected Top up to fix the recipient in the shared send form, got {action:?}"
        );
    });
}

/// Move to wallet shares the send form with an owned-address restriction.
#[test]
fn home_move_to_wallet_opens_shared_send() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        let transfer_key = key(1, Purpose::TRANSFER);
        seed_identity(&app_context, vec![transfer_key.clone()], vec![transfer_key]);

        let mut harness = mount_home(app_context);
        harness.get_by_label("Move to wallet").click();
        harness.run();

        let (action, _outcome) = last_result(&harness);
        assert!(
            matches!(action, AppAction::AddScreen(Screen::WalletSendScreen(screen)) if screen.identity_flow().is_some_and(|(identity, mode)| *mode == IdentitySendMode::MoveToWallet && identity.identity.id() == Identifier::from([0xA5; 32]))),
            "expected Move to wallet to use the bound withdrawal form, got {action:?}"
        );
    });
}

/// Send shares the wallet form with its sender fixed to this identity.
#[test]
fn home_send_opens_shared_send_with_fixed_sender() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let mut harness = mount_home(app_context);
        harness.get_by_label("Send").click();
        harness.run();

        let (action, _outcome) = last_result(&harness);
        assert!(
            matches!(action, AppAction::AddScreen(Screen::WalletSendScreen(screen)) if screen.identity_flow().is_some_and(|(identity, mode)| *mode == IdentitySendMode::Send && identity.identity.id() == Identifier::from([0xA5; 32]))),
            "expected Send to fix the sender in the shared send form, got {action:?}"
        );
    });
}

/// Add contact opens its existing flow from Home.
#[test]
fn home_add_contact_opens_contact_screen() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        let identity_id =
            seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let mut harness = mount_home(app_context);
        harness.state_mut().1.record_saved(
            identity_id,
            ProfileFields {
                display_name: "Alex".to_string(),
                ..Default::default()
            },
        );
        harness.run();

        let button = harness.get_by_label("Add contact");
        assert!(
            !button.accesskit_node().is_disabled(),
            "Add contact must enable once the identity has a social profile"
        );
        button.click();
        harness.run();

        let (action, _) = last_result(&harness);
        assert!(
            matches!(action, AppAction::AddScreen(_)),
            "Add contact opens the add-contact screen"
        );
    });
}

#[test]
fn home_profile_avatar_requests_its_image() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let id = seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);
        let mut harness = mount_home(app_context);
        harness.state_mut().1.record_saved(
            id,
            ProfileFields {
                display_name: "Alex".into(),
                avatar_url: "https://example.com/avatar.png".into(),
                ..Default::default()
            },
        );
        harness.run_steps(1);
        assert!(
            matches!(harness.state().2.as_ref(), Some((AppAction::BackendTask(
            dash_evo_tool::backend_task::BackendTask::DashPayTask(task)), _))
            if matches!(task.as_ref(), dash_evo_tool::backend_task::dashpay::DashPayTask::FetchAvatar { url }
                if url == "https://example.com/avatar.png")),
            "Home must load the published profile photo"
        );
    });
}

#[test]
fn home_username_summary_limits_to_three_and_routes_add_and_overflow() {
    with_isolated_data_dir(|| {
        let (_runtime, ctx) = fresh_app_context();
        let id = super::identity_usernames::seed_username_identity(
            &ctx,
            0xB4,
            "Alex",
            &["alex2", "alex3", "alex4", "alex5"],
            0,
            true,
        );
        let mut harness = mount_home(ctx);
        assert!(harness.query_by_label("@alex2, @alex3, @alex4").is_some());
        assert!(harness.query_by_label_contains("@alex5").is_none());
        harness.get_by_label("…").click();
        harness.run();
        assert_eq!(last_result(&harness).1, HomeOutcome::GoToUsernames);
        harness.get_by_label("Add username").click();
        harness.run();
        let AppAction::AddScreen(Screen::RegisterDpnsNameScreen(screen)) = &last_result(&harness).0
        else {
            panic!("expected username registration");
        };
        assert_eq!(
            screen
                .selected_qualified_identity
                .as_ref()
                .map(|q| q.identity.id()),
            Some(id)
        );
    });
}

#[test]
fn home_without_payments_has_no_empty_activity_placeholder() {
    with_isolated_data_dir(|| {
        let (_runtime, ctx) = fresh_app_context();
        seed_identity(&ctx, vec![], vec![]);
        let harness = mount_home(ctx);
        assert!(harness.query_by_label("Recent activity").is_none());
        assert!(harness.query_by_label("Recent payments").is_none());
        assert!(harness.query_by_label("Advanced details").is_none());
        assert!(harness.query_by_label("Receive").is_some());
    });
}

#[test]
fn home_profile_avatar_decodes_and_caches_published_image() {
    with_isolated_data_dir(|| {
        let (_runtime, ctx) = fresh_app_context();
        let id = seed_identity(&ctx, vec![], vec![]);
        let mut harness = mount_home(ctx);
        let url = "https://example.com/photo.png";
        harness.state_mut().1.record_saved(
            id,
            ProfileFields {
                avatar_url: url.into(),
                display_name: "Alex".into(),
                ..Default::default()
            },
        );
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(8, 8)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("png");
        harness.state_mut().3.seed(url, bytes.into_inner());
        harness.run();
        assert!(
            harness.state().3.ready_texture(url).is_some(),
            "the profile image must be rendered through the shared avatar cache"
        );
        harness.run();
        assert!(!harness.state().3.is_loading(url));
    });
}

#[test]
fn home_failed_avatar_does_not_retry_each_frame() {
    with_isolated_data_dir(|| {
        let (_runtime, ctx) = fresh_app_context();
        let id = seed_identity(&ctx, vec![], vec![]);
        let mut harness = mount_home(ctx);
        let url = "https://example.com/broken.png";
        harness.state_mut().1.record_saved(
            id,
            ProfileFields {
                avatar_url: url.into(),
                display_name: "Alex".into(),
                ..Default::default()
            },
        );
        harness.state_mut().3.seed(url, b"not an image".to_vec());
        harness.run();
        assert!(harness.state().3.is_failed(url));
        harness.state_mut().2 = None;
        harness.run();
        assert!(
            harness.state().2.is_none(),
            "failed image must keep its fallback without a fetch loop"
        );
    });
}

#[test]
fn home_payment_preview_rejects_stale_dispatches() {
    use dash_evo_tool::backend_task::BackendTaskSuccessResult;
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        seed_identity(&app_context, vec![], vec![]);
        let identity = app_context.resolve_selected_identity().expect("identity");
        let mut state = HomeState::default();
        let AppAction::BackendTaskWithContext { context: stale, .. } =
            state.load_payments(&identity)
        else {
            panic!("payment load");
        };
        state.refresh_payments();
        let AppAction::BackendTaskWithContext {
            context: current, ..
        } = state.load_payments(&identity)
        else {
            panic!("new payment load");
        };
        let result = BackendTaskSuccessResult::DashPayPaymentHistory(vec![(
            String::new(),
            "Alice".into(),
            1_000,
            true,
            String::new(),
        )]);
        state.record_payments(&stale, &result);
        assert!(state.payments.is_empty());
        state.record_payments(&current, &result);
        assert_eq!(state.payments.len(), 1);
        state = HomeState::default();
        state.record_payments(&current, &result);
        assert!(
            state.payments.is_empty(),
            "switching identities discards the old preview"
        );
    });
}

/// Renders one frame at a time until `wanted` recognises the action a frame
/// produced, so a test does not depend on which frame a load goes out in.
fn step_until<T>(
    harness: &mut Harness<'static, HomeHarnessState>,
    what: &str,
    wanted: impl Fn(&AppAction) -> Option<T>,
) -> T {
    for _ in 0..8 {
        harness.state_mut().2 = None;
        harness.step();
        let found = harness
            .state()
            .2
            .as_ref()
            .and_then(|(action, _)| wanted(action));
        if let Some(found) = found {
            return found;
        }
    }
    panic!("{what} was never dispatched");
}

/// One recorded payment for the selected identity, loaded the way an arrival
/// refresh loads it.
fn record_one_payment(harness: &mut Harness<'static, HomeHarnessState>, app_context: &AppContext) {
    use dash_evo_tool::backend_task::BackendTaskSuccessResult;
    let identity = app_context.resolve_selected_identity().expect("identity");
    let state = &mut harness.state_mut().0;
    state.refresh_payments();
    let AppAction::BackendTaskWithContext { context, .. } = state.load_payments(&identity) else {
        panic!("payment load");
    };
    state.record_payments(
        &context,
        &BackendTaskSuccessResult::DashPayPaymentHistory(vec![(
            String::new(),
            "Alice".into(),
            1_000,
            true,
            String::new(),
        )]),
    );
}

#[test]
fn home_payments_follow_an_identity_selected_outside_the_hub() {
    use dash_evo_tool::backend_task::dashpay::DashPayTask;
    use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let second = seed_identity_with_id(&app_context, 0xB6, vec![], vec![]);
        let first = seed_identity(&app_context, vec![], vec![]);
        let mut harness = mount_home(app_context.clone());
        record_one_payment(&mut harness, &app_context);
        let first_request = {
            let state = &mut harness.state_mut().0;
            state.refresh_payments();
            let identity = app_context.resolve_selected_identity().expect("identity");
            assert_eq!(identity.identity.id(), first);
            let AppAction::BackendTaskWithContext { context, .. } = state.load_payments(&identity)
            else {
                panic!("payment load");
            };
            context
        };
        harness.run();
        assert!(harness.query_by_label("Recent payments").is_some());

        // Another screen's identity selector changes the selection and returns
        // without refreshing the hub; the same `HomeState` keeps rendering.
        app_context.set_selected_identity(Some(second));
        let second_request = step_until(
            &mut harness,
            "the newly selected identity's payment history load",
            |action| match action {
                AppAction::BackendTaskWithContext {
                    task: BackendTask::DashPayTask(task),
                    context,
                } => match &**task {
                    DashPayTask::LoadPaymentHistory { identity }
                        if identity.identity.id() == second =>
                    {
                        Some(context.clone())
                    }
                    _ => None,
                },
                _ => None,
            },
        );
        harness.run();
        assert!(
            harness.query_by_label("Recent payments").is_none(),
            "one identity's payments must never be shown under another"
        );

        let late = BackendTaskSuccessResult::DashPayPaymentHistory(vec![(
            String::new(),
            "Alice".into(),
            1_000,
            true,
            String::new(),
        )]);
        harness.state_mut().0.record_payments(&first_request, &late);
        harness.run();
        assert!(
            harness.query_by_label("Recent payments").is_none(),
            "a late answer for the previous identity must be discarded"
        );

        let current = BackendTaskSuccessResult::DashPayPaymentHistory(vec![(
            String::new(),
            "Bob".into(),
            2_000,
            false,
            String::new(),
        )]);
        harness
            .state_mut()
            .0
            .record_payments(&second_request, &current);
        harness.run();
        assert!(harness.query_by_label("Bob").is_some());
    });
}

#[test]
fn home_contacts_follow_an_identity_selected_outside_the_hub() {
    use dash_evo_tool::backend_task::BackendTask;
    use dash_evo_tool::backend_task::dashpay::{ContactData, DashPayTask};
    use dash_sdk::dpp::document::{Document, DocumentV0};
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let second = seed_identity_with_id(&app_context, 0xB6, vec![], vec![]);
        let first = seed_identity(&app_context, vec![], vec![]);
        // A request the first identity sent to the second: one document, listed
        // as sent for one identity and as received for the other.
        let request = Identifier::from([0xC7; 32]);
        let incoming_for_second = vec![(
            request,
            Document::V0(DocumentV0 {
                id: request,
                owner_id: first,
                ..Default::default()
            }),
        )];
        let mut harness = mount_home(app_context.clone());
        {
            let contacts = &mut harness.state_mut().4;
            contacts.record_contacts(vec![ContactData {
                identity_id: Identifier::from([0xD8; 32]),
                nickname: Some("Bao".into()),
                note: None,
                is_hidden: false,
                account_reference: 0,
                username: None,
                display_name: None,
                avatar_url: None,
                bio: None,
            }]);
            // The first identity withdraws that request; the paid action runs.
            assert!(contacts.begin_request(request));
        }
        harness.run();
        assert!(harness.query_by_label("Bao").is_some());

        // Another screen's identity selector changes the selection and returns
        // without refreshing the hub; the same `ContactsState` keeps rendering.
        app_context.set_selected_identity(Some(second));
        let loaded_for = step_until(
            &mut harness,
            "the newly selected identity's contact load",
            |action| match action {
                AppAction::BackendTasks(tasks, _) => Some(
                    tasks
                        .iter()
                        .filter_map(|task| match task {
                            BackendTask::DashPayTask(task) => match &**task {
                                DashPayTask::LoadContacts { identity }
                                | DashPayTask::LoadContactRequests { identity } => {
                                    Some(identity.identity.id())
                                }
                                _ => None,
                            },
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            },
        );
        assert_eq!(loaded_for, [second, second]);
        harness.run();
        assert!(
            harness.query_by_label("Bao").is_none(),
            "one identity's contacts must never be shown under another"
        );
        assert!(
            harness
                .query_by_label("No contacts yet. Add someone by their username.")
                .is_some()
        );

        // The second identity's list arrives and holds the very request the
        // first identity is still withdrawing: it must stay guarded.
        harness
            .state_mut()
            .4
            .record_requests(incoming_for_second, vec![]);
        harness.run();
        assert!(
            harness
                .get_by_label("Accept")
                .accesskit_node()
                .is_disabled(),
            "a request with a paid action still running must not be acted on again"
        );

        // The withdrawal fails and releases its guard.
        harness.state_mut().4.release_request(&request);
        harness.run();
        assert!(
            !harness
                .get_by_label("Accept")
                .accesskit_node()
                .is_disabled()
        );
    });
}

const SETUP_CHECKLIST: &str = "Finish setting up your identity";

fn contact(byte: u8, name: &str) -> dash_evo_tool::backend_task::dashpay::ContactData {
    dash_evo_tool::backend_task::dashpay::ContactData {
        identity_id: Identifier::from([byte; 32]),
        nickname: Some(name.into()),
        note: None,
        is_hidden: false,
        account_reference: 0,
        username: None,
        display_name: None,
        avatar_url: None,
        bio: None,
    }
}

fn record_profile(harness: &mut Harness<'static, HomeHarnessState>, id: Identifier) {
    harness.state_mut().1.record_saved(
        id,
        ProfileFields {
            display_name: "Ann".into(),
            bio: String::new(),
            avatar_url: String::new(),
        },
    );
}

/// Until the profile and the contacts are read, Home cannot tell what setup
/// is left, so the checklist stays away instead of showing for a moment.
#[test]
fn home_shows_no_setup_checklist_while_its_inputs_load() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let id = seed_identity(&app_context, vec![], vec![]);

        let mut harness = mount_home(app_context.clone());
        harness.run();
        assert!(
            harness.query_by_label(SETUP_CHECKLIST).is_none(),
            "nothing is read yet"
        );

        record_profile(&mut harness, id);
        harness.run();
        assert!(
            harness.query_by_label(SETUP_CHECKLIST).is_none(),
            "the contacts are not read yet"
        );

        let mut harness = mount_home(app_context);
        harness.state_mut().4.record_contacts(vec![]);
        harness.run();
        assert!(
            harness.query_by_label(SETUP_CHECKLIST).is_none(),
            "the profile is not read yet"
        );
    });
}

/// Once everything is read and setup is unfinished, the checklist shows.
#[test]
fn home_shows_the_setup_checklist_once_its_inputs_are_known() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let id = seed_identity(&app_context, vec![], vec![]);
        let mut harness = mount_home(app_context);
        record_profile(&mut harness, id);
        harness.state_mut().4.record_contacts(vec![]);
        harness.run();
        assert!(harness.query_by_label(SETUP_CHECKLIST).is_some());
    });
}

/// Once everything is read and setup is finished, there is no checklist.
#[test]
fn home_shows_no_setup_checklist_when_setup_is_finished() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let id = super::identity_usernames::seed_username_identity(
            &app_context,
            0x62,
            "Finished",
            &["finished"],
            1_000,
            true,
        );
        let mut harness = mount_home(app_context);
        record_profile(&mut harness, id);
        harness
            .state_mut()
            .4
            .record_contacts(vec![contact(0x63, "Bao")]);
        harness.run();
        assert!(
            harness.query_by_label("Bao").is_some(),
            "the contact is shown"
        );
        assert!(harness.query_by_label(SETUP_CHECKLIST).is_none());
    });
}

#[test]
fn home_contact_preview_shows_the_first_six_contacts() {
    use dash_evo_tool::backend_task::dashpay::ContactData;
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        seed_identity(&app_context, vec![], vec![]);
        let mut harness = mount_home(app_context);
        let names = ["Ana", "Bao", "Cy", "Dee", "Eli", "Fay", "Gus"];
        harness.state_mut().4.record_contacts(
            names
                .iter()
                .zip(1u8..)
                .map(|(name, byte)| ContactData {
                    identity_id: Identifier::from([byte; 32]),
                    nickname: Some((*name).into()),
                    note: None,
                    is_hidden: false,
                    account_reference: 0,
                    username: None,
                    display_name: None,
                    avatar_url: None,
                    bio: None,
                })
                .collect(),
        );
        harness.run();
        for name in &names[..6] {
            assert!(
                harness.query_by_label(name).is_some(),
                "{name} must be shown"
            );
        }
        assert!(
            harness.query_by_label("Gus").is_none(),
            "the preview stops at six contacts"
        );
    });
}

#[test]
fn home_payments_survive_a_refresh_for_the_same_identity() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        seed_identity(&app_context, vec![], vec![]);
        let mut harness = mount_home(app_context.clone());
        record_one_payment(&mut harness, &app_context);
        harness.state_mut().0.refresh_payments();
        harness.run();
        assert!(
            harness.query_by_label("Alice").is_some(),
            "a refresh keeps the preview on screen while it reloads"
        );
    });
}

#[test]
fn home_receive_shares_identity_id_when_no_username_is_set() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        seed_identity(&app_context, vec![], vec![]);
        let mut harness = mount_home(app_context);
        harness.get_by_label("Receive").click();
        harness.run();
        assert!(harness.query_by_label("Receive Dash").is_some());
        assert!(harness.query_by_label("Copy identity ID").is_some());
        harness.get_by_label("Done").click();
        harness.run();
        assert!(harness.query_by_label("Receive Dash").is_none());
    });
}

/// The hub refreshes usernames by itself, so an identity can gain one while
/// Receive is open. The dialog then names and copies the username, and its QR
/// code must encode the same value, not the identity ID it opened with.
#[test]
fn home_receive_code_follows_a_username_gained_while_it_is_open() {
    use super::identity_usernames::seed_username_identity;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let id = seed_username_identity(&app_context, 0x61, "Receiver", &[], 1_000, true);
        let mut harness = mount_home(app_context.clone());
        harness.get_by_label("Receive").click();
        harness.run();
        let encoded = |harness: &Harness<'static, HomeHarnessState>| -> Option<String> {
            harness
                .state()
                .0
                .receive_qr
                .as_ref()
                .map(|qr| qr.value.clone())
        };
        assert_eq!(encoded(&harness), Some(id.to_string(Encoding::Base58)));

        seed_username_identity(&app_context, 0x61, "Receiver", &["alice"], 1_000, true);
        harness.run();
        assert!(
            harness.query_by_label("Copy username").is_some(),
            "the open dialog offers the username it now has"
        );
        assert_eq!(
            encoded(&harness).as_deref(),
            Some("alice.dash"),
            "the code must encode what the dialog shows and copies"
        );
    });
}

#[test]
fn home_payment_preview_opens_real_payment_history() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        seed_identity(&app_context, vec![], vec![]);
        let mut harness = mount_home(app_context);
        harness.state_mut().0.payments =
            vec![(String::new(), "Alice".into(), 100_000, true, String::new())];
        harness.run();
        assert!(harness.query_by_label("Alice").is_some());
        harness.get_by_label("See all payments").click();
        harness.run();
        assert!(matches!(
            last_result(&harness).0,
            AppAction::AddScreen(Screen::DashPayScreen(_))
        ));
    });
}
