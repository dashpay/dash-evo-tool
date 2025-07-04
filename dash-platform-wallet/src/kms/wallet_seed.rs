use std::fmt::Debug;

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{AeadMutInPlace, Buffer, Payload},
};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::kms::{
    Secret,
    encryption::{derive_password_key, encrypt_message},
    generic::unlocked::AAD,
};

pub type WalletSeedHash = [u8; 32];

/// Decrypted seed
///
/// Not publicly exposed, as decrypted seed should never leave the KMS context.
#[derive(Clone, ZeroizeOnDrop, PartialEq, Eq)]
pub(super) struct DecryptedSeed(Vec<u8>);

impl DecryptedSeed {
    /// Creates a new `DecryptedSeed` from a byte slice.
    pub fn new(seed: Vec<u8>) -> Self {
        Self(seed)
    }

    pub fn as_bytes<const N: usize>(&self) -> &[u8; N] {
        self.0
            .as_slice()
            .try_into()
            .expect("DecryptedSeed should always be of fixed size")
    }
}

impl Debug for DecryptedSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DecryptedSeed")
            .field(&"[REDACTED]") // Do not expose the actual seed
            .finish()
    }
}

impl AsMut<[u8]> for DecryptedSeed {
    fn as_mut(&mut self) -> &mut [u8] {
        self.0.as_mut()
    }
}

impl AsRef<[u8]> for DecryptedSeed {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Buffer for DecryptedSeed {
    fn extend_from_slice(&mut self, other: &[u8]) -> aes_gcm::aead::Result<()> {
        self.0.extend_from_slice(other);
        Ok(())
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }
}

impl Zeroize for DecryptedSeed {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl<'msg, 'aad> From<&'msg DecryptedSeed> for Payload<'msg, 'aad> {
    fn from(decrypted_seed: &'msg DecryptedSeed) -> Self {
        Payload {
            msg: decrypted_seed.as_ref(),
            aad: AAD,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WalletSeed {
    Open(OpenWalletSeed),
    Closed(ClosedWalletSeed),
}
#[derive(Debug, Clone, PartialEq)]
pub struct OpenKeyItem<const N: usize> {
    pub seed: DecryptedSeed,
    pub wallet_info: ClosedKeyItem,
}

// Type alias for OpenWalletSeed with a fixed seed size of 64 bytes
pub type OpenWalletSeed = OpenKeyItem<64>;

#[derive(Debug, Clone, PartialEq)]
pub struct ClosedKeyItem {
    pub seed_hash: WalletSeedHash, // SHA-256 hash of the seed
    pub encrypted_seed: Vec<u8>,
    pub salt: Vec<u8>,
    pub nonce: Vec<u8>,
    pub password_hint: Option<String>,
}

pub type ClosedWalletSeed = ClosedKeyItem;

impl WalletSeed {
    /// Opens the wallet by decrypting the seed using the provided password.
    pub fn open(&mut self, user_id: &[u8], password: &Secret) -> Result<(), String> {
        match self {
            WalletSeed::Open(_) => {
                // Wallet is already open
                Ok(())
            }
            WalletSeed::Closed(closed_seed) => {
                // Try to decrypt the seed
                let wallet_info = closed_seed.clone();
                let seed = self.decrypt_seed(user_id, password)?;
                let open_wallet_seed = OpenWalletSeed { seed, wallet_info };
                *self = WalletSeed::Open(open_wallet_seed);
                Ok(())
            }
        }
    }

    /// Opens the wallet by decrypting the seed without using a password.
    pub fn open_no_password(&mut self) -> Result<(), String> {
        self.open(&[], &Secret::new(vec![]).map_err(|e| e.to_string())?)
    }

    /// Closes the wallet by securely erasing the seed and transitioning to Closed state.
    // Allow dead_code: This method provides explicit wallet closure functionality,
    // useful for security-conscious applications requiring manual wallet management
    #[allow(dead_code)]
    pub fn close(&mut self) {
        match self {
            WalletSeed::Open(open_seed) => {
                // Zeroize the seed
                open_seed.seed.zeroize();
                // Transition back to ClosedWalletSeed
                let closed_seed = open_seed.wallet_info.clone();
                *self = WalletSeed::Closed(closed_seed);
            }
            WalletSeed::Closed(_) => {
                // Wallet is already closed
            }
        }
    }
    /// Encrypt the seed using AES-256-GCM.
    /// ## Returns
    ///
    /// Returns a tuple containing the encrypted seed and nonce.
    #[allow(clippy::type_complexity)]
    fn encrypt_seed<const N: usize>(
        seed: DecryptedSeed,
        user_id: &[u8],
        password: &Secret,
    ) -> Result<(Vec<u8>, Vec<u8>), String> {
        encrypt_message(&seed, user_id, password)
    }

    /// Decrypt the seed using AES-256-GCM.
    ///
    /// user_id is used as salt for the key derivation.
    fn decrypt_seed(&self, user_id: &[u8], password: &Secret) -> Result<DecryptedSeed, String> {
        let (mut seed, nonce) = match self {
            WalletSeed::Closed(ClosedKeyItem {
                encrypted_seed,
                nonce,
                ..
            }) => (DecryptedSeed::new(encrypted_seed.clone()), nonce),
            WalletSeed::Open(OpenKeyItem { seed, .. }) => {
                return Ok(seed.clone());
            }
        };

        // Derive the key
        let key = derive_password_key(user_id, password)?;

        // Create cipher instance
        let mut cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|e| e.to_string())?;

        cipher
            .decrypt_in_place(nonce.as_slice().into(), AAD, &mut seed)
            .map_err(|e| e.to_string())?;
        // .decrypt(Nonce::from_slice(nonce), encrypted_seed.as_slice());

        Ok(seed)
    }
}

impl Drop for WalletSeed {
    fn drop(&mut self) {
        // Securely erase sensitive data
        if let WalletSeed::Open(open_seed) = self {
            open_seed.seed.zeroize();
        }
    }
}

pub(super) fn compute_seed_hash(seed: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(seed);
    let result = hasher.finalize();
    let mut seed_hash = [0u8; 32];
    seed_hash.copy_from_slice(&result);
    seed_hash
}
