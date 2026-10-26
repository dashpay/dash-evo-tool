//! Lock-free DashPay snapshots for the egui frame.
//!
//! platform-wallet's `DashPaySyncManager` keeps every identity's DashPay
//! state (contacts, requests, profiles, payments) in its `ManagedIdentity`
//! and refreshes it in the background. The frame cannot await that state, so
//! [`WalletBackend::refresh_dashpay_snapshots`] reads it off the UI thread,
//! combines it with this device's own records, and publishes one
//! [`DashPaySnapshot`] per identity that
//! [`WalletBackend::dashpay_snapshot`] serves without blocking.
//!
//! Snapshots are rebuilt after every completed sync pass and after every
//! DashPay write this app makes, so a write is visible as soon as it lands.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;
use dash_sdk::platform::Identifier;
use platform_wallet::wallet::identity::state::DashPayState;
use platform_wallet::wallet::identity::types::dashpay::contact_request::ContactRequest;
use platform_wallet::wallet::identity::types::dashpay::profile::DashPayProfile;

use super::WalletBackend;
use super::dashpay::{kv_payment_timestamps, payment_to_det, profile_to_det};
use crate::model::dashpay::{
    ContactData, ContactDetails, DashPaySnapshot, PendingContactRequest, ProfileState,
    StoredPayment, StoredProfile, payment_txid_from_storage_key, shown_contact_details,
};

/// The published snapshots and what refreshing them needs to remember.
#[derive(Default)]
pub(super) struct DashPaySnapshots {
    by_owner: ArcSwap<BTreeMap<Identifier, Arc<DashPaySnapshot>>>,
    /// Serializes rebuilds, so a rebuild that read state before a write can
    /// never publish after the rebuild that follows the write.
    rebuild: tokio::sync::Mutex<()>,
    /// The sync pass the snapshots were last refreshed after, in Unix seconds;
    /// `0` before the first.
    refreshed_after_pass: AtomicU64,
    /// Profiles of identities platform-wallet does not sync: those no wallet
    /// manages, and those whose wallet is not loaded (a locked one).
    // TODO(upstream): wallet-less identities are not synced; drop this once
    // platform-wallet syncs the profiles of its unowned identities.
    unsynced_profiles: std::sync::Mutex<BTreeMap<Identifier, ProfileState>>,
    /// Counterparties whose username was looked up this session.
    usernames_looked_up: std::sync::Mutex<BTreeSet<Identifier>>,
}

/// This device's records a snapshot combines with platform-wallet's state.
pub(super) trait SnapshotOverlays {
    /// The owner withdrew the request they sent to `counterparty`.
    fn is_withdrawn(&self, owner: &Identifier, counterparty: &Identifier) -> bool;
    /// The owner declined the request `counterparty` sent them.
    fn is_declined(&self, owner: &Identifier, counterparty: &Identifier) -> bool;
    /// platform-wallet held the owner's details for `contact` at some point.
    fn details_synced(&self, owner: &Identifier, contact: &Identifier) -> bool;
    /// The private memo this device kept for `contact`.
    fn private_memo(&self, owner: &Identifier, contact: &Identifier) -> Option<ContactDetails>;
    fn username(&self, identity: &Identifier) -> Option<String>;
    /// When this device first saw the payment, and when it confirmed.
    fn payment_times(&self, storage_key: &str) -> (i64, Option<i64>);
    /// When the owner's own profile was created and last updated.
    fn profile_times(&self, owner: &Identifier) -> (i64, i64);
}

