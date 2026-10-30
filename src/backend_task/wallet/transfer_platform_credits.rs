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
        use crate::wallet_backend::PlatformPathIndex;
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;

        let backend = self.wallet_backend()?;

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

        // The pure address→path index is built before the secret scope; the
        // backend signs each input with a JIT signer over the borrowed HD seed.
        let path_index = {
            let wallet = self.wallet_arc(&seed_hash)?;
            let wallet = wallet.read()?;
            PlatformPathIndex::from_wallet(&wallet, self.network)
        };
        let entries = backend
            .transfer_platform_address_credits(
                &seed_hash,
                &path_index,
                inputs,
                outputs,
                fee_strategy,
            )
            .await?;

        // Show what platform-wallet committed, so both views agree.
        self.apply_platform_address_push(vec![(seed_hash, entries)]);

        Ok(BackendTaskSuccessResult::PlatformCreditsTransferred { seed_hash })
    }
}
