//! Usernames card: every name an identity owns or asked for, in one list.

use std::sync::Arc;

use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use egui::{ColorImage, RichText, TextureHandle, Ui};

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::identity::IdentityTask;
use crate::context::AppContext;
use crate::model::dpns::normalize_dpns_label;
use crate::model::dpns_usernames::{
    RequestPhase, UsernameRequest, can_register_usernames, dpns_signing_requirement,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::MessageType;
use crate::ui::ScreenType;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::identity::funding_common::generate_qr_code_image;
use crate::ui::identity::register_dpns_name_screen::{RegisterDpnsNameSource, status_line};
use crate::ui::identity::username_copy::{Tone, format_date, phase_label};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};

/// One row of the card, in display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsernameRow {
    /// A registered name.
    Active {
        name: String,
        /// Shown as the identity's main name.
        is_main: bool,
        /// Whether the `Main` badge appears (only with more than one name).
        show_main_badge: bool,
        acquired_at: TimestampMillis,
    },
    /// A request still in its community vote.
    Pending(UsernameRequest),
    /// A request that ended without the name, kept for 30 days.
    Outcome(UsernameRequest),
}

fn same_name(a: &str, b: &str) -> bool {
    normalize_dpns_label(a) == normalize_dpns_label(b)
}

/// Build the rows: main name, other names, pending requests, then finished outcomes.
pub fn username_rows(
    identity: &QualifiedIdentity,
    main: Option<&str>,
    requests: &[UsernameRequest],
) -> Vec<UsernameRow> {
    // Registered names, plus names won in a vote that the identity's stored
    // names do not list yet (they are re-read in the background).
    let mut names: Vec<(String, TimestampMillis)> = identity
        .dpns_names
        .iter()
        .filter(|n| !n.name.trim().is_empty())
        .map(|n| (n.name.clone(), n.acquired_at))
        .collect();
    for won in requests.iter().filter(|r| r.phase == RequestPhase::Won) {
        if !names.iter().any(|(name, _)| same_name(name, &won.label)) {
            names.push((won.label.clone(), won.decided_at.unwrap_or(0)));
        }
    }
    let show_main_badge = names.len() > 1;
    let mut rows: Vec<UsernameRow> = Vec::new();
    let main_index = main
        .and_then(|main| names.iter().position(|(name, _)| name == main))
        .unwrap_or(0);
    for (index, (name, acquired_at)) in names
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == main_index)
        .chain(names.iter().enumerate().filter(|(i, _)| *i != main_index))
    {
        rows.push(UsernameRow::Active {
            name: name.clone(),
            is_main: index == main_index,
            show_main_badge,
            acquired_at: *acquired_at,
        });
    }
    let owned = |label: &str| names.iter().any(|(name, _)| same_name(name, label));
    rows.extend(
        requests
            .iter()
            .filter(|r| r.phase.is_pending() && !owned(&r.label))
            .cloned()
            .map(UsernameRow::Pending),
    );
    rows.extend(
        requests
            .iter()
            .filter(|r| {
                matches!(
                    r.phase,
                    RequestPhase::Lost | RequestPhase::Locked | RequestPhase::NoWinner
                )
            })
            .cloned()
            .map(UsernameRow::Outcome),
    );
    rows
}

/// Stateful renderer for the Usernames card (holds the open QR code).
#[derive(Default)]
pub struct UsernamesCard {
    qr: Option<(String, TextureHandle)>,
}

