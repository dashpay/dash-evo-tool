use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::identity_load_registry::{IdentityLoadGuard, IdentityLoadToken};
use crate::model::qualified_identity::IdentityType;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::{Identifier, Identity};

impl AppContext {
    pub(super) fn begin_identity_load_and_validate_type(
        &self,
        identity_type: IdentityType,
        identity: &Identity,
        load_token: Option<IdentityLoadToken>,
    ) -> Result<IdentityLoadGuard, TaskError> {
        let load_guard = self.begin_identity_load(identity.id(), load_token)?;
        super::load_identity::validate_loaded_identity_type(identity_type, identity)?;
        Ok(load_guard)
    }

    pub(super) fn finish_identity_load_after_persist(
        &self,
        identity_id: &Identifier,
        load_guard: IdentityLoadGuard,
    ) {
        if let Err(error) = self.clear_forgotten_identity_after_explicit_load(identity_id) {
            tracing::warn!(
                ?error,
                identity_id = %identity_id,
                "Persisted identity but could not clear its forgotten marker"
            );
        }
        load_guard.loaded();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::identity_load_registry::IdentityLoadPhase;
    use crate::context::test_support::{open_persister_fault_connection, test_app_context};
    use dash_sdk::dpp::identity::Purpose;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
        IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
    };
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::IdentityPublicKey;
    use std::collections::BTreeMap;

    #[test]
    fn rejected_identity_type_reports_failed_load() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let platform_version = PlatformVersion::latest();
        let mut owner_key = IdentityPublicKey::random_key(1, Some(1), platform_version);
        owner_key.set_purpose(Purpose::OWNER);
        let identity = Identity::new_with_id_and_keys(
            Identifier::from([0x71; 32]),
            BTreeMap::from([(owner_key.id(), owner_key)]),
            platform_version,
        )
        .expect("identity");
        let identity_id = identity.id();
        let token = ctx
            .mark_identity_load_submitted(identity_id)
            .expect("submit load");

        let error = ctx
            .begin_identity_load_and_validate_type(IdentityType::User, &identity, Some(token))
            .expect_err("a user load must reject an identity with an owner key");

        assert!(matches!(
            error,
            TaskError::IdentityIsMasternode {
                identity_id: rejected_id
            } if rejected_id == identity_id
        ));
        assert_eq!(
            ctx.identity_load_phase(&identity_id, token),
            Some(IdentityLoadPhase::Failed),
            "type validation must happen after the load becomes reportable"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cleanup_failure_after_persist_still_reports_loaded() {
        use crate::app::TaskResult;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");
        let identity_id = Identifier::from([0x72; 32]);
        ctx.record_forgotten_identity(&identity_id)
            .expect("record forgotten marker");
        let fault_connection = open_persister_fault_connection(&backend);
        fault_connection
            .execute_batch(
                "CREATE TRIGGER fail_forgotten_marker_cleanup
                 BEFORE DELETE ON meta_global
                 WHEN OLD.key LIKE 'det:forgotten_identity:%'
                 BEGIN
                     SELECT RAISE(FAIL, 'injected forgotten marker cleanup failure');
                 END;",
            )
            .expect("install cleanup failure trigger");
        let token = ctx
            .mark_identity_load_submitted(identity_id)
            .expect("submit load");
        let load_guard = ctx
            .begin_identity_load(identity_id, Some(token))
            .expect("claim load");

        ctx.finish_identity_load_after_persist(&identity_id, load_guard);

        assert_eq!(
            ctx.identity_load_phase(&identity_id, token),
            Some(IdentityLoadPhase::Loaded),
            "marker cleanup is non-essential after durable persistence"
        );
        assert!(
            ctx.is_identity_forgotten(&identity_id)
                .expect("read retained marker"),
            "the injected cleanup fault must leave the marker in place"
        );

        fault_connection
            .execute_batch("DROP TRIGGER fail_forgotten_marker_cleanup;")
            .expect("remove cleanup failure trigger");
        backend.shutdown().await;
    }
}
