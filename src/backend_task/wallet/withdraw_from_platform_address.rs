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
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        use crate::wallet_backend::{DetPlatformSigner, PlatformPathIndex};
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::dpp::withdrawal::Pooling;
        use dash_sdk::platform::transition::address_credit_withdrawal::WithdrawAddressFunds;

        // Clone wallet and SDK before the async operation to avoid holding guards across await
        let (wallet, sdk) = {
            let wallet_arc = {
                let wallets = self.wallets.read()?;
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or(crate::backend_task::error::TaskError::WalletNotFound)?
            };
            let wallet = wallet_arc.read()?.clone();
            let sdk = self.sdk.load().as_ref().clone();
            (wallet, sdk)
        };

        // Deduct fee from the specified input (should be the one with highest balance)
        let fee_strategy = vec![AddressFundsFeeStrategyStep::DeductFromInput(
            fee_payer_index,
        )];

        // Sign each withdrawal input through a JIT platform signer that borrows
        // the HD seed only for the duration of the SDK call. The pure path
        // index is built before the secret scope; the seed zeroizes on return.
        let network = self.network;
        let path_index = PlatformPathIndex::from_wallet(&wallet, network);
        let backend = self.wallet_backend()?;
        let _result = backend
            .secret_access()
            .with_secret_session(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                async |session| {
                    let plaintext = session.plaintext();
                    let seed = plaintext.expose_hd_seed().ok_or(
                        crate::backend_task::error::TaskError::ContactWalletSeedUnavailable,
                    )?;
                    let signer = DetPlatformSigner::from_held(seed, network, &path_index);
                    sdk.withdraw_address_funds(
                        inputs,
                        None, // No change output
                        fee_strategy,
                        core_fee_per_byte,
                        Pooling::Never,
                        output_script,
                        &signer,
                        None,
                    )
                    .await
                    .map_err(crate::backend_task::error::TaskError::from)
                },
            )
            .await?;

        // Trigger a balance refresh
        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressWithdrawal { seed_hash })
    }
}
