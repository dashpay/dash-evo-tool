use dash_sdk::dashcore_rpc::dashcore::Address;
#[cfg(test)]
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dpp::address_funds::{PLATFORM_HRP_MAINNET, PLATFORM_HRP_TESTNET, PlatformAddress};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;

/// Checks if a string looks like a Platform address (bech32m with dash/tdash HRP per DIP-18).
///
/// This checks whether the string starts with a known Platform HRP followed by the
/// bech32 separator '1'. It does NOT fully validate the address — use
/// `PlatformAddress::from_bech32m_string()` for that.
pub fn is_platform_address_string(s: &str) -> bool {
    for hrp in [PLATFORM_HRP_MAINNET, PLATFORM_HRP_TESTNET] {
        if s.len() > hrp.len()
            && s[..hrp.len()].eq_ignore_ascii_case(hrp)
            && s.as_bytes()[hrp.len()] == b'1'
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
