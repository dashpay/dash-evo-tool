//! Get a username for one identity: choose a name, consent to a community vote
//! when needed, confirm the payment, and see the result.

use crate::app::AppAction;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTask, RegisterDpnsNameInput};
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::context::connection_status::OverallConnectionState;
use crate::model::dpns::{
    DpnsNameValidationResult, DpnsRegistrationOutcome, contest_durations, suggest_uncontested,
    validate_dpns_name,
};
use crate::model::dpns_usernames::{
    UsernameAvailability, can_register_usernames, dpns_signing_requirement,
};
use crate::model::fee_estimation::{contest_fee_credits, format_credits_as_dash};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::user_role::UserRole;
use crate::model::wallet::Wallet;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{
    MessageBanner, OptionOverlayExt, OverlayConfig, OverlayHandle, ResultBannerExt,
};
use crate::ui::helpers::{TransactionType, add_key_chooser_with_doc_type};
use crate::ui::identity::username_copy::{
    self, AvailabilityRow, Tone, USERNAME_RULES, availability_line, offers_suggestions,
};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::{Context, Frame, Margin};
use egui::{RichText, Ui};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use super::get_selected_wallet;

/// Wait after the last keystroke before asking the network about a name.
const AVAILABILITY_DEBOUNCE: Duration = Duration::from_millis(400);

/// Where the user navigated from to reach this screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegisterDpnsNameSource {
    /// The Identities hub, the only place that opens this screen.
    #[default]
    Identities,
}

/// Where the user is in the flow.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Choose,
    Confirm,
    Submitting,
    Done(DpnsRegistrationOutcome),
}

/// Live availability state of the typed label.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Availability {
    /// Nothing typed, or the format is invalid.
    Idle,
    /// Waiting for typing to settle.
    Debouncing { label: String, since: Instant },
    /// A check is in flight or answered.
    Row { label: String, row: AvailabilityRow },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistrationReview {
    label: String,
    identity_id: dash_sdk::platform::Identifier,
    network: dash_sdk::dpp::dashcore::Network,
    needs_vote: bool,
    registration_fee: u64,
    contest_fee: u64,
}

pub struct RegisterDpnsNameScreen {
    pub app_context: Arc<AppContext>,
    /// The identity the username is for, taken from the app's selected identity.
    pub selected_qualified_identity: Option<QualifiedIdentity>,
    name_input: String,
    availability: Availability,
    step: Step,
    consent_dialog: Option<ConfirmationDialog>,
    review: Option<RegistrationReview>,
    /// Signing key chosen under Advanced; `None` lets the backend pick the default.
    selected_key: Option<IdentityPublicKey>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
    completed_fee_result: Option<FeeResult>,
    /// Source of navigation to this screen
    pub source: RegisterDpnsNameSource,
    /// Full-window block while the registration runs; torn down on every terminal result.
    op_overlay: Option<OverlayHandle>,
    registration_context: Option<BackendTaskContext>,
    registration_error_handled: bool,
    /// Label of the availability check in flight. One check runs at a time, so a
    /// failure always belongs to this label.
    check_in_flight: Option<String>,
}

impl RegisterDpnsNameScreen {
    pub fn new(app_context: &Arc<AppContext>, source: RegisterDpnsNameSource) -> Self {
        let mut screen = Self {
            app_context: app_context.clone(),
            selected_qualified_identity: None,
            name_input: String::new(),
            availability: Availability::Idle,
            step: Step::Choose,
            consent_dialog: None,
            review: None,
            selected_key: None,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            completed_fee_result: None,
            source,
            op_overlay: None,
            registration_context: None,
            registration_error_handled: false,
            check_in_flight: None,
        };
        screen.reload_identity();
        screen
    }

    /// Get the username for `identity_id` instead of the app's selected identity.
    pub fn select_identity(&mut self, identity_id: dash_sdk::platform::Identifier) {
        self.load_identity(Some(identity_id));
    }

    /// Re-read the identity so balance and keys stay current (e.g. after a top-up).
    fn reload_identity(&mut self) {
        let wanted = self
            .selected_qualified_identity
            .as_ref()
            .map(|qi| qi.identity.id())
            .or_else(|| self.app_context.selected_identity_id());
        self.load_identity(wanted);
    }

