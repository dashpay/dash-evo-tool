//! Per-wallet balance reads for wallet selectors.

use super::AppContext;
use crate::model::wallet::balance_summary::{
    WalletBalanceEntry, WalletBalanceSummary, WalletChoice,
};
use crate::wallet_backend::poison::RwLockRecover;

impl AppContext {
    /// One entry per loaded wallet for a wallet selector: its name and its
    /// Core, Platform address and shielded balances, in the wallets' stable
    /// order with recovery-phrase wallets first.
    ///
    /// Reads only in-memory snapshots, so it is safe to call every frame.
    /// Identity balances need a storage read and come separately from
    /// [`Self::identity_credits_by_wallet`].
    pub fn wallet_selector_entries(&self, include_single_key: bool) -> Vec<WalletBalanceEntry> {
        let wallet_context = self.wallet_context();
        let mut entries: Vec<_> = wallet_context
            .wallets()
            .keys()
            .map(|seed_hash| {
                let (_, asset_lock_inputs, _) = self.asset_lock_probe_snapshot(seed_hash);
                WalletBalanceEntry {
                    choice: WalletChoice::Hd(*seed_hash),
                    name: wallet_context.hd_alias(seed_hash),
                    balances: WalletBalanceSummary::default()
                        .with_core_duffs(
                            self.snapshot_balance(seed_hash).total,
                            asset_lock_inputs.final_funds_duffs,
                        )
                        .with_platform_duffs(self.platform_balance_duffs(seed_hash))
                        .with_shielded_credits(self.shielded_balance_credits(seed_hash)),
                }
            })
            .collect();
        if include_single_key {
            entries.extend(
                wallet_context
                    .single_key_wallets()
                    .iter()
                    .map(|(key_hash, wallet)| {
                        let wallet = wallet.read_recover();
                        WalletBalanceEntry {
                            choice: WalletChoice::SingleKey(*key_hash),
                            name: wallet_context.single_alias(&wallet.address.to_string()),
                            balances: WalletBalanceSummary::default().with_core_duffs(
                                wallet.total_balance_duffs(),
                                wallet.confirmed_balance_duffs(),
                            ),
                        }
                    }),
            );
        }
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;
    use crate::model::wallet::Wallet;
    use crate::model::wallet::single_key::SingleKeyWallet;
    use dash_sdk::dpp::dashcore::Network;
    use std::sync::{Arc, RwLock};

    fn hd_wallet(
        seed_byte: u8,
        alias: &str,
    ) -> (crate::model::wallet::WalletSeedHash, Arc<RwLock<Wallet>>) {
        let wallet = Wallet::new_from_seed(
            [seed_byte; 64],
            Network::Testnet,
            Some(alias.to_string()),
            None,
        )
        .expect("wallet");
        (wallet.seed_hash(), Arc::new(RwLock::new(wallet)))
    }

    #[test]
    fn entries_carry_each_wallets_name_and_pushed_balances() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ctx = test_app_context(dir.path());
        let (seed_hash, wallet) = hd_wallet(0x11, "Main");
        ctx.wallet_context().insert_test_wallet(seed_hash, wallet);
        ctx.platform_balances
            .lock()
            .expect("platform balances")
            .insert(seed_hash, 300);
        ctx.shielded_balances
            .lock()
            .expect("shielded balances")
            .insert(seed_hash, 7_000);

        assert_eq!(
            ctx.wallet_selector_entries(false),
            vec![WalletBalanceEntry {
                choice: WalletChoice::Hd(seed_hash),
                name: Some("Main".to_string()),
                balances: WalletBalanceSummary::default()
                    .with_platform_duffs(300)
                    .with_shielded_credits(7_000),
            }]
        );
    }

    #[test]
    fn single_key_wallets_are_listed_only_on_request() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ctx = test_app_context(dir.path());
        let (seed_hash, wallet) = hd_wallet(0x12, "Main");
        ctx.wallet_context().insert_test_wallet(seed_hash, wallet);
        let key = SingleKeyWallet::new([0x21; 32], Network::Testnet, None, Some("Key 1".into()))
            .expect("single-key wallet");
        let key_hash = key.key_hash();
        ctx.wallet_context().insert_test_single_key(
            Network::Testnet,
            key_hash,
            Arc::new(RwLock::new(key)),
        );

        let hd_only = ctx.wallet_selector_entries(false);
        assert_eq!(
            hd_only.iter().map(|e| e.choice).collect::<Vec<_>>(),
            vec![WalletChoice::Hd(seed_hash)]
        );

        let all = ctx.wallet_selector_entries(true);
        assert_eq!(
            all.iter()
                .map(|e| (e.choice, e.name.clone()))
                .collect::<Vec<_>>(),
            vec![
                (WalletChoice::Hd(seed_hash), Some("Main".to_string())),
                (WalletChoice::SingleKey(key_hash), Some("Key 1".to_string())),
            ]
        );
    }
}
