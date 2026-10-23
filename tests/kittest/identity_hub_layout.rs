//! Identity hub layout on ordinary and wide windows.
//!
//! The hub tabs follow the 2026-09-22 identities wireframes (A05 Home, A06
//! Contacts, A08 Profile): content starts at the tab bar's left edge and grows
//! with the available window width, while narrow layouts stay on screen.

use crate::identity_home::key;
use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::backend_task::dashpay::{ContactData, DashPayTask};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::dashpay::ProfileSnapshot;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
};
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::identity::{IdentityHubScreen, IdentityHubTab};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Identity, Purpose};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

/// An ordinary laptop window and a wide desktop window.
const WIDTHS: [f32; 2] = [1280.0, 2000.0];

/// Slack for rounding and stroke widths when comparing edges.
const EPSILON: f32 = 2.0;

type HubHarness = Harness<'static, (IdentityHubScreen, Vec<AppAction>)>;

fn mount_hub(app_context: &Arc<AppContext>, width: f32) -> HubHarness {
    // The hub holds its proof-verifying reads until the masternode list is
    // synced; these tests start on a synced network.
    app_context.connection_status().set_masternodes_ready(true);
    Harness::builder()
        .with_size(egui::vec2(width, 1100.0))
        .build_ui_state(
            |ui, state: &mut (IdentityHubScreen, Vec<AppAction>)| {
                let action = state.0.ui(ui);
                if !matches!(action, AppAction::None) {
                    state.1.push(action);
                }
            },
            (IdentityHubScreen::new(app_context), Vec::new()),
        )
}

/// Seeds and selects a user identity that owns two usernames.
fn seed_identity_with_usernames(app_context: &Arc<AppContext>) -> Identifier {
    seed_identity_named(app_context, &["alex", "alexander"])
}

/// The longest username Dash Platform accepts, in a wide letter.
fn longest_username() -> String {
    "w".repeat(63)
}

/// Seeds and selects a user identity that owns `names`.
fn seed_identity_named(app_context: &Arc<AppContext>, names: &[&str]) -> Identifier {
    let auth = key(1, Purpose::AUTHENTICATION);
    let identity = Identity::new_with_id_and_keys(
        Identifier::from([0xA7; 32]),
        BTreeMap::from([(auth.id(), auth)]),
        PlatformVersion::latest(),
    )
    .expect("identity");
    let identity_id = identity.id();
    let qualified_identity = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: None,
        private_keys: KeyStorage::default(),
        dpns_names: names
            .iter()
            .map(|name| DPNSNameInfo {
                name: (*name).into(),
                acquired_at: 1_700_000_000_000,
            })
            .collect(),
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

/// Renders frames until the hub asks for `owner`'s social profile, then answers
/// with a published one so every tab shows its populated state.
fn deliver_profile(hub: &mut HubHarness, owner: Identifier) {
    for _ in 0..16 {
        let request = hub.state().1.iter().find_map(|action| match action {
            AppAction::BackendTaskWithContext {
                task: BackendTask::DashPayTask(task),
                context,
            } if matches!(**task, DashPayTask::LoadProfile { .. }) => Some(context.clone()),
            _ => None,
        });
        if let Some(request) = request {
            let app_context = hub.state().0.app_context.clone();
            hub.state_mut().0.display_backend_task_result(
                &request,
                BackendTaskSuccessResult::DashPayProfile(ProfileSnapshot {
                    network: app_context.network(),
                    owner,
                    revision: app_context.identity_profile_revision(owner),
                    profile: Some(("Alex".into(), String::new(), String::new())),
                }),
            );
            hub.state_mut().1.clear();
            return;
        }
        hub.step();
    }
    panic!("the hub never asked for the social profile");
}

fn deliver_contact(hub: &mut HubHarness, owner: Identifier) {
    deliver_contact_named(hub, owner, "Bao", None);
}

fn deliver_contact_named(
    hub: &mut HubHarness,
    owner: Identifier,
    nickname: &str,
    username: Option<String>,
) {
    deliver_contact_with(hub, owner, nickname, username, false);
}

fn deliver_contact_with(
    hub: &mut HubHarness,
    owner: Identifier,
    nickname: &str,
    username: Option<String>,
    is_hidden: bool,
) {
    let app_context = hub.state().0.app_context.clone();
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts {
        identity: app_context.resolve_selected_identity().expect("identity"),
    }));
    hub.state_mut().0.display_backend_task_result(
        &BackendTaskContext::from(&task),
        BackendTaskSuccessResult::DashPayContactsWithInfo {
            identity: owner,
            contacts: vec![ContactData {
                identity_id: Identifier::from([0xD8; 32]),
                nickname: Some(nickname.into()),
                note: None,
                is_hidden,
                account_reference: 0,
                username,
                display_name: None,
                avatar_url: None,
                bio: None,
            }],
        },
    );
}

