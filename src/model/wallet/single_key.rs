//! Single Key Wallet - A wallet backed by a single private key (not HD derived)
//!
//! This module provides support for importing and using individual private keys
//! as wallets, similar to the functionality in platform-tui.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, PrivateKey, PublicKey, TxOut};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use zeroize::Zeroize;

use super::encryption::derive_password_key;

/// Hash of the private key, used as a unique identifier
pub type SingleKeyHash = [u8; 32];

/// A wallet backed by a single private key
#[derive(Debug, Clone, PartialEq)]
pub struct SingleKeyWallet {
    /// The private key data (open or closed/encrypted)
    pub private_key_data: SingleKeyData,
    /// Whether a password is required to access the private key
    pub uses_password: bool,
    /// The public key derived from the private key
    pub public_key: PublicKey,
    /// The P2PKH address derived from the public key
    pub address: Address,
    /// Optional alias/name for this wallet
    pub alias: Option<String>,
    /// SHA-256 hash of the private key (used as identifier)
    pub key_hash: SingleKeyHash,
    /// Confirmed balance in duffs
    pub confirmed_balance: u64,
    /// Unconfirmed balance in duffs
    pub unconfirmed_balance: u64,
    /// Total balance in duffs
    pub total_balance: u64,
    /// UTXOs for this address
    pub utxos: HashMap<OutPoint, TxOut>,
}

/// Private key data - either open (decrypted) or closed (encrypted)
#[derive(Debug, Clone, PartialEq)]
pub enum SingleKeyData {
    Open(OpenSingleKey),
    Closed(ClosedSingleKey),
}

/// An open (decrypted) single key
#[derive(Clone, PartialEq)]
pub struct OpenSingleKey {
    /// The raw 32-byte private key
    pub private_key: [u8; 32],
    /// The closed key info for re-encryption
    pub key_info: ClosedSingleKey,
}

impl std::fmt::Debug for OpenSingleKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSingleKey")
            .field("key_hash", &hex::encode(self.key_info.key_hash))
            .finish()
    }
}

/// A closed (encrypted) single key
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedSingleKey {
    /// SHA-256 hash of the private key
    pub key_hash: SingleKeyHash,
    /// The encrypted private key
    pub encrypted_private_key: Vec<u8>,
    /// Salt used for key derivation
    pub salt: Vec<u8>,
    /// Nonce used for encryption
    pub nonce: Vec<u8>,
}

impl SingleKeyData {
    /// Opens the key by decrypting it using the provided password
    pub fn open(&mut self, password: &str) -> Result<(), String> {
        match self {
            SingleKeyData::Open(_) => Ok(()),
            SingleKeyData::Closed(closed) => {
                let private_key = closed.decrypt_private_key(password)?;
                let open_key = OpenSingleKey {
                    private_key,
                    key_info: closed.clone(),
                };
                *self = SingleKeyData::Open(open_key);
                Ok(())
            }
        }
    }

    /// Opens the key without a password (for keys stored without encryption)
    pub fn open_no_password(&mut self) -> Result<(), String> {
        match self {
            SingleKeyData::Open(_) => Ok(()),
            SingleKeyData::Closed(closed) => {
                let private_key: [u8; 32] = closed
                    .encrypted_private_key
                    .clone()
                    .try_into()
                    .map_err(|e: Vec<u8>| {
                        format!("incorrect key size, expected 32 bytes, got {}", e.len())
                    })?;
                let open_key = OpenSingleKey {
                    private_key,
                    key_info: closed.clone(),
                };
                *self = SingleKeyData::Open(open_key);
                Ok(())
            }
        }
    }

    /// Closes the key by securely erasing the decrypted data
    #[allow(dead_code)]
    pub fn close(&mut self) {
        if let SingleKeyData::Open(open_key) = self {
            let key_info = open_key.key_info.clone();
            open_key.private_key.zeroize();
            *self = SingleKeyData::Closed(key_info);
        }
    }

    /// Returns true if the key is open (decrypted)
    pub fn is_open(&self) -> bool {
        matches!(self, SingleKeyData::Open(_))
    }

    /// Get the key hash
    pub fn key_hash(&self) -> SingleKeyHash {
        match self {
            SingleKeyData::Open(open) => open.key_info.key_hash,
            SingleKeyData::Closed(closed) => closed.key_hash,
        }
    }
}

