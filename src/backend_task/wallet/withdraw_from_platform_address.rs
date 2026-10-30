use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::Address;
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
        use crate::wallet_backend::PlatformPathIndex;
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;

        let network = self.network;
        let wallet_arc = self.wallet_arc(&seed_hash)?;
        let backend = self.wallet_backend()?;

        // Deduct fee from the specified input (should be the one with highest balance)
        let fee_strategy = vec![AddressFundsFeeStrategyStep::DeductFromInput(
            fee_payer_index,
        )];

        // Core-address form of each spent input, for signer-path reconciliation.
        let input_addresses: Vec<Address> = inputs
            .keys()
            .map(|pa| pa.to_address_with_network(network))
            .collect();

        // Self-heal before signing, with the seed the backend borrows for the
        // spend: cache the xpub and register each input's derivation path if
        // the coordinator reported its balance without one, so a visible
        // balance is always signable. The guard drops before any await.
        let prepare_path_index = |seed: &[u8; 64]| {
            let mut wallet = wallet_arc.write()?;
            wallet.ensure_platform_payment_account_xpub(seed, network);
            for addr in &input_addresses {
                wallet.reconcile_platform_address(addr, network);
            }
            Ok(PlatformPathIndex::from_wallet(&wallet, network))
        };
        let entries = backend
            .withdraw_platform_address_credits(
                &seed_hash,
                prepare_path_index,
                inputs,
                output_script,
                core_fee_per_byte,
                fee_strategy,
            )
            .await?;

        // Show what platform-wallet committed, so both views agree.
        self.apply_platform_address_push(vec![(seed_hash, entries)]);

        Ok(BackendTaskSuccessResult::PlatformAddressWithdrawal { seed_hash })
    }
}
