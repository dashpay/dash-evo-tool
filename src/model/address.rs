use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dpp::address_funds::{PLATFORM_HRP_MAINNET, PLATFORM_HRP_TESTNET, PlatformAddress};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;

/// A destination address whose bech32m network prefix does not match the active
/// network — e.g. a mainnet `dash1…` address used on testnet.
///
/// Funds-safety guard: sending to a cross-network address would misdirect
/// credits, so both the GUI and the MCP tools reject it up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "This address belongs to a different network. Please check you are using the correct network."
)]
pub struct AddressNetworkMismatch;

/// The bech32m human-readable prefix that Platform and Orchard addresses use on
/// `network` (`dash` on mainnet, `tdash` otherwise). The two families share the
/// HRP; an Orchard address additionally begins its data section with `z`.
fn platform_hrp_for_network(network: Network) -> &'static str {
    match network {
        Network::Mainnet => PLATFORM_HRP_MAINNET,
        _ => PLATFORM_HRP_TESTNET,
    }
}

/// Case-insensitive ASCII prefix test that never panics on multi-byte input.
fn starts_with_ignore_ascii_case(s: &str, prefix: &str) -> bool {
    let (s, prefix) = (s.as_bytes(), prefix.as_bytes());
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Validate that a bech32m **Platform** address is for `network` by its prefix.
///
/// The single source of truth for Platform address↔network checks. Parses the
/// human-readable prefix (`dash1…` mainnet, `tdash1…` testnet, case-insensitive)
/// and rejects a cross-network address. It does not validate the address body —
/// callers still parse it with `PlatformAddress::from_bech32m_string`.
///
/// # Errors
///
/// Returns [`AddressNetworkMismatch`] when the prefix is not the one `network`
/// expects (including any input that is not a recognized Platform prefix).
pub fn validate_platform_address_for_network(
    address: &str,
    network: Network,
) -> Result<(), AddressNetworkMismatch> {
    let expected = format!("{}1", platform_hrp_for_network(network));
    if starts_with_ignore_ascii_case(address.trim(), &expected) {
        Ok(())
    } else {
        Err(AddressNetworkMismatch)
    }
}

/// Validate that a bech32m **Orchard** (shielded) address is for `network`.
///
/// The Orchard twin of [`validate_platform_address_for_network`]: same HRP, but
/// the data section begins with `z` (`dash1z…` mainnet, `tdash1z…` testnet,
/// case-insensitive). Does not validate the address body — callers still parse
/// it with `OrchardAddress::from_bech32m_string`.
///
/// # Errors
///
/// Returns [`AddressNetworkMismatch`] when the prefix is not the one `network`
/// expects.
pub fn validate_orchard_address_for_network(
    address: &str,
    network: Network,
) -> Result<(), AddressNetworkMismatch> {
    let expected = format!("{}1z", platform_hrp_for_network(network));
    if starts_with_ignore_ascii_case(address.trim(), &expected) {
        Ok(())
    } else {
        Err(AddressNetworkMismatch)
    }
}

/// Checks if a string looks like a Platform address (bech32m with dash/tdash HRP per DIP-18).
///
/// This checks whether the string starts with a known Platform HRP followed by the
/// bech32 separator '1'. It does NOT fully validate the address — use
/// `PlatformAddress::from_bech32m_string()` for that.
///
/// Uses byte-level comparison throughout (`as_bytes()`) to avoid the panic that
/// `s[..hrp.len()]` would produce for non-ASCII (multi-byte UTF-8) input whose
/// HRP-length byte offset falls inside a multi-byte codepoint.
pub fn is_platform_address_string(s: &str) -> bool {
    let bytes = s.as_bytes();
    for hrp in [PLATFORM_HRP_MAINNET, PLATFORM_HRP_TESTNET] {
        let hrp_bytes = hrp.as_bytes();
        if bytes.len() > hrp_bytes.len()
            && bytes[..hrp_bytes.len()].eq_ignore_ascii_case(hrp_bytes)
            && bytes[hrp_bytes.len()] == b'1'
        {
            return true;
        }
    }
    false
}

/// Classification of a Dash address for filtering and display purposes.
///
/// This enum represents the four recognized address categories. It is used
/// by `AddressInput` to configure which address types are accepted and to
/// label entries in the autocomplete dropdown.
///
/// Unlike the internal detection concept, there is no `Unknown` variant here --
/// an address either falls into one of these categories or fails validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressKind {
    /// Core L1 address (P2PKH / P2SH, Base58Check).
    Core,
    /// Platform L2 address (Bech32m per DIP-18).
    Platform,
    /// Shielded Orchard address (dash1z... / tdash1z...).
    Shielded,
    /// Identity identifier (Base58-encoded Identifier).
    Identity,
}

