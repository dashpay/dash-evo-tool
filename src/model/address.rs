use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dashcore_rpc::dashcore::{Address, Network};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;

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

    /// All address kinds in detection priority order.
    pub const ALL: [AddressKind; 4] = [
        AddressKind::Core,
        AddressKind::Platform,
        AddressKind::Shielded,
        AddressKind::Identity,
    ];

    /// Detect the address kind from a raw input string.
    ///
    /// Priority: Shielded > Platform > Core > Identity (Base58 fallback).
    /// Returns `None` for empty or unrecognized input.
    pub fn detect(input: &str, _network: Network) -> Option<AddressKind> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        // 1. Shielded (dash1z... / tdash1z...)
        if trimmed.starts_with("dash1z") || trimmed.starts_with("tdash1z") {
            return Some(AddressKind::Shielded);
        }

        // 2. Platform (Bech32m per DIP-18, but NOT shielded — already excluded above)
        if crate::ui::helpers::is_platform_address_string(trimmed) {
            return Some(AddressKind::Platform);
        }

        // 3. Core (Base58Check)
        if trimmed.parse::<Address<NetworkUnchecked>>().is_ok() {
            return Some(AddressKind::Core);
        }

        // 4. Identity (Base58 fallback)
        if Identifier::from_string(trimmed, Encoding::Base58).is_ok() {
            return Some(AddressKind::Identity);
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
        assert_eq!(AddressKind::detect("", Network::Testnet), None);
        assert_eq!(AddressKind::detect("   ", Network::Testnet), None);
    }

    #[test]
    fn detect_shielded_mainnet() {
        assert_eq!(
            AddressKind::detect("dash1z_some_shielded_addr", Network::Mainnet),
            Some(AddressKind::Shielded)
        );
    }

    #[test]
    fn detect_shielded_testnet() {
        assert_eq!(
            AddressKind::detect("tdash1z_some_shielded_addr", Network::Testnet),
            Some(AddressKind::Shielded)
        );
    }

    #[test]
    fn detect_shielded_priority_over_platform() {
        // dash1z starts with "dash1" which could match platform, but shielded wins
        assert_eq!(
            AddressKind::detect("dash1z_test", Network::Mainnet),
            Some(AddressKind::Shielded)
        );
    }

    #[test]
    fn detect_platform_testnet() {
        assert_eq!(
            AddressKind::detect("tdash1qwer1234", Network::Testnet),
            Some(AddressKind::Platform)
        );
    }

    #[test]
    fn detect_platform_mainnet() {
        assert_eq!(
            AddressKind::detect("dash1qwer1234", Network::Mainnet),
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
            AddressKind::detect(&addr.to_string(), Network::Testnet),
            Some(AddressKind::Core)
        );
    }

    #[test]
    fn detect_identity_base58_fallback() {
        let id = Identifier::random();
        let id_str = id.to_string(Encoding::Base58);
        // Some random identifiers parse as Core addresses. Skip those for
        // this test — only assert identity detection for ones that do not.
        if AddressKind::detect(&id_str, Network::Testnet) == Some(AddressKind::Core) {
            return;
        }
        assert_eq!(
            AddressKind::detect(&id_str, Network::Testnet),
            Some(AddressKind::Identity)
        );
    }

    #[test]
    fn detect_garbage_returns_none() {
        assert_eq!(
            AddressKind::detect("not-an-address", Network::Testnet),
            None
        );
    }
}
