//! Usernames card: every name an identity owns or asked for, in one list.

use std::sync::Arc;

use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use egui::{Align, Layout, RichText, Sides, Ui};

use crate::app::AppAction;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskContext};
use crate::context::AppContext;
use crate::model::dpns::normalize_dpns_label;
use crate::model::dpns_usernames::{
    RegistrationKeyStatus, RequestPhase, UsernameRequest, dpns_signing_requirement,
    plan_restores_registration_key, registration_key_status,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::legacy_recovery_section::host_offer;
use crate::ui::components::pill;
use crate::ui::components::qr_code_image::QrCodeImage;
use crate::ui::identity::register_dpns_name_screen::{RegisterDpnsNameSource, status_line};
use crate::ui::identity::username_copy::{
    ENTER_KEY_TO_REGISTER, IMPORT_OR_RESTORE_WALLET_TO_REGISTER, IMPORT_WALLET_TO_REGISTER,
    NEEDS_KEY_TO_REGISTER, REENTER_KEY_TO_REGISTER, RESTORE_FROM_PREVIOUS_VERSION_TO_REGISTER,
    RESTORE_KEYS_TO_REGISTER, Tone, finished_request_detail, pending_request_detail, phase_label,
    registered_on_line,
};
use crate::ui::masternodes::KeyVocabulary;
use crate::ui::state::legacy_recovery::LegacyRecoveryState;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Spacing};
use crate::ui::{RootScreenType, ScreenType};

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

/// A username in bold, shortened with "…" to the room left on its line less
/// `reserve` (points kept for what follows it). Hovering shows the full name.
fn name_label(ui: &mut Ui, text: &str, reserve: f32) {
    let width = (ui.available_width() - reserve).max(0.0);
    ui.scope(|ui| {
        ui.set_max_width(width);
        ui.add(egui::Label::new(RichText::new(text).strong()).truncate());
    });
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
    // names have not listed yet (they are re-read in the background). A win the
    // stored names listed before and no longer do has left the identity.
    let mut names: Vec<(String, TimestampMillis)> = identity
        .dpns_names
        .iter()
        .filter(|n| !n.name.trim().is_empty())
        .map(|n| (n.name.clone(), n.acquired_at))
        .collect();
    for won in requests
        .iter()
        .filter(|r| r.phase == RequestPhase::Won && !r.reflected_in_owned_names)
    {
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

/// What to tell an identity that cannot register a username from this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyHint {
    /// The identity has no suitable key: add one.
    AddKey,
    /// The previous version's saved data holds the key: restore it in place.
    Restore,
    /// The install was updated but nothing is offered in place: restore from Settings.
    PreviousVersion,
    /// An earlier version saved nothing: enter the key.
    EnterKey,
    /// An early version saved the key in a form this version cannot open:
    /// enter it again.
    ReenterKey,
    /// The key comes from a wallet that is not on this device: import it.
    ImportWallet,
    /// The same on an updated install: import the wallet or restore from Settings.
    ImportWalletOrPreviousVersion,
}

impl KeyHint {
    /// The hint for `status`, or `None` when the identity can register.
    /// `restorable_here` is whether the restore offer holds the missing key,
    /// `updated_install` whether an earlier version saved identities at all.
    pub(crate) fn for_status(
        status: RegistrationKeyStatus,
        restorable_here: bool,
        updated_install: bool,
    ) -> Option<Self> {
        match status {
            RegistrationKeyStatus::Ready => None,
            RegistrationKeyStatus::NoSuitableKey => Some(Self::AddKey),
            RegistrationKeyStatus::KeySavedByOlderVersion => Some(Self::ReenterKey),
            RegistrationKeyStatus::KeyNotOnDevice | RegistrationKeyStatus::KeyWalletNotOnDevice
                if restorable_here =>
            {
                Some(Self::Restore)
            }
            RegistrationKeyStatus::KeyNotOnDevice if updated_install => Some(Self::PreviousVersion),
            RegistrationKeyStatus::KeyNotOnDevice => Some(Self::EnterKey),
            RegistrationKeyStatus::KeyWalletNotOnDevice if updated_install => {
                Some(Self::ImportWalletOrPreviousVersion)
            }
            RegistrationKeyStatus::KeyWalletNotOnDevice => Some(Self::ImportWallet),
        }
    }

    /// The sentence shown for this hint.
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::AddKey => NEEDS_KEY_TO_REGISTER,
            Self::Restore => RESTORE_KEYS_TO_REGISTER,
            Self::PreviousVersion => RESTORE_FROM_PREVIOUS_VERSION_TO_REGISTER,
            Self::EnterKey => ENTER_KEY_TO_REGISTER,
            Self::ReenterKey => REENTER_KEY_TO_REGISTER,
            Self::ImportWallet => IMPORT_WALLET_TO_REGISTER,
            Self::ImportWalletOrPreviousVersion => IMPORT_OR_RESTORE_WALLET_TO_REGISTER,
        }
    }
}

