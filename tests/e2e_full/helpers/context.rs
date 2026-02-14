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
