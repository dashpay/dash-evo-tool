use std::fmt::Display;

use dash_sdk::dpp::dashcore::{Network, bip32::DerivationPath};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::kms::KeyType;

pub type SeedHash = [u8; 32];
/// Generic key handle used in the [GenericKms], used to identify keys in the KMS.
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub enum KeyHandle {
    /// Represents a raw public key
    RawKey(Vec<u8>, KeyType),
    /// Represents a seed used for key derivation.
    DerivationSeed {
        seed_hash: SeedHash,
        network: Network,
    },
    /// Key derived from a seed using a derivation path.
    Derived {
        seed_hash: SeedHash,             // Hash of the seed to use to derive the key
        derivation_path: DerivationPath, // Derivation path for the key; TODO:
        network: Network, // Network for which the key is derived (e.g., Dash, Devnet)
    },
}

impl Display for KeyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key_string = match self {
            KeyHandle::RawKey(bytes, key_type) => {
                format!(
                    "RawKey(bytes={}, type={:?})",
                    hex::encode_upper(bytes),
                    key_type
                )
            }
            KeyHandle::DerivationSeed { seed_hash, network } => {
                format!(
                    "DerivationSeed(seed_hash={}, network={})",
                    hex::encode_upper(seed_hash),
                    network
                )
            }
            KeyHandle::Derived {
                seed_hash,
                derivation_path,
                network,
            } => {
                format!(
                    "Derived(seed_hash={}, derivation_path={}, network={})",
                    hex::encode_upper(seed_hash),
                    derivation_path,
                    network
                )
            }
        };
        write!(f, "{}", key_string)
    }
}

impl Serialize for KeyHandle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_str())
    }
}

impl<'de> Deserialize<'de> for KeyHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        match s {
            // Match RawKey(bytes=HEX, type=TYPE)
            s if s.starts_with("RawKey(bytes=") && s.contains(", type=") && s.ends_with(")") => {
                let inner = &s[13..s.len() - 1]; // Remove "RawKey(" and ")"

                // Find the split point between bytes and type
                if let Some(type_pos) = inner.find(", type=") {
                    let hex_str = &inner[0..type_pos];
                    let type_str = &inner[type_pos + 7..]; // Skip ", type="

                    let bytes = hex::decode(hex_str).map_err(|e| {
                        serde::de::Error::custom(format!("Invalid hex in RawKey: {}", e))
                    })?;

                    // Parse KeyType from debug format
                    let key_type = match type_str {
                        s if s.contains("ECDSA_SECP256K1") => KeyType::ecdsa_secp256k1(),
                        s if s.contains("EDDSA_25519_HASH160") => KeyType::Raw {
                            alogirhm: crate::kms::IdentityKeyType::EDDSA_25519_HASH160,
                        },
                        s if s.contains("BLS12_381") => KeyType::Raw {
                            alogirhm: crate::kms::IdentityKeyType::BLS12_381,
                        },
                        s if s.contains("BIP13_SCRIPT_HASH") => KeyType::Raw {
                            alogirhm: crate::kms::IdentityKeyType::BIP13_SCRIPT_HASH,
                        },
                        s if s.contains("ECDSA_HASH160") => KeyType::Raw {
                            alogirhm: crate::kms::IdentityKeyType::ECDSA_HASH160,
                        },
                        _ => {
                            // Default to ECDSA_SECP256K1 for unknown types
                            KeyType::ecdsa_secp256k1()
                        }
                    };

                    Ok(KeyHandle::RawKey(bytes, key_type))
                } else {
                    Err(serde::de::Error::custom(
                        "Invalid RawKey format: missing type",
                    ))
                }
            }
            // Match Derived(seed_hash=HEX, derivation_path=PATH, network=NETWORK)
            s if s.starts_with("Derived(") && s.ends_with(")") => {
                let inner = &s[8..s.len() - 1]; // Remove "Derived(" and ")"

                // Parse "seed_hash=HEX, derivation_path=PATH, network=NETWORK"
                let parts: Vec<&str> = inner.split(", ").collect();
                if parts.len() != 3 {
                    return Err(serde::de::Error::custom(
                        "Invalid Derived format: expected seed_hash, derivation_path, and network",
                    ));
                }

                let seed_hash_part = parts[0];
                let derivation_path_part = parts[1];
                let network_part = parts[2];

                let hex_str = seed_hash_part
                    .strip_prefix("seed_hash=")
                    .ok_or(serde::de::Error::custom("Missing seed_hash= prefix"))?;

                let derivation_path_str = derivation_path_part
                    .strip_prefix("derivation_path=")
                    .ok_or(serde::de::Error::custom("Missing derivation_path= prefix"))?;

                let network_str = network_part
                    .strip_prefix("network=")
                    .ok_or(serde::de::Error::custom("Missing network= prefix"))?;

                let seed_hash = hex::decode(hex_str)
                    .map_err(|e| {
                        serde::de::Error::custom(format!("Invalid hex in seed_hash: {}", e))
                    })?
                    .try_into()
                    .map_err(|_| {
                        serde::de::Error::custom("Invalid seed_hash length, expected 32 bytes")
                    })?;

                let derivation_path =
                    derivation_path_str.parse::<DerivationPath>().map_err(|e| {
                        serde::de::Error::custom(format!("Invalid derivation path: {}", e))
                    })?;

                let network = network_str
                    .parse::<Network>()
                    .map_err(|e| serde::de::Error::custom(format!("Invalid network: {}", e)))?;

                Ok(KeyHandle::Derived {
                    seed_hash,
                    derivation_path,
                    network,
                })
            }
            // Match DerivationSeed(seed_hash=HEX, network=NETWORK)
            s if s.starts_with("DerivationSeed(") && s.ends_with(")") => {
                let inner = &s[15..s.len() - 1]; // Remove "DerivationSeed(" and ")"

                // Parse "seed_hash=HEX, network=NETWORK"
                let parts: Vec<&str> = inner.split(", ").collect();
                if parts.len() != 2 {
                    return Err(serde::de::Error::custom(
                        "Invalid DerivationSeed format: expected seed_hash and network",
                    ));
                }

                let seed_hash_part = parts[0];
                let network_part = parts[1];

                let hex_str = seed_hash_part
                    .strip_prefix("seed_hash=")
                    .ok_or(serde::de::Error::custom("Missing seed_hash= prefix"))?;

                let network_str = network_part
                    .strip_prefix("network=")
                    .ok_or(serde::de::Error::custom("Missing network= prefix"))?;

                let seed_hash = hex::decode(hex_str)
                    .map_err(|e| {
                        serde::de::Error::custom(format!("Invalid hex in seed_hash: {}", e))
                    })?
                    .try_into()
                    .map_err(|_| {
                        serde::de::Error::custom("Invalid seed_hash length, expected 32 bytes")
                    })?;

                let network = network_str
                    .parse::<Network>()
                    .map_err(|e| serde::de::Error::custom(format!("Invalid network: {}", e)))?;

                Ok(KeyHandle::DerivationSeed { seed_hash, network })
            }
            // Unknown format
            _ => Err(serde::de::Error::custom("Unknown GenericKeyHandle format")),
        }
    }
}
