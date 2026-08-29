//! Test-only lock instrumentation, scoped to the test that installs it.
//!
//! Concurrency fixtures need one fact the code cannot otherwise report: *this
//! thread has reached that lock and cannot proceed*. Measuring it by elapsed
//! time is unsound — a thread that has not started yet and a thread parked on a
//! lock are indistinguishable by a clock — so the lock sites count waiters
//! instead.
//!
//! A process-global counter would be equally unsound, for a subtler reason. The
//! test binary runs tests in parallel, and every one of them shares the same
//! `identity_record_lock` and roster lock. A global waiter count therefore
//! reports the *suite's* lock traffic, not the test's: an unrelated test parked
//! on the same lock satisfies a fixture's release condition, freeing a reader
//! whose peer has not arrived, and the test that exists to catch a lost update
//! passes without one. That is the timeout bug again with a different clock —
//! worse, in fact, because it depends on what else happens to be running and so
//! decays silently and unreproducibly.
//!
//! A [`LockProbe`] is owned by one test. Only threads that [`LockProbe::attach`]
//! themselves to it report to it, so a probe counts exactly the traffic its own
//! test created and a busy neighbour is invisible to it.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Which lock a thread is waiting on, or asking for.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LockSite {
    /// Blocked acquiring the process-wide roster lock.
    RosterWait,
    /// Blocked acquiring an identity's record lock in
    /// [`AppContext::store_discovered_identity`](crate::context::AppContext::store_discovered_identity).
    DiscoveryRecordWait,
    /// Asked for an identity's record lock. Counts requests, not waits, and is
    /// never decremented — a test waits for it to rise, having taken the lock
    /// itself, which makes "asked for it" and "blocked on it" the same fact.
    RecordRequest,
}

#[derive(Default)]
struct Counts {
    roster_waiters: AtomicUsize,
    discovery_record_waiters: AtomicUsize,
    record_requests: AtomicUsize,
}

impl Counts {
    fn slot(&self, site: LockSite) -> &AtomicUsize {
        match site {
            LockSite::RosterWait => &self.roster_waiters,
            LockSite::DiscoveryRecordWait => &self.discovery_record_waiters,
            LockSite::RecordRequest => &self.record_requests,
        }
    }
}

thread_local! {
    static ATTACHED: RefCell<Option<Arc<Counts>>> = const { RefCell::new(None) };
}

/// One test's private view of lock activity on the threads it owns.
#[derive(Clone, Default)]
pub(crate) struct LockProbe {
    counts: Arc<Counts>,
}

impl LockProbe {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Report the calling thread's lock activity to this probe until the
    /// returned guard is dropped.
    ///
    /// Hold the guard for the thread's whole run. Dropping it matters on a
    /// pooled thread — a tokio worker outlives the test that used it, and an
    /// attachment left behind would point a later test's lock waits at a probe
    /// nobody reads, so the later test's own probe would stay at zero and its
    /// wait would never finish.
    pub(crate) fn attach(&self) -> Attachment {
        ATTACHED.with(|attached| {
            *attached.borrow_mut() = Some(Arc::clone(&self.counts));
        });
        Attachment
    }

    /// Threads of this test currently blocked acquiring `site`'s lock, or —
    /// for [`LockSite::RecordRequest`] — that have asked for one.
    pub(crate) fn count(&self, site: LockSite) -> usize {
        self.counts.slot(site).load(Ordering::SeqCst)
    }
}

/// Detaches its thread from a [`LockProbe`] when dropped.
#[must_use = "the thread stays attached only while this guard is alive"]
pub(crate) struct Attachment;

impl Drop for Attachment {
    fn drop(&mut self) {
        ATTACHED.with(|attached| {
            *attached.borrow_mut() = None;
        });
    }
}

/// Count the calling thread as waiting at `site` until the returned guard is
/// dropped. A thread attached to no probe counts nowhere.
///
/// Bind the guard and drop it once the lock is *acquired*: the fact under
/// measurement is "waiting", not "holding".
pub(crate) fn enter_wait(site: LockSite) -> WaitGuard {
    let counts = ATTACHED.with(|attached| attached.borrow().clone());
    if let Some(counts) = &counts {
        counts.slot(site).fetch_add(1, Ordering::SeqCst);
    }
    WaitGuard { counts, site }
}

/// Record that the calling thread asked for `site`'s lock. Not paired with a
/// decrement — see [`LockSite::RecordRequest`].
pub(crate) fn note_request(site: LockSite) {
    ATTACHED.with(|attached| {
        if let Some(counts) = attached.borrow().as_ref() {
            counts.slot(site).fetch_add(1, Ordering::SeqCst);
        }
    });
}

pub(crate) struct WaitGuard {
    counts: Option<Arc<Counts>>,
    site: LockSite,
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        if let Some(counts) = &self.counts {
            counts.slot(self.site).fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the type: one test's probe must not see another
    /// test's threads. An unattached thread standing in for the busy
    /// neighbour must leave every count at zero.
    #[test]
    fn a_probe_counts_only_the_threads_attached_to_it() {
        let mine = LockProbe::new();
        let theirs = LockProbe::new();

        std::thread::scope(|scope| {
            scope.spawn(|| {
                let _attached = mine.attach();
                let _waiting = enter_wait(LockSite::RosterWait);
                while mine.count(LockSite::RosterWait) == 0 {
                    std::thread::yield_now();
                }
            });
            scope.spawn(|| {
                let _attached = theirs.attach();
                let _waiting = enter_wait(LockSite::RosterWait);
                while theirs.count(LockSite::RosterWait) == 0 {
                    std::thread::yield_now();
                }
            });
            // The neighbour: same lock site, no probe.
            scope.spawn(|| {
                let _waiting = enter_wait(LockSite::RosterWait);
                std::thread::yield_now();
            });
        });

        assert_eq!(
            mine.count(LockSite::RosterWait),
            0,
            "every waiter this probe saw must have released",
        );
        assert_eq!(
            theirs.count(LockSite::RosterWait),
            0,
            "one probe's waiter must never be counted by another",
        );
    }

    /// A pooled thread runs later tests. An attachment that outlived its test
    /// would point their lock waits at a probe nobody reads, and their own
    /// probe would never leave zero.
    #[test]
    fn dropping_the_attachment_detaches_the_thread() {
        let probe = LockProbe::new();
        {
            let _attached = probe.attach();
            let _waiting = enter_wait(LockSite::RecordRequest);
            assert_eq!(probe.count(LockSite::RecordRequest), 1);
        }
        note_request(LockSite::RecordRequest);
        assert_eq!(
            probe.count(LockSite::RecordRequest),
            0,
            "activity after the attachment is dropped belongs to no probe",
        );
    }
}
