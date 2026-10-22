//! Contacts tab master–detail: selecting a contact shows its details and its
//! private note beside the list (under it on a narrow window), and Save note
//! dispatches one encrypted `contactInfo` write with the fee shown beforehand.
//! Nothing is signed or broadcast — dispatched tasks are only captured.

use crate::identity_hub_layout::{
    EPSILON, HubHarness, assert_within_content, content_left, hub_for, seed_identity_with_usernames,
};
use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::backend_task::dashpay::{ContactData, DashPayTask};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use dash_evo_tool::model::dashpay::ContactInfoField;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::identity::IdentityHubTab;
use dash_evo_tool::ui::identity::contacts::{
    DETAIL_HEADING, NO_SELECTION, NOTE_HEADING, SAVE_NOTE_LABEL, SAVING_NOTE_LABEL,
};
use dash_sdk::platform::Identifier;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

const WIDE: f32 = 1280.0;
const NARROW: f32 = 900.0;
const CONTACT: [u8; 32] = [0xD8; 32];

fn bao(note: Option<&str>) -> ContactData {
    ContactData {
        identity_id: Identifier::from(CONTACT),
        nickname: None,
        note: note.map(str::to_string),
        is_hidden: false,
        account_reference: 0,
        username: Some("bao".into()),
        display_name: Some("Bao Nguyen".into()),
        avatar_url: None,
        bio: Some("Coffee and code.".into()),
    }
}

fn deliver(hub: &mut HubHarness, owner: Identifier, contact: ContactData) {
    let app_context = hub.state().0.app_context.clone();
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts {
        identity: app_context.resolve_selected_identity().expect("identity"),
    }));
    hub.state_mut().0.display_backend_task_result(
        &BackendTaskContext::from(&task),
        BackendTaskSuccessResult::DashPayContactsWithInfo {
            identity: owner,
            contacts: vec![contact],
        },
    );
}

/// The Contacts tab with Bao listed, then Bao selected.
fn hub_with_bao_selected(
    app_context: &std::sync::Arc<dash_evo_tool::context::AppContext>,
    width: f32,
    note: Option<&'static str>,
) -> (HubHarness, Identifier) {
    let owner = seed_identity_with_usernames(app_context);
    let mut hub = hub_for(
        app_context,
        owner,
        width,
        IdentityHubTab::Contacts,
        move |hub, owner| deliver(hub, owner, bao(note)),
    );
    hub.get_by_role_and_label(Role::Button, "Bao Nguyen")
        .click();
    hub.run_steps(4);
    (hub, owner)
}

fn note_editor(hub: &HubHarness) -> egui_kittest::Node<'_> {
    hub.get_by_role(Role::MultilineTextInput)
}

#[test]
fn home_contact_opens_contacts_with_selected_details() {
    for width in [WIDE, NARROW] {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            let owner = seed_identity_with_usernames(&app_context);
            let mut hub = hub_for(
                &app_context,
                owner,
                width,
                IdentityHubTab::Home,
                |hub, owner| deliver(hub, owner, bao(Some("Met at the meetup"))),
            );
            hub.state_mut().1.clear();
            hub.get_by_role_and_label(Role::Button, "Bao Nguyen")
                .click();
            hub.run_steps(4);

            assert_eq!(hub.state().0.selected_tab(), IdentityHubTab::Contacts);
            assert!(hub.query_by_label(DETAIL_HEADING).is_some());
            assert!(hub.query_by_label("Coffee and code.").is_some());
            assert_eq!(
                note_editor(&hub).value().as_deref(),
                Some("Met at the meetup")
            );
            assert!(
                hub.state()
                    .1
                    .iter()
                    .all(|action| !matches!(action, AppAction::AddScreen(_)))
            );

            deliver(&mut hub, owner, bao(Some("Updated note")));
            hub.run_steps(4);
            assert_eq!(note_editor(&hub).value().as_deref(), Some("Updated note"));
        });
    }
}

