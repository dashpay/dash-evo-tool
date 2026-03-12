use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use std::collections::BTreeMap;
use std::sync::Arc;

impl AppContext {
    /// Transfer credits between Platform addresses
    pub(crate) async fn transfer_platform_credits(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        inputs: BTreeMap<PlatformAddress, Credits>,
        outputs: BTreeMap<PlatformAddress, Credits>,
        fee_payer_index: u16,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::platform::transition::transfer_address_funds::TransferAddressFunds;

        // Clone wallet and SDK before the async operation to avoid holding guards across await
        let (wallet, sdk) = {
            let wallet_arc = {
                let wallets = self
                    .wallets
                    .read()
                    .map_err(|_| crate::backend_task::error::TaskError::LockPoisoned {
                        resource: "wallets",
                    })?;
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or(crate::backend_task::error::TaskError::WalletNotFound)?
            };
            let wallet = wallet_arc
                .read()
                .map_err(|_| crate::backend_task::error::TaskError::LockPoisoned {
                    resource: "wallet",
                })?
                .clone();
            let sdk = self.sdk.load().as_ref().clone();
            (wallet, sdk)
        };

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
            .transfer_address_funds(inputs, outputs, fee_strategy, &wallet, None)
            .await
            .map_err(crate::backend_task::error::TaskError::from)?;

        // Update wallet balances from the proof-verified response (no extra fetch needed)
        self.update_wallet_platform_address_info_from_sdk(seed_hash, &address_infos)?;

        Ok(BackendTaskSuccessResult::PlatformCreditsTransferred { seed_hash })
    }
}
