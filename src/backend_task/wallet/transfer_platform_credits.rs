use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::WalletId;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use std::collections::BTreeMap;
use std::sync::Arc;

impl AppContext {
    /// Transfer credits between Platform addresses
    pub(crate) async fn transfer_platform_credits(
        self: &Arc<Self>,
        seed_hash: WalletId,
        inputs: BTreeMap<PlatformAddress, Credits>,
        outputs: BTreeMap<PlatformAddress, Credits>,
        fee_payer_index: u16,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::platform::transition::transfer_address_funds::TransferAddressFunds;

        // Get the platform wallet for signing (PlatformAddressWallet implements Signer<PlatformAddress>)
        let platform_wallet = self.require_platform_wallet(&seed_hash)?;
        let sdk = self.sdk.load().as_ref().clone();

        // Deduct fee from the specified input address (should be the one with highest balance).
        let fee_strategy = vec![AddressFundsFeeStrategyStep::DeductFromInput(
            fee_payer_index,
        )];

        tracing::info!(
            "transfer_platform_credits: fee_payer_index={}, inputs={}, outputs={}",
            fee_payer_index,
            inputs.len(),
            outputs.len()
        );
        for (idx, (addr, amount)) in inputs.iter().enumerate() {
            tracing::info!("  Input {}: {:?} -> {}", idx, addr, amount);
        }

        // Use the SDK to transfer - returns proof-verified updated address infos
        let address_infos = sdk
            .transfer_address_funds(
                inputs,
                outputs,
                fee_strategy,
                platform_wallet.platform(),
                None,
            )
            .await
            .map_err(crate::backend_task::error::TaskError::from)?;

        // Update wallet balances from the proof-verified response (no extra fetch needed)
        self.update_wallet_platform_address_info_from_sdk(seed_hash, &address_infos)?;

        Ok(BackendTaskSuccessResult::PlatformCreditsTransferred { seed_hash })
    }
}