/// The links that act on `hint`. [`KeyHint::Restore`] has none: its action is
/// the restore offer the card renders.
pub(crate) fn key_hint_links(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    hint: KeyHint,
) -> AppAction {
    let mut action = AppAction::None;
    if matches!(
        hint,
        KeyHint::PreviousVersion | KeyHint::ImportWalletOrPreviousVersion
    ) && ui.link("Open Settings").clicked()
    {
        action =
            AppAction::SetMainScreenThenGoToMainScreen(RootScreenType::RootScreenNetworkChooser);
    }
    if matches!(
        hint,
        KeyHint::ImportWallet | KeyHint::ImportWalletOrPreviousVersion
    ) && ui.link("Open wallets").clicked()
    {
        action =
            AppAction::SetMainScreenThenGoToMainScreen(RootScreenType::RootScreenWalletsBalances);
    }
    // No keys link for a key that comes from a missing wallet: the keys screen
    // has nowhere to enter a key this identity already has an entry for.
    let keys_link = match hint {
        KeyHint::AddKey => Some("Add a key"),
        KeyHint::PreviousVersion | KeyHint::EnterKey | KeyHint::ReenterKey => Some("Open keys"),
        KeyHint::Restore | KeyHint::ImportWallet | KeyHint::ImportWalletOrPreviousVersion => None,
    };
    if keys_link.is_some_and(|label| ui.link(label).clicked()) {
        action =
            AppAction::AddScreen(ScreenType::Keys(identity.clone()).create_screen(app_context));
    }
    action
}

/// Stateful renderer for the Usernames card (holds the open QR code).
#[derive(Default)]
pub struct UsernamesCard {
    qr: Option<(String, QrCodeImage)>,
    /// The restore offer for the identity whose registration key is not on
    /// this device; dropped once that identity can register.
    recovery: Option<(dash_sdk::platform::Identifier, LegacyRecoveryState)>,
}

/// Which register button the card offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GetUsername {
    /// The identity has no username yet.
    First,
    /// The identity already has at least one username.
    Another,
}

