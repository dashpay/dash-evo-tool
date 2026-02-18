use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_sdk::platform::Identifier;

/// Shared test state persisted across phases within a single test run.
/// Replaces the react-native test-context.ts JSON file approach.
pub struct TestContext {
    pub wallet_seed_hash: Option<WalletSeedHash>,
    pub receive_address: Option<String>,
    pub balance_duffs: u64,
    pub spv_synced: bool,
    pub network: String,
    pub wallet_reused: bool,
    /// max_balance() snapshot taken before send-to-self (Phase 2)
    pub pre_send_balance: u64,
    /// wallet.transactions.len() snapshot taken before send-to-self (Phase 2)
    pub pre_send_tx_count: usize,
    /// Identity ID created in Phase 5
    pub identity_id: Option<Identifier>,
    /// DPNS name registered in Phase 6
    pub dpns_name: Option<String>,
    /// SPV header height at setup completion (needed for tx lock time)
    pub header_height: u32,
}

impl TestContext {
    /// Returns the wallet seed hash, panicking if not yet set.
    pub fn seed_hash(&self) -> &WalletSeedHash {
        self.wallet_seed_hash
            .as_ref()
            .expect("wallet_seed_hash must be set (did Phase 0 complete?)")
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self {
            wallet_seed_hash: None,
            receive_address: None,
            balance_duffs: 0,
            spv_synced: false,
            network: "testnet".to_string(),
            wallet_reused: false,
            pre_send_balance: 0,
            pre_send_tx_count: 0,
            identity_id: None,
            dpns_name: None,
            header_height: 0,
        }
    }
}

/// Format first 4 bytes of a seed hash as a hex prefix string.
pub fn seed_hash_prefix(hash: &WalletSeedHash) -> String {
    hash[..4].iter().map(|b| format!("{:02x}", b)).collect()
}
