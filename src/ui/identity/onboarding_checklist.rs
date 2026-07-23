//! Onboarding checklist — three-step guided setup strip shown on the
//! Identity Home tab until the user either completes all steps or dismisses
//! the checklist.
//!
//! See design-spec §B.2 (checklist zone #4). The three steps, in order:
//!
//! 1. `Pick a username`
//! 2. `Set a display name` — hidden by callers when the user has previously
//!    dismissed the social-profile card (treated as a deliberate skip; the
//!    caller is responsible for honoring that decision).
//! 3. `Add your first contact`
//!
//! Each step renders with either a filled check mark (complete) or an empty
//! circle (pending). A dismiss button (`×`) in the top-right corner reports
//! `dismissed == true` so the caller can persist the dismissal.
//!
//! Follows `docs/COMPONENT_DESIGN_PATTERN.md`: private fields + builder +
//! response struct implementing [`ComponentResponse`].

use crate::ui::components::component_trait::ComponentResponse;
use crate::ui::theme::{DashColors, ResponseExt, Shape, Spacing, Typography};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui};

/// The three canonical onboarding steps. `Hidden` is applied by the caller
/// (see docs/ai-design/2026-04-22-identity-dashpay-redesign/design-spec.md
/// §B.2) to honor a user's skip of the social profile card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChecklistStep {
    PickUsername,
    SetDisplayName,
    AddFirstContact,
}

impl ChecklistStep {
    /// All steps in rendering order.
    pub const ALL: [ChecklistStep; 3] = [
        ChecklistStep::PickUsername,
        ChecklistStep::SetDisplayName,
        ChecklistStep::AddFirstContact,
    ];

    /// Alex-facing label. Exact wording from design-spec §B.2.
    pub fn label(self) -> &'static str {
        match self {
            ChecklistStep::PickUsername => "Pick a username",
            ChecklistStep::SetDisplayName => "Set a display name",
            ChecklistStep::AddFirstContact => "Add your first contact",
        }
    }

    /// Short, complete-sentence description rendered in a tooltip.
    pub fn tooltip(self) -> &'static str {
        match self {
            ChecklistStep::PickUsername => "Pick a Dash username so people can pay you by name.",
            ChecklistStep::SetDisplayName => {
                "Add a display name so contacts can recognise you on DashPay."
            }
            ChecklistStep::AddFirstContact => {
                "Find someone by username and add them to your contacts."
            }
        }
    }

    /// Descriptive sub-line shown below the label when the step is pending.
    /// Wireframe §B.2 (V3).
    pub fn subtext_pending(self) -> &'static str {
        match self {
            ChecklistStep::PickUsername => "Pick a name so people can pay you by name.",
            ChecklistStep::SetDisplayName => "This is how you appear to contacts.",
            ChecklistStep::AddFirstContact => "Add someone by username to send with one click.",
        }
    }

    /// Descriptive sub-line shown below the label when the step is complete.
    /// Returns `None` for steps where a generic "done" message is sufficient
    /// (the caller may supply a richer string, e.g. "You are @{handle}." for
    /// PickUsername via [`OnboardingChecklist::with_handle`]).
    pub fn subtext_done(self) -> Option<&'static str> {
        match self {
            ChecklistStep::PickUsername => None, // caller injects "@handle" via with_handle()
            ChecklistStep::SetDisplayName => Some("Your display name is set."),
            ChecklistStep::AddFirstContact => Some("You have contacts."),
        }
    }

    /// Label for the inline action button shown next to pending steps.
    /// Wireframe §B.2 (V3).
    pub fn action_label(self) -> &'static str {
        match self {
            ChecklistStep::PickUsername => "Pick a username",
            ChecklistStep::SetDisplayName => "Set display name",
            ChecklistStep::AddFirstContact => "Add a contact",
        }
    }
}

/// Action that the checklist emits in a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChecklistAction {
    /// User clicked the row for this step to act on it.
    Activated(ChecklistStep),
    /// User clicked the dismiss (`×`) button.
    Dismissed,
}

