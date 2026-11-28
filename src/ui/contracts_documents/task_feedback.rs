//! User notifications for contract task outcomes.
use crate::backend_task::error::ContractSaveWarning;
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheUpdate};
use crate::ui::task_feedback::{DomainFeedback, Feedback};
use crate::ui::{MessageType, components::MessageBanner};

/// The contract domain's reactions.
pub(crate) struct ContractsFeedback;

impl DomainFeedback for ContractsFeedback {
    fn on_success(
        &self,
        feedback: &Feedback<'_>,
        _context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> Vec<CacheUpdate> {
        if let BackendTaskSuccessResult::RegisteredContract {
            save_warning,
            earlier_version,
            ..
        } = result
        {
            match (save_warning, earlier_version) {
                (Some(warning), true) => {
                    let contract_id = warning.contract_id();
                    let banner = MessageBanner::set_global(
                        feedback.ctx,
                        format!(
                            "Your contract was registered from your earlier attempt, so your latest changes are not in it, and it could not be saved on this device. To apply your changes, first open Contracts, choose Load Contracts and enter contract ID {contract_id}. Then choose Update Contract."
                        ),
                        MessageType::Warning,
                    );
                    banner.with_details(warning);
                    banner.disable_auto_dismiss();
                }
                (Some(warning), false) => show_contract_save_warning(feedback.ctx, warning),
                (None, true) => {
                    MessageBanner::set_global(
                        feedback.ctx,
                        "Your contract was registered from your earlier attempt, so your latest changes are not in it. To apply them, open Contracts and choose Update Contract.",
                        MessageType::Info,
                    )
                    .disable_auto_dismiss();
                }
                (None, false) => {}
            }
        }
        Vec::new()
    }
}

/// Tell the user a confirmed registration could not be saved on this device.
/// Stays until dismissed: it carries the contract ID needed to add it here.
pub(crate) fn show_contract_save_warning(ctx: &egui::Context, warning: &ContractSaveWarning) {
    let banner = MessageBanner::set_global(ctx, warning, MessageType::Warning);
    banner.with_details(warning);
    banner.disable_auto_dismiss();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::FeeResult;
    use crate::backend_task::error::TaskError;
    use crate::ui::components::message_banner::global_banner_texts;
    use crate::ui::task_feedback::ResultDelivery;

    fn react(ctx: &egui::Context, result: &BackendTaskSuccessResult) {
        let contexts = std::collections::BTreeMap::new();
        let feedback = Feedback {
            ctx,
            contexts: &contexts,
            delivery: ResultDelivery {
                sender_in_view: true,
                sender_gone: false,
            },
        };
        ContractsFeedback.on_success(&feedback, &BackendTaskContext::Other, result);
    }

    /// A registration saved here needs no banner; one that could not be
    /// saved tells the user how to add the contract, without registering it
    /// again.
    #[test]
    fn an_unsaved_registration_is_reported_as_a_warning() {
        let ctx = egui::Context::default();
        react(
            &ctx,
            &BackendTaskSuccessResult::RegisteredContract {
                fee_result: FeeResult::estimated_only(1),
                save_warning: None,
                earlier_version: false,
            },
        );
        assert!(global_banner_texts(&ctx).is_empty());

        let contract_id = dash_sdk::platform::Identifier::from([0x42; 32]);
        react(
            &ctx,
            &BackendTaskSuccessResult::RegisteredContract {
                fee_result: FeeResult::estimated_only(1),
                save_warning: Some(ContractSaveWarning::Contract {
                    contract_id,
                    source: std::sync::Arc::new(TaskError::WalletBackendNotYetWired),
                }),
                earlier_version: false,
            },
        );
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(
            texts[0].contains("was registered on the network"),
            "{texts:?}"
        );
        assert!(texts[0].contains("Load Contracts"), "{texts:?}");
        assert!(texts[0].contains(&format!("{contract_id}")), "{texts:?}");
    }

    /// A registration Platform holds as an earlier attempt sent it says so,
    /// with how to apply the latest changes.
    #[test]
    fn an_earlier_version_registered_is_reported() {
        let ctx = egui::Context::default();
        react(
            &ctx,
            &BackendTaskSuccessResult::RegisteredContract {
                fee_result: FeeResult::estimated_only(1),
                save_warning: None,
                earlier_version: true,
            },
        );
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("earlier attempt"), "{texts:?}");
        assert!(texts[0].contains("Update Contract"), "{texts:?}");
    }

    /// An earlier version that could not be saved here gets one banner with
    /// the steps in order: load it by its ID, then update it.
    #[test]
    fn an_earlier_unsaved_version_gets_one_ordered_instruction() {
        let ctx = egui::Context::default();
        let contract_id = dash_sdk::platform::Identifier::from([0x42; 32]);
        react(
            &ctx,
            &BackendTaskSuccessResult::RegisteredContract {
                fee_result: FeeResult::estimated_only(1),
                save_warning: Some(ContractSaveWarning::Contract {
                    contract_id,
                    source: std::sync::Arc::new(TaskError::WalletBackendNotYetWired),
                }),
                earlier_version: true,
            },
        );
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        let text = &texts[0];
        assert!(text.contains("earlier attempt"), "{text}");
        assert!(text.contains(&format!("{contract_id}")), "{text}");
        let load = text.find("Load Contracts").expect("says to load it");
        let update = text.find("Update Contract").expect("says to update it");
        assert!(load < update, "{text}");
    }
}
