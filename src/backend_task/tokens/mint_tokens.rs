use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::TaskResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::token::MintRecipient;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::group::group_action_status::GroupActionStatus;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::tokens::transitions::MintResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn mint_tokens(
        &self,
        sending_identity: &QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        amount: u64,
        optional_recipient: Option<Identifier>,
        group_info: Option<GroupStateTransitionInfoStatus>,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // The identity the tokens go to: the chosen one when the token allows
        // choosing, otherwise its configured destination.
        let mint_recipient = data_contract
            .expected_token_configuration(token_position)
            .ok()
            .and_then(|config| {
                MintRecipient::resolve(config.distribution_rules(), optional_recipient).identity()
            });
        let actor = self.token_actor(sending_identity).await?;
        let common = self.token_action_common(data_contract.clone(), token_position, &signing_key);

        self.execute_token_op(
            actor.mint(common, amount, optional_recipient, public_note, group_info),
            |result| self.cache_mint_result(&data_contract, token_position, mint_recipient, result),
            BackendTaskSuccessResult::MintedTokens,
        )
        .await
    }

    /// Reflects a broadcast mint's proven result in the cached token balance
    /// of the identity it credited, so My Tokens updates before the next
    /// sync. A closed group mint that names no recipient is credited to the
    /// token's configured destination ([`group_mint_recipient`]), never the
    /// proposer that signed it; with neither known, nothing is cached.
    fn cache_mint_result(
        &self,
        data_contract: &DataContract,
        token_position: u16,
        optional_recipient: Option<Identifier>,
        result: MintResult,
    ) {
        // Using the result, update the balance of the recipient identity
        if let Some(token_id) = data_contract.token_id(token_position) {
            match result {
                // Standard mint result - direct balance update
                MintResult::TokenBalance(identity_id, amount) => {
                    if let Err(e) =
                        self.insert_token_identity_balance(&token_id, &identity_id, amount)
                    {
                        tracing::error!("Failed to update token balance: {}", e);
                    }
                }

                // Historical document - extract recipient and amount from document
                MintResult::HistoricalDocument(document) => {
                    if let (Some(recipient_value), Some(amount_value)) =
                        (document.get("recipientId"), document.get("amount"))
                        && let (Value::Identifier(recipient_bytes), Value::U64(amount)) =
                            (recipient_value, amount_value)
                        && let Ok(recipient_id) = Identifier::from_bytes(recipient_bytes)
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &recipient_id, *amount)
                    {
                        tracing::error!(
                            "Failed to update token balance from historical document: {}",
                            e
                        );
                    }
                }

                // Group action with document - assume completed if document exists
                MintResult::GroupActionWithDocument(_, Some(document)) => {
                    if let (Some(recipient_value), Some(amount_value)) =
                        (document.get("recipientId"), document.get("amount"))
                        && let (Value::Identifier(recipient_bytes), Value::U64(amount)) =
                            (recipient_value, amount_value)
                        && let Ok(recipient_id) = Identifier::from_bytes(recipient_bytes)
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &recipient_id, *amount)
                    {
                        tracing::error!(
                            "Failed to update token balance from group action document: {}",
                            e
                        );
                    }
                }

                // Group action with balance - only update if action is closed
                MintResult::GroupActionWithBalance(_, status, Some(amount)) => {
                    if matches!(status, GroupActionStatus::ActionClosed)
                        && let Some(recipient_id) = group_mint_recipient(
                            optional_recipient,
                            data_contract
                                .expected_token_configuration(token_position)
                                .ok(),
                        )
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &recipient_id, amount)
                    {
                        tracing::error!("Failed to update token balance from group action: {}", e);
                    }
                }

                // Other variants don't require balance updates
                _ => {}
            }
        }
    }
}

