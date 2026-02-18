pub mod phase_00_setup;
pub mod phase_01_faucet;
pub mod phase_02_wallet;
pub mod phase_03_platform;
pub mod phase_04_tokens;
pub mod phase_05_identity;
#[allow(dead_code)] // Phase 6 is skipped until SPV mempool support lands
pub mod phase_06_dpns;
pub mod phase_07_teardown;
pub mod phase_smoke;
