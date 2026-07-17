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
        use crate::wallet_backend::{DetPlatformSigner, PlatformPathIndex};
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::platform::transition::transfer_address_funds::TransferAddressFunds;

        let backend = self.wallet_backend()?;

        // Clone wallet and SDK before the async operation to avoid holding guards across await
        let (wallet, sdk) = {
            let wallet = self.wallet_arc(&seed_hash)?.read()?.clone();
            let sdk = backend.sdk().clone();
            (wallet, sdk)
        };

        // Deduct fee from the specified input address (should be the one with highest balance).
        let fee_strategy = vec![AddressFundsFeeStrategyStep::DeductFromInput(
            fee_payer_index,
        )];

        // Per-input address/amount detail is intentionally not logged: the
        // summary line carries the aggregate counts, and per-input financial
        // detail does not belong in plaintext logs at the default level.
        tracing::info!(
            "transfer_platform_credits: fee_payer_index={}, inputs={}, outputs={}",
            fee_payer_index,
            inputs.len(),
            outputs.len()
        );

        // Build the pure address→path index before entering the secret scope,
        // then sign each input through a JIT platform signer that borrows the
        // HD seed for the duration of the SDK call only. The seed zeroizes when
        // the scope returns — it never enters this layer by value.
        let network = self.network;
        let path_index = PlatformPathIndex::from_wallet(&wallet, network);
        let (address_infos, _height) = backend
            .secret_access()
            .with_secret_session(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                async |session| {
                    let plaintext = session.plaintext();
                    let seed = plaintext
                        .expose_hd_seed()
                        .ok_or(crate::backend_task::error::TaskError::WalletLocked)?;
                    let signer = DetPlatformSigner::from_held(seed, network, &path_index);
                    sdk.transfer_address_funds(inputs, outputs, fee_strategy, &signer, None)
                        .await
                        .map_err(crate::backend_task::error::TaskError::from)
                },
            )
            .await?;

        // Update wallet balances from the proof-verified response (no extra fetch needed)
        self.update_wallet_platform_address_info_from_sdk(seed_hash, &address_infos)?;

        Ok(BackendTaskSuccessResult::PlatformCreditsTransferred { seed_hash })
    }
}