/// One identity's snapshot from its synced state and this device's records.
///
/// `profile_synced` says whether a sync pass has run, which tells a profile
/// that was never published apart from one not yet synced. `own_profiles`
/// holds the profiles of every identity this app manages: platform-wallet
/// does not keep a contact profile for a contact that is one of them.
pub(super) fn build_snapshot(
    owner: &Identifier,
    dashpay: &DashPayState,
    profile_synced: bool,
    own_profiles: &BTreeMap<Identifier, DashPayProfile>,
    overlays: &impl SnapshotOverlays,
) -> DashPaySnapshot {
    let profile = match &dashpay.profile {
        Some(profile) => {
            let (created_at, updated_at) = overlays.profile_times(owner);
            ProfileState::Present(Box::new(profile_to_det(
                owner, profile, created_at, updated_at,
            )))
        }
        None if profile_synced => ProfileState::Absent,
        None => ProfileState::Unknown,
    };
    let profile_of = |id: &Identifier| {
        dashpay
            .contact_profiles
            .get(id)
            .and_then(|entry| entry.profile.as_ref())
            .or_else(|| own_profiles.get(id))
    };
    let established = dashpay.established_contacts();
    // A reply to a withdrawn request is a new request to answer, not a
    // contact, even though platform-wallet established the pair.
    let is_contact =
        |id: &Identifier| established.contains_key(id) && !overlays.is_withdrawn(owner, id);

    let contacts = established
        .values()
        .filter(|contact| {
            contact.contact_identity_id != *owner && is_contact(&contact.contact_identity_id)
        })
        .map(|contact| {
            let id = contact.contact_identity_id;
            let local = ContactDetails {
                nickname: contact.alias.clone(),
                note: contact.note.clone(),
                hidden: contact.is_hidden,
            };
            let details = shown_contact_details(
                None,
                Some(local),
                overlays.details_synced(owner, &id),
                overlays.private_memo(owner, &id),
            );
            let profile = profile_of(&id);
            ContactData {
                identity_id: id,
                nickname: details.nickname,
                note: details.note,
                is_hidden: details.hidden,
                account_reference: contact.accepted_accounts.first().copied().unwrap_or(0),
                username: overlays.username(&id),
                display_name: profile.and_then(|p| p.display_name.clone()),
                avatar_url: profile.and_then(|p| p.avatar_url.clone()),
                bio: profile.and_then(|p| p.bio.clone()),
            }
        })
        .collect();

    let pending = |counterparty: &Identifier, request: &ContactRequest| {
        let profile = profile_of(counterparty);
        PendingContactRequest {
            counterparty_id: *counterparty,
            created_at_ms: request.created_at,
            account_reference: request.account_reference,
            has_auto_accept_proof: request.auto_accept_proof.is_some(),
            username: overlays.username(counterparty),
            display_name: profile.and_then(|p| p.display_name.clone()),
            avatar_url: profile.and_then(|p| p.avatar_url.clone()),
        }
    };
    // platform-wallet may keep an answered request in its incoming map or
    // only inside the established contact, so both are read.
    let mut incoming: BTreeMap<Identifier, PendingContactRequest> = dashpay
        .incoming_contact_requests()
        .iter()
        .map(|(sender, request)| (*sender, pending(sender, request)))
        .collect();
    for contact in established.values() {
        incoming
            .entry(contact.contact_identity_id)
            .or_insert_with(|| pending(&contact.contact_identity_id, &contact.incoming_request));
    }
    let mut incoming: Vec<PendingContactRequest> = incoming
        .into_values()
        .filter(|request| {
            let sender = &request.counterparty_id;
            *sender != *owner && !is_contact(sender) && !overlays.is_declined(owner, sender)
        })
        .collect();
    incoming.sort_by_key(|request| request.created_at_ms);

    let mut outgoing: Vec<PendingContactRequest> = dashpay
        .sent_contact_requests()
        .iter()
        .filter(|(recipient, _)| {
            **recipient != *owner
                && !established.contains_key(*recipient)
                && !overlays.is_withdrawn(owner, recipient)
        })
        .map(|(recipient, request)| pending(recipient, request))
        .collect();
    outgoing.sort_by_key(|request| request.created_at_ms);

    DashPaySnapshot {
        profile,
        contacts,
        incoming,
        outgoing,
        payments: payments(owner, dashpay, overlays),
    }
}