#[test]
fn contact_details_pay_label_is_centered() {
    fn pay_text(shape: &egui::Shape, button: egui::Rect) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == "Pay" => {
                let rect = text.visual_bounding_rect();
                button.contains(rect.center()).then_some(rect)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| pay_text(shape, button)),
            _ => None,
        }
    }

    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(dash_evo_tool::model::user_role::UserRole::Power);
        let (hub, _) = hub_with_bao_selected(&app_context, WIDE, None);
        let heading = hub.get_by_label(DETAIL_HEADING).rect();
        let button = hub
            .query_all_by_role_and_label(Role::Button, "Pay")
            .map(|node| node.rect())
            .find(|rect| rect.left() >= heading.left() - EPSILON && rect.top() > heading.bottom())
            .expect("Pay in the contact detail pane");
        let text = hub
            .output()
            .shapes
            .iter()
            .find_map(|shape| pay_text(&shape.shape, button))
            .expect("painted Pay label");
        assert!(
            (text.center().x - button.center().x).abs() <= 1.5,
            "Pay must be centered: text {text:?}, button {button:?}"
        );
    });
}

/// The note writes the Save note click dispatched.
fn saved_notes(hub: &HubHarness) -> Vec<(Identifier, ContactInfoField<Option<String>>)> {
    hub.state()
        .1
        .iter()
        .filter_map(|action| match dispatched_task(action)?.0 {
            BackendTask::DashPayTask(task) => match task.as_ref() {
                DashPayTask::UpdateContactInfo {
                    contact_id, update, ..
                } => Some((*contact_id, update.note.clone())),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The task an action dispatches, with the context its result comes back under.
fn dispatched_task(action: &AppAction) -> Option<(&BackendTask, BackendTaskContext)> {
    match action {
        AppAction::BackendTask(task) => Some((task, BackendTaskContext::from(task))),
        AppAction::BackendTaskWithContext { task, context } => Some((task, context.clone())),
        _ => None,
    }
}

#[test]
fn selecting_a_contact_shows_its_details_beside_the_list() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let owner = seed_identity_with_usernames(&app_context);
        let mut hub = hub_for(
            &app_context,
            owner,
            WIDE,
            IdentityHubTab::Contacts,
            |hub, owner| deliver(hub, owner, bao(Some("Met at the meetup"))),
        );
        assert!(
            hub.query_by_label(NO_SELECTION).is_some(),
            "the pane says how to fill it before a contact is chosen"
        );
        assert!(hub.query_by_role(Role::MultilineTextInput).is_none());

        hub.get_by_role_and_label(Role::Button, "Bao Nguyen")
            .click();
        hub.run_steps(4);

        assert!(hub.query_by_label(NO_SELECTION).is_none());
        let heading = hub.get_by_label(DETAIL_HEADING).rect();
        let row = hub.get_by_role_and_label(Role::Button, "Bao Nguyen").rect();
        assert!(
            heading.left() > row.right(),
            "on a wide window the details sit beside the list: {heading:?} vs {row:?}"
        );
        assert_within_content(&hub, "the contact detail pane", heading, WIDE);

        let in_pane = |label: &str| {
            hub.query_all_by_label(label)
                .any(|node| node.rect().left() >= heading.left() - EPSILON)
        };
        for label in ["Bao Nguyen", "@bao", "Coffee and code.", NOTE_HEADING] {
            assert!(in_pane(label), "the detail pane shows {label}");
        }
        assert_eq!(
            note_editor(&hub).value().as_deref(),
            Some("Met at the meetup"),
            "the stored note is loaded into the editor"
        );
        let editor = note_editor(&hub).rect();
        assert_within_content(&hub, "the note editor", editor, WIDE);
        assert!(
            editor.left() >= heading.left() - 24.0,
            "the note editor belongs to the pane, not the list"
        );
    });
}

#[test]
fn save_note_shows_the_fee_first_and_sends_the_note_once() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let (mut hub, owner) = hub_with_bao_selected(&app_context, WIDE, None);

        assert!(
            hub.query_by_label_contains("Estimated fee for saving this note: up to")
                .is_some(),
            "the fee is shown before the user saves"
        );
        hub.get_by_role_and_label(Role::Button, SAVE_NOTE_LABEL)
            .click();
        hub.run_steps(2);
        assert!(
            saved_notes(&hub).is_empty(),
            "an unchanged note has nothing to save"
        );

        note_editor(&hub).focus();
        hub.run();
        note_editor(&hub).type_text("Met at the meetup");
        hub.run();
        hub.get_by_role_and_label(Role::Button, SAVE_NOTE_LABEL)
            .click();
        hub.run_steps(2);

        assert_eq!(
            saved_notes(&hub),
            [(
                Identifier::from(CONTACT),
                ContactInfoField::Replace(Some("Met at the meetup".into()))
            )],
            "Save note dispatches one write of the typed note for that contact"
        );
        hub.get_by_role_and_label(Role::Button, SAVING_NOTE_LABEL)
            .click();
        hub.run_steps(2);
        assert_eq!(
            saved_notes(&hub).len(),
            1,
            "a click while the paid write runs buys nothing"
        );

        hub.state_mut().0.display_task_result(
            BackendTaskSuccessResult::DashPayContactInfoUpdated {
                identity: owner,
                contact_id: Identifier::from(CONTACT),
            },
        );
        hub.run_steps(4);

        hub.get_by_role_and_label(Role::Button, SAVE_NOTE_LABEL)
            .click();
        hub.run_steps(2);
        assert_eq!(
            saved_notes(&hub).len(),
            1,
            "the saved note is not offered for saving again"
        );
        assert_eq!(
            note_editor(&hub).value().as_deref(),
            Some("Met at the meetup")
        );
    });
}

