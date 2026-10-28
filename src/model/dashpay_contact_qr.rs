//! DashPay contact QR codes (DIP-0015 `dash:?du=<username>&dapk=<key>`).
//!
//! The format, the key blob and the proof all come from platform-wallet's
//! `auto_accept` module; this type only validates a scanned code once and
//! carries the result to the backend. The `dapk` key is the QR owner's
//! auto-accept key, a short-lived bearer credential: it lets the scanner sign
//! the `autoAcceptProof` of its contact request, nothing else.

use std::sync::Arc;

use dash_sdk::dpp::secp256k1::SecretKey;
use dash_sdk::platform::Identifier;
use platform_wallet::error::PlatformWalletError;
use platform_wallet::wallet::identity::crypto::auto_accept::{
    decode_auto_accept_key_blob, parse_dashpay_contact_uri, sign_auto_accept_proof,
};

/// A scanned contact QR code that passed validation.
#[derive(Clone, PartialEq)]
pub struct ContactQrCode {
    username: String,
    key: SecretKey,
    expires_at: u32,
}

/// Why a scanned text is not a usable contact QR code.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ContactQrCodeError {
    /// The text is not a DIP-0015 contact code, or its key cannot be read.
    /// Codes from older Dash Evo Tool versions (`dash:?di=…`) land here.
    #[error(
        "This is not a DashPay contact QR code. Ask your contact for a new code and try again."
    )]
    Unreadable {
        #[source]
        source: Arc<PlatformWalletError>,
    },
    /// The code names something that is not a Dash username.
    #[error(
        "This QR code does not name a valid Dash username. Ask your contact for a new code and try again."
    )]
    InvalidUsername,
}

impl ContactQrCode {
    /// Parse and validate a scanned or pasted contact QR code.
    pub fn parse(text: &str) -> Result<Self, ContactQrCodeError> {
        let unreadable = |source| ContactQrCodeError::Unreadable {
            source: Arc::new(source),
        };
        let (username, key_blob) = parse_dashpay_contact_uri(text.trim()).map_err(unreadable)?;
        let username = username.trim().to_string();
        if username.is_empty() || crate::model::dpns::validate_dpns_input(&username).is_err() {
            return Err(ContactQrCodeError::InvalidUsername);
        }
        let (key, expires_at) = decode_auto_accept_key_blob(&key_blob).map_err(unreadable)?;
        Ok(Self {
            username,
            key,
            expires_at,
        })
    }

    /// The QR owner's Dash username, as written in the code.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The username in its full `.dash` form, so a lookup always resolves it
    /// through DPNS and never mistakes it for an identity ID.
    pub fn full_username(&self) -> String {
        if crate::model::dpns::has_dash_suffix(&self.username) {
            self.username.clone()
        } else {
            format!("{}.dash", self.username)
        }
    }

    /// The Unix time (seconds) after which the code no longer auto-accepts.
    pub fn expires_at(&self) -> u32 {
        self.expires_at
    }

    /// Whether the code has expired at `now_secs` (Unix seconds).
    pub fn is_expired_at(&self, now_secs: u64) -> bool {
        now_secs > u64::from(self.expires_at)
    }

    /// The `autoAcceptProof` for a contact request from `sender` to
    /// `recipient` (the QR owner) carrying `account_reference`.
    pub fn sign_proof(
        &self,
        sender: &Identifier,
        recipient: &Identifier,
        account_reference: u32,
    ) -> Vec<u8> {
        sign_auto_accept_proof(
            &self.key,
            sender,
            recipient,
            account_reference,
            self.expires_at,
        )
    }
}

