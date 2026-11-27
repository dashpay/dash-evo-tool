//! Per-wallet record of DET operations that move funds a Max quote is made
//! from, so a quote made before one is never offered after it.
//!
//! One record covers the operations that can take or release Core input
//! reservations (Core send and asset-lock Max); another covers shields from
//! Platform, whose spend the live Platform balances keep showing until a
//! sync pass reads it back (shield-from-Platform Max). The Core case:
//!
//! Reservations live in key-wallet's crate-private `ReservationSet`, which
//! exposes no list, count or generation, and every one in this process is
//! taken or released inside an upstream build DET calls. Such a call reserves
//! at an instant DET cannot see, so no quote is made while one is open for
//! the wallet ([`SpendActivity::quote_state`] reports it busy), and the
//! wallet's revision advances when one opens ([`SpendActivity::begin`]) and
//! when it closes (the guard's drop). A quote that started before an
//! operation opened carries the earlier revision and stops matching; the
//! first quote after the last one closes sees every reservation it left.
//! Nothing re-quotes on a timer. Passive TTL expiry follows block height,
//! which the snapshot publish already tracks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;

#[derive(Debug, Default, Clone, Copy)]
struct WalletSpends {
    /// Advanced when an operation opens and when it closes.
    revision: u64,
    /// Operations opened and not yet closed.
    in_flight: u32,
}

/// Lock scope: the map is only touched under a short, synchronous critical
/// section that never awaits and takes no other lock.
#[derive(Debug, Default)]
pub(super) struct SpendActivity {
    wallets: Mutex<HashMap<WalletSeedHash, WalletSpends>>,
}

/// What a Max quote for one wallet may rely on, read atomically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct QuoteState {
    /// The key a quote made now is valid under.
    pub(super) key: u64,
    /// An operation is open: no quote can see its reservation yet.
    pub(super) busy: bool,
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

    /// The key a Max quote made now for `seed_hash` is valid under, or
    /// [`TaskError::SpendInProgress`] while an operation is open.
    pub(super) fn idle_quote_key(&self, seed_hash: &WalletSeedHash) -> Result<u64, TaskError> {
        let state = self.quote_state(seed_hash);
        if state.busy {
            return Err(TaskError::SpendInProgress);
        }
        Ok(state.key)
    }

    /// The key and busy flag for a Max quote on `seed_hash`.
    pub(super) fn quote_state(&self, seed_hash: &WalletSeedHash) -> QuoteState {
        let spends = self.lock().get(seed_hash).copied().unwrap_or_default();
        QuoteState {
            key: spends.revision,
            busy: spends.in_flight > 0,
        }
    }
}

/// An open reserving operation; see [`SpendActivity::begin`].
#[must_use = "the operation closes when the guard drops"]
pub(crate) struct SpendGuard {
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
        let idle = activity.quote_state(&WALLET);
        assert_eq!(idle, activity.quote_state(&WALLET), "an idle key is stable");
        assert!(!idle.busy);

        let guard = activity.begin(&WALLET);
        let open = activity.quote_state(&WALLET);
        assert_ne!(open.key, idle.key);
        assert!(open.busy);
        drop(guard);
        let closed = activity.quote_state(&WALLET);
        assert_ne!(closed.key, open.key);
        assert_ne!(closed.key, idle.key);
        assert!(!closed.busy);
        assert_eq!(
            activity.quote_state(&[4; 32]),
            QuoteState {
                key: 0,
                busy: false
            },
            "other wallets are untouched"
        );
    }

    /// Overlapping operations keep the wallet busy until the last closes.
    #[test]
    fn the_wallet_stays_busy_until_every_operation_closes() {
        let activity = Arc::new(SpendActivity::default());
        let first = activity.begin(&WALLET);
        let second = activity.begin(&WALLET);
        drop(first);
        assert!(activity.quote_state(&WALLET).busy);
        drop(second);
        assert!(!activity.quote_state(&WALLET).busy);
    }
}