/// Response returned by [`OnboardingChecklist::show`].
///
/// `has_changed` is `true` when the user either activates a step or
/// dismisses the checklist — callers should react by routing the user to
/// the right screen and / or persisting the dismissal.
#[derive(Clone, Debug, Default)]
pub struct ChecklistResponse {
    action: Option<ChecklistAction>,
    has_changed: bool,
    changed_value: Option<ChecklistAction>,
}

impl ChecklistResponse {
    fn new(action: Option<ChecklistAction>) -> Self {
        Self {
            action,
            has_changed: action.is_some(),
            changed_value: action,
        }
    }

    /// The action, if any, produced this frame.
    pub fn action(&self) -> Option<ChecklistAction> {
        self.action
    }

    /// True if the user clicked the dismiss button.
    pub fn dismissed(&self) -> bool {
        matches!(self.action, Some(ChecklistAction::Dismissed))
    }

    /// Step the user activated this frame, if any.
    pub fn activated_step(&self) -> Option<ChecklistStep> {
        match self.action {
            Some(ChecklistAction::Activated(step)) => Some(step),
            _ => None,
        }
    }
}

impl ComponentResponse for ChecklistResponse {
    type DomainType = ChecklistAction;

    fn has_changed(&self) -> bool {
        self.has_changed
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.changed_value
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// Onboarding checklist widget.
#[derive(Clone, Debug)]
pub struct OnboardingChecklist {
    steps: Vec<ChecklistStep>,
    completed: Vec<ChecklistStep>,
    /// Primary DPNS handle (without the leading `@`). When set, the done-state
    /// subtext for `PickUsername` reads "You are @{handle}." instead of a
    /// generic fallback. Set via [`with_handle`](Self::with_handle).
    handle: Option<String>,
    /// A DPNS username the identity has requested but not yet been awarded
    /// (without the `.dash` suffix). When set, `PickUsername` is complete and
    /// shows voting-status subtext instead of the generic completion copy. Set via
    /// [`with_pending_username`](Self::with_pending_username).
    pending_username: Option<String>,
}

impl OnboardingChecklist {
    /// Construct a checklist with all three steps visible by default. Hide a
    /// step by calling [`hide`](Self::hide) (used by callers to honor a
    /// previous "skip the social profile" decision).
    pub fn new() -> Self {
        Self {
            steps: ChecklistStep::ALL.to_vec(),
            completed: Vec::new(),
            handle: None,
            pending_username: None,
        }
    }

    /// Attach the identity's primary DPNS handle (without the leading `@`).
    /// When set, the `PickUsername` done-state subtext reads
    /// `"You are @{handle}."` instead of a generic message.
    pub fn with_handle(mut self, handle: impl Into<String>) -> Self {
        let h = handle.into();
        if !h.trim().is_empty() {
            self.handle = Some(h);
        }
        self
    }

    /// Attach a DPNS username the identity has requested but not yet been
    /// awarded (without the `.dash` suffix). This completes `PickUsername`,
    /// shows voting-status subtext, and hides the action button.
    pub fn with_pending_username(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if !name.trim().is_empty() {
            self.pending_username = Some(name);
            self = self.mark_complete(ChecklistStep::PickUsername);
        }
        self
    }

    /// Whether `PickUsername` has a requested-but-unawarded username attached.
    fn pick_username_has_pending_request(&self, step: ChecklistStep) -> bool {
        step == ChecklistStep::PickUsername && self.pending_username.is_some()
    }

    /// Mark a step as complete. No-op if the step was already complete.
    pub fn mark_complete(mut self, step: ChecklistStep) -> Self {
        if !self.completed.contains(&step) {
            self.completed.push(step);
        }
        self
    }

    /// Hide a step so it never renders. Useful when the user has explicitly
    /// skipped an optional section (e.g. the social profile card).
    pub fn hide(mut self, step: ChecklistStep) -> Self {
        self.steps.retain(|s| *s != step);
        // Also remove from completed so the "all done" check stays honest.
        self.completed.retain(|s| *s != step);
        self
    }

    /// Returns `true` when every visible step is marked complete. Callers
    /// may stop rendering the checklist at that point.
    pub fn all_complete(&self) -> bool {
        !self.steps.is_empty() && self.steps.iter().all(|s| self.completed.contains(s))
    }

    /// Accessor for the currently visible steps in rendering order.
    pub fn visible_steps(&self) -> &[ChecklistStep] {
        &self.steps
    }

    /// Accessor reporting whether a specific step is complete.
    pub fn is_complete(&self, step: ChecklistStep) -> bool {
        self.completed.contains(&step)
    }

    /// Render the checklist. Returns a response describing any click.
    ///
    /// The checklist renders as a single card with a row per visible step and
    /// a small dismiss button in the top-right corner. Completed rows show a
    /// filled Dash-blue circle with a white check mark; pending rows show an
    /// empty outlined circle.
    pub fn show(&self, ui: &mut Ui) -> ChecklistResponse {
        let dark_mode = ui.ctx().global_style().visuals.dark_mode;

        let frame = Frame::new()
            .fill(DashColors::surface(dark_mode))
            .stroke(Stroke::new(
                Shape::BORDER_WIDTH,
                DashColors::border_light(dark_mode),
            ))
            .corner_radius(CornerRadius::same(Shape::RADIUS_MD))
            .inner_margin(Margin::same(Spacing::MD as i8));

        let mut action: Option<ChecklistAction> = None;

        frame.show(ui, |ui| {
            // Header row: heading + "Hide this for now" ghost button (V3).
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Finish setting up your identity")
                        .size(16.0)
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Wireframe §B.2: "Hide this for now" labeled button so the
                    // dismiss affordance is clearly worded, not an ambiguous `×`.
                    let dismiss_resp = ui
                        .add(
                            egui::Label::new(
                                RichText::new("Hide this for now")
                                    .color(DashColors::text_secondary(dark_mode)),
                            )
                            .sense(Sense::click()),
                        )
                        .clickable_tooltip(
                            "Hide the setup checklist. You can find these actions on Settings \
                             and Contacts anytime.",
                        );
                    if dismiss_resp.clicked() {
                        action = Some(ChecklistAction::Dismissed);
                    }
                });
            });
            ui.add_space(Spacing::SM);

            for step in &self.steps {
                let complete = self.completed.contains(step);
                let handle_ref = self.handle.as_deref();
                if self.paint_step_row(ui, dark_mode, *step, complete, handle_ref) {
                    action = Some(ChecklistAction::Activated(*step));
                }
                ui.add_space(Spacing::XS);
            }
        });

        ChecklistResponse::new(action)
    }

    /// Paint a single step row. Returns `true` when the user clicked anywhere
    /// in the row (the bullet, label, subtext, or inline action button).
    ///
    /// Each pending row shows: bullet circle + label + subtext + action button.
    /// Each complete row shows: filled check circle + struck-through label +
    /// completion subtext. The entire row region (including the circle and the
    /// whitespace) is a single click surface (T10 / V3).
    fn paint_step_row(
        &self,
        ui: &mut Ui,
        dark_mode: bool,
        step: ChecklistStep,
        complete: bool,
        handle: Option<&str>,
    ) -> bool {
        // Wrap the whole row in a clickable scope so the bullet circle and
        // surrounding whitespace are part of the hit area, not just the label
        // text (T10). We still place the inline action button separately so it
        // receives its own visual hover feedback.
        let row_scope = ui.scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
            ui.horizontal(|ui| {
                // Circle bullet.
                let size = 20.0;
                let (rect, _resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
                let painter = ui.painter();
                let center = rect.center();
                let radius = size * 0.5;

                if complete {
                    painter.circle_filled(center, radius, DashColors::DASH_BLUE);
                    let check_color = Color32::WHITE;
                    let p1 = egui::pos2(center.x - 4.0, center.y);
                    let p2 = egui::pos2(center.x - 1.0, center.y + 3.0);
                    let p3 = egui::pos2(center.x + 4.0, center.y - 3.0);
                    painter.line_segment([p1, p2], Stroke::new(1.8, check_color));
                    painter.line_segment([p2, p3], Stroke::new(1.8, check_color));
                } else {
                    painter.circle_stroke(
                        center,
                        radius - 1.0,
                        Stroke::new(1.5, DashColors::border(dark_mode)),
                    );
                }

                ui.add_space(Spacing::SM);

                // Content column: label + subtext [+ action button].
                ui.vertical(|ui| {
                    // Label — struck through when complete.
                    let text_color = if complete {
                        DashColors::text_secondary(dark_mode)
                    } else {
                        DashColors::text_primary(dark_mode)
                    };
                    let mut rich = RichText::new(step.label()).strong().color(text_color);
                    if complete {
                        rich = rich.strikethrough();
                    }
                    ui.label(rich);

                    // Descriptive subtext (V3).
                    let subtext: String = if self.pick_username_has_pending_request(step) {
                        let name =
                            crate::model::contested_name::sanitize_pending_username_for_display(
                                self.pending_username.as_deref().unwrap_or_default(),
                            );
                        format!(
                            "Your request for {name}.dash is pending while Dash masternodes vote."
                        )
                    } else if complete {
                        // For PickUsername done, prefer "You are @{handle}."
                        if step == ChecklistStep::PickUsername {
                            match handle {
                                Some(h) => format!("You are @{h}."),
                                None => "Your username is set.".to_string(),
                            }
                        } else {
                            step.subtext_done().unwrap_or("Done.").to_string()
                        }
                    } else {
                        step.subtext_pending().to_string()
                    };
                    ui.label(
                        RichText::new(subtext)
                            .font(Typography::hint())
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                });
            });
        });

        // The inline action button for pending items is placed outside the
        // scope so it gets its own visual affordance, but its click still
        // counts as a row activation. A username whose request is already in
        // flight hides its button — re-picking is not the next action.
        let mut action_clicked = false;
        if !complete && !self.pick_username_has_pending_request(step) {
            ui.horizontal(|ui| {
                ui.add_space(20.0 + Spacing::SM); // align with content column
                let btn_resp = ui
                    .add(
                        egui::Label::new(
                            RichText::new(step.action_label())
                                .small()
                                .color(DashColors::DASH_BLUE)
                                .underline(),
                        )
                        .sense(Sense::click()),
                    )
                    .clickable_tooltip(step.tooltip());
                if btn_resp.clicked() {
                    action_clicked = true;
                }
            });
        }

        row_scope.response.clicked() || action_clicked
    }
}

