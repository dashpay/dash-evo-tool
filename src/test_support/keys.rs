//! Key material for tests, generated at runtime.
//!
//! The repository never commits private keys, seeds, WIFs or recovery phrases
//! (see AGENTS.md, "Never commit secrets"). A deterministic byte pattern such
//! as `[0x2B; 32]` is still a valid private key, so tests draw their key
//! material from here instead.
//!
//! - [`seed`], [`secret_bytes`], [`secret_key`], [`private_key`], [`wif`] and
//!   [`mnemonic`] return fresh random material on every call. Generate once
//!   into a local and reuse it when a test needs the same key twice.
//! - [`labelled_seed`], [`labelled_secret`] and [`labelled_wif`] are for
//!   helpers whose contract is "the same index gives the same key" (e.g.
//!   `addr(n)`). The bytes are derived from a per-process random salt, so a
//!   `(domain, index)` label maps to the same key within one test-binary run
//!   and to a different key on every run. Each helper passes its own
//!   `domain`, so two helpers called with the same index still get different
//!   keys.
//!
//! To replay a failure that depends on labelled keys, set
//! `DET_TEST_KEY_SALT` to the 64-hex-character salt the failing run printed
//! to stderr.

use std::cell::Cell;
use std::sync::OnceLock;

use dash_sdk::dpp::dashcore::hashes::{Hash, sha256};
use dash_sdk::dpp::dashcore::{Network, PrivateKey};
use dash_sdk::dpp::secp256k1::SecretKey;

/// Environment variable that pins the labelled-key salt for replay.
pub const SALT_ENV: &str = "DET_TEST_KEY_SALT";

/// A random 64-byte HD wallet seed.
pub fn seed() -> [u8; 64] {
    rand::random()
}

/// Random bytes that form a valid secp256k1 secret key.
pub fn secret_bytes() -> [u8; 32] {
    loop {
        let candidate: [u8; 32] = rand::random();
        if SecretKey::from_secret_bytes(candidate).is_ok() {
            return candidate;
        }
    }
}

/// A random secp256k1 secret key.
pub fn secret_key() -> SecretKey {
    SecretKey::from_secret_bytes(secret_bytes()).expect("secret_bytes returns a valid scalar")
}

/// A random compressed private key on `network`.
pub fn private_key(network: Network) -> PrivateKey {
    PrivateKey::from_byte_array(&secret_bytes(), network)
        .expect("secret_bytes returns a valid scalar")
}

/// The WIF encoding of a random compressed private key on `network`.
pub fn wif(network: Network) -> String {
    private_key(network).to_wif()
}

/// A random English BIP39 mnemonic with `word_count` words (12, 15, 18, 21
/// or 24).
pub fn mnemonic(word_count: usize) -> bip39::Mnemonic {
    assert!(
        matches!(word_count, 12 | 15 | 18 | 21 | 24),
        "BIP39 word count must be 12, 15, 18, 21 or 24, got {word_count}"
    );
    let entropy: [u8; 32] = rand::random();
    let entropy_len = word_count / 3 * 4;
    bip39::Mnemonic::from_entropy(&entropy[..entropy_len]).expect("valid entropy length")
}

/// A 64-byte HD seed that is stable for `(domain, index)` within this
/// process.
pub fn labelled_seed(domain: &str, index: u64) -> [u8; 64] {
    let mut seed = [0u8; 64];
    seed[..32].copy_from_slice(&derive("seed", domain, index, 0));
    seed[32..].copy_from_slice(&derive("seed", domain, index, 1));
    seed
}

/// Valid secp256k1 secret-key bytes that are stable for `(domain, index)`
/// within this process.
pub fn labelled_secret(domain: &str, index: u64) -> [u8; 32] {
    (0u32..)
        .map(|attempt| derive("secret", domain, index, attempt))
        .find(|candidate| SecretKey::from_secret_bytes(*candidate).is_ok())
        .expect("an unbounded search finds a valid scalar")
}

/// The WIF of a compressed private key on `network` that is stable for
/// `(domain, index)` within this process.
pub fn labelled_wif(domain: &str, index: u64, network: Network) -> String {
    PrivateKey::from_byte_array(&labelled_secret(domain, index), network)
        .expect("labelled_secret returns a valid scalar")
        .to_wif()
}

/// SHA-256(salt ‖ kind ‖ len(domain) ‖ domain ‖ index ‖ counter).
fn derive(kind: &str, domain: &str, index: u64, counter: u32) -> [u8; 32] {
    let mut input = Vec::with_capacity(64 + kind.len() + domain.len());
    input.extend_from_slice(salt());
    input.extend_from_slice(kind.as_bytes());
    input.extend_from_slice(&(domain.len() as u64).to_be_bytes());
    input.extend_from_slice(domain.as_bytes());
    input.extend_from_slice(&index.to_be_bytes());
    input.extend_from_slice(&counter.to_be_bytes());
    sha256::Hash::hash(&input).to_byte_array()
}

/// The per-process salt behind the labelled keys: `DET_TEST_KEY_SALT` when
/// set, otherwise random.
///
/// The salt is printed to stderr once per thread, so it lands in the
/// captured output of every test that uses labelled keys.
fn salt() -> &'static [u8; 32] {
    static SALT: OnceLock<[u8; 32]> = OnceLock::new();
    thread_local! {
        static REPORTED: Cell<bool> = const { Cell::new(false) };
    }
    let salt = SALT.get_or_init(|| match std::env::var(SALT_ENV) {
        Ok(pinned) => {
            let bytes = hex::decode(pinned.trim())
                .unwrap_or_else(|e| panic!("{SALT_ENV} must be hex: {e}"));
            bytes
                .try_into()
                .unwrap_or_else(|_| panic!("{SALT_ENV} must be 32 bytes (64 hex characters)"))
        }
        Err(_) => rand::random(),
    });
    if !REPORTED.replace(true) {
        eprintln!("test key salt: {SALT_ENV}={}", hex::encode(salt));
    }
    salt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_draws_differ() {
        assert_ne!(seed(), seed());
        assert_ne!(secret_bytes(), secret_bytes());
        assert_ne!(wif(Network::Testnet), wif(Network::Testnet));
    }

    #[test]
    fn labelled_draws_are_stable_per_label() {
        assert_eq!(labelled_seed("a", 7), labelled_seed("a", 7));
        assert_ne!(labelled_seed("a", 7), labelled_seed("a", 8));
        assert_ne!(labelled_seed("a", 7), labelled_seed("b", 7));
        assert_eq!(labelled_secret("a", 7), labelled_secret("a", 7));
        assert_ne!(labelled_secret("a", 7), labelled_secret("a", 8));
        assert_ne!(labelled_secret("a", 7), labelled_secret("b", 7));
        assert_ne!(labelled_secret("a", 7)[..], labelled_seed("a", 7)[..32]);
    }

    #[test]
    fn generated_material_is_usable() {
        assert!(SecretKey::from_secret_bytes(labelled_secret("a", 1)).is_ok());
        let key = private_key(Network::Testnet);
        assert!(key.compressed);
        assert_eq!(
            PrivateKey::from_wif(&key.to_wif()).expect("round trip"),
            key
        );
        for words in [12, 24] {
            assert_eq!(mnemonic(words).word_count(), words);
        }
    }
}
