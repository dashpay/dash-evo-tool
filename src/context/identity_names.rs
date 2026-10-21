use super::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

/// Accepted names and load revisions for this network, including explicit clears.
#[derive(Debug, Default)]
pub(super) struct ProfileName {
    revision: u64,
    accepted_revision: u64,
    operation: std::sync::Arc<tokio::sync::Mutex<()>>,
    accepted: Option<Option<String>>,
}

impl ProfileName {
    fn accept(&mut self, name: Option<&str>) {
        self.accepted = Some(
            name.map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        );
    }
}

impl AppContext {
    /// Read persisted profile fields already held in wallet memory without blocking.
    pub(crate) fn cached_identity_profile(
        &self,
        id: Identifier,
    ) -> Option<crate::model::dashpay::StoredProfile> {
        self.wallet_backend()
            .ok()?
            .dashpay_view()
            .cached_profile(&id)
    }

    /// Read a profile name without network requests, disk reads, or waiting on wallet state.
    pub fn identity_display_name(&self, id: Identifier) -> Option<String> {
        self.identity_display_name_or(id, None)
    }

    /// Include an already-loaded screen profile before falling back to an unnamed identity.
    pub(crate) fn identity_display_name_or(
        &self,
        id: Identifier,
        profile_name: Option<&str>,
    ) -> Option<String> {
        let accepted = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .and_then(|entry| entry.accepted.clone());
        accepted.unwrap_or_else(|| {
            self.wallet_backend()
                .ok()
                .and_then(|backend| backend.dashpay_view().cached_display_name(&id))
                .or_else(|| {
                    profile_name
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                })
        })
    }

    /// Name an identity consistently with its profile, main username, or shortened identifier.
    pub fn identity_display_label(&self, identity: &QualifiedIdentity) -> String {
        let display_name = self.identity_display_name(identity.identity.id());
        let main_username = self.main_username(identity);
        identity.display_name_label_with_username(display_name.as_deref(), main_username.as_deref())
    }

    /// The identity's alias, profile name or main username, or `None` when it has none.
    pub fn identity_name(&self, identity: &QualifiedIdentity) -> Option<String> {
        let display_name = self.identity_display_name(identity.identity.id());
        let main_username = self.main_username(identity);
        identity.name_with_username(display_name.as_deref(), main_username.as_deref())
    }

    /// Serialize profile fetch/mirror/write operations for this owner and network.
    pub(crate) async fn lock_identity_profile(
        &self,
        id: Identifier,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        let operation = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(id)
            .or_default()
            .operation
            .clone();
        operation.lock_owned().await
    }

    /// Revision used to reject results queued before a newer operation completed.
    pub fn identity_profile_revision(&self, id: Identifier) -> u64 {
        self.identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .map_or(0, |entry| entry.accepted_revision)
    }

    pub(crate) fn profile_snapshot_is_current(
        &self,
        snapshot: &crate::model::dashpay::ProfileSnapshot,
    ) -> bool {
        self.network == snapshot.network
            && self.identity_profile_revision(snapshot.owner) == snapshot.revision
    }

    pub(crate) fn begin_identity_profile_load(&self, id: Identifier) -> u64 {
        let mut names = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = names.entry(id).or_default();
        entry.revision += 1;
        entry.revision
    }

    pub(crate) fn record_identity_profile_name(
        &self,
        id: Identifier,
        revision: u64,
        name: Option<&str>,
    ) -> bool {
        let mut names = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = names
            .get_mut(&id)
            .filter(|entry| entry.revision == revision)
        else {
            return false;
        };
        entry.accept(name);
        entry.accepted_revision = revision;
        true
    }

    pub(crate) fn save_identity_profile_name(&self, id: Identifier, name: Option<&str>) -> u64 {
        let mut names = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = names.entry(id).or_default();
        entry.revision += 1;
        entry.accept(name);
        entry.accepted_revision = entry.revision;
        entry.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;

    #[tokio::test]
    async fn profile_operation_lock_orders_mirror_and_save_per_owner() {
        let dir = tempfile::tempdir().unwrap();
        let context = test_app_context(dir.path());
        let id = Identifier::from([43; 32]);
        let mirror = context.lock_identity_profile(id).await;
        let load_revision = context.begin_identity_profile_load(id);
        let (started, waiting) = tokio::sync::oneshot::channel();
        let writer_context = context.clone();
        let writer = tokio::spawn(async move {
            started.send(()).unwrap();
            let _guard = writer_context.lock_identity_profile(id).await;
            writer_context.save_identity_profile_name(id, Some("Saved"))
        });
        waiting.await.unwrap();
        tokio::task::yield_now().await;
        assert!(
            !writer.is_finished(),
            "save cannot overtake an awaiting mirror"
        );
        let other = context
            .lock_identity_profile(Identifier::from([44; 32]))
            .await;
        assert!(context.record_identity_profile_name(id, load_revision, Some("Fetched")));
        drop(other);
        drop(mirror);
        let save_revision = writer.await.unwrap();
        assert!(save_revision > load_revision);
        assert_eq!(context.identity_display_name(id).as_deref(), Some("Saved"));
        assert!(
            !context.profile_snapshot_is_current(&crate::model::dashpay::ProfileSnapshot {
                network: context.network,
                owner: id,
                revision: load_revision,
                profile: None
            })
        );
    }

    #[test]
    fn identity_display_name_updates_reject_stale_loads_and_isolate_networks() {
        let dir = tempfile::tempdir().unwrap();
        let context = test_app_context(dir.path());
        let id = Identifier::from([42; 32]);
        assert_eq!(context.identity_display_name(id), None);
        let stale = context.begin_identity_profile_load(id);
        context.save_identity_profile_name(id, Some("Alex Profile"));
        assert!(!context.record_identity_profile_name(id, stale, Some("Old Profile")));
        assert_eq!(
            context.identity_display_name(id).as_deref(),
            Some("Alex Profile")
        );
        let pill = crate::ui::identity::identity_pill::IdentityPill::new(
            context.identity_display_name(id).as_deref(),
            Some("alex.dash"),
            "abcdefghijk",
        );
        assert_eq!(pill.resolved_label(), "Alex Profile");
        let stale = context.begin_identity_profile_load(id);
        context.save_identity_profile_name(id, None);
        assert!(!context.record_identity_profile_name(id, stale, Some("Old Profile")));
        assert_eq!(
            context.identity_display_name_or(id, Some("Old profile")),
            None
        );
        let old = context.begin_identity_profile_load(id);
        let new = context.begin_identity_profile_load(id);
        assert!(!context.record_identity_profile_name(id, old, Some("Old")));
        assert!(context.record_identity_profile_name(id, new, Some("Latest")));
        assert_eq!(context.identity_display_name(id).as_deref(), Some("Latest"));
        assert_eq!(
            context.identity_display_name(Identifier::from([7; 32])),
            None
        );
        let other_dir = tempfile::tempdir().unwrap();
        let other_network = crate::context::test_support::test_app_context_for_network(
            other_dir.path(),
            dash_sdk::dpp::dashcore::Network::Mainnet,
        );
        assert_eq!(other_network.identity_display_name(id), None);
    }
}
