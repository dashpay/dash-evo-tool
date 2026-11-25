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
            save_warning: Some(warning),
            ..
        } = result
        {
            show_contract_save_warning(feedback.ctx, warning);
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
            },
        );
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(
            texts[0].contains("was registered on the network"),
            "{texts:?}"
        );
        assert!(texts[0].contains("Load Contracts"), "{texts:?}");
        assert!(texts[0].contains(&contract_id.to_string()), "{texts:?}");
    }
}