    fn load_identity(&mut self, wanted: Option<dash_sdk::platform::Identifier>) {
        let identities = self
            .app_context
            .load_local_user_identities()
            .unwrap_or_default();
        let current = self
            .selected_qualified_identity
            .as_ref()
            .map(|qi| qi.identity.id());
        let identity = wanted
            .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
            .or_else(|| identities.first().cloned());
        if identity.as_ref().map(|qi| qi.identity.id()) != current {
            self.on_label_changed();
            self.selected_key = None;
            self.selected_wallet = identity.as_ref().and_then(|qi| {
                get_selected_wallet(qi, Some(&self.app_context), None)
                    .or_show_error(self.app_context.egui_ctx())
                    .unwrap_or(None)
            });
            self.wallet_open_attempted = false;
        }
        self.selected_qualified_identity = identity;
    }

    fn identity_label(&self) -> String {
        self.selected_qualified_identity
            .as_ref()
            .map(|qi| self.app_context.identity_display_label(qi))
            .unwrap_or_default()
    }

    fn label(&self) -> &str {
        self.name_input.trim()
    }

    /// The answered availability for the label currently typed, if any.
    fn current_availability(&self) -> Option<UsernameAvailability> {
        match &self.availability {
            Availability::Row {
                label,
                row: AvailabilityRow::Known(availability),
            } if label == self.label() => Some(*availability),
            _ => None,
        }
    }

    fn registration_fee(&self) -> u64 {
        self.app_context.fee_estimator().estimate_document_batch(2)
    }

    fn contest_fee(&self) -> u64 {
        contest_fee_credits(self.app_context.sdk_platform_version())
    }

    /// Restart the debounce after the typed label changed.
    fn on_label_changed(&mut self) {
        self.review = None;
        self.consent_dialog = None;
        self.step = Step::Choose;
        let label = self.label().to_owned();
        self.availability = if validate_dpns_name(&label) == DpnsNameValidationResult::Valid {
            Availability::Debouncing {
                label,
                since: Instant::now(),
            }
        } else {
            Availability::Idle
        };
    }

    /// Dispatch the availability check once typing has settled.
    fn poll_debounce(&mut self, ctx: &Context) -> AppAction {
        let Availability::Debouncing { label, since } = &self.availability else {
            return AppAction::None;
        };
        let remaining = AVAILABILITY_DEBOUNCE.saturating_sub(since.elapsed());
        if !remaining.is_zero() || self.check_in_flight.is_some() {
            ctx.request_repaint_after(remaining.max(AVAILABILITY_DEBOUNCE / 4));
            return AppAction::None;
        }
        let label = label.clone();
        self.check_availability(label)
    }