impl Default for OnboardingChecklist {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // UT-CHECKLIST-01 — Onboarding checklist completion.
    //
    // Preconditions: checklist with three steps, `Pick a username` marked
    // complete. Expected: first step rendered with check mark; remaining two
    // with empty circle.
    #[test]
    fn checklist_marks_pick_username_complete_only() {
        let checklist = OnboardingChecklist::new().mark_complete(ChecklistStep::PickUsername);
        assert!(checklist.is_complete(ChecklistStep::PickUsername));
        assert!(!checklist.is_complete(ChecklistStep::SetDisplayName));
        assert!(!checklist.is_complete(ChecklistStep::AddFirstContact));
        assert!(!checklist.all_complete());
        assert_eq!(checklist.visible_steps(), ChecklistStep::ALL.as_slice());
    }

    // UT-CHECKLIST-02 — Dismiss persists.
    //
    // Preconditions: checklist rendered; user clicks the dismiss button.
    // Expected: response reports dismissed == true; caller must persist via
    // settings.
    #[test]
    fn dismiss_response_reports_dismissed_true() {
        let resp = ChecklistResponse::new(Some(ChecklistAction::Dismissed));
        assert!(resp.dismissed());
        assert!(resp.has_changed());
        assert!(resp.is_valid());
        assert_eq!(resp.changed_value(), &Some(ChecklistAction::Dismissed));
        // The response API lets callers persist the dismissal — the widget
        // itself does not mutate disk. This is the contract in UT-CHECKLIST-02.
        assert_eq!(resp.activated_step(), None);
    }