/// The hub on `tab` for an identity with a published profile and one contact.
fn hub_on(app_context: &Arc<AppContext>, width: f32, tab: IdentityHubTab) -> HubHarness {
    let owner = seed_identity_with_usernames(app_context);
    hub_for(app_context, owner, width, tab, |hub, owner| {
        deliver_contact(hub, owner)
    })
}

fn hub_for(
    app_context: &Arc<AppContext>,
    owner: Identifier,
    width: f32,
    tab: IdentityHubTab,
    contacts: impl FnOnce(&mut HubHarness, Identifier),
) -> HubHarness {
    let mut hub = mount_hub(app_context, width);
    deliver_profile(&mut hub, owner);
    hub.state_mut().0.select_tab(tab);
    hub.run_steps(4);
    contacts(&mut hub, owner);
    hub.run_steps(4);
    hub
}

/// The tab bar's left edge, where every tab's content starts.
fn content_left(hub: &HubHarness) -> f32 {
    hub.get_by_label("Home").rect().left()
}

fn assert_within_content(hub: &HubHarness, what: &str, rect: egui::Rect, width: f32) {
    assert!(rect.left() >= content_left(hub) - EPSILON);
    let limit = width + EPSILON;
    assert!(
        rect.right() <= limit,
        "at {width} px, {what} ends at {} but the content ends at {limit}",
        rect.right()
    );
}

#[test]
fn home_stays_within_the_content_width() {
    for width in WIDTHS {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            let hub = hub_on(&app_context, width, IdentityHubTab::Home);
            let add_contact = hub
                .get_by_role_and_label(Role::Button, "Add contact")
                .rect();
            assert_within_content(&hub, "Home's Add contact button", add_contact, width);
        });
    }
}

#[test]
fn profile_keeps_the_form_and_the_usernames_card_side_by_side() {
    for width in WIDTHS {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            let hub = hub_on(&app_context, width, IdentityHubTab::Settings);
            let tab_bottom = hub.get_by_label("Home").rect().bottom();

            let usernames = hub.get_by_label("Usernames").rect();
            let form_right = hub
                .query_all_by_role(Role::TextInput)
                .chain(hub.query_all_by_role(Role::MultilineTextInput))
                .map(|field| field.rect().right())
                .fold(f32::MIN, f32::max);
            assert!(
                form_right < usernames.left(),
                "at {width} px, the profile form ({form_right}) must end before the \
                 usernames card ({})",
                usernames.left()
            );

            let another = hub
                .get_by_role_and_label(Role::Button, "Get another username")
                .rect();
            assert!(
                another.width() < (width - content_left(&hub)) / 2.0,
                "at {width} px, Get another username must fit the right column, is {}",
                another.width()
            );
            assert_within_content(&hub, "the usernames card", another, width);

            let menu = hub
                .query_all_by_label("•••")
                .map(|node| node.rect())
                .find(|rect| rect.top() > tab_bottom)
                .expect("a username has a menu");
            assert!(
                (menu.right() - another.right()).abs() <= 24.0,
                "at {width} px, a username's menu belongs at the card's right edge: \
                 {menu:?} vs {another:?}"
            );

            if let Some(badge) = hub.query_by_label("User identity") {
                let advanced = hub.get_by_label("Advanced").rect();
                assert!(
                    badge.rect().top() > advanced.bottom(),
                    "at {width} px, the identity type belongs under Advanced"
                );
            }
        });
    }
}