impl Drop for SingleKeyData {
    fn drop(&mut self) {
        if let SingleKeyData::Open(open_key) = self {
            open_key.private_key.zeroize();
        }
    }
}

impl ClosedSingleKey {
    /// Compute the hash of a private key
    pub fn compute_key_hash(private_key: &[u8; 32]) -> SingleKeyHash {
        let mut hasher = Sha256::new();
        hasher.update(private_key);
        let result = hasher.finalize();
        let mut key_hash = [0u8; 32];
        key_hash.copy_from_slice(&result);
        key_hash
    }

    /// Encrypt a private key with a password
    #[allow(clippy::type_complexity)]
    pub fn encrypt_private_key(
        private_key: &[u8; 32],
        password: &str,
    ) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
        use super::encryption::encrypt_message;
        encrypt_message(private_key, password)
    }

    /// Decrypt the private key using a password
    #[allow(deprecated)]
    pub fn decrypt_private_key(&self, password: &str) -> Result<[u8; 32], String> {
        let key = derive_password_key(password, &self.salt)?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
        let nonce_arr = Nonce::from_slice(&self.nonce);
        let decrypted = cipher
            .decrypt(nonce_arr, self.encrypted_private_key.as_slice())
            .map_err(|e| e.to_string())?;

        decrypted.try_into().map_err(|e: Vec<u8>| {
            format!(
                "invalid private key length, expected 32 bytes, got {} bytes",
                e.len()
            )
        })
    }
}

impl SingleKeyWallet {
    /// Create a new SingleKeyWallet from a private key
    ///
    /// # Arguments
    /// * `private_key_bytes` - The 32-byte private key
    /// * `network` - The network (mainnet, testnet, etc.)
    /// * `password` - Optional password to encrypt the key
    /// * `alias` - Optional alias for the wallet
    pub fn new(
        private_key_bytes: [u8; 32],
        network: Network,
        password: Option<&str>,
        alias: Option<String>,
    ) -> Result<Self, String> {
        let secp = Secp256k1::new();

        // Create PrivateKey and derive public key and address
        let private_key =
            PrivateKey::from_byte_array(&private_key_bytes, network).map_err(|e| e.to_string())?;
        let public_key = private_key.public_key(&secp);
        let address = Address::p2pkh(&public_key, network);

        let key_hash = ClosedSingleKey::compute_key_hash(&private_key_bytes);

        let (private_key_data, uses_password) = if let Some(pwd) = password {
            let (encrypted, salt, nonce) =
                ClosedSingleKey::encrypt_private_key(&private_key_bytes, pwd)?;
            let closed = ClosedSingleKey {
                key_hash,
                encrypted_private_key: encrypted,
                salt,
                nonce,
            };
            // Keep it open after creation
            (
                SingleKeyData::Open(OpenSingleKey {
                    private_key: private_key_bytes,
                    key_info: closed,
                }),
                true,
            )
        } else {
            // No password - store raw bytes as "encrypted"
            let closed = ClosedSingleKey {
                key_hash,
                encrypted_private_key: private_key_bytes.to_vec(),
                salt: vec![],
                nonce: vec![],
            };
            (
                SingleKeyData::Open(OpenSingleKey {
                    private_key: private_key_bytes,
                    key_info: closed,
                }),
                false,
            )
        };

        Ok(Self {
            private_key_data,
            uses_password,
            public_key,
            address,
            alias,
            key_hash,
            confirmed_balance: 0,
            unconfirmed_balance: 0,
            total_balance: 0,
            utxos: HashMap::new(),
        })
    }

