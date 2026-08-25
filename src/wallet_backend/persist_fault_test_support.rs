//! Fault injection for the account-registration persist path, faithful to the
//! [`PlatformWalletPersistence`] retry contract.
//!
//! Decorates the real [`SqlitePersister`] so a test can make individual
//! `store` calls fail with a chosen [`PersistenceErrorKind`] while every other
//! call reaches the real database. The injected failures mirror what the real
//! persister does with the buffer: a `Transient` failure parks the changeset
//! so a later write still commits it, and a terminal failure drops the whole
//! per-wallet buffer — including anything a previous transient failure had
//! staged, because `flush_inner` takes the merged buffer and
//! `handle_flush_error` restores it only for transient errors. Assertions can
//! therefore be made against real persisted rows rather than a fake's
//! bookkeeping.
//!
//! The staged buffer is keyed by [`WalletId`], matching the real `Buffer`'s
//! `HashMap<WalletId, PlatformWalletChangeSet>`: an unkeyed buffer would let a
//! two-wallet test commit one wallet's changeset under another wallet's id and
//! still pass.
//!
//! Only `store` faults, and only `store_commits_inline`, `store`, `flush` and
//! `load` are routed at all — the trait's defaults cover the rest. The
//! decorator has a single call site, the identity-funding account registration
//! write, and that path issues one `store` per attempt and never flushes: the
//! decorated persister answers `store_commits_inline`, so a successful `store`
//! is already the commit. A
//! flush-time fault would therefore be unreachable rather than merely unused,
//! so `flush` is a plain passthrough — it still completes a changeset an
//! injected transient failure parked, per the trait's contract — and
//! [`store_calls`](PersistFaults::store_calls) /
//! [`flush_calls`](PersistFaults::flush_calls) are what a test pins that shape
//! with.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use platform_wallet::changeset::{
    ClientStartState, Merge, PersistenceCapabilities, PersistenceError, PersistenceErrorKind,
    PlatformWalletChangeSet, PlatformWalletPersistence,
};
use platform_wallet::wallet::platform_wallet::WalletId;
use platform_wallet_storage::SqlitePersister;

/// Queued faults plus the per-wallet changesets a transient fault left staged.
///
/// Inert until [`arm`](Self::arm) queues something, so production-shaped tests
/// pay nothing for its presence.
#[derive(Default)]
pub(crate) struct PersistFaults {
    queue: Mutex<VecDeque<PersistenceErrorKind>>,
    staged: Mutex<HashMap<WalletId, PlatformWalletChangeSet>>,
    /// One-based index of the `store` call before which the staged buffer is
    /// dropped; `0` disables the probe.
    discard_before_write: std::sync::atomic::AtomicUsize,
    store_calls: std::sync::atomic::AtomicUsize,
    flush_calls: std::sync::atomic::AtomicUsize,
}

impl PersistFaults {
    /// Queue `kinds`, served one per `store` call in order.
    pub(crate) fn arm(&self, kinds: impl IntoIterator<Item = PersistenceErrorKind>) {
        self.queue.lock().expect("fault queue").extend(kinds);
    }

    /// `true` while an injected transient failure still holds an uncommitted
    /// changeset for any wallet — the persister-buffer state a caller must not
    /// assume it owns.
    pub(crate) fn has_staged_changeset(&self) -> bool {
        self.staged
            .lock()
            .expect("staged changeset")
            .values()
            .any(|changeset| !changeset.is_empty())
    }

    /// Drop every staged changeset the way an unrelated writer's terminal
    /// flush does: `flush_inner` takes the whole per-wallet buffer, and a
    /// terminal failure discards it without the account-registration caller
    /// ever seeing the error.
    pub(crate) fn discard_staged(&self) {
        self.staged.lock().expect("staged changeset").clear();
    }

    /// Arm a one-shot foreign drain immediately before the `write`-th `store`
    /// call, as if an unrelated writer had taken the shared buffer and lost it
    /// terminally in that window. Counting calls (rather than firing on the
    /// next one) is what places the drain *between* two attempts of one retry
    /// loop.
    pub(crate) fn discard_staged_before_write(&self, write: usize) {
        self.discard_before_write
            .store(write, std::sync::atomic::Ordering::Relaxed);
    }

    /// Count this write and, if it is the armed one, drop the staged buffer.
    fn apply_pending_discard(&self) {
        let remaining = self
            .discard_before_write
            .load(std::sync::atomic::Ordering::Relaxed);
        if remaining == 0 {
            return;
        }
        self.discard_before_write
            .store(remaining - 1, std::sync::atomic::Ordering::Relaxed);
        if remaining == 1 {
            self.discard_staged();
        }
    }

    fn next(&self) -> Option<PersistenceErrorKind> {
        self.queue.lock().expect("fault queue").pop_front()
    }

    /// `store` calls the registration path has made since this backend was
    /// built. One per persist attempt.
    pub(crate) fn store_calls(&self) -> usize {
        self.store_calls.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// `flush` calls the registration path has made since this backend was
    /// built. Zero unless a follow-up write has crept back in: a committed
    /// `store` needs none, and one would expose the registration to an
    /// unrelated writer's failure.
    pub(crate) fn flush_calls(&self) -> usize {
        self.flush_calls.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn record_store(&self) {
        self.store_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn record_flush(&self) {
        self.flush_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn stage(&self, wallet_id: WalletId, changeset: PlatformWalletChangeSet) {
        self.staged
            .lock()
            .expect("staged changeset")
            .entry(wallet_id)
            .or_default()
            .merge(changeset);
    }

    fn take_staged(&self, wallet_id: &WalletId) -> Option<PlatformWalletChangeSet> {
        self.staged
            .lock()
            .expect("staged changeset")
            .remove(wallet_id)
            .filter(|changeset| !changeset.is_empty())
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
    /// Forwarded, not defaulted: the trait's conservative `false` would put
    /// the registration path on its buffered-backend branch and test a write
    /// shape DET never runs.
    fn store_commits_inline(&self) -> bool {
        self.inner.store_commits_inline()
    }

    fn persistence_capabilities(&self) -> PersistenceCapabilities {
        self.inner.persistence_capabilities()
    }

    fn store(
        &self,
        wallet_id: WalletId,
        changeset: PlatformWalletChangeSet,
    ) -> Result<(), PersistenceError> {
        self.faults.record_store();
        self.faults.apply_pending_discard();
        match self.faults.next() {
            // Contract: a transient failure preserves the changeset, so a
            // later write still commits it.
            Some(PersistenceErrorKind::Transient) => {
                self.faults.stage(wallet_id, changeset);
                Err(Self::injected(PersistenceErrorKind::Transient))
            }
            // A terminal failure takes the whole merged buffer with it, not
            // just the incoming delta.
            Some(kind) => {
                self.faults.take_staged(&wallet_id);
                Err(Self::injected(kind))
            }
            None => {
                self.faults.stage(wallet_id, changeset);
                match self.faults.take_staged(&wallet_id) {
                    Some(staged) => self.inner.store(wallet_id, staged),
                    None => Ok(()),
                }
            }
        }
    }

    /// Passthrough — the queue serves `store` only. The registration path
    /// makes no `flush` call, so faulting one would test nothing; the counter
    /// is what a test asserts on instead.
    fn flush(&self, wallet_id: WalletId) -> Result<(), PersistenceError> {
        self.faults.record_flush();
        if let Some(staged) = self.faults.take_staged(&wallet_id) {
            self.inner.store(wallet_id, staged)?;
        }
        self.inner.flush(wallet_id)
    }

    fn load(&self) -> Result<ClientStartState, PersistenceError> {
        self.inner.load()
    }
}