/// The identity a closed group mint credited: the recipient the mint named,
/// or else the token's configured destination, which Platform mints to when
/// none is named. `None` when neither is known, so no balance is cached.
fn group_mint_recipient(
    optional_recipient: Option<Identifier>,
    token_config: Option<&TokenConfiguration>,
) -> Option<Identifier> {
    optional_recipient.or_else(|| {
        token_config?
            .distribution_rules()
            .new_tokens_destination_identity()
            .copied()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Setters;

    fn config_with_destination(destination: Option<Identifier>) -> TokenConfiguration {
        let mut config = TokenConfigurationV0::default_most_restrictive();
        config
            .distribution_rules
            .set_new_tokens_destination_identity(destination);
        config.into()
    }

    /// A contract holding one token, at position 0, whose configured
    /// destination for new tokens is `destination`.
    fn contract_with_destination(destination: Option<Identifier>) -> DataContract {
        use dash_sdk::dpp::data_contract::config::DataContractConfig;
        use dash_sdk::dpp::data_contract::v1::DataContractV1;
        use dash_sdk::dpp::version::PlatformVersion;
        use std::collections::BTreeMap;

        DataContract::V1(DataContractV1 {
            id: Identifier::random(),
            version: 1,
            owner_id: Identifier::random(),
            document_types: BTreeMap::new(),
            config: DataContractConfig::default_for_version(PlatformVersion::latest())
                .expect("contract config"),
            schema_defs: None,
            groups: BTreeMap::new(),
            tokens: BTreeMap::from([(0, config_with_destination(destination))]),
            keywords: vec![],
            created_at: None,
            updated_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            created_at_epoch: None,
            updated_at_epoch: None,
            description: None,
        })
    }

    fn closed_group_mint(balance: u64) -> MintResult {
        MintResult::GroupActionWithBalance(
            Default::default(),
            GroupActionStatus::ActionClosed,
            Some(balance),
        )
    }

    /// A closed group mint that names no recipient caches the returned
    /// balance under the token's configured destination and leaves the
    /// proposer's cached balance alone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn closed_group_mint_caches_the_balance_under_the_configured_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let destination = Identifier::random();
        let proposer = Identifier::random();
        let contract = contract_with_destination(Some(destination));
        let token_id = contract.token_id(0).expect("token id");
        ctx.insert_token_identity_balance(&token_id, &proposer, 5)
            .expect("seed proposer balance");

        ctx.cache_mint_result(&contract, 0, None, closed_group_mint(1_000));

        let balances = ctx
            .wallet_backend()
            .expect("wallet backend")
            .token_balances();
        assert_eq!(balances.get(&destination, &token_id), Some(1_000));
        assert_eq!(
            balances.get(&proposer, &token_id),
            Some(5),
            "the proposer did not receive the minted tokens"
        );
    }

    /// With no named recipient and no configured destination the credited
    /// identity is unknown, so nothing is cached.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn closed_group_mint_without_a_destination_caches_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let proposer = Identifier::random();
        let contract = contract_with_destination(None);
        let token_id = contract.token_id(0).expect("token id");
        ctx.insert_token_identity_balance(&token_id, &proposer, 5)
            .expect("seed proposer balance");

        ctx.cache_mint_result(&contract, 0, None, closed_group_mint(1_000));

        let balances = ctx
            .wallet_backend()
            .expect("wallet backend")
            .token_balances();
        assert_eq!(
            balances.list(),
            vec![(proposer, token_id, 5)],
            "no balance may be written when the credited identity is unknown"
        );
    }

    #[test]
    fn a_named_recipient_is_credited() {
        let named = Identifier::random();
        let config = config_with_destination(Some(Identifier::random()));
        assert_eq!(
            group_mint_recipient(Some(named), Some(&config)),
            Some(named)
        );
    }

    /// A mint that names no recipient goes to the token's configured
    /// destination, never to the proposer that signed it.
    #[test]
    fn an_unnamed_recipient_is_the_configured_destination() {
        let destination = Identifier::random();
        let config = config_with_destination(Some(destination));
        assert_eq!(group_mint_recipient(None, Some(&config)), Some(destination));
    }

    #[test]
    fn no_recipient_and_no_destination_caches_nothing() {
        let config = config_with_destination(None);
        assert_eq!(group_mint_recipient(None, Some(&config)), None);
        assert_eq!(group_mint_recipient(None, None), None);
    }
}