    /// Create from a WIF-encoded private key string
    pub fn from_wif(
        wif: &str,
        password: Option<&str>,
        alias: Option<String>,
    ) -> Result<Self, String> {
        let private_key = PrivateKey::from_wif(wif).map_err(|e| e.to_string())?;
        let network = private_key.network;
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&private_key.inner[..]);
        Self::new(key_bytes, network, password, alias)
    }

    /// Create from a hex-encoded private key string
    pub fn from_hex(
        hex_str: &str,
        network: Network,
        password: Option<&str>,
        alias: Option<String>,
    ) -> Result<Self, String> {
        let bytes = hex::decode(hex_str).map_err(|e| e.to_string())?;
        if bytes.len() != 32 {
            return Err(format!(
                "Invalid private key length: expected 32 bytes, got {}",
                bytes.len()
            ));
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);
        Self::new(key_bytes, network, password, alias)
    }

    /// Returns true if the wallet is open (private key is decrypted)
    pub fn is_open(&self) -> bool {
        self.private_key_data.is_open()
    }

    /// Open the wallet with a password
    pub fn open(&mut self, password: &str) -> Result<(), String> {
        self.private_key_data.open(password)
    }

    /// Open the wallet without a password
    pub fn open_no_password(&mut self) -> Result<(), String> {
        self.private_key_data.open_no_password()
    }

    /// Get the key hash (identifier)
    pub fn key_hash(&self) -> SingleKeyHash {
        self.key_hash
    }

    /// Get the encrypted private key bytes
    pub fn encrypted_private_key(&self) -> &[u8] {
        match &self.private_key_data {
            SingleKeyData::Open(open) => &open.key_info.encrypted_private_key,
            SingleKeyData::Closed(closed) => &closed.encrypted_private_key,
        }
    }

    /// Get the salt
    pub fn salt(&self) -> &[u8] {
        match &self.private_key_data {
            SingleKeyData::Open(open) => &open.key_info.salt,
            SingleKeyData::Closed(closed) => &closed.salt,
        }
    }

    /// Get the nonce
    pub fn nonce(&self) -> &[u8] {
        match &self.private_key_data {
            SingleKeyData::Open(open) => &open.key_info.nonce,
            SingleKeyData::Closed(closed) => &closed.nonce,
        }
    }

    /// Get the private key if the wallet is open
    pub fn private_key(&self, network: Network) -> Option<PrivateKey> {
        match &self.private_key_data {
            SingleKeyData::Open(open) => {
                PrivateKey::from_byte_array(&open.private_key, network).ok()
            }
            SingleKeyData::Closed(_) => None,
        }
    }

    /// Calculate balance from UTXOs
    pub fn utxo_balance(&self) -> u64 {
        self.utxos.values().map(|tx_out| tx_out.value).sum()
    }

    /// Get the confirmed balance
    pub fn confirmed_balance_duffs(&self) -> u64 {
        if self.total_balance > 0 || self.confirmed_balance > 0 || self.unconfirmed_balance > 0 {
            self.confirmed_balance
        } else {
            self.utxo_balance()
        }
    }

    /// Get the unconfirmed balance
    pub fn unconfirmed_balance_duffs(&self) -> u64 {
        self.unconfirmed_balance
    }

    /// Get the total balance
    pub fn total_balance_duffs(&self) -> u64 {
        if self.total_balance > 0 {
            self.total_balance
        } else {
            self.utxo_balance()
        }
    }

    /// Update balances
    pub fn update_balances(&mut self, confirmed: u64, unconfirmed: u64, total: u64) {
        self.confirmed_balance = confirmed;
        self.unconfirmed_balance = unconfirmed;
        self.total_balance = total;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_single_key_wallet_no_password() {
        let private_key = [42u8; 32];
        let wallet = SingleKeyWallet::new(
            private_key,
            Network::Testnet,
            None,
            Some("Test".to_string()),
        )
        .expect("Failed to create wallet");

        assert!(wallet.is_open());
        assert!(!wallet.uses_password);
        assert_eq!(wallet.alias, Some("Test".to_string()));
        assert!(wallet.private_key(Network::Testnet).is_some());
    }

    #[test]
    fn test_create_single_key_wallet_with_password() {
        let private_key = [42u8; 32];
        let password = "secret123";
        let wallet = SingleKeyWallet::new(
            private_key,
            Network::Testnet,
            Some(password),
            Some("Encrypted".to_string()),
        )
        .expect("Failed to create wallet");

        assert!(wallet.is_open());
        assert!(wallet.uses_password);
    }

    #[test]
    fn test_from_hex() {
        let hex_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let wallet = SingleKeyWallet::from_hex(hex_key, Network::Testnet, None, None)
            .expect("Failed to create from hex");

        assert!(wallet.is_open());
        assert!(!wallet.address.to_string().is_empty());
    }
}
