use std::collections::HashMap;

use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;

fn retain_other_identities(identities: &mut HashMap<u32, Identity>, identity_id: &Identifier) {
    identities.retain(|_, identity| identity.id() != *identity_id);
}

impl AppContext {
    pub(super) fn unload_identity(
        &self,
        identity_id: Identifier,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.delete_local_qualified_identity(&identity_id)?;

        let wallets = self.wallets.read()?;
        for wallet in wallets.values() {
            retain_other_identities(&mut wallet.write()?.identities, &identity_id);
        }
        drop(wallets);

        if self.selected_identity_id() == Some(identity_id) {
            self.set_selected_identity(None);
        }
        let mut pending = self.pending_identity_selection.lock()?;
        if *pending == Some(identity_id) {
            *pending = None;
        }

        tracing::info!(
            target = "backend_task::identity::unload_identity",
            identity = %identity_id,
            "Unloaded identity and its local device state",
        );
        Ok(BackendTaskSuccessResult::UnloadedIdentity(identity_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::version::PlatformVersion;

    #[test]
    fn identity_unload_evicts_only_target_from_wallet_cache() {
        let platform_version = PlatformVersion::latest();
        let target_id = Identifier::from([0x11; 32]);
        let sibling_id = Identifier::from([0x22; 32]);
        let target = Identity::create_basic_identity(target_id, platform_version)
            .expect("create target identity");
        let sibling = Identity::create_basic_identity(sibling_id, platform_version)
            .expect("create sibling identity");
        let mut identities = HashMap::from([(3, target), (7, sibling)]);

        retain_other_identities(&mut identities, &target_id);

        assert_eq!(identities.len(), 1, "only the target must be evicted");
        assert_eq!(
            identities.get(&7).map(IdentityGettersV0::id),
            Some(sibling_id),
            "the sibling identity must remain cached"
        );
    }
}
