use crate::context::AppContext;
use crate::platform::identity::IdentityRegistrationInfo;
use dash_sdk::Sdk;

impl AppContext {
    pub(super) async fn register_identity(
        &self,
        input: IdentityRegistrationInfo,
    ) -> Result<(), String> {
        let IdentityRegistrationInfo {
            alias_input, amount, keys, identity_index, wallet
        } = input;
        let sdk = self.sdk.clone();
        let mut wallet = wallet.write().unwrap();
        let asset_lock_transaction = wallet.asset_lock_transaction(sdk.network, amount, identity_index, Some(self))?;

        self.core_client
        Ok(())
    }
}