    fn check_availability(&mut self, label: String) -> AppAction {
        let Some(identity_id) = self
            .selected_qualified_identity
            .as_ref()
            .map(|identity| identity.identity.id())
        else {
            return AppAction::None;
        };
        self.availability = Availability::Row {
            label: label.clone(),
            row: AvailabilityRow::Checking,
        };
        self.check_in_flight = Some(label.clone());
        AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::CheckUsernameAvailability { identity_id, label },
        ))
    }

    fn payment_review(&self) -> Option<RegistrationReview> {
        let availability = self.current_availability()?;
        if !availability.allows_registration() {
            return None;
        }
        Some(RegistrationReview {
            label: self.label().to_owned(),
            identity_id: self.selected_qualified_identity.as_ref()?.identity.id(),
            network: self.app_context.network,
            needs_vote: availability.needs_vote(),
            registration_fee: self.registration_fee(),
            contest_fee: if availability.needs_vote() {
                self.contest_fee()
            } else {
                0
            },
        })
    }

    fn review_is_current(&self) -> bool {
        self.review.is_some() && self.review == self.payment_review()
    }

    fn continue_clicked(&mut self) {
        let Some(review) = self.payment_review() else {
            return;
        };
        let needs_vote = review.needs_vote;
        self.review = Some(review);
        if !needs_vote {
            self.step = Step::Confirm;
            return;
        }
        let durations = contest_durations(
            self.app_context.network,
            self.app_context.sdk_platform_version(),
        );
        let lines = username_copy::consent_lines(
            self.label(),
            durations.total,
            durations.join,
            self.contest_fee(),
        );
        self.consent_dialog = Some(
            ConfirmationDialog::new(
                username_copy::consent_title(self.label()),
                lines.join("\n\n"),
            )
            .confirm_text(Some("I understand, continue"))
            .cancel_text(Some("Choose another name")),
        );
    }

    /// Dispatch the registration and raise the blocking overlay.
    fn begin_registration(&mut self, ctx: &Context) -> AppAction {
        if self.step != Step::Confirm {
            return AppAction::None;
        }
        if !self.review_is_current() {
            self.on_label_changed();
            return AppAction::None;
        }
        let Some(review) = &self.review else {
            return AppAction::None;
        };
        let Some(identity) = self.selected_qualified_identity.as_ref() else {
            return AppAction::None;
        };
        let task = IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
            qualified_identity: identity.clone(),
            name_input: review.label.clone(),
            approved_contest_fee: review.contest_fee,
            signing_key_id: self.selected_key.as_ref().map(|key| key.id()),
        });
        self.step = Step::Submitting;
        self.raise_progress_overlay(ctx);
        let task = BackendTask::IdentityTask(task);
        let context = BackendTaskContext::for_dispatch_on(&task, self.app_context.network());
        self.registration_context = Some(context.clone());
        AppAction::BackendTaskWithContext { task, context }
    }

    fn raise_progress_overlay(&mut self, ctx: &Context) {
        let description = format!(
            "Registering @{name}. Keep Dash Evo Tool open until this finishes.",
            name = self.label()
        );
        self.op_overlay
            .raise(ctx, description, OverlayConfig::default());
    }

    /// Test seam: run the exact production overlay-raise the Pay button uses.
    #[doc(hidden)]
    pub fn raise_progress_overlay_for_test(&mut self, ctx: &Context) {
        self.raise_progress_overlay(ctx);
    }

    /// Test seam: press Pay, dispatching the registration and raising the overlay.
    #[doc(hidden)]
    pub fn begin_registration_for_test(&mut self, ctx: &Context) -> AppAction {
        self.begin_registration(ctx)
    }

    /// Test seam: type `label` as if the user had entered it.
    #[doc(hidden)]
    pub fn type_label_for_test(&mut self, label: &str) {
        self.name_input = label.to_owned();
        self.on_label_changed();
    }

    /// Test seam: the checked label as if the debounce had elapsed.
    #[doc(hidden)]
    pub fn flush_debounce_for_test(&mut self) -> AppAction {
        let label = self.label().to_owned();
        self.check_availability(label)
    }

    /// Test seam: move to the confirm step for the typed label.
    #[doc(hidden)]
    pub fn open_confirm_for_test(&mut self) {
        self.review = self.payment_review();
        self.step = Step::Confirm;
    }

    /// Test seam: whether the flow is on the confirm step.
    #[doc(hidden)]
    pub fn is_confirming_for_test(&self) -> bool {
        self.step == Step::Confirm
    }

    fn render_choose(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        let has_names = self
            .selected_qualified_identity
            .as_ref()
            .is_some_and(|qi| !qi.dpns_names.is_empty());
        ui.heading(if has_names {
            "Get another username"
        } else {
            "Get a username"
        });
        ui.label(
            RichText::new(format!(
                "For {identity}. People will use it to find and pay you. You can't change a username later.",
                identity = self.identity_label()
            ))
            .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(12.0);

        ui.label(RichText::new("Username").strong());
        let edit = ui
            .horizontal(|ui| {
                ui.label("@");
                let response = ui.text_edit_singleline(&mut self.name_input);
                ui.label(".dash");
                response
            })
            .inner;
        if edit.changed() {
            self.on_label_changed();
        }
        action |= self.poll_debounce(ui.ctx());

        let label = self.label().to_owned();
        if !label.is_empty() {
            ui.add_space(6.0);
            render_format_checks(ui, &label, dark_mode);
            if let Availability::Row {
                label: checked,
                row,
            } = &self.availability
                && checked == &label
            {
                let row = row.clone();
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let (tone, text) = availability_line(&label, &row);
                    status_line(ui, tone, &text, dark_mode);
                    if row == AvailabilityRow::CantCheck && ui.button("Try again").clicked() {
                        action |= self.check_availability(label.clone());
                    }
                });
                if offers_suggestions(&row) {
                    let suggestions = suggest_uncontested(&label);
                    if !suggestions.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Try one without a vote:");
                            for suggestion in suggestions {
                                if ui.button(format!("@{suggestion}")).clicked() {
                                    self.name_input = suggestion;
                                    self.on_label_changed();
                                }
                            }
                        });
                    }
                }
            }
        }

        ui.add_space(10.0);
        egui::CollapsingHeader::new("Username rules")
            .default_open(false)
            .show(ui, |ui| {
                for rule in USERNAME_RULES {
                    ui.label(format!("• {rule}"));
                }
            });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode).clicked() {
                action |= AppAction::PopScreen;
            }
            let can_continue = self
                .current_availability()
                .is_some_and(UsernameAvailability::allows_registration);
            if ComponentStyles::add_primary_button_enabled(ui, can_continue, "Continue")
                .disabled_tooltip("Choose an available username to continue.")
                .clicked()
                && can_continue
            {
                self.continue_clicked();
            }
        });
        action
    }

    fn render_confirm(&mut self, ui: &mut Ui) -> AppAction {
        if self.step == Step::Confirm && !self.review_is_current() {
            self.on_label_changed();
            return self.render_choose(ui);
        }
        let Some(review) = self.review.clone() else {
            return AppAction::None;
        };
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        let Some(identity) = self.selected_qualified_identity.clone() else {
            return action;
        };
        let name = review.label;
        let needs_vote = review.needs_vote;
        let registration_fee = review.registration_fee;
        let total = registration_fee.saturating_add(review.contest_fee);
        let balance = identity.identity.balance();

        ui.heading("Review and pay");
        ui.add_space(8.0);
        Frame::new()
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(6.0)
            .show(ui, |ui| {
                egui::Grid::new("username_confirm_rows")
                    .num_columns(2)
                    .spacing([24.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Username");
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("@{name}")).strong());
                            if needs_vote {
                                crate::ui::components::pill::accent_pill(
                                    ui,
                                    "Community vote",
                                    DashColors::WARNING_BRIGHT,
                                    None,
                                );
                            }
                        });
                        ui.end_row();
                        ui.label("Registration fee");
                        ui.label(format!(
                            "about {fee}",
                            fee = format_credits_as_dash(registration_fee)
                        ));
                        ui.end_row();
                        if needs_vote {
                            ui.label("Community vote fee (not returned)");
                            ui.label(format_credits_as_dash(review.contest_fee));
                            ui.end_row();
                        }
                        ui.label(RichText::new("Total").strong());
                        ui.label(RichText::new(format_credits_as_dash(total)).strong());
                        ui.end_row();
                    });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Paid from {identity}'s balance · Available {balance}",
                        identity = self.identity_label(),
                        balance = format_credits_as_dash(balance)
                    ))
                    .color(DashColors::text_secondary(dark_mode)),
                );
            });

        if self.app_context.user_role().at_least(UserRole::Power) {
            egui::CollapsingHeader::new("Advanced")
                .default_open(false)
                .show(ui, |ui| {
                    let document_type = self
                        .app_context
                        .dpns_contract
                        .document_type_cloned_for_name("domain")
                        .ok();
                    if self.selected_key.is_none() {
                        self.selected_key = document_type
                            .as_ref()
                            .and_then(|doc_type| identity.document_signing_key(&doc_type.as_ref()))
                            .cloned();
                    }
                    action |= add_key_chooser_with_doc_type(
                        ui,
                        &self.app_context,
                        &identity,
                        &mut self.selected_key,
                        TransactionType::DocumentAction,
                        document_type.as_ref(),
                    );
                });
        }

        if let Some(wallet) = &self.selected_wallet {
            if !self.wallet_open_attempted {
                if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                    MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                        .disable_auto_dismiss();
                }
                self.wallet_open_attempted = true;
            }
            if wallet_needs_unlock(wallet) {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    status_line(
                        ui,
                        Tone::Caution,
                        "Unlock the wallet that holds this identity's keys to continue.",
                        dark_mode,
                    );
                    if ui.button("Unlock wallet").clicked() {
                        self.wallet_unlock_popup.open();
                    }
                });
            }
        }

        let missing = total.saturating_sub(balance);
        if missing > 0 {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                status_line(
                    ui,
                    Tone::Caution,
                    &username_copy::low_balance_line(balance, missing),
                    dark_mode,
                );
                if ui.button("Top up").clicked() {
                    action |= AppAction::AddScreen(
                        ScreenType::TopUpIdentity(identity.clone())
                            .create_screen(&self.app_context),
                    );
                }
            });
        }

        let synced =
            self.app_context.connection_status().overall_state() == OverallConnectionState::Synced;
        let locked = self
            .selected_wallet
            .as_ref()
            .is_some_and(wallet_needs_unlock);
        let can_pay = synced && missing == 0 && !locked && self.step == Step::Confirm;
        let tooltip = if !synced {
            "Available after sync finishes."
        } else if locked {
            "Unlock the wallet to continue."
        } else {
            "Add funds to this identity to continue."
        };
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ComponentStyles::add_secondary_button(ui, "Back", dark_mode).clicked() {
                self.step = Step::Choose;
            }
            let pay = ComponentStyles::add_primary_button_enabled(
                ui,
                can_pay,
                username_copy::pay_label(&name, total, needs_vote),
            )
            .disabled_tooltip(tooltip);
            if pay.clicked() && can_pay {
                action |= self.begin_registration(ui.ctx());
            }
        });
        action
    }

    fn render_done(&mut self, ui: &mut Ui, outcome: DpnsRegistrationOutcome) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        let name = self.label().to_owned();
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            match outcome {
                DpnsRegistrationOutcome::Registered => {
                    ui.heading(username_copy::registered_heading(&name));
                    ui.label(username_copy::NAME_REGISTERED_LINE);
                }
                DpnsRegistrationOutcome::PendingCommunityVote => {
                    ui.heading(username_copy::request_sent_heading(&name));
                    let request = self.selected_qualified_identity.as_ref().and_then(|qi| {
                        self.app_context
                            .username_requests_for(&qi.identity.id())
                            .into_iter()
                            .find(|r| r.label == name)
                    });
                    if let Some(request) = request
                        && let (Some(join_end), Some(end)) = (request.join_end, request.end)
                    {
                        ui.label(username_copy::request_schedule_line(join_end, end));
                    }
                    ui.label(username_copy::request_uncontested_line(&name));
                    ui.label(
                        RichText::new(username_copy::REQUEST_RESULT_NOTE)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                }
            }
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if outcome == DpnsRegistrationOutcome::PendingCommunityVote
                    && let Some(identity) = &self.selected_qualified_identity
                    && ComponentStyles::add_secondary_button(ui, "View request status", dark_mode)
                        .clicked()
                {
                    let status = ScreenType::UsernameRequestStatus {
                        identity_id: identity.identity.id(),
                        normalized_label: crate::model::dpns::normalize_dpns_label(&name),
                    }
                    .create_screen(&self.app_context);
                    action = AppAction::PopThenAddScreenToMainScreen(
                        RootScreenType::RootScreenIdentityHub,
                        status,
                    );
                }
                if ComponentStyles::add_primary_button(ui, "Go to my identity").clicked() {
                    action = AppAction::SetMainScreenThenPopScreen(
                        RootScreenType::RootScreenIdentityHub,
                    );
                }
            });
        });
        action
    }
}

