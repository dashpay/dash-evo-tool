use crate::model::wallet::WalletSeedHash;

#[derive(Debug, Clone, PartialEq)]
pub enum WalletTask {
    GenerateReceiveAddress { seed_hash: WalletSeedHash },
}
