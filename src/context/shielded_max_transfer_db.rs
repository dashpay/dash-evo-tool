//! Read access to the stored "Max" transfers from the Core wallet to the
//! shielded balance, for screens deciding whether to show one.

use super::AppContext;
use crate::model::wallet::WalletSeedHash;

impl AppContext {
    /// Whether `seed_hash` has a stored, unsettled Max transfer. One k/v read;
    /// call it on arrival rather than every frame. An unreadable record counts
    /// as stored, so the status read that follows reports the failure.
    pub fn has_pending_shielded_max_transfer(&self, seed_hash: &WalletSeedHash) -> bool {
        let Ok(backend) = self.wallet_backend() else {
            return false;
        };
        backend
            .shielded_max_transfer_records()
            .pending(self.network(), seed_hash)
            .map_or(true, |record| record.is_some())
    }
}