/// The owner's payments, newest first.
///
/// Older versions recorded each incoming payment under `txid:vout`;
/// platform-wallet records the same payment under the bare txid. Where both
/// exist the older entry is kept, since it carries the time this device saw
/// the payment.
// TODO(upstream): per-output payment keys; until platform-wallet keys
// payments per output, the older composite entries shadow its bare-txid twin.
fn payments(
    owner: &Identifier,
    dashpay: &DashPayState,
    overlays: &impl SnapshotOverlays,
) -> Vec<StoredPayment> {
    let composite_txids: BTreeSet<&str> = dashpay
        .payments
        .keys()
        .map(String::as_str)
        .filter(|key| payment_txid_from_storage_key(key) != *key)
        .map(payment_txid_from_storage_key)
        .collect();
    let mut out: Vec<StoredPayment> = dashpay
        .payments
        .iter()
        .filter(|(key, _)| !composite_txids.contains(key.as_str()))
        .map(|(key, entry)| payment_to_det(owner, key, entry, overlays.payment_times(key)))
        .collect();
    out.sort_by_key(|payment| std::cmp::Reverse(payment.created_at));
    out
}

impl SnapshotOverlays for WalletBackend {
    fn is_withdrawn(&self, owner: &Identifier, counterparty: &Identifier) -> bool {
        self.dashpay_is_withdrawn(owner, counterparty)
    }

    fn is_declined(&self, owner: &Identifier, counterparty: &Identifier) -> bool {
        self.dashpay_is_declined(owner, counterparty)
    }

    fn details_synced(&self, owner: &Identifier, contact: &Identifier) -> bool {
        self.dashpay_details_synced(owner, contact)
            .unwrap_or_else(|error| {
                tracing::debug!(?error, "DashPay snapshot: details-synced record not read");
                false
            })
    }

    fn private_memo(&self, owner: &Identifier, contact: &Identifier) -> Option<ContactDetails> {
        self.dashpay_get_private_info(owner, contact)
            .unwrap_or_else(|error| {
                tracing::debug!(?error, "DashPay snapshot: private memo not read");
                None
            })
            .map(|memo| ContactDetails {
                nickname: Some(memo.nickname).filter(|s| !s.is_empty()),
                note: Some(memo.notes).filter(|s| !s.is_empty()),
                hidden: memo.is_hidden,
            })
    }

    fn username(&self, identity: &Identifier) -> Option<String> {
        self.contact_usernames().get(identity)
    }

    fn payment_times(&self, storage_key: &str) -> (i64, Option<i64>) {
        kv_payment_timestamps(&self.kv(), storage_key)
    }

    fn profile_times(&self, owner: &Identifier) -> (i64, i64) {
        self.inner.profile_timestamps.get(&self.kv(), owner)
    }
}

impl WalletBackend {
    /// The latest DashPay snapshot of `owner`, or `None` before one was
    /// built (no wallet manages `owner`, or the first refresh has not run).
    /// Lock-free; safe to call every frame.
    pub fn dashpay_snapshot(&self, owner: &Identifier) -> Option<Arc<DashPaySnapshot>> {
        self.inner
            .dashpay_snapshots
            .by_owner
            .load()
            .get(owner)
            .cloned()
    }

    /// Whether a sync pass completed since the snapshots were last refreshed
    /// after one. Cheap; safe to call every frame.
    pub(crate) fn dashpay_pass_completed(&self) -> bool {
        let pass = self
            .inner
            .pwm
            .dashpay_sync()
            .last_sync_unix_seconds()
            .unwrap_or(0);
        pass != self
            .inner
            .dashpay_snapshots
            .refreshed_after_pass
            .load(Ordering::Acquire)
    }