impl std::fmt::Debug for ContactQrCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContactQrCode")
            .field("username", &self.username)
            .field("key", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::secp256k1::PublicKey;
    use platform_wallet::wallet::identity::crypto::auto_accept::{
        encode_auto_accept_key_blob, encode_dashpay_contact_uri,
        verify_auto_accept_proof_with_pubkey,
    };

    const EXPIRY: u32 = 1_900_000_000;

    fn key() -> SecretKey {
        SecretKey::from_secret_bytes(rand::random()).expect("key")
    }

    fn uri(username: &str, key: &SecretKey) -> String {
        encode_dashpay_contact_uri(username, &encode_auto_accept_key_blob(key, EXPIRY))
    }

    /// A code platform-wallet builds parses back to its username, expiry and
    /// key, and the proof it signs verifies against the owner's public key
    /// for exactly that sender, recipient and account reference.
    #[test]
    fn a_platform_wallet_code_parses_and_signs_a_verifiable_proof() {
        let key = key();
        let qr = ContactQrCode::parse(&uri("alice", &key)).expect("parse");
        assert_eq!(qr.username(), "alice");
        assert_eq!(qr.full_username(), "alice.dash");
        assert_eq!(qr.expires_at(), EXPIRY);

        let (scanner, owner) = (Identifier::from([1; 32]), Identifier::from([2; 32]));
        let proof = qr.sign_proof(&scanner, &owner, 7);
        let public = PublicKey::from_secret_key(&key);
        assert!(verify_auto_accept_proof_with_pubkey(
            &public, &proof, &scanner, &owner, 7
        ));
        assert!(!verify_auto_accept_proof_with_pubkey(
            &public, &proof, &scanner, &owner, 8
        ));
        assert!(!verify_auto_accept_proof_with_pubkey(
            &public,
            &proof,
            &Identifier::from([3; 32]),
            &owner,
            7
        ));
    }

    /// Codes from older Dash Evo Tool versions named the identity (`di=`)
    /// instead of the username (`du=`) and are refused.
    #[test]
    fn an_old_identity_id_code_is_refused() {
        let blob = encode_auto_accept_key_blob(&key(), EXPIRY);
        let old = format!(
            "dash:?di={}&dapk={}",
            Identifier::from([9; 32])
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
            dash_sdk::dpp::platform_value::string_encoding::encode(
                &blob,
                dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58
            )
        );
        assert!(matches!(
            ContactQrCode::parse(&old),
            Err(ContactQrCodeError::Unreadable { .. })
        ));
    }

    #[test]
    fn malformed_codes_are_refused() {
        for text in [
            "",
            "hello",
            "http:?du=alice&dapk=abc",
            "dash:?du=alice",
            "dash:?du=alice&dapk=0OIl",
            "dash:?du=alice&dapk=abc",
        ] {
            assert!(
                matches!(
                    ContactQrCode::parse(text),
                    Err(ContactQrCodeError::Unreadable { .. })
                ),
                "{text}"
            );
        }
    }

    #[test]
    fn a_code_without_a_dash_username_is_refused() {
        for username in ["", "  ", "alice.com"] {
            assert!(
                matches!(
                    ContactQrCode::parse(&uri(username, &key())),
                    Err(ContactQrCodeError::InvalidUsername)
                ),
                "{username:?}"
            );
        }
    }

    #[test]
    fn a_full_username_is_kept_as_is() {
        let qr = ContactQrCode::parse(&uri("alice.dash", &key())).expect("parse");
        assert_eq!(qr.full_username(), "alice.dash");
    }

    #[test]
    fn expiry_is_inclusive_of_the_last_second() {
        let qr = ContactQrCode::parse(&uri("alice", &key())).expect("parse");
        assert!(!qr.is_expired_at(u64::from(EXPIRY)));
        assert!(qr.is_expired_at(u64::from(EXPIRY) + 1));
    }

    /// The bearer key never reaches logs through `Debug`.
    #[test]
    fn debug_output_hides_the_key() {
        let key = key();
        let qr = ContactQrCode::parse(&uri("alice", &key)).expect("parse");
        let debug = format!("{qr:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(&hex::encode(key.to_secret_bytes())));
    }
}