/// Per-keystroke format checks, each as text plus icon.
fn render_format_checks(ui: &mut Ui, label: &str, dark_mode: bool) {
    let validation = validate_dpns_name(label);
    let length_ok = !matches!(
        validation,
        DpnsNameValidationResult::TooShort | DpnsNameValidationResult::TooLong
    );
    let chars_ok = !matches!(validation, DpnsNameValidationResult::InvalidCharacter(_));
    let tone = |ok| if ok { Tone::Positive } else { Tone::Negative };
    status_line(
        ui,
        tone(length_ok),
        username_copy::LENGTH_CHECK_LABEL,
        dark_mode,
    );
    status_line(
        ui,
        tone(chars_ok),
        username_copy::CHARACTERS_CHECK_LABEL,
        dark_mode,
    );
    if matches!(
        validation,
        DpnsNameValidationResult::StartsWithHyphen | DpnsNameValidationResult::EndsWithHyphen
    ) && let Some(message) = validation.error_message()
    {
        status_line(ui, Tone::Negative, &message, dark_mode);
    }
}

/// One status line: icon plus text, colored by tone.
pub(crate) fn status_line(ui: &mut Ui, tone: Tone, text: &str, dark_mode: bool) {
    let color = match tone {
        Tone::Neutral => DashColors::text_secondary(dark_mode),
        Tone::Positive => DashColors::success_color(dark_mode),
        Tone::Caution => DashColors::warning_color(dark_mode),
        Tone::Negative => DashColors::error_color(dark_mode),
    };
    ui.label(RichText::new(format!("{} {text}", tone.icon())).color(color));
}

