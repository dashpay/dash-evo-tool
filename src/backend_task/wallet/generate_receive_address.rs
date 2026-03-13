use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::{DerivationPathReference, DerivationPathType, WalletSeedHash};
use crate::spv::CoreBackendMode;
use std::sync::Arc;

impl AppContext {
    pub(crate) async fn generate_receive_address(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        let wallet_arc = {
            let wallets = self.wallets.read()?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(crate::backend_task::error::TaskError::WalletNotFound)?
        };

        let address_string = if self.core_backend_mode() == CoreBackendMode::Spv {
            let derived = self
                .spv_manager
                .next_bip44_receive_address(seed_hash, 0)
                .await
                .map_err(crate::backend_task::error::TaskError::UserInput)?;

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
                .map_err(crate::backend_task::error::TaskError::UserInput)?
                .to_string()
        };

        Ok(BackendTaskSuccessResult::GeneratedReceiveAddress {
            seed_hash,
            address: address_string,
        })
    }
}
