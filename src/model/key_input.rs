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
use zeroize::Zeroizing;

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
/// The bytes are decoded straight into [`Zeroizing`], so this copy of the key
/// is wiped when the caller drops it.
///
/// # Errors
///
/// Returns a [`KeyInputError`] naming `key_name` when the input has a hex/WIF
/// length but fails to decode, or when its length matches no supported format.
pub fn verify_key_input(
    untrimmed_private_key: Secret,
    key_name: &str,
) -> Result<Option<Zeroizing<[u8; 32]>>, KeyInputError> {
    let private_key = untrimmed_private_key.expose_secret().trim();
    match private_key.len() {
        64 => {
            let mut bytes = Zeroizing::new([0u8; 32]);
            hex::decode_to_slice(private_key, bytes.as_mut_slice())
                .map(|()| Some(bytes))
                .map_err(|_| KeyInputError::NotHex {
                    key_name: key_name.to_owned(),
                })
        }
        51 | 52 => PrivateKey::from_wif(private_key)
            .map(|key| Some(key.inner.to_bytes()))
            .map_err(|_| KeyInputError::BadWif {
                key_name: key_name.to_owned(),
            }),
        0 => Ok(None),
        _ => Err(KeyInputError::UnsupportedLength {
            key_name: key_name.to_owned(),
        }),
    }
}

/// Decode hex-encoded secret bytes into memory that is wiped on drop.
///
/// Accepts and rejects exactly the inputs [`hex::decode`] does, with the same
/// error, but decodes straight into a [`Zeroizing`] buffer of the final size,
/// so the decoded bytes, partly decoded ones included, leave no plain copy
/// behind.
///
/// # Errors
///
/// Returns the [`hex::FromHexError`] `hex::decode` would: `OddLength` for an
/// odd number of characters, otherwise `InvalidHexCharacter` for the first
/// character that is not a hex digit.
pub fn decode_hex_secret(input: &str) -> Result<Zeroizing<Vec<u8>>, hex::FromHexError> {
    // Sized to `len / 2`, so `decode_to_slice` never reports a length
    // mismatch: it checks for an odd length first, as `hex::decode` does.
    let mut bytes = Zeroizing::new(vec![0u8; input.len() / 2]);
    hex::decode_to_slice(input, bytes.as_mut_slice())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dashcore_rpc::dashcore::Network;

    /// `decode_hex_secret` accepts and rejects exactly what `hex::decode`
    /// does, with the same bytes and the same error, so callers that switch
    /// to it keep their behaviour and messages.
    #[test]
    fn decode_hex_secret_matches_hex_decode() {
        for input in [
            "",
            "00",
            "0aFf",
            &"ab".repeat(32),
            &"cd".repeat(33),
            "abc",
            "zz",
            "0g",
            &format!("{}x1", "a".repeat(62)),
            "not hex at all",
        ] {
            assert_eq!(
                decode_hex_secret(input).map(|bytes| bytes.to_vec()),
                hex::decode(input),
                "input {input:?}"
            );
        }
    }

    #[test]
    fn empty_input_is_not_supplied() {
        assert_eq!(verify_key_input(Secret::new(""), "Owner").unwrap(), None);
        // Whitespace-only trims to empty and is also treated as "not supplied".
        assert_eq!(verify_key_input(Secret::new("   "), "Owner").unwrap(), None);
    }

    #[test]
    fn valid_hex_decodes_to_bytes() {
        let bytes = crate::test_support::keys::secret_bytes();
        let hex_str = hex::encode(bytes);
        assert_eq!(
            verify_key_input(Secret::new(hex_str), "User Key")
                .unwrap()
                .as_deref(),
            Some(&bytes)
        );
    }

    /// The decoded key is handed out in `Zeroizing`, so the caller's copy is
    /// wiped when it is dropped.
    #[test]
    fn decoded_key_is_returned_zeroizing() {
        let secret = crate::test_support::keys::secret_bytes();
        let key: Option<Zeroizing<[u8; 32]>> =
            verify_key_input(Secret::new(hex::encode(secret)), "Owner").unwrap();
        assert_eq!(key.as_deref(), Some(&secret));
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
        let bytes = crate::test_support::keys::secret_bytes();
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
            verify_key_input(Secret::new(wif), "Owner")
                .unwrap()
                .as_deref(),
            Some(&bytes)
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
        let bytes = crate::test_support::keys::secret_bytes();
        let padded = format!("  {}\n", hex::encode(bytes));
        assert_eq!(
            verify_key_input(Secret::new(padded), "User Key")
                .unwrap()
                .as_deref(),
            Some(&bytes)
        );
    }
}