impl ScreenLike for RegisterDpnsNameScreen {
    fn display_backend_task_error(&mut self, context: &BackendTaskContext, error: &TaskError) {
        self.registration_error_handled = false;
        if self.registration_context.as_ref() != Some(context) {
            return;
        }
        self.registration_context = None;
        self.op_overlay.take_and_clear();
        if let TaskError::UsernameNoLongerAvailable { availability } = error {
            self.step = Step::Choose;
            self.availability = Availability::Row {
                label: self.label().to_owned(),
                row: AvailabilityRow::Known(*availability),
            };
            self.registration_error_handled = true;
        } else if matches!(
            error,
            // An unconfirmed request may be paid for: Pay must not come back
            // without a fresh availability check and a fresh review.
            TaskError::UsernameRegistrationTermsChanged
                | TaskError::UsernameRegistrationUnconfirmed { .. }
        ) {
            self.on_label_changed();
        } else {
            self.step = Step::Confirm;
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        if std::mem::take(&mut self.registration_error_handled) {
            return true;
        }
        if matches!(error, TaskError::UsernameAvailabilityCheckFailed { .. })
            && self.step != Step::Submitting
            && let Some(failed) = self.check_in_flight.take()
        {
            if let Availability::Row { label, row } = &mut self.availability
                && *row == AvailabilityRow::Checking
                && failed == *label
            {
                *row = AvailabilityRow::CantCheck;
            }
            return true;
        }
        false
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        result: BackendTaskSuccessResult,
    ) {
        if matches!(result, BackendTaskSuccessResult::RegisteredDpnsName { .. })
            && self.registration_context.as_ref() != Some(context)
        {
            return;
        }
        self.display_task_result(result);
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::RegisteredDpnsName {
                outcome,
                fee_result,
            } => {
                self.op_overlay.take_and_clear();
                self.registration_context = None;
                self.completed_fee_result = Some(fee_result);
                self.step = Step::Done(outcome);
            }
            BackendTaskSuccessResult::UsernameAvailability {
                label,
                availability,
            } => {
                if self.check_in_flight.as_ref() == Some(&label) {
                    self.check_in_flight = None;
                }
                if matches!(&self.availability, Availability::Row { label: current, .. } if *current == label)
                {
                    self.availability = Availability::Row {
                        label,
                        row: AvailabilityRow::Known(availability),
                    };
                }
            }
            _ => {}
        }
    }