#[test]
fn contacts_sections_start_at_the_content_edge_with_one_add_button() {
    for width in WIDTHS {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            let hub = hub_on(&app_context, width, IdentityHubTab::Contacts);
            let left = content_left(&hub);

            let heading = hub
                .get_by_label(dash_evo_tool::ui::identity::contacts::RECEIVED_HEADING)
                .rect();
            assert!(
                heading.left() - left <= 24.0,
                "at {width} px, section headings start at the content edge ({left}), \
                 not at {}",
                heading.left()
            );

            let add_buttons = hub
                .query_all_by_label(dash_evo_tool::ui::identity::contacts::ADD_BY_USERNAME_LABEL)
                .count();
            assert_eq!(
                add_buttons, 1,
                "at {width} px, Add by username is offered once when there are contacts"
            );

            let view_profile = hub
                .get_by_label(dash_evo_tool::ui::identity::contacts::VIEW_PROFILE_LABEL)
                .rect();
            assert_within_content(&hub, "a contact's View Profile button", view_profile, width);
        });
    }
}

/// Both the header and tab content expand when the user widens the window.
#[test]
fn header_and_tabs_expand_with_the_window() {
    for (tab, content_label) in [
        (IdentityHubTab::Home, "Add contact"),
        (
            IdentityHubTab::Contacts,
            dash_evo_tool::ui::identity::contacts::VIEW_PROFILE_LABEL,
        ),
        (IdentityHubTab::Settings, "Get another username"),
    ] {
        let mut edges = Vec::new();
        for width in WIDTHS {
            with_isolated_data_dir(|| {
                let (_runtime, app_context) = fresh_app_context();
                let hub = hub_on(&app_context, width, tab);
                let header = hub
                    .get_by_role_and_label(Role::Button, "Move to wallet")
                    .rect();
                let content = hub
                    .get_by_role_and_label(Role::Button, content_label)
                    .rect();
                assert_within_content(&hub, "Move to wallet", header, width);
                assert_within_content(&hub, content_label, content, width);
                edges.push((header.right(), content.right()));
            });
        }
        let growth = WIDTHS[1] - WIDTHS[0];
        assert!(
            edges[1].0 - edges[0].0 > growth * 0.9,
            "{tab:?}'s header must use the added window width: {edges:?}"
        );
        assert!(
            edges[1].1 - edges[0].1 > growth * 0.5,
            "{tab:?}'s content must expand with the window: {edges:?}"
        );
    }
}

/// The longest username Platform accepts never pushes a username's menu, the
/// card or the Profile tab past the content column: the name is shortened to
/// the room its badge and menu leave.
#[test]
fn the_longest_username_stays_within_the_usernames_card() {
    for width in WIDTHS {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            let long = longest_username();
            let owner = seed_identity_named(&app_context, &[&long, "alex"]);
            let hub = hub_for(
                &app_context,
                owner,
                width,
                IdentityHubTab::Settings,
                |_, _| {},
            );
            let tab_bottom = hub.get_by_label("Home").rect().bottom();

            let another = hub
                .get_by_role_and_label(Role::Button, "Get another username")
                .rect();
            assert_within_content(&hub, "the usernames card", another, width);

            // The shortened name, its Main badge and its menu sit side by side.
            let name = hub
                .query_all_by_label_contains("@www")
                .map(|node| node.rect())
                .find(|rect| rect.top() > tab_bottom)
                .expect("the long username is listed");
            let badge = hub.get_by_label("Main").rect();
            let menu = hub
                .query_all_by_label("•••")
                .map(|node| node.rect())
                .filter(|rect| rect.top() > tab_bottom)
                .min_by(|a, b| a.top().total_cmp(&b.top()))
                .expect("the long username has a menu");
            assert!(
                name.right() <= badge.left() + EPSILON && badge.right() <= menu.left() + EPSILON,
                "at {width} px, name {name:?}, badge {badge:?} and menu {menu:?} must not overlap"
            );
            for menu in hub
                .query_all_by_label("•••")
                .map(|node| node.rect())
                .filter(|rect| rect.top() > tab_bottom)
            {
                assert!(
                    menu.right() <= another.right() + EPSILON,
                    "at {width} px, a username's menu ({menu:?}) must stay inside the card \
                     ({another:?})"
                );
            }
        });
    }
}