#[test]
fn a_narrow_window_stacks_the_details_under_the_contact() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let (hub, _) = hub_with_bao_selected(&app_context, NARROW, Some("Met at the meetup"));
        assert!(
            NARROW - content_left(&hub) < 760.0,
            "the tab must have less than 760 pt for this case"
        );

        let row = hub.get_by_role_and_label(Role::Button, "Bao Nguyen").rect();
        let heading = hub.get_by_label(DETAIL_HEADING).rect();
        assert!(
            heading.top() > row.bottom(),
            "on a narrow window the details stack under the contact: {heading:?} vs {row:?}"
        );
        for rect in [heading, note_editor(&hub).rect()] {
            assert!(
                rect.right() <= NARROW + EPSILON,
                "the pane stays inside the window: {rect:?}"
            );
        }
    });
}

/// The pane is laid out the same in both layouts: the two-column split must not
/// leak its column gap into the spacing inside the columns.
#[test]
fn the_detail_pane_keeps_its_spacing_in_both_layouts() {
    let name_offset = |width: f32| {
        with_isolated_data_dir(|| {
            let (_runtime, app_context) = fresh_app_context();
            let (hub, _) = hub_with_bao_selected(&app_context, width, None);
            let heading = hub.get_by_label(DETAIL_HEADING).rect();
            let name = hub
                .query_all_by_label("Bao Nguyen")
                .map(|node| node.rect())
                .find(|rect| rect.top() > heading.bottom() && rect.left() > heading.left())
                .expect("the pane shows the contact's name");
            name.left() - heading.left()
        })
    };
    let (wide, narrow) = (name_offset(WIDE), name_offset(NARROW));
    assert!(
        (wide - narrow).abs() <= EPSILON,
        "the name sits {wide} pt into the pane on a wide window but {narrow} pt on a narrow one"
    );
}

/// A contact's @username lines up with its name, which starts after the
/// selectable row's padding.
#[test]
fn a_contacts_username_lines_up_with_its_name() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let (hub, _) = hub_with_bao_selected(&app_context, WIDE, None);
        let row = hub.get_by_role_and_label(Role::Button, "Bao Nguyen").rect();
        let handle = hub
            .query_all_by_label("@bao")
            .map(|node| node.rect())
            .find(|rect| rect.left() < row.right())
            .expect("the row shows the username");
        let padding = hub.ctx.global_style().spacing.button_padding.x;
        assert!(
            (handle.left() - (row.left() + padding)).abs() <= EPSILON,
            "the username starts at {} but the name text at {}",
            handle.left(),
            row.left() + padding
        );
    });
}

