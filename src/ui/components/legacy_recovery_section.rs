//! The passive, per-identity offer to restore keys stranded in the previous
//! version's saved data (issue #889).
//!
//! Render-only: it shows what a
//! [`RecoveryPlan`](crate::model::legacy_recovery::RecoveryPlan) found and
//! reports the user's approval back to the screen, which owns the dispatch.
//! The listed items *are* the preview, so pressing Restore approves exactly
//! what is on screen — key bytes are never rendered and never logged.

use egui::{InnerResponse, RichText, Ui};

use crate::model::legacy_recovery::{RecoveryItem, RecoveryPlan};
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::theme::DashColors;

/// Section lead-in. Avoids "migration", "blob" and "vault" — the user knows
/// only that they upgraded and that some keys did not come with them.
const INTRO: &str = "Some keys for this identity from your previous Dash Evo Tool version haven't been brought \
     across.";
/// Names what pressing the button does, and — the reassurance that matters
/// most here — what it cannot do.
const RESTORE_TOOLTIP: &str = "Bring the keys listed above back into this identity. Keys already saved here are left \
     exactly as they are.";
const RESTORE_LABEL: &str = "Restore keys…";
/// Shown in place of the button while the restore runs.
const RESTORING_LABEL: &str = "Restoring…";
/// Introduces the items this flow cannot restore, so they are never silently
/// dropped from a list the user reads as complete.
const EXCLUDED_INTRO: &str = "These keys cannot be brought back automatically:";
/// Confirms a restore that actually put keys back.
const RESTORED_MESSAGE: &str =
    "Your keys from the previous Dash Evo Tool version have been restored to this identity.";
/// Confirms a restore that found everything already in place — the outcome of
/// repeating a restore, which is safe and changes nothing.
const NOTHING_RESTORED_MESSAGE: &str =
    "These keys were already saved for this identity, so nothing needed restoring.";

/// The banner text for a finished restore, shared by every screen that offers
/// one so the wording is a single translation unit. `restored` is whether
/// anything actually landed.
pub fn completion_message(restored: bool) -> &'static str {
    if restored {
        RESTORED_MESSAGE
    } else {
        NOTHING_RESTORED_MESSAGE
    }
}

/// What the user approved this frame.
#[derive(Debug, Clone)]
pub struct LegacyRecoverySectionResponse {
    /// The section's own response, for layout composition.
    pub response: egui::Response,
    /// The approved items, set only on the frame Restore was pressed.
    approved: Option<Vec<RecoveryItem>>,
}

impl ComponentResponse for LegacyRecoverySectionResponse {
    type DomainType = Vec<RecoveryItem>;

    fn has_changed(&self) -> bool {
        self.approved.is_some()
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.approved
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// The recovery offer for one identity.
///
/// Borrows the plan the backend computed; an empty plan renders nothing at all,
/// so the affordance disappears on its own once there is nothing left to
/// restore. Holds no key material — only public labels.
pub struct LegacyRecoverySection<'a> {
    plan: &'a RecoveryPlan,
    restoring: bool,
}

impl<'a> LegacyRecoverySection<'a> {
    /// The offer for `plan`.
    pub fn new(plan: &'a RecoveryPlan) -> Self {
        Self {
            plan,
            restoring: false,
        }
    }

    /// Replace the button with a progress line while a restore is in flight,
    /// so the same restore cannot be dispatched twice.
    pub fn restoring(mut self, restoring: bool) -> Self {
        self.restoring = restoring;
        self
    }
}

impl Component for LegacyRecoverySection<'_> {
    type DomainType = Vec<RecoveryItem>;
    type Response = LegacyRecoverySectionResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        let dark_mode = ui.style().visuals.dark_mode;
        let mut approved = None;

        let response = ui
            .vertical(|ui| {
                if self.plan.is_empty() {
                    return;
                }
                ui.label(RichText::new(INTRO).color(DashColors::warning_color(dark_mode)));

                for item in self.plan.preview_items() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(item.label()).color(DashColors::text_primary(dark_mode)),
                        );
                    });
                }

                if !self.plan.items.is_empty() {
                    if self.restoring {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new(RESTORING_LABEL)
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                        });
                    } else if ui
                        .button(RESTORE_LABEL)
                        .on_hover_text(RESTORE_TOOLTIP)
                        .clicked()
                    {
                        // The previewed set is the approved set: the user
                        // approves exactly the list they just read.
                        approved = Some(self.plan.approved_items());
                    }
                }

                if !self.plan.excluded.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(EXCLUDED_INTRO).color(DashColors::text_secondary(dark_mode)),
                    );
                    for (item, reason) in &self.plan.excluded {
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(item.label())
                                    .color(DashColors::text_secondary(dark_mode)),
                            )
                            .on_hover_text(reason.explanation());
                        });
                    }
                }
            })
            .response;

        InnerResponse::new(
            LegacyRecoverySectionResponse {
                response: response.clone(),
                approved,
            },
            response,
        )
    }

    fn current_value(&self) -> Option<Self::DomainType> {
        None
    }
}
