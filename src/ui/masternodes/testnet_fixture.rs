//! Testnet node fixture loader for the Fill-Random dev convenience (FR-12).
//!
//! A local `.testnet_nodes.yml` file (Testnet-only, developer machines) supplies
//! real masternode/evonode ProTxHashes and keys so a developer can populate the
//! load form with one click. The fixture is a dev tool: it is only ever surfaced
//! inside the Expert-Mode-gated Masternodes tab, on Testnet, when the file is
//! present and parses.
//!
//! Divergence from the legacy add-existing-identity screen (§FR-12 / TC-FR12-04,
//! PROJ-004): a **malformed** file is swallowed to `None` (logged at `debug`),
//! never surfaced as an error banner — the button simply does not appear.

use serde::Deserialize;
use std::collections::HashMap;

/// The fixture file name, resolved relative to the process working directory.
pub const TESTNET_NODES_FILE: &str = ".testnet_nodes.yml";

#[derive(Debug, Clone, Deserialize)]
pub struct KeyInfo {
    pub private_key: String,
}

/// A regular masternode fixture entry. Has **no** payout key — so Fill-Random
/// for a Masternode populates Voting + Owner only (TC-FR12-07).
#[derive(Debug, Clone, Deserialize)]
pub struct MasternodeInfo {
    #[serde(rename = "pro-tx-hash")]
    pub pro_tx_hash: String,
    pub owner: KeyInfo,
    pub voter: KeyInfo,
}

/// A high-performance (evo) masternode fixture entry, carrying all three keys.
#[derive(Debug, Clone, Deserialize)]
pub struct HpMasternodeInfo {
    #[serde(rename = "protx-tx-hash")]
    pub protx_tx_hash: String,
    pub owner: KeyInfo,
    pub voter: KeyInfo,
    pub payout: KeyInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestnetNodes {
    pub masternodes: HashMap<String, MasternodeInfo>,
    pub hp_masternodes: HashMap<String, HpMasternodeInfo>,
}

/// Load the testnet node fixture. Returns `None` for **both** a missing and a
/// malformed file — a malformed file is logged at `debug` and treated as absent
/// (TC-FR12-04), so the Fill-Random button never appears on bad input and no
/// error banner is surfaced.
pub fn load_testnet_nodes() -> Option<TestnetNodes> {
    let content = std::fs::read_to_string(TESTNET_NODES_FILE).ok()?;
    match serde_yaml_ng::from_str::<TestnetNodes>(&content) {
        Ok(nodes) => Some(nodes),
        Err(e) => {
            tracing::debug!("Ignoring malformed {TESTNET_NODES_FILE}: {e}");
            None
        }
    }
}