    fn refresh(&mut self) {
        self.reload_identity();
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let identity_label = self.identity_label();
        let breadcrumbs = vec![
            ("Identities", AppAction::OpenIdentityPicker),
            (identity_label.as_str(), AppAction::PopScreen),
            ("Get a username", AppAction::None),
        ];
        let mut action = add_top_panel(ui, &self.app_context, breadcrumbs, vec![]);
        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenIdentityHub);

        action |= island_central_panel(ui, |ui| {
            let mut inner = AppAction::None;
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let dark_mode = ui.style().visuals.dark_mode;
                    if let Step::Done(outcome) = self.step {
                        inner = self.render_done(ui, outcome);
                        return;
                    }
                    let Some(identity) = &self.selected_qualified_identity else {
                        status_line(
                            ui,
                            Tone::Negative,
                            "Load or create an identity first, then come back to get a username.",
                            dark_mode,
                        );
                        return;
                    };
                    if !can_register_usernames(
                        identity,
                        dpns_signing_requirement(&self.app_context.dpns_contract),
                    ) {
                        status_line(
                            ui,
                            Tone::Negative,
                            username_copy::NEEDS_KEY_TO_REGISTER,
                            dark_mode,
                        );
                        return;
                    }
                    inner = match self.step.clone() {
                        Step::Choose => self.render_choose(ui),
                        Step::Confirm | Step::Submitting => self.render_confirm(ui),
                        Step::Done(_) => AppAction::None,
                    };
                });
            inner
        });

        if let Some(dialog) = self.consent_dialog.as_mut() {
            match dialog.show(ui).inner.dialog_response {
                Some(ConfirmationStatus::Confirmed) => {
                    self.consent_dialog = None;
                    if self.review_is_current() {
                        self.step = Step::Confirm;
                    } else {
                        self.on_label_changed();
                    }
                }
                Some(ConfirmationStatus::Canceled) => {
                    self.consent_dialog = None;
                    self.review = None;
                }
                None => {}
            }
        }

        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            self.wallet_unlock_popup
                .show(&ctx, wallet, &self.app_context);
        }

        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_regression_unrelated_error_keeps_paid_registration_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let mut screen = RegisterDpnsNameScreen::new(&ctx, RegisterDpnsNameSource::Identities);
        screen.step = Step::Submitting;
        screen.raise_progress_overlay(ctx.egui_ctx());
        screen.display_message("Background refresh failed.", MessageType::Error);
        assert_eq!(screen.step, Step::Submitting);
        assert!(screen.op_overlay.is_some());
    }

    #[test]
    fn only_the_matching_registration_dispatch_releases_payment() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let mut screen = RegisterDpnsNameScreen::new(&ctx, RegisterDpnsNameSource::Identities);
        let context = BackendTaskContext::for_dispatch(&BackendTask::ContestedResourceTask(
            crate::backend_task::contested_names::ContestedResourceTask::QueryDPNSContests,
        ));
        screen.registration_context = Some(context.clone());
        screen.step = Step::Submitting;
        screen.raise_progress_overlay(ctx.egui_ctx());
        let error = TaskError::UsernameNoLongerAvailable {
            availability: UsernameAvailability::Locked,
        };
        screen.display_backend_task_error(&BackendTaskContext::Unknown, &error);
        assert!(!screen.display_task_error(&error));
        assert_eq!(screen.step, Step::Submitting);
        assert!(screen.op_overlay.is_some());
        screen.display_backend_task_error(&context, &error);
        assert!(screen.display_task_error(&error));
        assert_eq!(screen.step, Step::Choose);
        assert!(screen.op_overlay.is_none());
    }

    /// USR-TC-035: on a protocol 14 network the confirm step charges the
    /// 0.1 DASH community vote fee and adds it to the registration fee.
    #[test]
    fn confirm_fees_follow_the_network_protocol_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let pv14 = dash_sdk::dpp::version::PlatformVersion::get(14).expect("PV14");
        ctx.sdk.store(Arc::new(
            dash_sdk::SdkBuilder::new_mock()
                .with_version(pv14)
                .build()
                .expect("mock sdk"),
        ));
        let screen = RegisterDpnsNameScreen::new(&ctx, RegisterDpnsNameSource::Identities);
        assert_eq!(screen.contest_fee(), 10_000_000_000);
        assert_eq!(format_credits_as_dash(screen.contest_fee()), "0.1 DASH");
    }
}
