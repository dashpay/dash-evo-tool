//! View-model for the Add Key screen's "Create from wallet" slot chooser.
//!
//! Owns everything the chooser shows — whether a wallet key can be created at
//! all, the load state of the public keys it needs, which slots are in use and
//! which one is selected — and recomputes it only on load, refresh and task
//! results, never per frame. Renders nothing; the screen maps
//! [`ChooserStatus`] to text and widgets.

use std::collections::BTreeSet;

use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

use crate::backend_task::BackendTask;
use crate::backend_task::wallet::WalletTask;
use crate::context::AppContext;
use crate::model::derived_identity_key::{
    default_derivation_index, derivation_index_limit, derivation_wallet, is_derivable_key_type,
    occupied_indices,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::WalletSeedHash;

/// Load cycle of the public keys the chooser needs to tell used slots apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyLoad {
    /// Some keys are missing and no load has been requested yet.
    Cold,
    /// A warm task is in flight.
    Loading,
    /// The last warm task failed (or finished without filling the cache);
    /// waits for an explicit retry.
    Failed,
    /// Every key in range is available.
    Ready,
}

/// What the chooser can show right now, in priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChooserStatus {
    /// The identity has no single wallet path on this device: keys can only be
    /// entered manually.
    NoWallet,
    /// The selected key type cannot be created from a wallet.
    UnsupportedKeyType,
    /// The wallet backend is not available.
    WalletUnavailable,
    /// Waiting for the identity to reload from the network.
    RefreshingIdentity,
    /// The slots' public keys are being loaded.
    Loading,
    /// Loading the slots' public keys failed; a retry is possible.
    LoadFailed,
    /// Every slot in range is used.
    NoFreeSlot,
    /// A free slot is selected.
    Ready,
}

pub struct DerivedKeyChooser {
    /// "Create from wallet" is on. Forced off while [`Self::is_possible`] is
    /// false.
    derived: bool,
    index: Option<u32>,
    wallet: Option<(WalletSeedHash, u32)>,
    derivable_type: bool,
    backend_available: bool,
    load: KeyLoad,
    limit: u32,
    occupied: BTreeSet<u32>,
    /// Slots the backend rejected as used during this screen's lifetime. Kept
    /// out of selection even if the local identity has not caught up yet.
    rejected: BTreeSet<u32>,
    /// The local identity's highest key id; the next added key gets the one
    /// after it.
    max_key_id: u32,
    refreshing_identity: bool,
    pending_identity_refresh: bool,
    identity_protected: bool,
}

impl DerivedKeyChooser {
    /// Build the chooser for `identity`. "Create from wallet" starts on only
    /// when the identity has a wallet path to derive from.
    pub fn new(app_context: &AppContext, identity: &QualifiedIdentity, key_type: KeyType) -> Self {
        let mut chooser = Self {
            derived: false,
            index: None,
            wallet: None,
            derivable_type: is_derivable_key_type(key_type),
            backend_available: false,
            load: KeyLoad::Cold,
            limit: 0,
            occupied: BTreeSet::new(),
            rejected: BTreeSet::new(),
            max_key_id: 0,
            refreshing_identity: false,
            pending_identity_refresh: false,
            identity_protected: false,
        };
        chooser.reload(app_context, identity);
        chooser
    }

    /// Recompute from the (possibly refreshed) identity and the cached public
    /// keys. One cache read; call on load, refresh and task results only.
    ///
    /// An in-flight or failed load is kept as is, so a refresh never starts a
    /// duplicate warm task and a failure waits for an explicit retry.
    pub fn reload(&mut self, app_context: &AppContext, identity: &QualifiedIdentity) {
        let was_possible = self.wallet.is_some();
        self.wallet = derivation_wallet(identity, app_context.network);
        if self.wallet.is_none() {
            self.derived = false;
        } else if !was_possible {
            self.derived = true;
        }
        let max_key_id = identity.identity.get_public_key_max_id();
        self.limit = derivation_index_limit(max_key_id);
        self.max_key_id = max_key_id;
        self.identity_protected = app_context
            .protected_identity_verify_scope(identity)
            .is_ok_and(|scope| scope.is_some());

        let backend = app_context.wallet_backend();
        self.backend_available = backend.is_ok();
        let (Some((seed_hash, identity_index)), Ok(backend)) = (self.wallet, backend) else {
            self.occupied.clear();
            self.index = None;
            return;
        };
        let cache = backend
            .auth_pubkey_cache()
            .get(app_context.network, &seed_hash);
        let warm = (0..self.limit).all(|index| {
            cache
                .get(app_context.network, identity_index, index)
                .is_some()
        });
        if warm {
            self.load = KeyLoad::Ready;
            self.occupied = occupied_indices(
                identity,
                app_context.network,
                seed_hash,
                identity_index,
                &cache,
            );
            self.occupied.extend(self.rejected.iter().copied());
            self.select_default_if_invalid();
        } else {
            if self.load == KeyLoad::Ready {
                self.load = KeyLoad::Cold;
            }
            self.occupied.clear();
            self.index = None;
        }
    }

    /// Update for a key-type change. No storage access.
    pub fn set_key_type(&mut self, key_type: KeyType) {
        self.derivable_type = is_derivable_key_type(key_type);
    }

    /// The warm task to dispatch, if the chooser needs keys it does not have.
    /// Marks the load in flight, so it is returned at most once per cycle.
    pub fn take_warm_task(&mut self) -> Option<BackendTask> {
        if !self.derived
            || !self.derivable_type
            || !self.backend_available
            || self.refreshing_identity
            || self.load != KeyLoad::Cold
        {
            return None;
        }
        let (seed_hash, identity_index) = self.wallet?;
        self.load = KeyLoad::Loading;
        Some(BackendTask::WalletTask(
            WalletTask::WarmIdentityAuthPubkeys {
                seed_hash,
                identity_index,
                key_count: self.limit,
            },
        ))
    }

