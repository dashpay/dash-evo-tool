use crate::model::wallet::{Wallet, WalletSeedHash};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct WatchOnlyAccount {
    pub seed_hash: WalletSeedHash,
    pub account_index: u32,
    pub xpub: ExtendedPubKey,
}

#[derive(Debug, Clone)]
pub struct WatchOnlyWalletAttachment {
    pub network: Network,
    pub accounts: Vec<WatchOnlyAccount>,
}

impl WatchOnlyWalletAttachment {
    pub fn from_wallets(
        network: Network,
        wallets: &BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>,
    ) -> Self {
        let mut accounts = Vec::new();
        for (seed_hash, wallet_arc) in wallets.iter() {
            let w = wallet_arc.read().expect("wallet lock poisoned");
            let xpub = w.master_bip44_ecdsa_extended_public_key;
            accounts.push(WatchOnlyAccount {
                seed_hash: *seed_hash,
                account_index: 0,
                xpub,
            });
        }
        Self { network, accounts }
    }
}