impl AddressKind {
    /// User-facing display name, suitable for i18n extraction.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Core => "Wallet address",
            Self::Platform => "Platform address",
            Self::Shielded => "Private address",
            Self::Identity => "Identity",
        }
    }

    /// Short label for use in parenthetical suffixes, e.g. "(Core)".
    pub fn short_label(&self) -> &'static str {
        match self {
            Self::Core => "Core",
            Self::Platform => "Platform",
            Self::Shielded => "Shielded",
            Self::Identity => "Identity",
        }
    }

    /// All supported address kinds.
    pub const ALL: [AddressKind; 4] = [
        AddressKind::Core,
        AddressKind::Platform,
        AddressKind::Shielded,
        AddressKind::Identity,
    ];

    /// Detect the address kind from a raw input string.
    ///
    /// Format-based detection only — network validation happens separately.
    /// Priority: Shielded > Platform > Core > Identity (Base58 fallback).
    /// Returns `None` for empty or unrecognized input.
    pub fn detect(input: &str) -> Option<AddressKind> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        // 1. Shielded (dash1z... / tdash1z...)
        if trimmed.starts_with("dash1z") || trimmed.starts_with("tdash1z") {
            return Some(AddressKind::Shielded);
        }

        // 1b. Shielded raw hex form (network-agnostic): 43 bytes = 86 hex chars.
        // Unambiguous — far too long for a Base58 Core address or Identity ID,
        // and it cannot start with the `dash1`/`tdash1` Platform HRP.
        if trimmed.len() == SHIELDED_ADDRESS_RAW_LEN * 2
            && trimmed.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Some(AddressKind::Shielded);
        }

        // 2. Platform (Bech32m per DIP-18, but NOT shielded — already excluded above)
        if is_platform_address_string(trimmed) {
            return Some(AddressKind::Platform);
        }

        // 3 & 4. Core vs Identity disambiguation.
        //
        // Both Core addresses and Identity IDs use Base58. Core addresses
        // on Dash always start with X/Y (mainnet) or y/8/7 (testnet).
        // If the input starts with a known Core prefix, try Core first.
        // Otherwise try Identity first to avoid misclassifying IDs as
        // Core addresses (they share the Base58 alphabet).
        let core_prefix = matches!(
            trimmed.as_bytes().first(),
            Some(b'X' | b'Y' | b'y' | b'8' | b'7')
        );

        if core_prefix {
            if trimmed.parse::<Address<NetworkUnchecked>>().is_ok() {
                return Some(AddressKind::Core);
            }
            if Identifier::from_string(trimmed, Encoding::Base58).is_ok() {
                return Some(AddressKind::Identity);
            }
        } else {
            if Identifier::from_string(trimmed, Encoding::Base58).is_ok() {
                return Some(AddressKind::Identity);
            }
            if trimmed.parse::<Address<NetworkUnchecked>>().is_ok() {
                return Some(AddressKind::Core);
            }
        }

        None
    }
}

impl std::fmt::Display for AddressKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