/// A long nickname and the longest username never push a contact's actions
/// past the content column.
#[test]
fn long_contact_names_stay_within_the_content_width() {
    for width in WIDTHS {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            let owner = seed_identity_with_usernames(&app_context);
            let nickname = "W".repeat(120);
            let hub = hub_for(
                &app_context,
                owner,
                width,
                IdentityHubTab::Contacts,
                |hub, owner| deliver_contact_named(hub, owner, &nickname, Some(longest_username())),
            );
            let view_profile = hub
                .get_by_label(dash_evo_tool::ui::identity::contacts::VIEW_PROFILE_LABEL)
                .rect();
            assert_within_content(&hub, "a contact's View Profile button", view_profile, width);
            assert!(
                view_profile.left() >= content_left(&hub),
                "at {width} px, View Profile stays on screen ({view_profile:?})"
            );

            // The name and the handle are shortened to the room the actions
            // leave, so neither runs under a button.
            let actions: Vec<egui::Rect> = hub
                .query_all_by_role(Role::Button)
                .map(|node| node.rect())
                .filter(|rect| {
                    rect.top() >= view_profile.top() - 30.0
                        && rect.bottom() <= view_profile.bottom() + 30.0
                })
                .collect();
            for text in ["WWWW", "@www"] {
                let text_rect = hub
                    .query_all_by_label_contains(text)
                    .map(|node| node.rect())
                    .find(|rect| rect.top() >= view_profile.top() - 30.0)
                    .unwrap_or_else(|| panic!("the contact row shows {text}"));
                for action in &actions {
                    assert!(
                        !text_rect.intersects(*action),
                        "at {width} px, {text}… ({text_rect:?}) runs under an action \
                         ({action:?})"
                    );
                }
            }
        });
    }
}

/// A hidden contact with a long nickname keeps its Unhide button inside the
/// content column.
#[test]
fn long_hidden_contact_names_stay_within_the_content_width() {
    for width in WIDTHS {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            let owner = seed_identity_with_usernames(&app_context);
            let nickname = "W".repeat(120);
            let mut hub = hub_for(
                &app_context,
                owner,
                width,
                IdentityHubTab::Contacts,
                |hub, owner| {
                    deliver_contact_with(hub, owner, &nickname, Some(longest_username()), true)
                },
            );
            hub.get_by_role(Role::CheckBox).click();
            hub.run_steps(4);
            let unhide = hub
                .get_by_role_and_label(
                    Role::Button,
                    dash_evo_tool::ui::identity::contacts::UNHIDE_LABEL,
                )
                .rect();
            assert_within_content(&hub, "a hidden contact's Unhide button", unhide, width);
            let name = hub
                .query_all_by_label_contains("WWWW")
                .map(|node| node.rect())
                .find(|rect| rect.top() >= unhide.top() - 30.0)
                .expect("the hidden contact is listed");
            assert!(
                !name.intersects(unhide),
                "at {width} px, the hidden contact's name ({name:?}) runs under Unhide \
                 ({unhide:?})"
            );
        });
    }
}

/// A window too narrow for the content width: the hub fits the window instead
/// of growing past it, and the Profile tab stacks its usernames card under the
/// form once the tab has less than 760 pt.
#[test]
fn a_narrow_window_keeps_the_hub_on_screen_and_stacks_the_profile() {
    const NARROW: f32 = 900.0;
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let hub = hub_on(&app_context, NARROW, IdentityHubTab::Settings);
        assert!(
            NARROW - content_left(&hub) < 760.0,
            "the tab must have less than 760 pt for this case"
        );

        for (what, label) in [
            ("the header's Move to wallet", "Move to wallet"),
            ("Get another username", "Get another username"),
        ] {
            let rect = hub.get_by_role_and_label(Role::Button, label).rect();
            assert!(
                rect.right() <= NARROW + EPSILON,
                "at {NARROW} px, {what} ends at {} past the window",
                rect.right()
            );
        }

        let usernames = hub.get_by_label("Usernames").rect();
        let form_bottom = hub
            .query_all_by_role(Role::TextInput)
            .chain(hub.query_all_by_role(Role::MultilineTextInput))
            .map(|field| field.rect().bottom())
            .fold(f32::MIN, f32::max);
        assert!(
            usernames.top() > form_bottom,
            "at {NARROW} px, the usernames card ({usernames:?}) stacks under the form \
             (ends at {form_bottom})"
        );
    });
}
