use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dpp::balances::credits::Credits;

/// Detected address type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressType {
    Core,
    Platform,
    Unknown,
}

/// Format a duff amount as a human-readable DASH string
pub fn format_dash(amount_duffs: u64) -> String {
    Amount::dash_from_duffs(amount_duffs).to_string()
}

/// Parse a DASH amount string into duffs
pub fn parse_amount_to_duffs(input: &str) -> Result<u64, String> {
    let amount = Amount::parse(input, DASH_DECIMAL_PLACES)?.with_unit_name("DASH");
    amount.dash_to_duffs()
}

/// Parse a DASH amount string into platform credits
pub fn parse_amount_to_credits(input: &str) -> Result<Credits, String> {
    let amount = Amount::parse(input, DASH_DECIMAL_PLACES)?.with_unit_name("DASH");
    let duffs = amount.dash_to_duffs()?;
    Ok(duffs as Credits * 1000)
}

/// Format platform credits as a human-readable DASH string
pub fn format_credits(credits: Credits) -> String {
    let dash = credits as f64 / 1000.0 / 100_000_000.0;
    format!("{:.8} DASH", dash)
}

/// Detect address type from the address string
pub fn detect_address_type(address: &str) -> AddressType {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return AddressType::Unknown;
    }

    // Check for Platform address (Bech32m format)
    if trimmed.starts_with("evo1") || trimmed.starts_with("tevo1") {
        return AddressType::Platform;
    }

    // Try to parse as Core address
    if trimmed.parse::<Address<NetworkUnchecked>>().is_ok() {
        return AddressType::Core;
    }

    AddressType::Unknown
}
