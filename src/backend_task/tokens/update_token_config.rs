use super::BackendTaskSuccessResult;
use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::ui::tokens::tokens_screen::IdentityTokenInfo;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::tokens::transitions::ConfigUpdateResult;
use dash_sdk::platform::{DataContract, Fetch, IdentityPublicKey};
use dash_sdk::{
    Sdk,
    dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem,
};
use std::sync::Arc;

impl AppContext {
    pub async fn update_token_config(
        &self,
        identity_token_info: IdentityTokenInfo,
        change_item: TokenConfigurationChangeItem,
        signing_key: &IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        validate_supported_change(&change_item)?;
        tracing::trace!(
            ?group_info,
            ?identity_token_info,
            ?change_item,
            "Updating token config for a token",
        );
        // Get the existing contract and identity for building the state transition
        // First, fetch the contract from the local database
        let existing_data_contract = &self
            .get_contract_by_id(&identity_token_info.data_contract.contract.id())?
            .ok_or(TaskError::DataContractNotFound)?
            .contract;

        // Then, fetch the identity from the local database
        let identity = self
            .get_identity_by_id(&identity_token_info.identity.identity.id())?
            .ok_or(TaskError::IdentityNotFoundLocally)?;

        let actor = self.token_actor(&identity).await?;
        let common = self.token_action_common(
            Arc::new(existing_data_contract.clone()),
            identity_token_info.token_position,
            signing_key,
        );
        let result = actor
            .update_config(common, change_item.clone(), public_note, group_info)
            .await?;
        let estimated_fee = self.fee_estimator().estimate_token_transition();
        let success = BackendTaskSuccessResult::UpdatedTokenConfig(
            change_item.to_string(),
            FeeResult::estimated_only(estimated_fee),
        );
        match config_update_outcome(&result) {
            ConfigUpdateOutcome::ProposalRecorded => {
                // The token keeps its settings until the group closes the action.
                tracing::info!(
                    "TokenConfigUpdate: group proposal recorded, waiting for other group members"
                );
                return Ok(success);
            }
            ConfigUpdateOutcome::Applied => {
                tracing::info!("TokenConfigUpdate: settings change applied");
            }
        }

        // Now update the data contract in the local database
        // The proof result contains an action document, not the updated contract,
        // so we need to fetch the updated contract from the platform
        let data_contract =
            DataContract::fetch(sdk, identity_token_info.data_contract.contract.id())
                .await
                .map_err(TaskError::from)?
                .ok_or(TaskError::DataContractNotFound)?;

        let token = data_contract
            .tokens()
            .get(&identity_token_info.token_position)
            .ok_or(TaskError::TokenPositionNotFound {
                position: identity_token_info.token_position,
            })?;

        // Then replace the contract in the local database
        self.replace_contract(
            identity_token_info.data_contract.contract.id(),
            &data_contract,
        )?;

        // insert_token uses upsert (ON CONFLICT DO UPDATE), so a separate
        // remove_token is not needed and avoids a window where the token is
        // missing if insert_token were to fail after remove_token.
        self.insert_token(
            &identity_token_info.token_id,
            &identity_token_info.token_alias,
            token.clone(),
            &identity_token_info.data_contract.contract.id(),
            identity_token_info.token_position,
        )?;

        Ok(success)
    }
}

/// What a settings update Platform accepted did to the token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigUpdateOutcome {
    /// The new setting is in effect.
    Applied,
    /// A group action that stays open: the setting changes once the group
    /// closes it, so the token's stored settings are still current.
    ProposalRecorded,
}

fn config_update_outcome(result: &ConfigUpdateResult) -> ConfigUpdateOutcome {
    match result {
        ConfigUpdateResult::Document(_)
        | ConfigUpdateResult::GroupActionWithDocument(_, Some(_)) => ConfigUpdateOutcome::Applied,
        ConfigUpdateResult::GroupActionWithDocument(_, None) => {
            ConfigUpdateOutcome::ProposalRecorded
        }
    }
}

fn validate_supported_change(item: &TokenConfigurationChangeItem) -> Result<(), TaskError> {
    if crate::model::token::token_change_item_is_supported(item) {
        Ok(())
    } else {
        Err(TaskError::TokenShieldedConfigurationUnsupported)
    }
}

#[cfg(test)]
mod sdk_adapter_tests {
    use super::*;
    use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;

    /// A group proposal Platform recorded without the history document (the
    /// action is still open) is a success that leaves the settings unchanged;
    /// a closed action or a direct update applies them.
    #[test]
    fn open_group_config_update_is_a_recorded_proposal() {
        use dash_sdk::dpp::document::{Document, DocumentV0};
        let document = || Document::V0(DocumentV0::default());
        assert_eq!(
            config_update_outcome(&ConfigUpdateResult::GroupActionWithDocument(1, None)),
            ConfigUpdateOutcome::ProposalRecorded
        );
        assert_eq!(
            config_update_outcome(&ConfigUpdateResult::GroupActionWithDocument(
                2,
                Some(document())
            )),
            ConfigUpdateOutcome::Applied
        );
        assert_eq!(
            config_update_outcome(&ConfigUpdateResult::Document(document())),
            ConfigUpdateOutcome::Applied
        );
    }

    #[test]
    fn backend_rejects_shielded_configuration_before_submission() {
        for item in [
            TokenConfigurationChangeItem::MinimumPoolNotesForOutgoing(17),
            TokenConfigurationChangeItem::MinimumPoolNotesForOutgoingControlGroup(
                AuthorizedActionTakers::NoOne,
            ),
            TokenConfigurationChangeItem::MinimumPoolNotesForOutgoingAdminGroup(
                AuthorizedActionTakers::NoOne,
            ),
        ] {
            assert!(matches!(
                validate_supported_change(&item),
                Err(TaskError::TokenShieldedConfigurationUnsupported)
            ));
        }
        assert!(
            validate_supported_change(&TokenConfigurationChangeItem::MaxSupply(Some(17))).is_ok()
        );
    }
}
