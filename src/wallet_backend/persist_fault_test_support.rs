//! Fault injection for the account-registration persist path, faithful to the
//! [`PlatformWalletPersistence`] retry contract.
//!
//! Decorates the real [`SqlitePersister`] so a test can make individual
//! `store` / `flush` calls fail with a chosen [`PersistenceErrorKind`] while
//! every other call reaches the real database. The injected failures honour the
//! trait's own contract: a `Transient` failure parks the changeset so a later
//! bare `flush` still commits it, a terminal failure discards it. Assertions
//! can therefore be made against real persisted rows rather than a fake's
//! bookkeeping.
//!
//! Only `store`, `flush` and `load` are routed — the decorator is used at one
//! call site (the identity-funding account registration write) and the trait's
//! defaults cover the rest.

use std::collections::VecDeque;
use std::sync::Mutex;

use platform_wallet::changeset::{
    ClientStartState, Merge, PersistenceCapabilities, PersistenceError, PersistenceErrorKind,
    PlatformWalletChangeSet, PlatformWalletPersistence,
};
use platform_wallet::wallet::platform_wallet::WalletId;
use platform_wallet_storage::SqlitePersister;

/// Queued faults plus the changeset a transient fault left staged.
///
/// Inert until [`arm`](Self::arm) queues something, so production-shaped tests
/// pay nothing for its presence.
#[derive(Default)]
pub(crate) struct PersistFaults {
    queue: Mutex<VecDeque<PersistenceErrorKind>>,
    staged: Mutex<PlatformWalletChangeSet>,
}

impl PersistFaults {
    /// Queue `kinds`, served one per `store` / `flush` call in order.
    pub(crate) fn arm(&self, kinds: impl IntoIterator<Item = PersistenceErrorKind>) {
        self.queue.lock().expect("fault queue").extend(kinds);
    }

    /// `true` while an injected transient failure still holds an uncommitted
    /// changeset — the persister-buffer state a caller must not discard.
    pub(crate) fn has_staged_changeset(&self) -> bool {
        !self.staged.lock().expect("staged changeset").is_empty()
    }

    fn next(&self) -> Option<PersistenceErrorKind> {
        self.queue.lock().expect("fault queue").pop_front()
    }

    fn stage(&self, changeset: PlatformWalletChangeSet) {
        self.staged
            .lock()
            .expect("staged changeset")
            .merge(changeset);
    }

    fn take_staged(&self) -> PlatformWalletChangeSet {
        std::mem::take(&mut *self.staged.lock().expect("staged changeset"))
    }
}

/// Borrowed decorator over the real persister; cheap to build per call.
pub(crate) struct PersistFaultInjector<'a> {
    inner: &'a SqlitePersister,
    faults: &'a PersistFaults,
}

impl<'a> PersistFaultInjector<'a> {
    pub(crate) fn new(inner: &'a SqlitePersister, faults: &'a PersistFaults) -> Self {
        Self { inner, faults }
    }

    fn injected(kind: PersistenceErrorKind) -> PersistenceError {
        PersistenceError::backend_with_kind(kind, "injected persistence fault")
    }
}

impl PlatformWalletPersistence for PersistFaultInjector<'_> {
    fn persistence_capabilities(&self) -> PersistenceCapabilities {
        self.inner.persistence_capabilities()
    }

    fn store(
        &self,
        wallet_id: WalletId,
        changeset: PlatformWalletChangeSet,
    ) -> Result<(), PersistenceError> {
        match self.faults.next() {
            // Contract: a transient `store` MUST preserve the changeset so a
            // later bare `flush` completes the write without re-supplying it.
            Some(PersistenceErrorKind::Transient) => {
                self.faults.stage(changeset);
                Err(Self::injected(PersistenceErrorKind::Transient))
            }
            Some(kind) => Err(Self::injected(kind)),
            None => {
                self.faults.stage(changeset);
                self.inner.store(wallet_id, self.faults.take_staged())
            }
        }
    }

    fn flush(&self, wallet_id: WalletId) -> Result<(), PersistenceError> {
        match self.faults.next() {
            Some(PersistenceErrorKind::Transient) => {
                Err(Self::injected(PersistenceErrorKind::Transient))
            }
            // Contract: a terminal failure drops the buffer — the staged
            // changeset is gone and no retry can recover it.
            Some(kind) => {
                self.faults.take_staged();
                Err(Self::injected(kind))
            }
            None => {
                let staged = self.faults.take_staged();
                if !staged.is_empty() {
                    self.inner.store(wallet_id, staged)?;
                }
                self.inner.flush(wallet_id)
            }
        }
    }

    fn load(&self) -> Result<ClientStartState, PersistenceError> {
        self.inner.load()
    }
}
