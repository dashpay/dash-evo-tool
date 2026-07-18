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
        use crate::wallet_backend::{DetPlatformSigner, PlatformPathIndex};
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::dpp::withdrawal::Pooling;
        use dash_sdk::platform::transition::address_credit_withdrawal::WithdrawAddressFunds;

        // Resolve the shared wallet handle and SDK before the async operation.
        let network = self.network;
        let wallet_arc = self.wallet_arc(&seed_hash)?;
        let backend = self.wallet_backend()?;
        let sdk = backend.sdk().clone();

        // Deduct fee from the specified input (should be the one with highest balance)
        let fee_strategy = vec![AddressFundsFeeStrategyStep::DeductFromInput(
            fee_payer_index,
        )];

        // Core-address form of each spent input, for signer-path reconciliation.
        let input_addresses: Vec<Address> = inputs
            .keys()
            .map(|pa| pa.to_address_with_network(network))
            .collect();

        // Sign each withdrawal input through a JIT platform signer that borrows
        // the HD seed only for the duration of the SDK call. The seed zeroizes
        // on return.
        let _result = backend
            .secret_access()
            .with_secret_session(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                async |session| {
                    let plaintext = session.plaintext();
                    let seed = plaintext
                        .expose_hd_seed()
                        .ok_or(crate::backend_task::error::TaskError::WalletLocked)?;

                    // Self-heal before signing: cache the xpub and register each
                    // input's derivation path if the coordinator reported its
                    // balance without one, so a visible balance is always
                    // signable. Guard drops before the SDK await below.
                    let path_index = {
                        let mut wallet = wallet_arc.write()?;
                        wallet.ensure_platform_payment_account_xpub(seed, network);
                        for addr in &input_addresses {
                            wallet.reconcile_platform_address(addr, network);
                        }
                        PlatformPathIndex::from_wallet(&wallet, network)
                    };

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