    /// Record that the snapshots are being refreshed after the latest
    /// completed pass; returns whether that pass is new.
    pub(crate) fn take_dashpay_pass(&self) -> bool {
        let pass = self
            .inner
            .pwm
            .dashpay_sync()
            .last_sync_unix_seconds()
            .unwrap_or(0);
        self.inner
            .dashpay_snapshots
            .refreshed_after_pass
            .swap(pass, Ordering::AcqRel)
            != pass
    }

    /// Whether DashPay reads may go to the network: the masternode list is
    /// synced, so proofs verify. Sent earlier they get the queried nodes
    /// banned.
    pub(crate) fn dashpay_network_ready(&self) -> bool {
        self.inner.coordinator_gate.has_fired()
    }

    /// Run one DashPay sync pass now, unless the network is not ready
    /// ([`Self::dashpay_network_ready`]) or a pass is already running.
    pub(crate) async fn dashpay_sync_now(&self) {
        if self.dashpay_network_ready() {
            self.inner.pwm.dashpay_sync().sync_now().await;
        }
    }

    /// Rebuild every snapshot from platform-wallet's current state and this
    /// device's records; returns whether any changed.
    pub(crate) async fn refresh_dashpay_snapshots(&self) -> bool {
        let snapshots = &self.inner.dashpay_snapshots;
        let _rebuild = snapshots.rebuild.lock().await;
        let wallets: Vec<Arc<platform_wallet::PlatformWallet>> = match self.inner.wallets.read() {
            Ok(map) => map.values().cloned().collect(),
            Err(_) => {
                tracing::warn!("DashPay snapshot refresh skipped: the wallet map is unreadable");
                return false;
            }
        };
        // Copied out so the overlay reads below hold no wallet lock.
        let mut states: Vec<(Identifier, DashPayState)> = Vec::new();
        for wallet in wallets {
            let state = wallet.state().await;
            states.extend(
                state
                    .identity_manager
                    .managed_identities()
                    .map(|managed| (managed.id(), managed.dashpay().clone())),
            );
        }
        let own_profiles: BTreeMap<Identifier, DashPayProfile> = states
            .iter()
            .filter_map(|(id, state)| Some((*id, state.profile.clone()?)))
            .collect();
        let profile_synced = self
            .inner
            .pwm
            .dashpay_sync()
            .last_sync_unix_seconds()
            .is_some();
        let mut rebuilt: BTreeMap<Identifier, Arc<DashPaySnapshot>> = states
            .iter()
            .map(|(owner, state)| {
                let snapshot = build_snapshot(owner, state, profile_synced, &own_profiles, self);
                (*owner, Arc::new(snapshot))
            })
            .collect();
        let unsynced = snapshots
            .unsynced_profiles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for (owner, profile) in unsynced {
            rebuilt.entry(owner).or_insert_with(|| {
                Arc::new(DashPaySnapshot {
                    profile,
                    ..Default::default()
                })
            });
        }
        let current = snapshots.by_owner.load();
        let changed = rebuilt.len() != current.len()
            || rebuilt
                .iter()
                .any(|(owner, snapshot)| current.get(owner).is_none_or(|old| old != snapshot));
        if changed {
            snapshots.by_owner.store(Arc::new(rebuilt));
        }
        changed
    }

    /// Publish `owner`'s DashPay profile as a read of an unsynced identity
    /// would: `display_name` and `avatar_url`, or no profile when
    /// `display_name` is `None`.
    #[cfg(any(test, feature = "testing"))]
    pub async fn publish_profile_for_test(
        &self,
        owner: Identifier,
        display_name: Option<&str>,
        avatar_url: Option<&str>,
    ) {
        let profile = display_name.map(|name| StoredProfile {
            identity_id: owner.to_vec(),
            display_name: Some(name.to_owned()),
            bio: None,
            avatar_url: avatar_url.map(str::to_owned),
            avatar_hash: None,
            avatar_fingerprint: None,
            avatar_bytes: None,
            public_message: None,
            created_at: 0,
            updated_at: 0,
        });
        self.record_unsynced_profile(owner, profile);
        self.refresh_dashpay_snapshots().await;
    }

