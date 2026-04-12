use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::{DerivationPathReference, DerivationPathType, WalletId};
use crate::spv::CoreBackendMode;
use std::sync::Arc;

impl AppContext {
    pub(crate) async fn generate_receive_address(
        self: &Arc<Self>,
        seed_hash: WalletId,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let wallet_arc = {
            let wallets = self.wallets.read()?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(TaskError::WalletNotFound)?
        };

        let address_string = if self.core_backend_mode() == CoreBackendMode::Spv {
            // Use PlatformWallet's CoreWallet for address derivation in SPV mode.
            let platform_wallet = self.require_platform_wallet(&seed_hash)?;
            let address = platform_wallet
                .core()
                .next_receive_address()
                .await
                .map_err(|e| TaskError::WalletAddressDerivationFailed {
                    detail: e.to_string(),
                })?;

            // Register the address in DET's address table so it shows in the UI.
            // Read derivation path from the ManagedWalletInfo accounts.
            {
                let wallet_info = platform_wallet.state().await;
                for acc in wallet_info.core_wallet.accounts.all_accounts() {
                    if let Some(ai) = acc.get_address_info(&address) {
                        let _ = self.register_spv_address(
                            &wallet_arc,
                            address.clone(),
                            ai.path.clone(),
                            DerivationPathType::CLEAR_FUNDS,
                            DerivationPathReference::BIP44,
                        );
                        break;
                    }
                }
            }

            address.to_string()
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