    #[test]
    fn empty_default_response_has_no_action() {
        let resp = ChecklistResponse::default();
        assert!(!resp.has_changed());
        assert!(resp.action().is_none());
        assert!(!resp.dismissed());
        assert_eq!(resp.activated_step(), None);
    }

    #[test]
    fn activated_step_response_has_step_value() {
        let resp = ChecklistResponse::new(Some(ChecklistAction::Activated(
            ChecklistStep::PickUsername,
        )));
        assert_eq!(resp.activated_step(), Some(ChecklistStep::PickUsername));
        assert!(!resp.dismissed());
    }

    #[test]
    fn hide_removes_step_from_visible_and_completed() {
        let checklist = OnboardingChecklist::new()
            .mark_complete(ChecklistStep::SetDisplayName)
            .hide(ChecklistStep::SetDisplayName);
        assert!(
            !checklist
                .visible_steps()
                .contains(&ChecklistStep::SetDisplayName)
        );
        assert!(!checklist.is_complete(ChecklistStep::SetDisplayName));
    }

    #[test]
    fn mark_complete_is_idempotent() {
        let checklist = OnboardingChecklist::new()
            .mark_complete(ChecklistStep::PickUsername)
            .mark_complete(ChecklistStep::PickUsername);
        assert!(checklist.is_complete(ChecklistStep::PickUsername));
    }