    /// Record the profile of an identity platform-wallet does not sync, or
    /// that it has none (`None`). A synced snapshot of the identity wins.
    pub(crate) fn record_unsynced_profile(
        &self,
        owner: Identifier,
        profile: Option<StoredProfile>,
    ) {
        let state = match profile {
            Some(profile) => ProfileState::Present(Box::new(profile)),
            None => ProfileState::Absent,
        };
        self.inner
            .dashpay_snapshots
            .unsynced_profiles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(owner, state);
    }

    /// Whether the profile of the unsynced identity `owner` was read.
    pub(crate) fn unsynced_profile_known(&self, owner: &Identifier) -> bool {
        self.inner
            .dashpay_snapshots
            .unsynced_profiles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(owner)
    }

    /// The counterparties in the snapshots whose username was not looked up
    /// this session; marks them as looked up.
    pub(crate) fn take_usernames_to_look_up(&self) -> Vec<Identifier> {
        let snapshots = self.inner.dashpay_snapshots.by_owner.load();
        let mut looked_up = self
            .inner
            .dashpay_snapshots
            .usernames_looked_up
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let counterparties: BTreeSet<Identifier> = snapshots
            .values()
            .flat_map(|snapshot| {
                snapshot
                    .contacts
                    .iter()
                    .map(|contact| contact.identity_id)
                    .chain(snapshot.incoming.iter().map(|r| r.counterparty_id))
                    .chain(snapshot.outgoing.iter().map(|r| r.counterparty_id))
            })
            .collect();
        counterparties
            .into_iter()
            .filter(|id| looked_up.insert(*id))
            .collect()
    }

    /// Stamp the payments this device has no time for with `now_ms`, the
    /// first time it sees them.
    pub(crate) fn stamp_unseen_dashpay_payments(&self, now_ms: i64) {
        let snapshots = self.inner.dashpay_snapshots.by_owner.load();
        for payment in snapshots.values().flat_map(|s| s.payments.iter()) {
            if payment.created_at != 0 {
                continue;
            }
            let confirmed_at = matches!(
                payment.status,
                crate::model::dashpay::PaymentStatus::Confirmed
            )
            .then_some(now_ms);
            // An entry keyed `txid:vout` always has a time, so the bare txid
            // is its key.
            if let Err(error) =
                self.dashpay_set_payment_timestamps(&payment.tx_id, now_ms, confirmed_at)
            {
                tracing::debug!(?error, "DashPay payment time not recorded");
            }
        }
    }

    /// Stamp the synced own profiles this device has no time for with
    /// `now_ms`, the first time it sees them.
    pub(crate) fn stamp_unseen_dashpay_profiles(&self, now_ms: i64) {
        let snapshots = self.inner.dashpay_snapshots.by_owner.load();
        for (owner, snapshot) in snapshots.iter() {
            if let ProfileState::Present(profile) = &snapshot.profile
                && profile.created_at == 0
                && let Err(error) = self.dashpay_initialize_profile_timestamps(owner, now_ms)
            {
                tracing::debug!(?error, "DashPay profile time not recorded");
            }
        }
    }