/// A fully validated address with its parsed typed payload.
///
/// This is the domain type produced by `AddressInput` via `ComponentResponse`.
/// Each variant carries the parsed representation for its address type.
#[derive(Debug, Clone)]
pub enum ValidatedAddress {
    /// A validated Core L1 address.
    Core(Address),
    /// A validated Platform L2 address with its canonical bech32m encoding.
    Platform {
        address: PlatformAddress,
        bech32m: String,
    },
    /// A validated shielded Orchard address (stored as the raw string).
    Shielded(String),
    /// A validated identity identifier with optional DPNS name.
    Identity {
        /// The parsed identity identifier.
        id: Identifier,
        /// Resolved DPNS name, if available from local data.
        dpns_name: Option<String>,
    },
}

impl ValidatedAddress {
    /// Returns the `AddressKind` for this validated address.
    pub fn kind(&self) -> AddressKind {
        match self {
            Self::Core(_) => AddressKind::Core,
            Self::Platform { .. } => AddressKind::Platform,
            Self::Shielded(_) => AddressKind::Shielded,
            Self::Identity { .. } => AddressKind::Identity,
        }
    }

    /// Returns the raw address string representation.
    pub fn to_address_string(&self) -> String {
        match self {
            Self::Core(addr) => addr.to_string(),
            Self::Platform { bech32m, .. } => bech32m.clone(),
            Self::Shielded(s) => s.clone(),
            Self::Identity { id, .. } => {
                id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
            }
        }
    }

    /// Returns the core address if this is a Core variant.
    pub fn as_core(&self) -> Option<&Address> {
        match self {
            Self::Core(addr) => Some(addr),
            _ => None,
        }
    }

    /// Returns the platform address if this is a Platform variant.
    pub fn as_platform(&self) -> Option<&PlatformAddress> {
        match self {
            Self::Platform { address, .. } => Some(address),
            _ => None,
        }
    }

    /// Returns the identity ID if this is an Identity variant.
    pub fn as_identity_id(&self) -> Option<&Identifier> {
        match self {
            Self::Identity { id, .. } => Some(id),
            _ => None,
        }
    }

    /// Returns the DPNS name if this is an Identity variant with a resolved name.
    pub fn dpns_name(&self) -> Option<&str> {
        match self {
            Self::Identity { dpns_name, .. } => dpns_name.as_deref(),
            _ => None,
        }
    }
}

impl std::fmt::Display for ValidatedAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core(addr) => write!(f, "{}", addr),
            Self::Platform { bech32m, .. } => write!(f, "{}", bech32m),
            Self::Shielded(s) => write!(f, "{}", s),
            Self::Identity {
                id,
                dpns_name: Some(name),
            } => write!(f, "{} ({})", name, id),
            Self::Identity {
                id,
                dpns_name: None,
            } => write!(
                f,
                "{}",
                id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
            ),
        }
    }
}

/// Raw byte length of an Orchard shielded address (recipient payload).
pub const SHIELDED_ADDRESS_RAW_LEN: usize = 43;

/// Parse a shielded (Orchard) recipient into its raw 43-byte form.
///
/// Accepts either the canonical Bech32m encoding (`dash1z…` mainnet,
/// `tdash1z…` testnet) or a raw hex string of exactly
/// [`SHIELDED_ADDRESS_RAW_LEN`] bytes. Returns `None` for any input that is
/// neither. This is the single source of truth for turning a shielded
/// recipient string into the bytes a `ShieldedTransfer` task needs, shared by
/// the send screen's dispatch and any validation path so the two cannot
/// diverge.
pub fn parse_shielded_recipient(input: &str) -> Option<Vec<u8>> {
    use dash_sdk::dpp::address_funds::OrchardAddress;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(addr) = OrchardAddress::from_bech32m_string(trimmed) {
        return Some(addr.to_raw_bytes().to_vec());
    }
    let bytes = hex::decode(trimmed).ok()?;
    (bytes.len() == SHIELDED_ADDRESS_RAW_LEN).then_some(bytes)
}

