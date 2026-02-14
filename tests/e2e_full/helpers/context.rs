use dash_evo_tool::model::wallet::WalletSeedHash;

/// Shared test state persisted across phases within a single test run.
/// Replaces the react-native test-context.ts JSON file approach.
pub struct TestContext {
    pub wallet_seed_hash: Option<WalletSeedHash>,
    pub receive_address: Option<String>,
    pub balance_duffs: u64,
    pub spv_synced: bool,
    pub network: String,
    pub wallet_reused: bool,
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
        }
    }
}

/// Format first 4 bytes of a seed hash as a hex prefix string.
pub fn seed_hash_prefix(hash: &WalletSeedHash) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        hash[0], hash[1], hash[2], hash[3]
    )
}