// -------------------------------------------------------------------
// A save's outcome reaches the hub even while another screen covers it.
// -------------------------------------------------------------------

/// Bao selected with a typed note and Save note clicked, with the hub mounted
/// on `app`'s context so it can be moved behind another screen.
fn hub_saving_a_note(
    app: &egui_kittest::Harness<'static, dash_evo_tool::app::AppState>,
) -> (HubHarness, Identifier, BackendTaskContext) {
    let app_context = app.state().current_app_context().clone();
    let (mut hub, owner) = hub_with_bao_selected(&app_context, WIDE, None);
    note_editor(&hub).focus();
    hub.run();
    note_editor(&hub).type_text("Met at the meetup");
    hub.run();
    hub.get_by_role_and_label(Role::Button, SAVE_NOTE_LABEL)
        .click();
    hub.run_steps(2);
    let context = hub
        .state()
        .1
        .iter()
        .find_map(|action| match dispatched_task(action)? {
            (BackendTask::DashPayTask(inner), context)
                if matches!(**inner, DashPayTask::UpdateContactInfo { .. }) =>
            {
                Some(context)
            }
            _ => None,
        })
        .expect("Save note dispatched the write");
    assert!(hub.query_by_label(SAVING_NOTE_LABEL).is_some());
    hub.state_mut().1.clear();
    (hub, owner, context)
}

fn contact_info_failure(
    owner: Identifier,
    source: dash_evo_tool::backend_task::error::TaskError,
) -> dash_evo_tool::backend_task::error::TaskError {
    dash_evo_tool::backend_task::error::TaskError::DashPayContactInfoActionFailed {
        identity_id: owner,
        contact_id: Identifier::from(CONTACT),
        source: Box::new(source),
    }
}

#[test]
fn a_note_saved_behind_another_screen_is_settled_on_return() {
    with_isolated_data_dir(|| {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let mut app =
            crate::support::mount_app(dash_evo_tool::ui::RootScreenType::RootScreenWalletsBalances);
        let (mut hub, owner, context) = hub_saving_a_note(&app);

        crate::identity_hub::loads::receive_task_results_behind_the_send_screen(
            &runtime,
            &mut app,
            &mut hub,
            vec![dash_evo_tool::app::TaskResult::Success {
                context,
                result: Box::new(BackendTaskSuccessResult::DashPayContactInfoUpdated {
                    identity: owner,
                    contact_id: Identifier::from(CONTACT),
                }),
            }],
        );
        hub.run_steps(2);

        assert!(
            hub.query_by_label(SAVING_NOTE_LABEL).is_none(),
            "a save that finished behind another screen must not stay Saving…"
        );
        hub.get_by_role_and_label(Role::Button, SAVE_NOTE_LABEL)
            .click();
        hub.run_steps(2);
        assert!(saved_notes(&hub).is_empty(), "the saved note is not dirty");
    });
}

#[test]
fn a_note_save_that_failed_behind_another_screen_can_be_retried() {
    with_isolated_data_dir(|| {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let mut app =
            crate::support::mount_app(dash_evo_tool::ui::RootScreenType::RootScreenWalletsBalances);
        let (mut hub, owner, context) = hub_saving_a_note(&app);

        crate::identity_hub::loads::receive_task_results_behind_the_send_screen(
            &runtime,
            &mut app,
            &mut hub,
            vec![dash_evo_tool::app::TaskResult::Error {
                context,
                error: contact_info_failure(
                    owner,
                    dash_evo_tool::backend_task::error::TaskError::WalletLocked,
                ),
            }],
        );
        hub.run_steps(2);

        hub.get_by_role_and_label(Role::Button, SAVE_NOTE_LABEL)
            .click();
        hub.run_steps(2);
        assert_eq!(
            saved_notes(&hub).len(),
            1,
            "a failed save releases its guard, so the kept note can be saved again"
        );
    });
}