    #[test]
    fn all_complete_true_only_when_every_visible_step_done() {
        let checklist = OnboardingChecklist::new()
            .mark_complete(ChecklistStep::PickUsername)
            .mark_complete(ChecklistStep::SetDisplayName)
            .mark_complete(ChecklistStep::AddFirstContact);
        assert!(checklist.all_complete());
    }

    #[test]
    fn all_complete_false_on_empty_checklist() {
        // A checklist with every step hidden is not "all complete" — there
        // is nothing to complete. This guards against a caller inadvertently
        // hiding the last step and then thinking the user finished setup.
        let checklist = OnboardingChecklist::new()
            .hide(ChecklistStep::PickUsername)
            .hide(ChecklistStep::SetDisplayName)
            .hide(ChecklistStep::AddFirstContact);
        assert!(!checklist.all_complete());
        assert!(checklist.visible_steps().is_empty());
    }

    #[test]
    fn labels_are_from_design_spec() {
        // Lock the exact strings — any future wording change should bump
        // design-spec §B.2 first.
        assert_eq!(ChecklistStep::PickUsername.label(), "Pick a username");
        assert_eq!(ChecklistStep::SetDisplayName.label(), "Set a display name");
        assert_eq!(
            ChecklistStep::AddFirstContact.label(),
            "Add your first contact"
        );
    }

    #[test]
    fn with_pending_username_ignores_blank_and_flags_only_pick_username() {
        let checklist = OnboardingChecklist::new().with_pending_username("  ");
        assert!(checklist.pending_username.is_none());

        let checklist = OnboardingChecklist::new().with_pending_username("det1");
        assert!(checklist.pick_username_has_pending_request(ChecklistStep::PickUsername));
        assert!(!checklist.pick_username_has_pending_request(ChecklistStep::SetDisplayName));
    }

    #[test]
    fn pending_username_marks_pick_username_complete() {
        let checklist = OnboardingChecklist::new().with_pending_username("det1");

        assert!(checklist.is_complete(ChecklistStep::PickUsername));
    }

    #[test]
    fn pending_username_contributes_to_all_complete() {
        let checklist = OnboardingChecklist::new()
            .with_pending_username("det1")
            .mark_complete(ChecklistStep::SetDisplayName)
            .mark_complete(ChecklistStep::AddFirstContact);

        assert!(checklist.all_complete());
    }

    /// A pending username request renders completed styling with voting-status
    /// copy instead of either completion or action prompts.
    #[test]
    fn pending_username_swaps_the_pick_username_subtext() {
        use egui::accesskit::Role;
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let checklist = OnboardingChecklist::new().with_pending_username("det1");
        let mut harness = Harness::builder().build_ui(move |ui| {
            checklist.show(ui);
        });
        harness.run();

        assert!(
            harness
                .query_by_label(
                    "Your request for det1.dash is pending while Dash masternodes vote."
                )
                .is_some(),
            "pending PickUsername must explain its voting status"
        );
        assert!(harness.query_by_label("Your username is set.").is_none());
        assert!(
            harness
                .query_by_label(ChecklistStep::PickUsername.subtext_pending())
                .is_none(),
            "the default 'pick a name' nag subtext must be gone while pending"
        );
        assert!(
            harness
                .query_all_by_role_and_label(
                    Role::Button,
                    ChecklistStep::PickUsername.action_label()
                )
                .next()
                .is_none(),
            "the pick-name action must be hidden while a request is pending"
        );
    }
}