    /// Forget every snapshot and session lookup, as when this network's
    /// wallets are cleared.
    pub(super) fn clear_dashpay_snapshots(&self) {
        let snapshots = &self.inner.dashpay_snapshots;
        snapshots.by_owner.store(Arc::default());
        snapshots
            .unsynced_profiles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        snapshots
            .usernames_looked_up
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform_wallet::wallet::identity::types::dashpay::payment::PaymentEntry;

    fn id(byte: u8) -> Identifier {
        Identifier::from([byte; 32])
    }

    #[derive(Default)]
    struct Overlays {
        withdrawn: BTreeSet<Identifier>,
        declined: BTreeSet<Identifier>,
        synced: BTreeSet<Identifier>,
        memos: BTreeMap<Identifier, ContactDetails>,
        usernames: BTreeMap<Identifier, String>,
        payment_times: BTreeMap<String, i64>,
    }

    impl SnapshotOverlays for Overlays {
        fn is_withdrawn(&self, _: &Identifier, counterparty: &Identifier) -> bool {
            self.withdrawn.contains(counterparty)
        }
        fn is_declined(&self, _: &Identifier, counterparty: &Identifier) -> bool {
            self.declined.contains(counterparty)
        }
        fn details_synced(&self, _: &Identifier, contact: &Identifier) -> bool {
            self.synced.contains(contact)
        }
        fn private_memo(&self, _: &Identifier, contact: &Identifier) -> Option<ContactDetails> {
            self.memos.get(contact).cloned()
        }
        fn username(&self, identity: &Identifier) -> Option<String> {
            self.usernames.get(identity).cloned()
        }
        fn payment_times(&self, storage_key: &str) -> (i64, Option<i64>) {
            (
                self.payment_times.get(storage_key).copied().unwrap_or(0),
                None,
            )
        }
        fn profile_times(&self, _: &Identifier) -> (i64, i64) {
            (0, 0)
        }
    }

    fn request(sender: Identifier, recipient: Identifier, created_at: u64) -> ContactRequest {
        ContactRequest::new(sender, recipient, 0, 0, 0, Vec::new(), 0, created_at)
    }

    /// The DashPay state `setup` records for `owner`.
    fn state_with(
        owner: Identifier,
        setup: impl FnOnce(&mut ManagedIdentityFixture),
    ) -> DashPayState {
        let mut fixture = ManagedIdentityFixture::new(owner);
        setup(&mut fixture);
        fixture.dashpay()
    }

    struct NoopPersister;

    impl platform_wallet::changeset::PlatformWalletPersistence for NoopPersister {
        fn store(
            &self,
            _: crate::wallet_backend::WalletId,
            _: platform_wallet::changeset::PlatformWalletChangeSet,
        ) -> Result<(), platform_wallet::changeset::PersistenceError> {
            Ok(())
        }

        fn flush(
            &self,
            _: crate::wallet_backend::WalletId,
        ) -> Result<(), platform_wallet::changeset::PersistenceError> {
            Ok(())
        }

        fn load(
            &self,
        ) -> Result<
            platform_wallet::changeset::ClientStartState,
            platform_wallet::changeset::PersistenceError,
        > {
            Ok(Default::default())
        }
    }

    /// Builds a `DashPayState` through upstream's own mutators, so the
    /// fixture holds the invariants platform-wallet maintains.
    struct ManagedIdentityFixture {
        owner: Identifier,
        managed: platform_wallet::wallet::identity::state::ManagedIdentity,
        persister: platform_wallet::WalletPersister,
    }

    impl ManagedIdentityFixture {
        fn new(owner: Identifier) -> Self {
            use dash_sdk::dpp::identity::{Identity, v0::IdentityV0};
            let identity = Identity::V0(IdentityV0 {
                id: owner,
                public_keys: BTreeMap::new(),
                balance: 0,
                revision: 0,
            });
            Self {
                owner,
                managed: platform_wallet::wallet::identity::state::ManagedIdentity::new(
                    identity, 0,
                ),
                persister: platform_wallet::WalletPersister::new([0; 32], Arc::new(NoopPersister)),
            }
        }

        fn sent(&mut self, recipient: Identifier, created_at: u64) {
            self.managed
                .add_sent_contact_request(
                    request(self.owner, recipient, created_at),
                    &self.persister,
                )
                .expect("sent request recorded");
        }

        fn received(&mut self, sender: Identifier, created_at: u64) {
            self.managed
                .add_incoming_contact_request(
                    request(sender, self.owner, created_at),
                    &self.persister,
                )
                .expect("incoming request recorded");
        }

        fn payment(&mut self, key: &str, entry: PaymentEntry) {
            self.managed
                .record_dashpay_payment(key.to_owned(), entry, &self.persister)
                .expect("payment recorded");
        }

        fn dashpay(&self) -> DashPayState {
            self.managed.dashpay().clone()
        }
    }

    fn build(owner: Identifier, state: &DashPayState, overlays: &Overlays) -> DashPaySnapshot {
        build_snapshot(&owner, state, true, &BTreeMap::new(), overlays)
    }

    #[test]
    fn requests_both_ways_make_a_contact_and_no_pending_request() {
        let (owner, peer) = (id(1), id(2));
        let state = state_with(owner, |f| {
            f.sent(peer, 10);
            f.received(peer, 20);
        });
        let snapshot = build(owner, &state, &Overlays::default());
        assert_eq!(
            snapshot
                .contacts
                .iter()
                .map(|c| c.identity_id)
                .collect::<Vec<_>>(),
            vec![peer]
        );
        assert!(snapshot.incoming.is_empty());
        assert!(snapshot.outgoing.is_empty());
    }

    #[test]
    fn unanswered_requests_are_pending_in_their_direction() {
        let (owner, to, from) = (id(1), id(2), id(3));
        let state = state_with(owner, |f| {
            f.sent(to, 10);
            f.received(from, 20);
        });
        let snapshot = build(owner, &state, &Overlays::default());
        assert!(snapshot.contacts.is_empty());
        assert_eq!(snapshot.outgoing.len(), 1);
        assert_eq!(snapshot.outgoing[0].counterparty_id, to);
        assert_eq!(snapshot.outgoing[0].created_at_ms, 10);
        assert_eq!(snapshot.incoming.len(), 1);
        assert_eq!(snapshot.incoming[0].counterparty_id, from);
    }

    #[test]
    fn a_withdrawn_request_is_hidden_and_a_reply_to_it_is_a_new_request() {
        let (owner, peer, other) = (id(1), id(2), id(3));
        let state = state_with(owner, |f| {
            f.sent(other, 5);
            f.sent(peer, 10);
            f.received(peer, 20);
        });
        let overlays = Overlays {
            withdrawn: [peer, other].into(),
            ..Default::default()
        };
        let snapshot = build(owner, &state, &overlays);
        assert!(snapshot.contacts.is_empty(), "the reply is not a contact");
        assert!(
            snapshot.outgoing.is_empty(),
            "withdrawn requests are hidden"
        );
        assert_eq!(
            snapshot
                .incoming
                .iter()
                .map(|r| r.counterparty_id)
                .collect::<Vec<_>>(),
            vec![peer],
            "the reply waits for an answer"
        );
    }

    #[test]
    fn a_declined_request_is_hidden() {
        let (owner, sender) = (id(1), id(2));
        let state = state_with(owner, |f| f.received(sender, 20));
        let overlays = Overlays {
            declined: [sender].into(),
            ..Default::default()
        };
        assert!(build(owner, &state, &overlays).incoming.is_empty());
    }

    #[test]
    fn a_contact_shows_the_memo_until_platform_wallet_held_details() {
        let (owner, peer) = (id(1), id(2));
        let state = state_with(owner, |f| {
            f.sent(peer, 10);
            f.received(peer, 20);
        });
        let memo = ContactDetails {
            nickname: Some("Bobby".into()),
            note: Some("from the old list".into()),
            hidden: false,
        };
        let mut overlays = Overlays {
            memos: [(peer, memo.clone())].into(),
            usernames: [(peer, "bob".to_owned())].into(),
            ..Default::default()
        };
        let contact = build(owner, &state, &overlays).contacts.remove(0);
        assert_eq!(contact.details(), memo);
        assert_eq!(contact.username.as_deref(), Some("bob"));

        overlays.synced.insert(peer);
        let contact = build(owner, &state, &overlays).contacts.remove(0);
        assert_eq!(
            contact.details(),
            ContactDetails::default(),
            "cleared details stay cleared once platform-wallet held them"
        );
    }

    #[test]
    fn the_profile_is_unknown_until_a_sync_pass_ran() {
        let owner = id(1);
        let state = DashPayState::default();
        let overlays = Overlays::default();
        let unsynced = build_snapshot(&owner, &state, false, &BTreeMap::new(), &overlays);
        assert_eq!(unsynced.profile, ProfileState::Unknown);
        assert_eq!(
            build(owner, &state, &overlays).profile,
            ProfileState::Absent
        );
    }

    #[test]
    fn an_older_per_output_payment_shadows_its_bare_txid_twin() {
        let (owner, peer) = (id(1), id(2));
        let state = state_with(owner, |f| {
            f.payment("aa:0", PaymentEntry::new_received(peer, 1_000, None));
            f.payment("aa", PaymentEntry::new_received(peer, 1_000, None));
            f.payment("bb", PaymentEntry::new_sent(peer, 500, Some("rent".into())));
        });
        let overlays = Overlays {
            payment_times: [("aa:0".to_owned(), 100), ("bb".to_owned(), 200)].into(),
            ..Default::default()
        };
        let payments = build(owner, &state, &overlays).payments;
        assert_eq!(
            payments
                .iter()
                .map(|p| (p.tx_id.as_str(), p.created_at))
                .collect::<Vec<_>>(),
            vec![("bb", 200), ("aa", 100)],
            "one row per payment, newest first"
        );
    }

    /// End to end through the wallet backend: a contact's details held in
    /// platform-wallet win over this device's older copy in the snapshot.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_snapshot_shows_the_details_platform_wallet_holds() {
        use crate::model::wallet::Wallet;
        use crate::model::wallet::birth_height::WalletOrigin;
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;

        let dir = tempfile::tempdir().expect("temp dir");
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = context.wallet_backend().expect("backend wired");
        let seed = [0x4Du8; 64];
        let wallet =
            Wallet::new_from_seed(seed, Network::Testnet, Some("w".into()), None).expect("wallet");
        let (seed_hash, _wallet) = context
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");
        backend
            .register_wallet_from_seed(&seed_hash, &seed, Some(0))
            .await
            .expect("register upstream");
        let (owner, contact) = (id(0x11), id(0x22));
        let identity =
            Identity::create_basic_identity(owner, PlatformVersion::latest()).expect("identity");
        backend
            .ensure_identity_managed(&seed_hash, &identity, 0)
            .await
            .expect("manage identity");
        let request = |from, to| {
            platform_wallet::ContactRequest::new(from, to, 0, 0, 0, vec![0u8; 96], 100_000, 1)
        };
        backend
            .record_sent_contact_request(&seed_hash, &owner, request(owner, contact))
            .await
            .expect("record sent request");
        backend
            .record_incoming_contact_request(&seed_hash, &owner, request(contact, owner))
            .await
            .expect("record incoming request");
        // This device's older copy.
        backend
            .dashpay_set_private_info(
                &owner,
                &contact,
                &crate::model::dashpay::ContactPrivateInfo {
                    nickname: "Old".to_string(),
                    notes: "old note".to_string(),
                    is_hidden: false,
                },
            )
            .expect("sidecar written");
        let synced = ContactDetails {
            nickname: Some("Bao".into()),
            note: None,
            hidden: true,
        };
        backend
            .set_contact_details_for_tests(&seed_hash, &owner, &contact, synced.clone())
            .await;

        backend.refresh_dashpay_snapshots().await;

        let snapshot = backend.dashpay_snapshot(&owner).expect("owner's snapshot");
        let [row] = snapshot.contacts.as_slice() else {
            panic!("expected one contact, got {:?}", snapshot.contacts);
        };
        assert_eq!(row.details(), synced);
    }
}