#[test]
fn bare_contact_info_failure_behind_another_screen_releases_only_its_save() {
    use dash_evo_tool::backend_task::error::TaskError;
    with_isolated_data_dir(|| {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let mut app =
            crate::support::mount_app(dash_evo_tool::ui::RootScreenType::RootScreenWalletsBalances);
        let (mut hub, _, context) = hub_saving_a_note(&app);
        hub.state_mut()
            .0
            .display_backend_task_error(&BackendTaskContext::Other, &TaskError::WalletLocked);
        hub.run_steps(2);
        assert!(
            hub.query_by_label(SAVING_NOTE_LABEL).is_some(),
            "unrelated errors keep the guard"
        );
        crate::identity_hub::loads::receive_task_results_behind_the_send_screen(
            &runtime,
            &mut app,
            &mut hub,
            vec![dash_evo_tool::app::TaskResult::Error {
                context,
                error: TaskError::WalletLocked,
            }],
        );
        hub.run_steps(2);
        assert!(hub.query_by_label(SAVING_NOTE_LABEL).is_none());
        hub.get_by_role_and_label(Role::Button, SAVE_NOTE_LABEL)
            .click();
        hub.run_steps(2);
        assert_eq!(saved_notes(&hub).len(), 1);
    });
}

#[test]
fn unreadable_details_found_behind_another_screen_still_ask_first() {
    use dash_evo_tool::backend_task::error::{ContactInfoReadError, TaskError};
    with_isolated_data_dir(|| {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let mut app =
            crate::support::mount_app(dash_evo_tool::ui::RootScreenType::RootScreenWalletsBalances);
        let (mut hub, owner, context) = hub_saving_a_note(&app);

        crate::identity_hub::loads::receive_task_results_behind_the_send_screen(
            &runtime,
            &mut app,
            &mut hub,
            vec![dash_evo_tool::app::TaskResult::Error {
                context,
                error: contact_info_failure(
                    owner,
                    TaskError::DashPayContactInfoRead {
                        source: ContactInfoReadError::DeserializeFailed,
                    },
                ),
            }],
        );
        hub.run_steps(2);

        assert!(
            hub.query_by_label("Save note and replace saved details?")
                .is_some(),
            "the overwrite question must be asked once the hub is back in view"
        );
    });
}

/// The outcome of a same-contact write another screen dispatched: the
/// legacy Contact Details screen sends plain tasks, so its context names the
/// contact but no dispatch.
fn foreign_write_context(hub: &HubHarness) -> BackendTaskContext {
    let app_context = hub.state().0.app_context.clone();
    BackendTaskContext::from(&BackendTask::DashPayTask(Box::new(
        DashPayTask::UpdateContactInfo {
            identity: app_context.resolve_selected_identity().expect("identity"),
            contact_id: Identifier::from(CONTACT),
            update: dash_evo_tool::model::dashpay::ContactInfoUpdate::note(Some(
                "from the other screen".into(),
            )),
        },
    )))
}

/// Another screen's write for the same contact finishes while the hub's own
/// note save is still queued behind it. Its success must not settle, show as
/// saved, or announce the hub's write; only the hub's own outcome does.
#[test]
fn a_foreign_same_contact_save_does_not_settle_the_hubs_note() {
    with_isolated_data_dir(|| {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let app =
            crate::support::mount_app(dash_evo_tool::ui::RootScreenType::RootScreenWalletsBalances);
        let (mut hub, owner, own_context) = hub_saving_a_note(&app);
        let foreign = foreign_write_context(&hub);
        let updated = || BackendTaskSuccessResult::DashPayContactInfoUpdated {
            identity: owner,
            contact_id: Identifier::from(CONTACT),
        };

        hub.state_mut()
            .0
            .display_backend_task_result(&foreign, updated());
        hub.run_steps(2);
        assert!(
            hub.query_by_label(SAVING_NOTE_LABEL).is_some(),
            "another screen's save must not settle the hub's queued note"
        );

        hub.state_mut()
            .0
            .display_backend_task_result(&own_context, updated());
        hub.run_steps(2);
        assert!(hub.query_by_label(SAVING_NOTE_LABEL).is_none());
    });
}

