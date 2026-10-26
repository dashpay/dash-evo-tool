use super::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

/// Names saved on this device and their save revisions for this network,
/// including explicit clears.
#[derive(Debug, Default)]
pub(super) struct ProfileName {
    revision: u64,
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
    /// `owner`'s DashPay snapshot; `None` until a sync produced one, or while
    /// the wallet backend is not wired.
    pub(crate) fn dashpay_snapshot(
        &self,
        owner: &Identifier,
    ) -> Option<std::sync::Arc<crate::model::dashpay::DashPaySnapshot>> {
        self.wallet_backend().ok()?.dashpay_snapshot(owner)
    }

    /// Read a profile name without network requests, disk reads, or waiting on wallet state.
    pub fn identity_display_name(&self, id: Identifier) -> Option<String> {
        self.identity_display_name_or(id, None)
    }

    /// The synced profile name, else the name saved on this device, else an
    /// already-loaded screen profile, before falling back to an unnamed
    /// identity.
    pub(crate) fn identity_display_name_or(
        &self,
        id: Identifier,
        profile_name: Option<&str>,
    ) -> Option<String> {
        if let Some(synced) = self.synced_display_name(id) {
            return Some(synced);
        }
        let accepted = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .and_then(|entry| entry.accepted.clone());
        accepted.unwrap_or_else(|| {
            profile_name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        })
    }

    /// The display name of `id`'s profile as platform-wallet holds it, or as
    /// its DashPay snapshot shows it for an identity platform-wallet does not
    /// sync.
    fn synced_display_name(&self, id: Identifier) -> Option<String> {
        use crate::model::dashpay::ProfileState;
        let backend = self.wallet_backend().ok()?;
        backend.dashpay_view().cached_display_name(&id).or_else(|| {
            match &backend.dashpay_snapshot(&id)?.profile {
                ProfileState::Present(profile) => profile
                    .display_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
                ProfileState::Absent | ProfileState::Unknown => None,
            }
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

    /// Serialize profile writes for this owner and network.
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

    /// Revision used to reject a save result queued before a newer save
    /// completed.
    pub fn identity_profile_revision(&self, id: Identifier) -> u64 {
        self.identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .map_or(0, |entry| entry.revision)
    }

    pub(crate) fn profile_snapshot_is_current(
        &self,
        snapshot: &crate::model::dashpay::ProfileSnapshot,
    ) -> bool {
        self.network == snapshot.network
            && self.identity_profile_revision(snapshot.owner) == snapshot.revision
    }

    pub(crate) fn save_identity_profile_name(&self, id: Identifier, name: Option<&str>) -> u64 {
        let mut names = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = names.entry(id).or_default();
        entry.revision += 1;
        entry.accept(name);
        entry.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;

    #[tokio::test]
    async fn profile_writes_are_serialized_per_owner() {
        let dir = tempfile::tempdir().unwrap();
        let context = test_app_context(dir.path());
        let id = Identifier::from([43; 32]);
        let first = context.lock_identity_profile(id).await;
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
            "a save cannot overtake a write still holding the lock"
        );
        let other = context
            .lock_identity_profile(Identifier::from([44; 32]))
            .await;
        drop(other);
        let earlier = context.save_identity_profile_name(id, Some("Earlier"));
        drop(first);
        let save_revision = writer.await.unwrap();
        assert!(save_revision > earlier);
        assert_eq!(context.identity_display_name(id).as_deref(), Some("Saved"));
        assert!(
            !context.profile_snapshot_is_current(&crate::model::dashpay::ProfileSnapshot {
                network: context.network,
                owner: id,
                revision: earlier,
                profile: None
            }),
            "the earlier save's result is stale"
        );
    }

    #[test]
    fn saved_names_show_until_cleared_and_isolate_networks() {
        let dir = tempfile::tempdir().unwrap();
        let context = test_app_context(dir.path());
        let id = Identifier::from([42; 32]);
        assert_eq!(context.identity_display_name(id), None);
        context.save_identity_profile_name(id, Some("Alex Profile"));
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
        context.save_identity_profile_name(id, None);
        assert_eq!(
            context.identity_display_name_or(id, Some("Old profile")),
            None
        );
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

    /// A name synced from the network replaces one saved on this device
    /// earlier, so a change made elsewhere shows.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_synced_name_replaces_a_name_saved_earlier() {
        let dir = tempfile::tempdir().unwrap();
        let context = test_app_context(dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(32);
        context
            .ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
                tx,
                context.egui_ctx().clone(),
            ))
            .await
            .expect("wire wallet backend");
        let id = Identifier::from([45; 32]);
        context.save_identity_profile_name(id, Some("Saved here"));
        context
            .wallet_backend()
            .unwrap()
            .publish_profile_for_test(id, Some("Changed elsewhere"), None)
            .await;
        assert_eq!(
            context.identity_display_name(id).as_deref(),
            Some("Changed elsewhere")
        );
    }
}
