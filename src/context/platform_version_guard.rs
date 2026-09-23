//! Guards the protocol version 13 tables DET builds, validates and verifies
//! with on mainnet and testnet against drift across platform pin bumps.
//!
//! Protocol 13 is live, so upstream must never change its table. The snapshot
//! covers what the client side reads: serialization and validation (`dpp`),
//! proof verification (`drive.methods.verify`), fees, system contract versions
//! and system limits. On a mismatch, diff the fixture, confirm the change is
//! additive (new fields at 0/None) or otherwise safe for a protocol 13 network,
//! then regenerate with `DET_BLESS_PLATFORM_VERSION_SNAPSHOT=1`.

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::sdk::min_protocol_version;

const SNAPSHOT_PATH: &str = "src/context/fixtures/platform_v13_client_tables.txt";
const SNAPSHOT: &str = include_str!("fixtures/platform_v13_client_tables.txt");

fn client_tables(platform_version: &PlatformVersion) -> String {
    format!(
        "protocol_version: {}\ndpp: {:#?}\ndrive.methods.verify: {:#?}\nfee_version: {:#?}\nsystem_data_contracts: {:#?}\nsystem_limits: {:#?}\n",
        platform_version.protocol_version,
        platform_version.dpp,
        platform_version.drive.methods.verify,
        platform_version.fee_version,
        platform_version.system_data_contracts,
        platform_version.system_limits,
    )
}

#[test]
fn protocol_13_client_tables_match_the_snapshot() {
    let actual = client_tables(PlatformVersion::get(13).expect("protocol 13"));
    if std::env::var_os("DET_BLESS_PLATFORM_VERSION_SNAPSHOT").is_some() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(SNAPSHOT_PATH);
        std::fs::write(&path, &actual).expect("write snapshot");
        return;
    }
    assert!(
        actual == SNAPSHOT,
        "The protocol 13 tables changed with the platform pin. Diff {SNAPSHOT_PATH} (rerun with \
         DET_BLESS_PLATFORM_VERSION_SNAPSHOT=1 to rewrite it), check the change is safe for \
         mainnet, and commit the new snapshot."
    );
}

/// Mainnet and testnet run protocol 13, and DET seeds its SDK from upstream's
/// per-network minimum: the snapshot above is the table DET starts with there.
#[test]
fn mainnet_and_testnet_start_at_protocol_13() {
    assert_eq!(min_protocol_version(Network::Mainnet), 13);
    assert_eq!(min_protocol_version(Network::Testnet), 13);
}