/// Likewise, another screen's failure for the same contact leaves the hub's
/// queued note save running.
#[test]
fn a_foreign_same_contact_failure_does_not_release_the_hubs_note() {
    use dash_evo_tool::backend_task::error::TaskError;
    with_isolated_data_dir(|| {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let app =
            crate::support::mount_app(dash_evo_tool::ui::RootScreenType::RootScreenWalletsBalances);
        let (mut hub, owner, _) = hub_saving_a_note(&app);
        let foreign = foreign_write_context(&hub);

        for error in [
            contact_info_failure(owner, TaskError::WalletLocked),
            TaskError::WalletLocked,
        ] {
            hub.state_mut()
                .0
                .display_backend_task_error(&foreign, &error);
            hub.state_mut().0.display_task_error(&error);
            hub.run_steps(2);
            assert!(
                hub.query_by_label(SAVING_NOTE_LABEL).is_some(),
                "another screen's failure must not release the hub's queued note: {error:?}"
            );
        }
    });
}

#[test]
fn failed_unhide_behind_another_screen_restores_the_hidden_row() {
    use dash_evo_tool::backend_task::error::TaskError;
    use dash_evo_tool::ui::identity::contacts::{SHOW_HIDDEN_LABEL, UNHIDE_LABEL};
    for bare_error in [false, true] {
        with_isolated_data_dir(|| {
            let runtime = tokio::runtime::Runtime::new().expect("runtime");
            let _guard = runtime.enter();
            let mut app = crate::support::mount_app(
                dash_evo_tool::ui::RootScreenType::RootScreenWalletsBalances,
            );
            let app_context = app.state().current_app_context().clone();
            let owner = seed_identity_with_usernames(&app_context);
            let mut hub = hub_for(
                &app_context,
                owner,
                WIDE,
                IdentityHubTab::Contacts,
                |hub, owner| {
                    let mut contact = bao(None);
                    contact.is_hidden = true;
                    deliver(hub, owner, contact);
                },
            );
            hub.get_by_label(&format!("{SHOW_HIDDEN_LABEL} · 1"))
                .click();
            hub.run_steps(2);
            hub.get_by_role_and_label(Role::Button, UNHIDE_LABEL)
                .click();
            hub.run_steps(2);
            let context = hub
                .state()
                .1
                .iter()
                .find_map(|action| match dispatched_task(action)? {
                    (BackendTask::DashPayTask(inner), context)
                        if matches!(**inner, DashPayTask::UpdateContactInfo { .. }) =>
                    {
                        Some(context)
                    }
                    _ => None,
                })
                .expect("Unhide dispatched a write");
            assert!(
                hub.query_by_role_and_label(Role::Button, "Bao Nguyen")
                    .is_some()
            );
            crate::identity_hub::loads::receive_task_results_behind_the_send_screen(
                &runtime,
                &mut app,
                &mut hub,
                vec![dash_evo_tool::app::TaskResult::Error {
                    context,
                    error: if bare_error {
                        TaskError::WalletLocked
                    } else {
                        contact_info_failure(owner, TaskError::WalletLocked)
                    },
                }],
            );
            hub.run_steps(2);
            assert!(
                hub.query_by_role_and_label(Role::Button, "Bao Nguyen")
                    .is_none(),
                "a failed unhide must not leave an active contact"
            );
            hub.state_mut().1.clear();
            hub.get_by_role_and_label(Role::Button, UNHIDE_LABEL)
                .click();
            hub.run_steps(2);
            assert!(
                hub.state()
                    .1
                    .iter()
                    .any(|action| matches!(dispatched_task(action),
                Some((BackendTask::DashPayTask(task), _))
                    if matches!(**task, DashPayTask::UpdateContactInfo { .. }))),
                "the restored hidden row permits a retry"
            );
        });
    }
}
