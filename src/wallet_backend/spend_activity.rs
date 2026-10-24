//! Per-wallet record of DET operations that can take or release Core input
//! reservations, so a Max quote made before one is never offered after it.
//!
//! Reservations live in key-wallet's crate-private `ReservationSet`, which
//! exposes no list, count or generation, and every one in this process is
//! taken or released inside an upstream build DET calls. DET therefore marks
//! the span of each such call: [`SpendActivity::begin`] on entry and the
//! guard's drop on exit both advance the wallet's revision. Inside a span the
//! reservation lands at an instant DET cannot see, so while any span is open
//! the quote key also rolls over every [`IN_FLIGHT_QUOTE_LIFETIME`]: a quote
//! taken before the build reserved goes stale within that bound, and the
//! reservation-aware quote that replaces it sees the reservation.
//! Passive TTL expiry follows block height, which the snapshot publish
//! already tracks.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::model::wallet::WalletSeedHash;

/// How long a Max quote stays usable while a reserving operation is open.
const IN_FLIGHT_QUOTE_LIFETIME: Duration = Duration::from_secs(2);

#[derive(Debug, Default, Clone, Copy)]
struct WalletSpends {
    /// Advanced on every operation entry and exit.
    revision: u64,
    /// Operations entered and not yet exited.
    in_flight: u32,
}

/// Lock scope: the map is only touched under a short, synchronous critical
/// section that never awaits and takes no other lock.
#[derive(Debug)]
pub(super) struct SpendActivity {
    wallets: Mutex<HashMap<WalletSeedHash, WalletSpends>>,
    epoch: Instant,
}

impl Default for SpendActivity {
    fn default() -> Self {
        Self {
            wallets: Mutex::new(HashMap::new()),
            epoch: Instant::now(),
        }
    }
}

impl SpendActivity {
    fn lock(&self) -> MutexGuard<'_, HashMap<WalletSeedHash, WalletSpends>> {
        // A panic mid-update leaves at worst a stale counter, which only
        // makes a quote re-request.
        self.wallets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Open a reserving operation on `seed_hash`; it closes when the guard
    /// drops, on every exit path.
    pub(super) fn begin(self: &Arc<Self>, seed_hash: &WalletSeedHash) -> SpendGuard {
        let mut wallets = self.lock();
        let spends = wallets.entry(*seed_hash).or_default();
        spends.revision = spends.revision.wrapping_add(1);
        spends.in_flight = spends.in_flight.saturating_add(1);
        SpendGuard {
            activity: Arc::clone(self),
            seed_hash: *seed_hash,
        }
    }

    /// The key a Max quote for `seed_hash` is valid under: the revision while
    /// nothing is in flight, and that revision with the current lifetime
    /// window while something is.
    pub(super) fn quote_key(&self, seed_hash: &WalletSeedHash) -> u64 {
        let spends = self.lock().get(seed_hash).copied().unwrap_or_default();
        if spends.in_flight == 0 {
            return spends.revision;
        }
        let window = self.epoch.elapsed().as_millis() / IN_FLIGHT_QUOTE_LIFETIME.as_millis();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (spends.revision, window).hash(&mut hasher);
        hasher.finish()
    }
}

/// An open reserving operation; see [`SpendActivity::begin`].
#[must_use = "the operation closes when the guard drops"]
pub(super) struct SpendGuard {
    activity: Arc<SpendActivity>,
    seed_hash: WalletSeedHash,
}

impl Drop for SpendGuard {
    fn drop(&mut self) {
        let mut wallets = self.activity.lock();
        let spends = wallets.entry(self.seed_hash).or_default();
        spends.revision = spends.revision.wrapping_add(1);
        spends.in_flight = spends.in_flight.saturating_sub(1);
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

    #[test]
    fn an_open_operation_expires_quotes_within_the_lifetime() {
        let activity = Arc::new(SpendActivity::default());
        let _guard = activity.begin(&WALLET);
        let first = activity.quote_key(&WALLET);
        std::thread::sleep(IN_FLIGHT_QUOTE_LIFETIME + Duration::from_millis(50));
        assert_ne!(activity.quote_key(&WALLET), first);
    }
}
