use crate::context::AppContext;
use crate::platform::identity::IdentityRegistrationInfo;
use dash_sdk::Sdk;

impl AppContext {
    pub(super) async fn register_identity(
        &self,
        sdk: &Sdk,
        input: IdentityRegistrationInfo,
    ) -> Result<(), String> {
        Ok(())
    }
}