impl UsernamesCard {
    /// Render the card for `identity`; returns any navigation the user asked for.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        app_context: &Arc<AppContext>,
        identity: &QualifiedIdentity,
    ) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        let identity_id = identity.identity.id();
        let main = app_context.main_username(identity);
        let requests = app_context.username_requests_for(&identity_id);
        let rows = username_rows(identity, main.as_deref(), &requests);
        let can_register = can_register_usernames(
            identity,
            dpns_signing_requirement(&app_context.dpns_contract),
        );

        if rows.is_empty() {
            ui.label(
                "This identity has no username yet. A username lets people pay you as @name instead of a long address.",
            );
            ui.add_space(6.0);
            action |=
                get_username_button(ui, app_context, identity, can_register, "Get a username");
            return action;
        }

        for row in rows {
            ui.add_space(4.0);
            match row {
                UsernameRow::Active {
                    name,
                    is_main,
                    show_main_badge,
                    acquired_at,
                } => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("@{name}")).strong());
                        if is_main && show_main_badge {
                            crate::ui::components::pill::accent_pill(
                                ui,
                                "Main",
                                DashColors::DASH_BLUE,
                                None,
                            );
                        }
                        ui.menu_button("•••", |ui| {
                            if ui.button("Copy username").clicked() {
                                ui.ctx().copy_text(format!("@{name}"));
                                ui.close();
                            }
                            if ui.button("Show QR code").clicked() {
                                self.open_qr(ui.ctx(), &name);
                                ui.close();
                            }
                            if !is_main && ui.button("Show as main").clicked() {
                                action = AppAction::BackendTask(BackendTask::IdentityTask(
                                    IdentityTask::SetMainUsername {
                                        identity_id,
                                        name: name.clone(),
                                    },
                                ));
                                ui.close();
                            }
                        })
                        .response
                        .clickable_tooltip(format!("More actions for @{name}"));
                    });
                    if acquired_at > 0 {
                        ui.label(
                            RichText::new(format!("Registered on {}.", format_date(acquired_at)))
                                .small()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                }
                UsernameRow::Pending(request) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("@{}", request.label)).strong());
                        status_line(ui, Tone::Caution, phase_label(request.phase), dark_mode);
                        if ui.button("View status").clicked() {
                            action = request_status_action(app_context, identity, &request);
                        }
                    });
                    let detail = match (request.phase, request.join_end, request.end) {
                        (RequestPhase::Joinable, Some(join_end), _) => format!(
                            "Others can ask for this name until {}. Masternodes can already vote.",
                            format_date(join_end)
                        ),
                        (RequestPhase::AwaitingOutcome, _, _) =>
                            "The estimated voting period has ended. View the status to check the outcome.".to_owned(),
                        (_, _, Some(end)) => {
                            format!("Voting ends around {}.", format_date(end))
                        }
                        _ => String::new(),
                    };
                    if !detail.is_empty() {
                        ui.label(
                            RichText::new(detail)
                                .small()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                }
                UsernameRow::Outcome(request) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("@{}", request.label)).strong());
                        status_line(ui, Tone::Negative, phase_label(request.phase), dark_mode);
                    });
                    let detail = match request.phase {
                        RequestPhase::Locked => {
                            "More votes went to locking this name, so no one can register it."
                                .to_owned()
                        }
                        RequestPhase::NoWinner => {
                            "The community vote ended without giving the name to anyone.".to_owned()
                        }
                        _ => request.decided_at.map_or_else(
                            || "The community vote ended.".to_owned(),
                            |at| format!("The community vote ended on {}.", format_date(at)),
                        ),
                    };
                    ui.label(
                        RichText::new(detail)
                            .small()
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.horizontal(|ui| {
                        if matches!(request.phase, RequestPhase::Lost | RequestPhase::NoWinner)
                            && ui.button("Choose another username").clicked()
                        {
                            action = register_action(app_context, identity);
                        }
                        if ui.button("Dismiss").clicked() {
                            action = AppAction::BackendTask(BackendTask::IdentityTask(
                                IdentityTask::DismissUsernameRequest {
                                    identity_id,
                                    normalized_label: request.normalized_label.clone(),
                                },
                            ));
                        }
                    });
                }
            }
        }

        ui.add_space(10.0);
        action |= get_username_button(
            ui,
            app_context,
            identity,
            can_register,
            "Get another username",
        );
        ui.label(
            RichText::new("Each username is a separate payment from this identity's balance.")
                .small()
                .color(DashColors::text_secondary(dark_mode)),
        );

        self.show_qr(ui);
        action
    }

    fn open_qr(&mut self, ctx: &egui::Context, name: &str) {
        let image: ColorImage = match generate_qr_code_image(&format!("{name}.dash")) {
            Ok(image) => image,
            Err(error) => {
                MessageBanner::set_global(
                    ctx,
                    "The QR code could not be created. Copy the username instead.",
                    MessageType::Error,
                )
                .with_details(error);
                return;
            }
        };
        let texture = ctx.load_texture("username_qr", image, egui::TextureOptions::NEAREST);
        self.qr = Some((name.to_owned(), texture));
    }

    fn show_qr(&mut self, ui: &mut Ui) {
        let Some((name, texture)) = &self.qr else {
            return;
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new(format!("@{name}"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.image((texture.id(), egui::vec2(220.0, 220.0)));
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        if !open || close {
            self.qr = None;
        }
    }
}

/// Open "Get a username" for `identity_id`, making it the selected identity.
pub(crate) fn register_action_for(
    app_context: &Arc<AppContext>,
    identity_id: dash_sdk::platform::Identifier,
) -> AppAction {
    app_context.set_selected_identity(Some(identity_id));
    AppAction::AddScreen(
        ScreenType::RegisterDpnsName(RegisterDpnsNameSource::Identities).create_screen(app_context),
    )
}

fn register_action(app_context: &Arc<AppContext>, identity: &QualifiedIdentity) -> AppAction {
    register_action_for(app_context, identity.identity.id())
}

fn request_status_action(
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    request: &UsernameRequest,
) -> AppAction {
    AppAction::AddScreen(
        ScreenType::UsernameRequestStatus {
            identity_id: identity.identity.id(),
            normalized_label: request.normalized_label.clone(),
        }
        .create_screen(app_context),
    )
}

fn get_username_button(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    can_register: bool,
    label: &str,
) -> AppAction {
    let mut action = AppAction::None;
    let reason = "Add a key to this identity to register usernames.";
    let clicked = ComponentStyles::add_primary_button_enabled(ui, can_register, label)
        .disabled_tooltip(reason)
        .clicked();
    if clicked && can_register {
        action = register_action(app_context, identity);
    }
    if !can_register {
        ui.horizontal(|ui| {
            ui.label(reason);
            if ui.link("Add a key").clicked() {
                action = AppAction::AddScreen(
                    ScreenType::Keys(identity.clone()).create_screen(app_context),
                );
            }
        });
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dpns::ContestDurations;
    use crate::model::qualified_identity::DPNSNameInfo;
    use std::time::Duration;

    fn identity(names: &[&str]) -> QualifiedIdentity {
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType};
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;
        let mut identity = QualifiedIdentity {
            identity: Identity::create_basic_identity([1; 32].into(), PlatformVersion::latest())
                .expect("identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: Vec::new(),
            associated_wallets: Default::default(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        };
        identity.dpns_names = names
            .iter()
            .map(|name| DPNSNameInfo {
                name: (*name).to_owned(),
                acquired_at: 1,
            })
            .collect();
        identity
    }

    fn request(label: &str, phase: RequestPhase) -> UsernameRequest {
        let mut request = UsernameRequest::submitted(
            label,
            0,
            ContestDurations {
                total: Duration::ZERO,
                join: Duration::ZERO,
            },
            None,
        );
        request.phase = phase;
        request
    }

    fn row_label(row: &UsernameRow) -> String {
        match row {
            UsernameRow::Active { name, is_main, .. } => {
                format!("{name}{}", if *is_main { "*" } else { "" })
            }
            UsernameRow::Pending(r) => format!("{}?", r.label),
            UsernameRow::Outcome(r) => format!("{}!", r.label),
        }
    }

    #[test]
    fn rows_list_every_state_in_order() {
        // USR-TC-020
        let requests = [
            request("ali", RequestPhase::Voting),
            request("novak", RequestPhase::Joinable),
            request("al", RequestPhase::Lost),
            request("aa", RequestPhase::Locked),
        ];
        let rows = username_rows(&identity(&["alice", "alice-design"]), None, &requests);
        let labels: Vec<_> = rows.iter().map(row_label).collect();
        assert_eq!(
            labels,
            ["alice*", "alice-design", "ali?", "novak?", "al!", "aa!"]
        );
        assert!(matches!(
            rows[0],
            UsernameRow::Active {
                show_main_badge: true,
                ..
            }
        ));
    }

    #[test]
    fn show_as_main_reorders_and_single_name_has_no_badge() {
        // USR-TC-021
        let rows = username_rows(&identity(&["a", "b"]), Some("b"), &[]);
        assert_eq!(rows.iter().map(row_label).collect::<Vec<_>>(), ["b*", "a"]);
        let rows = username_rows(&identity(&["a"]), None, &[]);
        assert!(matches!(
            rows[0],
            UsernameRow::Active {
                show_main_badge: false,
                ..
            }
        ));
    }

    #[test]
    fn owning_a_name_keeps_other_pending_requests() {
        // USR-FR-001: an identity owning @a still lists its pending request @b.
        let rows = username_rows(
            &identity(&["a"]),
            None,
            &[
                request("b", RequestPhase::Voting),
                request("c", RequestPhase::Joinable),
            ],
        );
        assert_eq!(rows.len(), 3);
    }
}