impl GetUsername {
    fn label(self) -> &'static str {
        match self {
            Self::First => "Get a username",
            Self::Another => "Get another username",
        }
    }
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
        let hint = self.key_hint(app_context, identity);

        if rows.is_empty() {
            ui.label(
                "This identity has no username yet. A username lets people pay you as @name instead of a long address.",
            );
            ui.add_space(6.0);
            action |= self.get_username_button(ui, app_context, identity, hint, GetUsername::First);
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
                    let badge = is_main && show_main_badge;
                    // The row's actions sit at the card's right edge, as in
                    // the A08 wireframe; they are laid out first so a long name
                    // is shortened to the room they and the badge leave.
                    Sides::new().shrink_left().truncate().show(
                        ui,
                        |ui| {
                            let badge_room = if badge {
                                pill::accent_pill_width(ui, "Main") + ui.spacing().item_spacing.x
                            } else {
                                0.0
                            };
                            name_label(ui, &format!("@{name}"), badge_room);
                            if badge {
                                pill::accent_pill(ui, "Main", DashColors::DASH_BLUE, None);
                            }
                        },
                        |ui| {
                            ui.menu_button("•••", |ui| {
                                if ui.button("Copy username").clicked() {
                                    ui.ctx().copy_text(format!("@{name}"));
                                    ui.close();
                                }
                                if ui.button("Show QR code").clicked() {
                                    self.open_qr(&name);
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
                        },
                    );
                    if acquired_at > 0 {
                        ui.label(
                            RichText::new(registered_on_line(acquired_at))
                                .small()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                }
                UsernameRow::Pending(request) => {
                    Sides::new().shrink_left().truncate().show(
                        ui,
                        |ui| name_label(ui, &format!("@{}", request.label), 0.0),
                        |ui| {
                            if ui.button("View status").clicked() {
                                action = request_status_action(app_context, identity, &request);
                            }
                            status_line(ui, Tone::Caution, phase_label(request.phase), dark_mode);
                        },
                    );
                    if let Some(detail) =
                        pending_request_detail(request.phase, request.join_end, request.end)
                    {
                        ui.label(
                            RichText::new(detail)
                                .small()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                }
                UsernameRow::Outcome(request) => {
                    Sides::new().shrink_left().truncate().show(
                        ui,
                        |ui| name_label(ui, &format!("@{}", request.label), 0.0),
                        |ui| status_line(ui, Tone::Negative, phase_label(request.phase), dark_mode),
                    );
                    ui.label(
                        RichText::new(finished_request_detail(request.phase, request.decided_at))
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

        ui.add_space(Spacing::SM);
        ui.separator();
        ui.add_space(Spacing::SM);
        action |= self.get_username_button(ui, app_context, identity, hint, GetUsername::Another);
        ui.label(
            RichText::new("Each username is a separate payment from this identity's balance.")
                .small()
                .color(DashColors::text_secondary(dark_mode)),
        );

        self.show_qr(ui);
        action
    }

    /// Decide what to tell `identity`, keeping the restore offer only while its
    /// registration key is not on this device or a restore of it still runs.
    fn key_hint(
        &mut self,
        app_context: &Arc<AppContext>,
        identity: &QualifiedIdentity,
    ) -> Option<KeyHint> {
        let required = dpns_signing_requirement(&app_context.dpns_contract());
        let status = registration_key_status(identity, required);
        let identity_id = identity.identity.id();
        let key_missing = status.key_is_unusable_here();
        let kept = self
            .recovery
            .as_ref()
            .is_some_and(|(id, offer)| *id == identity_id && (key_missing || offer.is_restoring()));
        if !kept {
            self.recovery = key_missing.then(|| {
                (
                    identity_id,
                    LegacyRecoveryState::new(app_context, identity_id),
                )
            });
        }
        let restorable_here = self.recovery().is_some_and(|offer| {
            offer
                .plan()
                .is_some_and(|plan| plan_restores_registration_key(identity, plan, required))
        });
        let updated_install = key_missing && app_context.has_legacy_identities();
        KeyHint::for_status(status, restorable_here, updated_install)
    }

    /// The restore offer this card hosts, for routing recovery results into it.
    pub(crate) fn recovery(&self) -> Option<&LegacyRecoveryState> {
        self.recovery.as_ref().map(|(_, offer)| offer)
    }

    /// Mutable access to the hosted restore offer.
    pub(crate) fn recovery_mut(&mut self) -> Option<&mut LegacyRecoveryState> {
        self.recovery.as_mut().map(|(_, offer)| offer)
    }

    /// The detection task the hosted offer still needs, marked as sent. The
    /// host dispatches it on a frame with no other action, so it never
    /// displaces something the user did.
    pub(crate) fn recovery_check(&mut self) -> Option<(BackendTask, BackendTaskContext)> {
        self.recovery_mut()?.ensure_checked()
    }

    /// The register button, and below it why it is disabled and what helps.
    fn get_username_button(
        &mut self,
        ui: &mut Ui,
        app_context: &Arc<AppContext>,
        identity: &QualifiedIdentity,
        hint: Option<KeyHint>,
        kind: GetUsername,
    ) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        // The first username is the card's call to action; another one is a
        // secondary, card-wide button below the list (A08).
        let button = match kind {
            GetUsername::First => {
                ComponentStyles::add_primary_button_enabled(ui, hint.is_none(), kind.label())
            }
            GetUsername::Another => {
                ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
                    if hint.is_none() {
                        ComponentStyles::add_secondary_button(ui, kind.label(), dark_mode)
                    } else {
                        ComponentStyles::add_primary_button_enabled(ui, false, kind.label())
                    }
                })
                .inner
            }
        };
        let Some(hint) = hint else {
            if button.clicked() {
                action = register_action(app_context, identity);
            }
            return action;
        };
        button.disabled_tooltip(hint.text());
        ui.horizontal_wrapped(|ui| {
            ui.label(hint.text());
            action = key_hint_links(ui, app_context, identity, hint);
        });
        if hint == KeyHint::Restore
            && let Some(offer) = self.recovery_mut()
            && let Some(approved) =
                host_offer(offer, KeyVocabulary::from(identity.identity_type), ui)
            && let Some((task, context)) = offer.restore(approved)
        {
            action = AppAction::BackendTaskWithContext { task, context };
        }
        action
    }

    fn open_qr(&mut self, name: &str) {
        let widget = QrCodeImage::new()
            .with_size(220.0)
            .with_texture_options(egui::TextureOptions::NEAREST)
            .with_fallback_text("The QR code could not be created. Copy the username instead.");
        self.qr = Some((name.to_owned(), widget));
    }

    fn show_qr(&mut self, ui: &mut Ui) {
        let Some((name, widget)) = &mut self.qr else {
            return;
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new(format!("@{name}"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                widget.show(ui, &format!("{name}.dash"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dpns::ContestDurations;
    use crate::model::qualified_identity::DPNSNameInfo;
    use std::time::Duration;

    fn identity(names: &[&str]) -> QualifiedIdentity {
        let mut identity = crate::context::test_support::bare_user_identity(
            [1; 32].into(),
            dash_sdk::dpp::dashcore::Network::Testnet,
        );
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

    /// A won name is listed while it waits to be read back into the identity's
    /// names. Once those names listed it and no longer do, it has left the
    /// identity and must not be offered as an owned, shareable username.
    #[test]
    fn win_that_left_the_identity_is_not_listed_as_owned() {
        let mut won = request("carol", RequestPhase::Won);
        let waiting = username_rows(&identity(&["alice"]), None, std::slice::from_ref(&won));
        assert_eq!(
            waiting.iter().map(row_label).collect::<Vec<_>>(),
            ["alice*", "carol"]
        );

        won.reflected_in_owned_names = true;
        let left = username_rows(&identity(&["alice"]), None, &[won]);
        assert_eq!(left.iter().map(row_label).collect::<Vec<_>>(), ["alice*"]);
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

    /// A testnet context whose previous-version database carries one identity
    /// row, the shape of an upgraded install.
    fn upgraded_context(dir: &std::path::Path) -> Arc<AppContext> {
        {
            let db = crate::database::Database::new(dir.join("data.db")).expect("legacy db");
            db.create_tables(true).expect("legacy schema");
            db.set_default_version().expect("legacy version");
            crate::database::test_helpers::LegacyIdentityFixture::new(
                [1; 32],
                Some(vec![0x01]),
                "testnet",
            )
            .insert(&db.locked_conn())
            .expect("stage a legacy identity row");
        }
        crate::context::test_support::test_app_context(dir)
    }

    /// An identity publishing a key that can register usernames, with or
    /// without that key held on this device.
    fn identity_publishing_a_signing_key(held: bool) -> QualifiedIdentity {
        crate::context::test_support::user_identity_with_signing_key(
            [1; 32].into(),
            dash_sdk::dpp::dashcore::Network::Testnet,
            held,
        )
    }

    /// Render the card for `identity`, collecting every action it returns.
    fn mount_card(
        app_context: Arc<AppContext>,
        identity: QualifiedIdentity,
    ) -> egui_kittest::Harness<'static, (UsernamesCard, Vec<AppAction>)> {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui_state(
                move |ui, state: &mut (UsernamesCard, Vec<AppAction>)| {
                    let action = state.0.show(ui, &app_context, &identity);
                    if !matches!(action, AppAction::None) {
                        state.1.push(action);
                    }
                },
                (UsernamesCard::default(), Vec::new()),
            );
        harness.run();
        harness
    }

    fn is_recovery_check(task: &(BackendTask, BackendTaskContext)) -> bool {
        matches!(
            task.0,
            BackendTask::IdentityTask(IdentityTask::CheckLegacyRecovery { .. })
        )
    }

    /// The previous version's data exists but has not offered this key (yet, or
    /// at all): the card points to the Settings restore, which also brings back
    /// a missing wallet, and has the existing detection to send exactly once.
    #[test]
    fn upgraded_install_without_an_offer_points_to_the_settings_restore() {
        use egui_kittest::kittest::Queryable;
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = upgraded_context(dir.path());
        let mut harness = mount_card(ctx, identity_publishing_a_signing_key(false));
        harness.run();

        assert!(
            harness
                .query_by_label(
                    "The key this identity needs to register usernames is not on this device. \
                     If you updated from an earlier version of Dash Evo Tool, open Settings and \
                     use Restore from Previous Version to bring back missing wallets and keys."
                )
                .is_some()
        );
        assert!(harness.query_by_label("Add a key").is_none());
        assert!(harness.query_by_label("Open keys").is_some());
        let check = harness.state_mut().0.recovery_check();
        assert!(check.as_ref().is_some_and(is_recovery_check));
        assert!(
            harness.state_mut().0.recovery_check().is_none(),
            "a check already sent must not be sent again",
        );

        harness.get_by_label("Open Settings").click();
        harness.run();
        assert!(harness.state().1.iter().any(|action| matches!(
            action,
            AppAction::SetMainScreenThenGoToMainScreen(
                crate::ui::RootScreenType::RootScreenNetworkChooser
            )
        )));
    }

    /// An identity that can already register asks nothing of the previous
    /// version's data, and neither does a fresh install.
    #[test]
    fn no_recovery_check_without_a_missing_key_or_previous_version_data() {
        let upgraded = tempfile::tempdir().expect("tempdir");
        let mut ready = mount_card(
            upgraded_context(upgraded.path()),
            identity_publishing_a_signing_key(true),
        );
        assert!(ready.state_mut().0.recovery_check().is_none());

        let fresh = tempfile::tempdir().expect("tempdir");
        let mut keyless = mount_card(
            crate::context::test_support::test_app_context(fresh.path()),
            identity_publishing_a_signing_key(false),
        );
        assert!(keyless.state_mut().0.recovery_check().is_none());
    }

    const IMPORT_WALLET: &str = "The key this identity needs to register usernames comes from a wallet that is not \
         on this device. Import that wallet again from its recovery phrase to register usernames.";

    /// An identity whose registration key is on file only as a reference into
    /// the wallet with `seed_hash`.
    fn identity_with_a_wallet_signing_key(
        seed_hash: crate::model::wallet::WalletSeedHash,
    ) -> QualifiedIdentity {
        let mut identity = identity_publishing_a_signing_key(false);
        crate::context::test_support::hold_as_wallet_reference(&mut identity, 1, seed_hash);
        identity
    }

    /// Press the register button and say whether it opened the registration
    /// screen. The disabled button is drawn in its own style and ignores
    /// clicks, so what it does is the only thing that tells the two apart.
    fn register_button_opens_registration<State>(
        harness: &mut egui_kittest::Harness<'static, (State, Vec<AppAction>)>,
    ) -> bool {
        use egui_kittest::kittest::Queryable;
        harness.get_by_label("Get a username").click();
        harness.run();
        harness
            .state()
            .1
            .iter()
            .any(|action| matches!(action, AppAction::AddScreen(_)))
    }

    /// The reported case: the key's wallet is not on this device, so signing
    /// would fail. The button is off and the card says to bring the wallet
    /// back — not to enter a key the keys screen has no field for.
    #[test]
    fn key_from_a_missing_wallet_says_to_import_the_wallet() {
        use egui_kittest::kittest::Queryable;
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let mut harness = mount_card(ctx, identity_with_a_wallet_signing_key([0x5A; 32]));

        assert!(!register_button_opens_registration(&mut harness));
        assert!(harness.query_by_label(IMPORT_WALLET).is_some());
        for absent in ["Open keys", "Add a key", "Open Settings"] {
            assert!(harness.query_by_label(absent).is_none(), "{absent}");
        }

        harness.get_by_label("Open wallets").click();
        harness.run();
        assert!(harness.state().1.iter().any(|action| matches!(
            action,
            AppAction::SetMainScreenThenGoToMainScreen(
                crate::ui::RootScreenType::RootScreenWalletsBalances
            )
        )));
    }

    /// On an updated install the Settings restore can bring the wallet back
    /// too, and the previous version's data is still asked for another key.
    #[test]
    fn upgraded_install_with_a_missing_wallet_also_points_to_the_settings_restore() {
        use egui_kittest::kittest::Queryable;
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = upgraded_context(dir.path());
        let mut harness = mount_card(ctx, identity_with_a_wallet_signing_key([0x5A; 32]));

        assert!(!register_button_opens_registration(&mut harness));
        assert!(
            harness
                .query_by_label(
                    "The key this identity needs to register usernames comes from a wallet that \
                     is not on this device. Import that wallet again from its recovery phrase. \
                     If you updated from an earlier version of Dash Evo Tool, you can also open \
                     Settings and use Restore from Previous Version to bring back missing wallets \
                     and keys."
                )
                .is_some()
        );
        assert!(harness.query_by_label("Open wallets").is_some());
        assert!(harness.query_by_label("Open Settings").is_some());
        assert!(harness.query_by_label("Open keys").is_none());
        let check = harness.state_mut().0.recovery_check();
        assert!(check.as_ref().is_some_and(is_recovery_check));
    }

    /// A wallet that is on this device keeps the identity ready even while
    /// locked: unlocking is something the app asks for when it signs.
    #[test]
    fn key_from_a_locked_wallet_on_this_device_stays_ready() {
        use egui_kittest::kittest::Queryable;
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let mut wallet = crate::model::wallet::test_support::open_wallet();
        wallet.wallet_seed.close();
        let seed_hash = wallet.seed_hash();
        let mut identity = identity_with_a_wallet_signing_key(seed_hash);
        identity
            .associated_wallets
            .insert(seed_hash, Arc::new(std::sync::RwLock::new(wallet)));
        let mut harness = mount_card(ctx, identity);

        assert!(harness.query_by_label("Open wallets").is_none());
        assert!(register_button_opens_registration(&mut harness));
    }

    /// The in-place restore outranks the wallet advice: a key the previous
    /// version can hand back makes the identity ready without the wallet.
    #[test]
    fn a_restorable_key_outranks_the_missing_wallet_advice() {
        let status = RegistrationKeyStatus::KeyWalletNotOnDevice;
        assert_eq!(
            KeyHint::for_status(status, true, true),
            Some(KeyHint::Restore)
        );
        assert_eq!(
            KeyHint::for_status(status, false, true),
            Some(KeyHint::ImportWalletOrPreviousVersion)
        );
        assert_eq!(
            KeyHint::for_status(status, false, false),
            Some(KeyHint::ImportWallet)
        );
    }

    /// A key an early version saved in a form this version cannot open is
    /// entered again; restoring from the previous version cannot open it
    /// either, so no restore is offered for it.
    #[test]
    fn a_key_saved_by_an_older_version_is_entered_again() {
        let status = RegistrationKeyStatus::KeySavedByOlderVersion;
        assert!(!status.key_is_unusable_here());
        for (restorable_here, updated_install) in
            [(true, true), (true, false), (false, true), (false, false)]
        {
            assert_eq!(
                KeyHint::for_status(status, restorable_here, updated_install),
                Some(KeyHint::ReenterKey)
            );
        }
    }

    /// The hub reads the identity again on every frame, so a wallet imported
    /// while the card is open enables the button without reopening anything.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn importing_the_wallet_enables_registration_on_the_open_card() {
        use crate::ui::identity::settings::SettingsTab;
        use egui_kittest::kittest::Queryable;
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let wallet = crate::model::wallet::test_support::open_wallet();
        let seed_hash = wallet.seed_hash();
        let identity = identity_with_a_wallet_signing_key(seed_hash);
        ctx.insert_local_qualified_identity(&identity, &None)
            .expect("seed identity");
        ctx.set_selected_identity(Some(identity.identity.id()));

        let render_ctx = Arc::clone(&ctx);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 1200.0))
            .build_ui_state(
                move |ui, state: &mut (SettingsTab, Vec<AppAction>)| {
                    let action = state.0.render(
                        ui,
                        &render_ctx,
                        render_ctx.resolve_selected_identity().as_ref(),
                        &mut Default::default(),
                    );
                    if !matches!(action, AppAction::None) {
                        state.1.push(action);
                    }
                },
                (SettingsTab::new(), Vec::new()),
            );
        harness.run();
        assert!(harness.query_by_label(IMPORT_WALLET).is_some());
        assert!(!register_button_opens_registration(&mut harness));

        ctx.wallet_context()
            .insert_test_wallet(seed_hash, Arc::new(std::sync::RwLock::new(wallet)));
        harness.run();
        assert!(harness.query_by_label(IMPORT_WALLET).is_none());
        assert!(register_button_opens_registration(&mut harness));
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