    /// Whether a warm for `(seed_hash, identity_index)` is this chooser's own.
    fn is_own_warm(&self, seed_hash: &WalletSeedHash, identity_index: u32) -> bool {
        self.wallet == Some((*seed_hash, identity_index))
    }

    /// A warm task for `identity_index` finished. A load that still finds keys
    /// missing becomes a failure instead of re-dispatching, so a lost race can
    /// never loop warm tasks.
    pub fn warm_finished(
        &mut self,
        app_context: &AppContext,
        identity: &QualifiedIdentity,
        identity_index: u32,
    ) {
        if self.load != KeyLoad::Loading
            || self.wallet.is_none_or(|(_, index)| index != identity_index)
        {
            return;
        }
        self.load = KeyLoad::Cold;
        self.reload(app_context, identity);
        if self.load == KeyLoad::Cold {
            self.load = KeyLoad::Failed;
        }
    }

    /// A warm task failed. Returns `true` when it was this chooser's own.
    pub fn warm_failed(&mut self, seed_hash: &WalletSeedHash, identity_index: u32) -> bool {
        if !self.is_own_warm(seed_hash, identity_index) {
            return false;
        }
        if self.load == KeyLoad::Loading {
            self.load = KeyLoad::Failed;
        }
        true
    }

    /// Retry a failed load; the next [`Self::take_warm_task`] dispatches it.
    pub fn retry(&mut self) {
        if self.load == KeyLoad::Failed {
            self.load = KeyLoad::Cold;
        }
    }

    /// The backend refused the selected slot. Keep it out of selection and
    /// reload the identity from the network so slots used elsewhere show up.
    pub fn slot_rejected(&mut self) {
        if let Some(index) = self.index.take() {
            self.rejected.insert(index);
            self.occupied.insert(index);
        }
        self.request_identity_refresh();
    }

    /// The backend could not confirm the selected slot's key and repaired its
    /// cache entry. Re-read slots after an identity reload.
    pub fn key_unconfirmed(&mut self) {
        self.request_identity_refresh();
    }

    /// Hold selection until the identity has been reloaded from the network.
    pub fn request_identity_refresh(&mut self) {
        self.refreshing_identity = true;
        self.pending_identity_refresh = true;
    }

    /// Mark the refresh as already dispatched by the screen itself.
    pub fn await_identity_refresh(&mut self) {
        self.index = None;
        self.refreshing_identity = true;
    }

    /// Whether the screen should dispatch an identity refresh now. Returns
    /// `true` once per request.
    pub fn take_identity_refresh(&mut self) -> bool {
        std::mem::take(&mut self.pending_identity_refresh)
    }

    /// The identity reload finished (successfully or not): resume selection
    /// with whatever the local record now holds.
    pub fn identity_refresh_finished(
        &mut self,
        app_context: &AppContext,
        identity: &QualifiedIdentity,
    ) {
        self.refreshing_identity = false;
        self.reload(app_context, identity);
    }

    fn select_default_if_invalid(&mut self) {
        if self
            .index
            .is_none_or(|index| index >= self.limit || self.occupied.contains(&index))
        {
            self.index = default_derivation_index(self.max_key_id, self.limit, &self.occupied);
        }
    }

    pub fn status(&self) -> ChooserStatus {
        if self.wallet.is_none() {
            ChooserStatus::NoWallet
        } else if !self.derivable_type {
            ChooserStatus::UnsupportedKeyType
        } else if !self.backend_available {
            ChooserStatus::WalletUnavailable
        } else if self.refreshing_identity {
            ChooserStatus::RefreshingIdentity
        } else {
            match self.load {
                KeyLoad::Cold | KeyLoad::Loading => ChooserStatus::Loading,
                KeyLoad::Failed => ChooserStatus::LoadFailed,
                KeyLoad::Ready if self.index.is_none() => ChooserStatus::NoFreeSlot,
                KeyLoad::Ready => ChooserStatus::Ready,
            }
        }
    }

    /// A wallet key can be created for this identity at all.
    pub fn is_possible(&self) -> bool {
        self.wallet.is_some()
    }

    pub fn derived(&self) -> bool {
        self.derived
    }

    /// Mutable toggle for the checkbox; stays off while impossible.
    pub fn derived_mut(&mut self) -> &mut bool {
        if self.wallet.is_none() {
            self.derived = false;
        }
        &mut self.derived
    }

    /// The selected slot, only while it can be submitted.
    pub fn selected_index(&self) -> Option<u32> {
        (self.status() == ChooserStatus::Ready)
            .then_some(self.index)
            .flatten()
    }

    /// Mutable selection for the slot list.
    pub fn index_mut(&mut self) -> &mut Option<u32> {
        &mut self.index
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    pub fn is_occupied(&self, index: u32) -> bool {
        self.occupied.contains(&index)
    }

    /// The slot matching the next key id, when it is free and differs from the
    /// selection: choosing it lets other wallet apps restore the key.
    pub fn suggested_index(&self) -> Option<u32> {
        let next = self.max_key_id.checked_add(1)?;
        (next < self.limit && !self.occupied.contains(&next) && self.index != Some(next))
            .then_some(next)
    }

    /// The identity is password-protected, but a created key is protected by
    /// the wallet instead.
    pub fn identity_protected(&self) -> bool {
        self.identity_protected
    }
}
