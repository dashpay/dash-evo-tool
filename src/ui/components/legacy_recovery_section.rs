//! The passive, per-identity offer to restore keys stranded in the previous
//! version's saved data (issue #889).
//!
//! Render-only: it shows what a
//! [`RecoveryPlan`](crate::model::legacy_recovery::RecoveryPlan) found and
//! reports the user's approval back to the screen, which owns the dispatch.
//! The listed items *are* the preview, so pressing Restore approves exactly
//! what is on screen — key bytes are never rendered and never logged.

use dash_sdk::dpp::identity::Purpose;
use egui::{InnerResponse, RichText, Ui};

use crate::model::legacy_recovery::{
    ExclusionReason, RecoveryItem, RecoveryItemDescriptor, RecoveryPlan,
};
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::masternodes::{KeyVocabulary, disambiguate_role_labels, role_label_and_tip};
use crate::ui::state::legacy_recovery::LegacyRecoveryState;
use crate::ui::theme::DashColors;

/// Section lead-in. Avoids "migration", "blob" and "vault" — the user knows
/// only that they upgraded and that some keys did not come with them.
const INTRO: &str = "Some keys for this identity from your previous Dash Evo Tool version haven't been brought \
     across.";
/// Replaces [`INTRO`] when the section has nothing to offer but still has
/// something to report. Without it the section would announce stranded keys and
/// then present no way to act on them.
const EXCLUSIONS_ONLY_INTRO: &str = "Your previous Dash Evo Tool version saved keys for this identity that cannot be brought back \
     automatically. Each one is listed below with what you can do instead.";
/// Names what pressing the button does, and — the reassurance that matters
/// most here — what it cannot do.
const RESTORE_TOOLTIP: &str = "Bring the keys listed above back into this identity. Keys already saved here are left \
     exactly as they are.";
/// No ellipsis: on an identity without a password this button restores the keys
/// on the spot rather than opening anything.
const RESTORE_LABEL: &str = "Restore keys";
/// Shown in place of the button while the restore runs.
const RESTORING_LABEL: &str = "Restoring…";
/// Heads the items this flow cannot restore when restorable ones are listed
/// above them, so the two lists are never read as one. On its own,
/// [`EXCLUSIONS_ONLY_INTRO`] already introduces the list.
const EXCLUDED_INTRO: &str = "These keys cannot be brought back automatically:";
/// Confirms a restore that actually put keys back.
const RESTORED_MESSAGE: &str =
    "Your keys from the previous Dash Evo Tool version have been restored to this identity.";
/// Confirms a restore that put nothing back, whether because the keys were
/// already in place (repeating a restore is safe and changes nothing) or
/// because the previous version's saved data no longer holds them. Worded to
/// be true of both, since the outcome the user sees is the same.
const NOTHING_RESTORED_MESSAGE: &str = "There was nothing left to restore for this identity.";

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

/// What the user can do about an item this flow cannot restore, as a complete
/// sentence for the Everyday User.
pub fn exclusion_explanation(reason: ExclusionReason) -> &'static str {
    match reason {
        ExclusionReason::LegacyEncryptedFormat => {
            "This key is saved in an older format that cannot be restored automatically. \
             Load this identity again and enter the key to bring it back."
        }
        ExclusionReason::NoMaterial => {
            "The previous version kept no copy of this key. \
             Load this identity again and enter the key to bring it back."
        }
        ExclusionReason::KeyNoLongerOnIdentity => {
            "This saved key is not one of the keys this identity uses now. \
             Load this identity again and enter the current key to bring it back."
        }
        ExclusionReason::LinkedIdentityUnverified => {
            "Only the previous version's saved data connects this to a separate voting or operator \
             identity, and that alone is not enough to restore it safely. \
             Load this identity again and enter the key to bring it back."
        }
        ExclusionReason::VoterLinkWithoutVotingKey => {
            "The voting key this link needs was not saved, so restoring the link alone would \
             report this node as able to vote when it cannot. \
             Load this identity again and enter the voting key to bring both back."
        }
    }
}

