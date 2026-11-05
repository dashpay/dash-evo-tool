//! Receiving addresses the Send form fetched for the user's own wallets.
//!
//! "My wallets" destinations land on an unused receiving address of the
//! chosen wallet. Deriving one is backend work (`WalletTask::GenerateReceiveAddress`
//! for Core, `WalletTask::GeneratePlatformReceiveAddress` for Platform); this
//! cache asks once per wallet and kind, keeps the answer while the form is
//! open, and asks again only after [`OwnReceiveAddresses::invalidate`] or once
//! an unanswered request has waited too long.

use crate::backend_task::BackendTask;
use crate::backend_task::wallet::WalletTask;
use crate::model::wallet::WalletSeedHash;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// How long an unanswered request blocks asking again.
const REQUEST_DEADLINE: Duration = Duration::from_secs(15);

/// Which kind of own receiving address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OwnAddressKind {
    Core,
    Platform,
}

#[derive(Debug, Clone)]
enum Entry {
    InFlight(Instant),
    Ready(String),
}

/// Fetch state of own receiving addresses, keyed by wallet and kind.
#[derive(Debug, Default)]
pub struct OwnReceiveAddresses {
    entries: BTreeMap<(WalletSeedHash, OwnAddressKind), Entry>,
}

impl OwnReceiveAddresses {
    /// The address fetched for `seed_hash`, if it arrived.
    pub fn get(&self, seed_hash: &WalletSeedHash, kind: OwnAddressKind) -> Option<&str> {
        match self.entries.get(&(*seed_hash, kind)) {
            Some(Entry::Ready(address)) => Some(address),
            _ => None,
        }
    }

    /// Whether a request for `seed_hash` is waiting for its answer.
    pub fn is_fetching(&self, seed_hash: &WalletSeedHash, kind: OwnAddressKind) -> bool {
        matches!(
            self.entries.get(&(*seed_hash, kind)),
            Some(Entry::InFlight(started)) if started.elapsed() < REQUEST_DEADLINE
        )
    }

    /// The task to dispatch for `seed_hash`, unless its address is known or
    /// was asked for recently.
    pub fn ensure_requested(
        &mut self,
        seed_hash: WalletSeedHash,
        kind: OwnAddressKind,
    ) -> Option<BackendTask> {
        match self.entries.get(&(seed_hash, kind)) {
            Some(Entry::Ready(_)) => return None,
            Some(Entry::InFlight(started)) if started.elapsed() < REQUEST_DEADLINE => return None,
            _ => {}
        }
        self.entries
            .insert((seed_hash, kind), Entry::InFlight(Instant::now()));
        Some(BackendTask::WalletTask(match kind {
            OwnAddressKind::Core => WalletTask::GenerateReceiveAddress { seed_hash },
            OwnAddressKind::Platform => WalletTask::GeneratePlatformReceiveAddress { seed_hash },
        }))
    }

    /// Whether a request for `seed_hash` was sent and not answered, however
    /// long ago.
    pub fn awaits(&self, seed_hash: &WalletSeedHash, kind: OwnAddressKind) -> bool {
        matches!(
            self.entries.get(&(*seed_hash, kind)),
            Some(Entry::InFlight(_))
        )
    }

    /// Take an address the backend returned. Only an answer for a wallet and
    /// kind this cache is waiting on is kept. Entries are keyed by wallet and
    /// kind alone, so another screen's address for the same wallet and kind
    /// can fill one; it is still an unused receiving address of that wallet.
    /// Returns whether it was kept.
    pub fn store(
        &mut self,
        seed_hash: WalletSeedHash,
        kind: OwnAddressKind,
        address: String,
    ) -> bool {
        match self.entries.get_mut(&(seed_hash, kind)) {
            Some(entry @ Entry::InFlight(_)) => {
                *entry = Entry::Ready(address);
                true
            }
            _ => false,
        }
    }

    /// Forget every address, so the next frame asks again.
    pub fn invalidate(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALLET: WalletSeedHash = [7; 32];

    #[test]
    fn asks_once_and_keeps_the_answer() {
        let mut cache = OwnReceiveAddresses::default();
        assert!(matches!(
            cache.ensure_requested(WALLET, OwnAddressKind::Core),
            Some(BackendTask::WalletTask(
                WalletTask::GenerateReceiveAddress { .. }
            ))
        ));
        assert!(cache.is_fetching(&WALLET, OwnAddressKind::Core));
        assert!(
            cache
                .ensure_requested(WALLET, OwnAddressKind::Core)
                .is_none()
        );
        cache.store(WALLET, OwnAddressKind::Core, "yQb7".into());
        assert_eq!(cache.get(&WALLET, OwnAddressKind::Core), Some("yQb7"));
        assert!(
            cache
                .ensure_requested(WALLET, OwnAddressKind::Core)
                .is_none()
        );
    }

    #[test]
    fn platform_addresses_are_asked_for_separately() {
        let mut cache = OwnReceiveAddresses::default();
        assert!(!cache.store(WALLET, OwnAddressKind::Core, "unasked".into()));
        assert_eq!(
            cache.get(&WALLET, OwnAddressKind::Core),
            None,
            "an address nobody asked for is ignored"
        );
        assert!(matches!(
            cache.ensure_requested(WALLET, OwnAddressKind::Platform),
            Some(BackendTask::WalletTask(
                WalletTask::GeneratePlatformReceiveAddress { .. }
            ))
        ));
    }

    #[test]
    fn invalidating_asks_again() {
        let mut cache = OwnReceiveAddresses::default();
        let _ = cache.ensure_requested(WALLET, OwnAddressKind::Core);
        cache.store(WALLET, OwnAddressKind::Core, "yQb7".into());
        cache.invalidate();
        assert!(cache.get(&WALLET, OwnAddressKind::Core).is_none());
        assert!(
            cache
                .ensure_requested(WALLET, OwnAddressKind::Core)
                .is_some()
        );
    }
}
