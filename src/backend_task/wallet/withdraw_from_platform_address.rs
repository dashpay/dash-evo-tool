use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::core_script::CoreScript;
use std::collections::BTreeMap;
use std::sync::Arc;

impl AppContext {
    /// Withdraw from Platform addresses to Core
    pub(crate) async fn withdraw_from_platform_address(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        inputs: BTreeMap<PlatformAddress, Credits>,
        output_script: CoreScript,
        core_fee_per_byte: u32,
        fee_payer_index: u16,
    ) -> Result<BackendTaskSuccessResult, String> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::dpp::withdrawal::Pooling;
        use dash_sdk::platform::transition::address_credit_withdrawal::WithdrawAddressFunds;

        // Clone wallet and SDK before the async operation to avoid holding guards across await
        let (wallet, sdk) = {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or_else(|| "Wallet not found".to_string())?
            };
            let wallet = wallet_arc.read().map_err(|e| e.to_string())?.clone();
            let sdk = self.sdk.read().map_err(|e| e.to_string())?.clone();
            (wallet, sdk)
        };

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
                &wallet,
                None,
            )
            .await
            .map_err(|e| format!("Failed to withdraw from Platform address: {}", e))?;

        // Trigger a balance refresh
        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressWithdrawal { seed_hash })
    }
}
