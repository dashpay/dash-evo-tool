use dash_sdk::dpp::dashcore::bip32::DerivationPath;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Generic key handle used in the [GenericKms], used to identify keys in the KMS.
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub enum KeyHandle {
    PublicKeyBytes(Vec<u8>), // Public key bytes, used for encryption and verification
    Derived {
        seed_hash: Vec<u8>,              // Hash of the seed to use to derive the key
        derivation_path: DerivationPath, // Derivation path for the key; TODO:
    },
}

impl Serialize for KeyHandle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let key_string = match self {
            KeyHandle::PublicKeyBytes(bytes) => {
                format!("PublicKeyBytes({})", hex::encode_upper(bytes))
            }
            KeyHandle::Derived {
                seed_hash,
                derivation_path,
            } => {
                format!(
                    "Derived(seed_hash={}, derivation_path={})",
                    hex::encode_upper(seed_hash),
                    derivation_path
                )
            }
        };
        serializer.serialize_str(&key_string)
    }
}

impl<'de> Deserialize<'de> for KeyHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        match s {
            // Match PublicKeyBytes(HEX)
            s if s.starts_with("PublicKeyBytes(") && s.ends_with(")") => {
                let hex_str = &s[15..s.len() - 1]; // Remove "PublicKeyBytes(" and ")"
                let bytes = hex::decode(hex_str).map_err(|e| {
                    serde::de::Error::custom(format!("Invalid hex in PublicKeyBytes: {}", e))
                })?;
                Ok(KeyHandle::PublicKeyBytes(bytes))
            }
            // Match Derived(seed_hash=HEX, derivation_path=PATH)
            s if s.starts_with("Derived(") && s.ends_with(")") => {
                let inner = &s[8..s.len() - 1]; // Remove "Derived(" and ")"

                // Parse "seed_hash=HEX, derivation_path=PATH"
                let parts: Vec<&str> = inner.splitn(2, ", derivation_path=").collect();
                if parts.len() != 2 {
                    return Err(serde::de::Error::custom("Invalid Derived format"));
                }

                let seed_hash_part = parts[0];
                let derivation_path_str = parts[1];

                let hex_str = seed_hash_part
                    .strip_prefix("seed_hash=")
                    .ok_or(serde::de::Error::custom("Missing seed_hash= prefix"))?;

                let seed_hash = hex::decode(hex_str).map_err(|e| {
                    serde::de::Error::custom(format!("Invalid hex in seed_hash: {}", e))
                })?;

                let derivation_path =
                    derivation_path_str.parse::<DerivationPath>().map_err(|e| {
                        serde::de::Error::custom(format!("Invalid derivation path: {}", e))
                    })?;

                Ok(KeyHandle::Derived {
                    seed_hash,
                    derivation_path,
                })
            }
            // Unknown format
            _ => Err(serde::de::Error::custom("Unknown GenericKeyHandle format")),
        }
    }
}
