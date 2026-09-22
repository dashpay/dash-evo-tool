//! Stateless private-key input parsing for identity loading.
//!
//! [`verify_key_input`] is the single source of truth for the raw private-key
//! format policy: an empty input means "not supplied", a 64-character input is
//! decoded as hex, and a 51- or 52-character input is decoded as WIF. It holds
//! no state — no `AppContext`, `Sdk`, DB, or `BackendTask` — so it is
//! exhaustively unit-testable without a network. Stateful enforcement (matching
//! the key against an identity's public keys) stays in the backend task.

use crate::model::secret::Secret;
use dash_sdk::dashcore_rpc::dashcore::PrivateKey;
use thiserror::Error;

/// Why a raw private-key input failed format validation.
///
/// Each variant carries the human-readable key name so the message names which
/// field to fix; the `Display` text is a complete, actionable sentence.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyInputError {
    /// A 64-character input that is not valid hexadecimal.
    #[error(
        "The {key_name} key is the length of a hex key but is not valid hexadecimal. Please check the key and try again."
    )]
    NotHex { key_name: String },

    /// A 51- or 52-character input that is not a valid WIF key.
    #[error(
        "The {key_name} key is the length of a WIF key but could not be read. Please check the key and try again."
    )]
    BadWif { key_name: String },

    /// An input whose length matches neither the hex nor the WIF format.
    #[error(
        "The {key_name} key is not a valid length. Enter a 64-character hex key or a 51- or 52-character WIF key."
    )]
    UnsupportedLength { key_name: String },
}

/// Validate and decode a raw private-key input string into 32 secret bytes.
///
/// The input is trimmed before inspection. Length decides the format: 64 chars
/// are decoded as hex, 51 or 52 chars as WIF. An empty input yields `Ok(None)`,
/// signalling the key was not supplied.
///
/// # Errors
///
/// Returns a [`KeyInputError`] naming `key_name` when the input has a hex/WIF
/// length but fails to decode, or when its length matches no supported format.
pub fn verify_key_input(
    untrimmed_private_key: Secret,
    key_name: &str,
) -> Result<Option<[u8; 32]>, KeyInputError> {
    let private_key = untrimmed_private_key.expose_secret().trim();
    match private_key.len() {
        64 => hex::decode(private_key)
            .ok()
            .and_then(|decoded| decoded.try_into().ok())
            .map(Some)
            .ok_or_else(|| KeyInputError::NotHex {
                key_name: key_name.to_owned(),
            }),
        51 | 52 => PrivateKey::from_wif(private_key)
            .map(|key| Some(key.inner.secret_bytes()))
            .map_err(|_| KeyInputError::BadWif {
                key_name: key_name.to_owned(),
            }),
        0 => Ok(None),
        _ => Err(KeyInputError::UnsupportedLength {
            key_name: key_name.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dashcore_rpc::dashcore::Network;

    #[test]
    fn empty_input_is_not_supplied() {
        assert_eq!(verify_key_input(Secret::new(""), "Owner").unwrap(), None);
        // Whitespace-only trims to empty and is also treated as "not supplied".
        assert_eq!(verify_key_input(Secret::new("   "), "Owner").unwrap(), None);
    }

    #[test]
    fn valid_hex_decodes_to_bytes() {
        let bytes = [7u8; 32];
        let hex_str = hex::encode(bytes);
        assert_eq!(
            verify_key_input(Secret::new(hex_str), "User Key").unwrap(),
            Some(bytes)
        );
    }

    #[test]
    fn hex_length_but_not_hex_is_rejected() {
        let err = verify_key_input(Secret::new("z".repeat(64)), "Voting").unwrap_err();
        assert_eq!(
            err,
            KeyInputError::NotHex {
                key_name: "Voting".to_owned()
            }
        );
        assert!(err.to_string().contains("Voting"));
        assert!(err.to_string().contains("hex"));
    }

    #[test]
    fn valid_wif_round_trips_to_secret_bytes() {
        let bytes = [9u8; 32];
        let wif = PrivateKey::from_byte_array(&bytes, Network::Testnet)
            .expect("valid private key")
            .to_wif();
        // WIF length must land in the 51/52 branch for the round-trip to hold.
        assert!(
            matches!(wif.len(), 51 | 52),
            "unexpected WIF length: {}",
            wif.len()
        );
        assert_eq!(
            verify_key_input(Secret::new(wif), "Owner").unwrap(),
            Some(bytes)
        );
    }

    #[test]
    fn wif_length_but_invalid_is_rejected() {
        let err = verify_key_input(Secret::new("z".repeat(52)), "Payout Address").unwrap_err();
        assert_eq!(
            err,
            KeyInputError::BadWif {
                key_name: "Payout Address".to_owned()
            }
        );
        assert!(err.to_string().contains("Payout Address"));
        assert!(err.to_string().contains("WIF"));
    }

    #[test]
    fn unsupported_length_is_rejected() {
        for bad in ["tooshort", &"a".repeat(63), &"b".repeat(65)] {
            let err = verify_key_input(Secret::new(bad), "Owner").unwrap_err();
            assert_eq!(
                err,
                KeyInputError::UnsupportedLength {
                    key_name: "Owner".to_owned()
                },
                "expected {bad:?} to be rejected as an unsupported length"
            );
        }
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_before_decoding() {
        let bytes = [3u8; 32];
        let padded = format!("  {}\n", hex::encode(bytes));
        assert_eq!(
            verify_key_input(Secret::new(padded), "User Key").unwrap(),
            Some(bytes)
        );
    }
}
