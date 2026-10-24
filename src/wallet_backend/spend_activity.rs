//! Per-wallet record of DET operations that can take or release Core input
//! reservations, so a Max quote made before one is never offered after it.
//!
//! Reservations live in key-wallet's crate-private `ReservationSet`, which
//! exposes no list, count or generation, and every one in this process is
//! taken or released inside an upstream build DET calls. DET therefore
//! advances the wallet's revision at the points it controls around each such
//! call: on entry ([`SpendActivity::begin`]), at the hand-off to the upstream
//! build, which is where reservations are taken ([`SpendGuard::mark`]), after
//! a build that returns before its broadcast settles, and on exit (the
//! guard's drop). Between those points a quote stays keyed on the revision it
//! was made under; nothing re-quotes on a timer, which would contend for the
//! write lock with the very build holding the reservations. Passive TTL
//! expiry follows block height, which the snapshot publish already tracks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::model::wallet::WalletSeedHash;

/// Lock scope: the map is only touched under a short, synchronous critical
/// section that never awaits and takes no other lock.
#[derive(Debug, Default)]
pub(super) struct SpendActivity {
    /// Per-wallet revision, advanced at every marked point.
    revisions: Mutex<HashMap<WalletSeedHash, u64>>,
}

impl SpendActivity {
    fn lock(&self) -> MutexGuard<'_, HashMap<WalletSeedHash, u64>> {
        // A panic mid-update leaves at worst a stale counter, which only
        // makes a quote re-request.
        self.revisions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn advance(&self, seed_hash: &WalletSeedHash) {
        let mut revisions = self.lock();
        let revision = revisions.entry(*seed_hash).or_default();
        *revision = revision.wrapping_add(1);
    }

    /// Open a reserving operation on `seed_hash`; it closes, advancing the
    /// revision again, when the guard drops on any exit path.
    pub(super) fn begin(self: &Arc<Self>, seed_hash: &WalletSeedHash) -> SpendGuard {
        self.advance(seed_hash);
        SpendGuard {
            activity: Arc::clone(self),
            seed_hash: *seed_hash,
        }
    }

    /// The key a Max quote for `seed_hash` is valid under.
    pub(super) fn quote_key(&self, seed_hash: &WalletSeedHash) -> u64 {
        self.lock().get(seed_hash).copied().unwrap_or_default()
    }
}

/// An open reserving operation; see [`SpendActivity::begin`].
#[must_use = "the operation closes when the guard drops"]
pub(super) struct SpendGuard {
    activity: Arc<SpendActivity>,
    seed_hash: WalletSeedHash,
}

impl SpendGuard {
    /// Advance the revision at a point where the operation's reservations may
    /// change: right before handing off to the upstream build, and after a
    /// build that returns before its broadcast settles.
    pub(super) fn mark(&self) {
        self.activity.advance(&self.seed_hash);
    }
}

impl Drop for SpendGuard {
    fn drop(&mut self) {
        self.activity.advance(&self.seed_hash);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALLET: WalletSeedHash = [3; 32];

    #[test]
    fn opening_and_closing_an_operation_each_move_the_key() {
        let activity = Arc::new(SpendActivity::default());
        let idle = activity.quote_key(&WALLET);
        assert_eq!(idle, activity.quote_key(&WALLET), "an idle key is stable");

        let guard = activity.begin(&WALLET);
        let open = activity.quote_key(&WALLET);
        assert_ne!(open, idle);
        drop(guard);
        let closed = activity.quote_key(&WALLET);
        assert_ne!(closed, open);
        assert_ne!(closed, idle);
        assert_eq!(
            activity.quote_key(&[4; 32]),
            0,
            "other wallets are untouched"
        );
    }

    /// A long operation (a proof wait can take minutes) must not make quotes
    /// expire on a timer: each re-quote is a dry run under the write lock the
    /// build itself needs. Only the marked hand-off moves the key.
    #[test]
    fn an_open_operation_moves_the_key_only_when_marked() {
        let activity = Arc::new(SpendActivity::default());
        let guard = activity.begin(&WALLET);
        let open = activity.quote_key(&WALLET);
        std::thread::sleep(std::time::Duration::from_millis(2_050));
        assert_eq!(
            activity.quote_key(&WALLET),
            open,
            "time alone moves nothing"
        );

        guard.mark();
        assert_ne!(
            activity.quote_key(&WALLET),
            open,
            "the hand-off moves the key"
        );
    }
}
