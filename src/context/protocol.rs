//! Network compatibility shared by task enforcement and connection feedback.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::sdk_wrapper::MIN_PLATFORM_VERSION;
use dash_sdk::ProtocolVersionInfo;

fn compatibility_error(info: ProtocolVersionInfo) -> Option<TaskError> {
    match info.verified_version {
        None => Some(TaskError::PlatformNetworkUnconfirmed),
        Some(verified_version) if verified_version < MIN_PLATFORM_VERSION.protocol_version => {
            Some(TaskError::PlatformNetworkTooOld { verified_version })
        }
        Some(verified_version) if info.newer_protocol_available => {
            Some(TaskError::PlatformNetworkTooNew { verified_version })
        }
        Some(_) => None,
    }
}

impl AppContext {
    /// Check the exact SDK captured by a task before it can write.
    pub(crate) fn ensure_sdk_write_supported(&self, sdk: &dash_sdk::Sdk) -> Result<(), TaskError> {
        match compatibility_error(self.protocol_version_info_for(sdk)) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Refuse writes until both active and retained wallet connections are supported.
    pub fn ensure_platform_write_supported(&self) -> Result<(), TaskError> {
        if let Some(error) = self.unsupported_platform_error() {
            return Err(error);
        }
        self.ensure_sdk_write_supported(&self.sdk.load())?;
        if let Ok(backend) = self.wallet_backend() {
            self.ensure_sdk_write_supported(backend.sdk())?;
        }
        Ok(())
    }

    /// Refresh a retained wallet SDK after a connection rebuild without resetting it.
    /// Each SDK must verify its own proof; a seed is never copied into that observation.
    pub(crate) async fn refresh_retained_wallet_protocol(
        &self,
        current: &dash_sdk::Sdk,
    ) -> Result<(), Box<dash_sdk::Error>> {
        if let Ok(backend) = self.wallet_backend()
            && !std::ptr::eq(current, backend.sdk())
            && compatibility_error(self.protocol_version_info_for(backend.sdk())).is_some()
        {
            use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
            use dash_sdk::platform::{DataContract, Fetch};
            DataContract::fetch(backend.sdk(), self.dpns_contract().id()).await?;
        }
        Ok(())
    }

    /// A verified incompatibility to show even before a user attempts a write.
    pub(crate) fn unsupported_platform_error(&self) -> Option<TaskError> {
        let current = compatibility_error(self.protocol_version_info());
        let wallet = self
            .wallet_backend()
            .ok()
            .and_then(|backend| compatibility_error(self.protocol_version_info_for(backend.sdk())));
        [current, wallet].into_iter().flatten().find(|error| {
            matches!(
                error,
                TaskError::PlatformNetworkTooOld { .. } | TaskError::PlatformNetworkTooNew { .. }
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::{
        prove_protocol_version, set_sdk_protocol_version, test_app_context,
    };

    #[test]
    fn minimum_protocol_14_requires_a_proof_and_recovers_after_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_app_context(dir.path());
        assert!(matches!(
            ctx.ensure_platform_write_supported(),
            Err(TaskError::PlatformNetworkUnconfirmed)
        ));
        prove_protocol_version(&ctx, 13);
        assert!(matches!(
            ctx.ensure_platform_write_supported(),
            Err(TaskError::PlatformNetworkTooOld {
                verified_version: 13
            })
        ));
        for version in [14, 15] {
            prove_protocol_version(&ctx, version);
            assert!(ctx.ensure_platform_write_supported().is_ok());
            assert!(ctx.unsupported_platform_error().is_none());
        }
        set_sdk_protocol_version(&ctx, 14);
        assert!(matches!(
            ctx.ensure_platform_write_supported(),
            Err(TaskError::PlatformNetworkUnconfirmed)
        ));
        prove_protocol_version(&ctx, dash_sdk::dpp::version::LATEST_VERSION + 1);
        assert!(matches!(
            ctx.ensure_platform_write_supported(),
            Err(TaskError::PlatformNetworkTooNew { .. })
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn minimum_protocol_14_checks_the_retained_wallet_connection_after_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_app_context(dir.path());
        let mut sdk = crate::context::test_support::mock_sdk_at_for(&ctx, 14);
        use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
        let contract = ctx.dpns_contract();
        sdk.mock()
            .expect_fetch(contract.id(), Some(contract.as_ref().clone()))
            .await
            .unwrap();
        ctx.sdk.store(std::sync::Arc::new(sdk));
        prove_protocol_version(&ctx, 13);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender).await.unwrap();
        let old_sdk = ctx.sdk.load_full();
        set_sdk_protocol_version(&ctx, 14);
        // The active SDK has not proved anything; the retained wallet SDK is already too old.
        assert!(matches!(
            ctx.unsupported_platform_error(),
            Some(TaskError::PlatformNetworkTooOld { .. })
        ));
        prove_protocol_version(&ctx, 14);
        assert!(matches!(
            ctx.ensure_platform_write_supported(),
            Err(TaskError::PlatformNetworkTooOld { .. })
        ));
        ctx.refresh_retained_wallet_protocol(&ctx.sdk.load_full())
            .await
            .unwrap();
        assert_eq!(
            old_sdk.protocol_version_number(),
            dash_sdk::dpp::version::LATEST_VERSION
        );
        // Upstream mocks raise the effective version but cannot supply a verified proof.
        crate::context::test_support::prove_sdk_protocol_version(&ctx, old_sdk, 14);
        assert!(ctx.ensure_platform_write_supported().is_ok());
        ctx.wallet_backend().unwrap().shutdown().await;
    }
}