/// The user-facing name (and role tooltip) of every item in `items`, in order.
///
/// Delegates to the shared role vocabulary
/// [`role_label_and_tip`](crate::ui::masternodes::role_label_and_tip), so an
/// item is named here exactly as the "Manage keys" list names the same key, and
/// two rows that would otherwise read alike are told apart by their key id.
pub fn recovery_item_labels(
    vocabulary: KeyVocabulary,
    items: &[&RecoveryItemDescriptor],
) -> Vec<(String, Option<&'static str>)> {
    let mut labels: Vec<(String, Option<&'static str>)> = items
        .iter()
        .map(|item| match &item.item {
            RecoveryItem::Key { .. } => {
                let (label, tip) = role_label_and_tip(
                    vocabulary,
                    item.is_on_voter_identity(),
                    item.purpose.unwrap_or(Purpose::AUTHENTICATION),
                    // The previous version's data records no retired state, and
                    // a key still restorable is one the identity still uses.
                    false,
                );
                (label.to_string(), tip)
            }
            RecoveryItem::VoterAssociation => ("Voting identity link".to_string(), None),
            RecoveryItem::OperatorAssociation => ("Operator identity link".to_string(), None),
            RecoveryItem::OwnerKeyAssociation => ("Owner key link".to_string(), None),
        })
        .collect();

    let key_ids: Vec<_> = items.iter().map(|item| item.key_id()).collect();
    disambiguate_role_labels(&mut labels, &key_ids);
    labels
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
    vocabulary: KeyVocabulary,
}

impl<'a> LegacyRecoverySection<'a> {
    /// The offer for `plan`, naming its keys in `vocabulary`.
    ///
    /// `vocabulary` is an argument rather than a builder option on purpose. This
    /// offer is hosted by three separate screens, and a default would let a
    /// fourth inherit the wrong wording silently: naming a user identity's
    /// transfer key a "payout address key" asserts it owns a masternode, and
    /// disagrees with the keys list rendered right below the offer. Required
    /// here, every host has to answer for the identity it is showing.
    pub fn new(plan: &'a RecoveryPlan, vocabulary: KeyVocabulary) -> Self {
        Self {
            plan,
            restoring: false,
            vocabulary,
        }
    }

    /// Replace the button with a progress line while a restore is in flight,
    /// so the same restore cannot be dispatched twice.
    pub fn restoring(mut self, restoring: bool) -> Self {
        self.restoring = restoring;
        self
    }
}

/// Render `state`'s offer for a hosting screen, returning the items the user
/// approved this frame. Renders nothing at all when detection found nothing, so
/// the offer appears only where it has something to say and retires itself once
/// a restore lands.
///
/// The whole of what a host owes the offer's *rendering*, so no host has to
/// restate it. Three screens host this one offer, and each restatement is a
/// place the next one can drift: naming a key differently, forgetting the
/// in-flight guard that stops a restore being dispatched twice, or showing an
/// empty section. A host supplies only the identity's `vocabulary` and whatever
/// separators frame it — see [`LegacyRecoveryState::has_offer`] for deciding
/// whether to draw those.
///
/// `vocabulary` stays an argument for the reason [`LegacyRecoverySection::new`]
/// gives: naming a user identity's transfer key a "payout address key" asserts
/// it owns a masternode.
pub fn host_offer(
    state: &LegacyRecoveryState,
    vocabulary: KeyVocabulary,
    ui: &mut Ui,
) -> Option<Vec<RecoveryItem>> {
    let restoring = state.is_restoring();
    let plan = state.plan().filter(|plan| !plan.is_empty())?;
    // `changed_value` hands back a reference into the response, so the approved
    // set has to be cloned out before the response is dropped.
    LegacyRecoverySection::new(plan, vocabulary)
        .restoring(restoring)
        .show(ui)
        .inner
        .changed_value()
        .clone()
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
                let restorable = !self.plan.items.is_empty();
                let intro = if restorable {
                    INTRO
                } else {
                    EXCLUSIONS_ONLY_INTRO
                };
                ui.label(RichText::new(intro).color(DashColors::warning_color(dark_mode)));

                let previewed = self.plan.preview_items();
                for (label, tip) in recovery_item_labels(self.vocabulary, &previewed) {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        let row = ui
                            .label(RichText::new(label).color(DashColors::text_primary(dark_mode)));
                        if let Some(tip) = tip {
                            row.on_hover_text(tip);
                        }
                    });
                }

                if restorable {
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
                    if restorable {
                        ui.label(
                            RichText::new(EXCLUDED_INTRO)
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                    let excluded: Vec<&RecoveryItemDescriptor> =
                        self.plan.excluded.iter().map(|(item, _)| item).collect();
                    let reasons = self.plan.excluded.iter().map(|(_, reason)| *reason);
                    for ((label, _), reason) in recovery_item_labels(self.vocabulary, &excluded)
                        .into_iter()
                        .zip(reasons)
                    {
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            // The reason is the one sentence that tells the user
                            // what to do instead, so it is read, not hovered:
                            // a tooltip reaches neither touch nor keyboard.
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(label).color(DashColors::text_primary(dark_mode)),
                                );
                                ui.label(
                                    RichText::new(exclusion_explanation(reason))
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                            });
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
