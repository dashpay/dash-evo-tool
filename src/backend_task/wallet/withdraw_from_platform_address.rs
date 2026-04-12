use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::WalletId;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::core_script::CoreScript;
use std::collections::BTreeMap;
use std::sync::Arc;

impl AppContext {
    /// Withdraw from Platform addresses to Core
    pub(crate) async fn withdraw_from_platform_address(
        self: &Arc<Self>,
        seed_hash: WalletId,
        inputs: BTreeMap<PlatformAddress, Credits>,
        output_script: CoreScript,
        core_fee_per_byte: u32,
        fee_payer_index: u16,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::dpp::withdrawal::Pooling;
        use dash_sdk::platform::transition::address_credit_withdrawal::WithdrawAddressFunds;

        // Get the platform wallet for signing (PlatformAddressWallet implements Signer<PlatformAddress>)
        let platform_wallet = self.require_platform_wallet(&seed_hash)?;
        let sdk = self.sdk.load().as_ref().clone();

        // Deduct fee from the specified input (should be the one with highest balance)
        let fee_strategy = vec![AddressFundsFeeStrategyStep::DeductFromInput(
            fee_payer_index,
        )];

        // Use the SDK to withdraw
        let _result = sdk
            .withdraw_address_funds(
                inputs,
                None, // No change output
                fee_strategy,
                core_fee_per_byte,
                Pooling::Never,
                output_script,
                platform_wallet.platform(),
                None,
            )
            .await
            .map_err(crate::backend_task::error::TaskError::from)?;

        // Trigger a balance refresh
        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressWithdrawal { seed_hash })
    }
}