/// A raw Orchard payload that does not decode to a valid shielded address.
///
/// Raised when the 43-byte payload is not a well-formed Orchard address (its
/// `pk_d` is not a valid curve point), so it cannot be rendered as bech32m.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("This shielded address could not be read. Please reopen the wallet and try again.")]
pub struct InvalidShieldedAddress;

/// Encode a raw 43-byte Orchard payload as its canonical Bech32m string
/// (`dash1z…` on mainnet, `tdash1z…` elsewhere).
///
/// The inverse of [`parse_shielded_recipient`] and the single source of truth
/// for rendering a shielded address the wallet backend hands us as raw bytes.
///
/// # Errors
///
/// Returns [`InvalidShieldedAddress`] when `raw` is not a well-formed Orchard
/// address.
pub fn encode_shielded_address(
    raw: &[u8; SHIELDED_ADDRESS_RAW_LEN],
    network: Network,
) -> Result<String, InvalidShieldedAddress> {
    dash_sdk::dpp::address_funds::OrchardAddress::from_raw_bytes(raw)
        .map(|addr| addr.to_bech32m_string(network))
        .map_err(|_| InvalidShieldedAddress)
}

/// Truncate an address string for display, showing a prefix and suffix
/// separated by an ellipsis.
///
/// Assumes ASCII input (Base58 and Bech32/Bech32m addresses are always ASCII).
/// Addresses shorter than `prefix_len + suffix_len + 3` characters are returned
/// unchanged (truncation would not save space).
pub fn truncate_address(addr: &str, prefix_len: usize, suffix_len: usize) -> String {
    let min_useful = prefix_len + suffix_len + 3; // 3 for "..."
    if addr.len() < min_useful {
        return addr.to_string();
    }
    format!(
        "{}...{}",
        &addr[..prefix_len],
        &addr[addr.len() - suffix_len..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shielded_recipient_accepts_exact_length_hex() {
        let raw = vec![0xABu8; SHIELDED_ADDRESS_RAW_LEN];
        let hex_str = hex::encode(&raw);
        assert_eq!(parse_shielded_recipient(&hex_str), Some(raw.clone()));
        // Surrounding whitespace is tolerated.
        assert_eq!(
            parse_shielded_recipient(&format!("  {hex_str}  ")),
            Some(raw)
        );
    }

    #[test]
    fn parse_shielded_recipient_rejects_wrong_length_hex() {
        // One byte short and one byte long — both invalid.
        assert_eq!(
            parse_shielded_recipient(&hex::encode(vec![0u8; SHIELDED_ADDRESS_RAW_LEN - 1])),
            None
        );
        assert_eq!(
            parse_shielded_recipient(&hex::encode(vec![0u8; SHIELDED_ADDRESS_RAW_LEN + 1])),
            None
        );
    }

    #[test]
    fn detect_classifies_43_byte_hex_as_shielded() {
        let hex_str = hex::encode(vec![0x11u8; SHIELDED_ADDRESS_RAW_LEN]);
        assert_eq!(AddressKind::detect(&hex_str), Some(AddressKind::Shielded));
        // Wrong length is not a shielded address.
        let short = hex::encode(vec![0x11u8; SHIELDED_ADDRESS_RAW_LEN - 1]);
        assert_ne!(AddressKind::detect(&short), Some(AddressKind::Shielded));
    }

    #[test]
    fn parse_shielded_recipient_rejects_empty_and_garbage() {
        assert_eq!(parse_shielded_recipient(""), None);
        assert_eq!(parse_shielded_recipient("   "), None);
        assert_eq!(parse_shielded_recipient("not-an-address"), None);
        // Bech32m for a different address family is not a shielded recipient.
        assert_eq!(parse_shielded_recipient("dash1qexampleplatform"), None);
    }

    // --- encode_shielded_address (raw -> bech32m) ---
    //
    // Vectors are derived through the real upstream ZIP-32 path
    // (`OrchardKeySet::from_seed`) rather than fabricated bytes: an Orchard
    // payload embeds a curve point, so arbitrary bytes are not a valid address
    // and would not exercise the encoder honestly.

    fn test_shielded_raw_address(network: Network) -> [u8; SHIELDED_ADDRESS_RAW_LEN] {
        let keys =
            platform_wallet::wallet::shielded::OrchardKeySet::from_seed(&[7u8; 64], network, 0)
                .expect("ZIP-32 derivation from a valid test seed");
        keys.address_at(0).to_raw_address_bytes()
    }

    #[test]
    fn encode_shielded_address_round_trips_with_parse() {
        let raw = test_shielded_raw_address(Network::Testnet);
        let encoded =
            encode_shielded_address(&raw, Network::Testnet).expect("valid Orchard payload encodes");
        // The encoder and the existing parser are exact inverses — a receive
        // address we render must parse back to the very bytes it came from,
        // or a user could copy an address that does not match wallet state.
        assert_eq!(
            parse_shielded_recipient(&encoded).as_deref(),
            Some(raw.as_slice()),
        );
    }

    #[test]
    fn encode_shielded_address_uses_network_prefix() {
        let mainnet = encode_shielded_address(
            &test_shielded_raw_address(Network::Mainnet),
            Network::Mainnet,
        )
        .expect("valid Orchard payload encodes");
        let testnet = encode_shielded_address(
            &test_shielded_raw_address(Network::Testnet),
            Network::Testnet,
        )
        .expect("valid Orchard payload encodes");

        assert!(mainnet.starts_with("dash1z"), "got {mainnet}");
        assert!(testnet.starts_with("tdash1z"), "got {testnet}");
        // The output satisfies the shielded network validator the send path
        // enforces, so an address we display is one the app will accept back.
        assert!(validate_orchard_address_for_network(&mainnet, Network::Mainnet).is_ok());
        assert!(validate_orchard_address_for_network(&testnet, Network::Testnet).is_ok());
        assert_eq!(AddressKind::detect(&testnet), Some(AddressKind::Shielded));
    }

    #[test]
    fn encode_shielded_address_rejects_malformed_payload() {
        // All-zero bytes are not a valid Orchard address (pk_d is not a valid
        // curve point) — the encoder must reject rather than emit a string
        // that no one can pay to.
        assert_eq!(
            encode_shielded_address(&[0u8; SHIELDED_ADDRESS_RAW_LEN], Network::Testnet),
            Err(InvalidShieldedAddress),
        );
    }

    #[test]
    fn invalid_shielded_address_message_is_user_facing() {
        // Everyday-User rule: what happened + what to do, no jargon.
        assert_eq!(
            InvalidShieldedAddress.to_string(),
            "This shielded address could not be read. Please reopen the wallet and try again.",
        );
    }

    #[test]
    fn address_kind_display_names() {
        assert_eq!(AddressKind::Core.display_name(), "Wallet address");
        assert_eq!(AddressKind::Platform.display_name(), "Platform address");
        assert_eq!(AddressKind::Shielded.display_name(), "Private address");
        assert_eq!(AddressKind::Identity.display_name(), "Identity");
    }

    #[test]
    fn address_kind_all_contains_four_variants() {
        assert_eq!(AddressKind::ALL.len(), 4);
    }

    #[test]
    fn validated_address_kind_round_trips() {
        let shielded = ValidatedAddress::Shielded("dash1z_test".to_string());
        assert_eq!(shielded.kind(), AddressKind::Shielded);
        assert_eq!(shielded.to_address_string(), "dash1z_test");
    }

    #[test]
    fn validated_address_accessors_return_none_for_wrong_variant() {
        let shielded = ValidatedAddress::Shielded("dash1z_test".to_string());
        assert!(shielded.as_core().is_none());
        assert!(shielded.as_platform().is_none());
        assert!(shielded.as_identity_id().is_none());
        assert!(shielded.dpns_name().is_none());
    }

    // --- AddressKind::detect tests ---

    #[test]
    fn detect_empty_returns_none() {
        assert_eq!(AddressKind::detect(""), None);
        assert_eq!(AddressKind::detect("   "), None);
    }

    #[test]
    fn detect_shielded_mainnet() {
        assert_eq!(
            AddressKind::detect("dash1z_some_shielded_addr"),
            Some(AddressKind::Shielded)
        );
    }

    #[test]
    fn detect_shielded_testnet() {
        assert_eq!(
            AddressKind::detect("tdash1z_some_shielded_addr"),
            Some(AddressKind::Shielded)
        );
    }

    #[test]
    fn detect_shielded_priority_over_platform() {
        // dash1z starts with "dash1" which could match platform, but shielded wins
        assert_eq!(
            AddressKind::detect("dash1z_test"),
            Some(AddressKind::Shielded)
        );
    }

    #[test]
    fn detect_platform_testnet() {
        assert_eq!(
            AddressKind::detect("tdash1qwer1234"),
            Some(AddressKind::Platform)
        );
    }

    #[test]
    fn detect_platform_mainnet() {
        assert_eq!(
            AddressKind::detect("dash1qwer1234"),
            Some(AddressKind::Platform)
        );
    }

    #[test]
    fn detect_core_address() {
        use dash_sdk::dashcore_rpc::dashcore::secp256k1::{Secp256k1, SecretKey};
        use dash_sdk::dashcore_rpc::dashcore::{PrivateKey, PublicKey};

        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let privkey = PrivateKey::new(sk, Network::Testnet);
        let pubkey = PublicKey::from_private_key(&secp, &privkey);
        let addr = Address::p2pkh(&pubkey, Network::Testnet);
        assert_eq!(
            AddressKind::detect(&addr.to_string()),
            Some(AddressKind::Core)
        );
    }

    #[test]
    fn detect_identity_base58() {
        // Identity IDs that don't start with a Core prefix (X/Y/y/8/7)
        // should always detect as Identity, not Core.
        for _ in 0..20 {
            let id = Identifier::random();
            let id_str = id.to_string(Encoding::Base58);
            let first = id_str.as_bytes()[0];
            if matches!(first, b'X' | b'Y' | b'y' | b'8' | b'7') {
                // Core prefix — detection correctly prefers Core. Skip.
                continue;
            }
            assert_eq!(
                AddressKind::detect(&id_str),
                Some(AddressKind::Identity),
                "Non-Core-prefix identifier {id_str} should detect as Identity"
            );
        }
    }

    #[test]
    fn detect_identity_with_core_prefix_still_works_when_not_valid_core() {
        // An Identity ID that happens to start with a Core prefix but
        // doesn't pass Core address parsing should still detect as Identity.
        // We test this by creating identifiers until we find one starting
        // with a Core prefix that isn't a valid Core address.
        for _ in 0..100 {
            let id = Identifier::random();
            let id_str = id.to_string(Encoding::Base58);
            let first = id_str.as_bytes()[0];
            if !matches!(first, b'X' | b'Y' | b'y' | b'8' | b'7') {
                continue;
            }
            // Has Core prefix — if it doesn't parse as Core, it should be Identity
            if id_str.parse::<Address<NetworkUnchecked>>().is_err() {
                assert_eq!(
                    AddressKind::detect(&id_str),
                    Some(AddressKind::Identity),
                    "Core-prefix identifier {id_str} that fails Core parse should detect as Identity"
                );
                return;
            }
        }
        // If all 100 parsed as valid Core, that's fine — test is probabilistic
    }

    #[test]
    fn detect_garbage_returns_none() {
        assert_eq!(AddressKind::detect("not-an-address"), None);
    }

    // --- Platform/Orchard address↔network validators ---

    #[test]
    fn platform_address_matches_its_network() {
        assert!(validate_platform_address_for_network("dash1qwer1234", Network::Mainnet).is_ok());
        assert!(validate_platform_address_for_network("tdash1qwer1234", Network::Testnet).is_ok());
    }

    #[test]
    fn platform_address_wrong_network_is_typed_mismatch() {
        // Mainnet address on testnet, and testnet address on mainnet.
        assert_eq!(
            validate_platform_address_for_network("dash1qwer1234", Network::Testnet),
            Err(AddressNetworkMismatch)
        );
        assert_eq!(
            validate_platform_address_for_network("tdash1qwer1234", Network::Mainnet),
            Err(AddressNetworkMismatch)
        );
    }

    #[test]
    fn orchard_address_matches_its_network() {
        assert!(validate_orchard_address_for_network("dash1zqwer1234", Network::Mainnet).is_ok());
        assert!(validate_orchard_address_for_network("tdash1zqwer1234", Network::Testnet).is_ok());
    }

    #[test]
    fn orchard_address_wrong_network_is_typed_mismatch() {
        assert_eq!(
            validate_orchard_address_for_network("dash1zqwer1234", Network::Testnet),
            Err(AddressNetworkMismatch)
        );
        assert_eq!(
            validate_orchard_address_for_network("tdash1zqwer1234", Network::Mainnet),
            Err(AddressNetworkMismatch)
        );
    }

    #[test]
    fn platform_validator_rejects_orchard_prefix_as_wrong_family_on_wrong_network() {
        // A mainnet Orchard address (`dash1z…`) is not a testnet Platform address.
        assert_eq!(
            validate_platform_address_for_network("dash1zqwer1234", Network::Testnet),
            Err(AddressNetworkMismatch)
        );
    }

    #[test]
    fn network_mismatch_message_is_user_facing() {
        // The Display string is the calm, jargon-free banner both layers show.
        assert_eq!(
            AddressNetworkMismatch.to_string(),
            "This address belongs to a different network. Please check you are using the correct network."
        );
    }

    #[test]
    fn network_validators_ignore_surrounding_whitespace_and_case() {
        assert!(validate_platform_address_for_network("  DASH1QWER  ", Network::Mainnet).is_ok());
        assert!(validate_orchard_address_for_network("  TDASH1ZQWER  ", Network::Testnet).is_ok());
    }

    // --- is_platform_address_string pitfall guard (TC-MN-031) ---
    //
    // The masternode withdraw tool rejects Platform destinations with this
    // guard, NOT the weaker first-char `resolve::validate_address`. A `dash1…`
    // / `tdash1…` string must be flagged as Platform so it cannot slip through
    // as a Core address.

    #[test]
    fn platform_address_string_detects_bech32m_hrp() {
        assert!(is_platform_address_string("dash1qwer1234"));
        assert!(is_platform_address_string("tdash1qwer1234"));
    }

    #[test]
    fn platform_address_string_rejects_core_addresses() {
        // Mainnet (X) and testnet (y) Core addresses are not Platform addresses.
        assert!(!is_platform_address_string(
            "XqHiz9VVXfjBnET2z6aZ9j5LKyuGNv3byP"
        ));
        assert!(!is_platform_address_string(
            "yQ9JNCT4S9zVHaKYbr1FUY4YkUMYxSzWAj"
        ));
    }

    // TC-MN-B1 — non-ASCII input must never panic.
    //
    // The old implementation did `s[..hrp.len()]` which is byte-slicing a
    // `&str`; that panics when byte offset `hrp.len()` (4 for "dash", 5 for
    // "tdash") falls inside a multi-byte UTF-8 codepoint.  The fix switches
    // to `as_bytes()` throughout.  This regression test must never panic.
    #[test]
    fn platform_address_string_non_ascii_does_not_panic() {
        // "日本ABC" — each CJK char is 3 UTF-8 bytes; total 9 bytes.
        // The old code: s.len()=9 > hrp.len()=4, then s[..4] panics because
        // byte 4 is inside "本".
        assert!(!is_platform_address_string("日本ABC"));
        // Also test strings that are shorter than the HRP in chars but longer
        // in bytes (no panic even without the length guard being satisfied).
        assert!(!is_platform_address_string("日"));
        // Non-ASCII that starts "like" a HRP (ASCII bytes matching then UTF-8)
        assert!(!is_platform_address_string("dash\u{00E9}test"));
    }
}
