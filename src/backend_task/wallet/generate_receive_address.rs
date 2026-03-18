use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::{DerivationPathReference, DerivationPathType, WalletSeedHash};
use crate::spv::CoreBackendMode;
use std::sync::Arc;

impl AppContext {
    pub(crate) async fn generate_receive_address(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let wallet_arc = {
            let wallets = self.wallets.read()?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(TaskError::WalletNotFound)?
        };

        // If the platform wallet bridge is available, log the CoreWallet's view of
        // the next receive address for comparison. This helps validate that the new
        // bridge produces consistent results before we switch to it fully.
        if let Some(platform_wallet) = self.get_platform_wallet(&seed_hash) {
            let core_wallet = platform_wallet.core();
            if let Ok(bridge_addr) = core_wallet.next_receive_address().await {
                tracing::debug!(
                    seed = %hex::encode(seed_hash),
                    bridge_address = %bridge_addr,
                    "PlatformWallet bridge next_receive_address"
                );
            }
        }

        let address_string = if self.core_backend_mode() == CoreBackendMode::Spv {
            let derived = self
                .spv_manager
                .next_bip44_receive_address(seed_hash, 0)
                .await
                .map_err(|e| TaskError::WalletAddressDerivationFailed { detail: e })?;

            let _ = self.register_spv_address(
                &wallet_arc,
                derived.address.clone(),
                derived.derivation_path.clone(),
                DerivationPathType::CLEAR_FUNDS,
                DerivationPathReference::BIP44,
            )?;

            derived.address.to_string()
        } else {
            let mut wallet = wallet_arc.write()?;
            wallet
                .receive_address(self.network, true, Some(self))
                .map_err(|e| TaskError::WalletAddressDerivationFailed { detail: e })?
                .to_string()
        };

        Ok(BackendTaskSuccessResult::GeneratedReceiveAddress {
            seed_hash,
            address: address_string,
        })
    }
}
