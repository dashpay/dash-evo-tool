use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::wallet::{Wallet, WalletSeedHash};
use bincode::de::{BorrowDecoder, Decoder};
use bincode::enc::Encoder;
use bincode::error::{DecodeError, EncodeError};
use bincode::{BorrowDecode, Decode, Encode};
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyID, Purpose, SecurityLevel};
use dash_sdk::dpp::key_wallet::bip32::ChildNumber;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq)]
pub struct WalletDerivationPath {
    pub(crate) wallet_seed_hash: WalletSeedHash,
    pub(crate) derivation_path: DerivationPath,
}

impl Encode for WalletDerivationPath {
    fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), EncodeError> {
        // Encode `wallet_seed_hash`
        self.wallet_seed_hash.encode(encoder)?;

        // Encode the length of the `DerivationPath`
        self.derivation_path.len().encode(encoder)?;

        // Encode each `ChildNumber` in the `DerivationPath`
        for child in &self.derivation_path {
            match child {
                ChildNumber::Normal { index } => {
                    0u8.encode(encoder)?; // Discriminant for Normal
                    index.encode(encoder)?;
                }
                ChildNumber::Hardened { index } => {
                    1u8.encode(encoder)?; // Discriminant for Hardened
                    index.encode(encoder)?;
                }
                ChildNumber::Normal256 { index } => {
                    2u8.encode(encoder)?; // Discriminant for Normal256
                    index.encode(encoder)?;
                }
                ChildNumber::Hardened256 { index } => {
                    3u8.encode(encoder)?; // Discriminant for Hardened256
                    index.encode(encoder)?;
                }
            }
        }

        Ok(())
    }
}

impl<C> Decode<C> for WalletDerivationPath {
    fn decode<D: Decoder<Context = C>>(decoder: &mut D) -> Result<Self, DecodeError> {
        // Decode `wallet_seed_hash`
        let wallet_seed_hash = WalletSeedHash::decode(decoder)?;

        // Decode the length of the `DerivationPath`
        let path_len = usize::decode(decoder)?;

        // Decode each `ChildNumber` in the `DerivationPath`
        let mut path = Vec::with_capacity(path_len);
        for _ in 0..path_len {
            let discriminant = u8::decode(decoder)?;
            let child_number = match discriminant {
                0 => ChildNumber::Normal {
                    index: u32::decode(decoder)?,
                },
                1 => ChildNumber::Hardened {
                    index: u32::decode(decoder)?,
                },
                2 => ChildNumber::Normal256 {
                    index: <[u8; 32]>::decode(decoder)?,
                },
                3 => ChildNumber::Hardened256 {
                    index: <[u8; 32]>::decode(decoder)?,
                },
                _ => return Err(DecodeError::OtherString("Invalid ChildNumber type".into())),
            };
            path.push(child_number);
        }

        let derivation_path = DerivationPath::from(path);
        Ok(Self {
            wallet_seed_hash,
            derivation_path,
        })
    }
}

impl<'de, C> BorrowDecode<'de, C> for WalletDerivationPath {
    fn borrow_decode<D: BorrowDecoder<'de, Context = C>>(
        decoder: &mut D,
    ) -> Result<Self, DecodeError> {
        // Decode `wallet_seed_hash`
        let wallet_seed_hash = WalletSeedHash::decode(decoder)?;

        // Decode the length of the `DerivationPath`
        let path_len = usize::decode(decoder)?;

        // Decode each `ChildNumber` in the `DerivationPath`
        let mut path = Vec::with_capacity(path_len);
        for _ in 0..path_len {
            let discriminant = u8::decode(decoder)?;
            let child_number = match discriminant {
                0 => ChildNumber::Normal {
                    index: u32::decode(decoder)?,
                },
                1 => ChildNumber::Hardened {
                    index: u32::decode(decoder)?,
                },
                2 => ChildNumber::Normal256 {
                    index: <[u8; 32]>::decode(decoder)?,
                },
                3 => ChildNumber::Hardened256 {
                    index: <[u8; 32]>::decode(decoder)?,
                },
                _ => return Err(DecodeError::OtherString("Invalid ChildNumber type".into())),
            };
            path.push(child_number);
        }

        let derivation_path = DerivationPath::from(path);
        Ok(Self {
            wallet_seed_hash,
            derivation_path,
        })
    }
}

#[derive(Clone, Encode, Decode, PartialEq)]
pub enum PrivateKeyData {
    AlwaysClear([u8; 32]), // This is for keys that are MEDIUM security level
    Clear([u8; 32]),
    Encrypted(Vec<u8>),
    AtWalletDerivationPath(WalletDerivationPath),
}

impl fmt::Debug for PrivateKeyData {
    /// Redacting `Debug`: never prints raw plaintext private-key bytes.
    ///
    /// The `Clear`/`AlwaysClear` variants hold raw identity private keys, so
    /// each is rendered as the variant name plus a SHA-256 fingerprint (for
    /// distinguishing keys in logs) and the byte length — never the key
    /// itself. `Encrypted` prints its length only. `KeyStorage`,
    /// `QualifiedIdentity`, and everything else that derives `Debug` delegate
    /// here, so redacting once protects the whole chain.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrivateKeyData::Clear(data) => f
                .debug_tuple("Clear")
                .field(&format_args!("fingerprint={}", fingerprint(data)))
                .finish(),
            PrivateKeyData::AlwaysClear(data) => f
                .debug_tuple("AlwaysClear")
                .field(&format_args!("fingerprint={}", fingerprint(data)))
                .finish(),
            PrivateKeyData::Encrypted(data) => f
                .debug_tuple("Encrypted")
                .field(&format_args!("{} bytes", data.len()))
                .finish(),
            PrivateKeyData::AtWalletDerivationPath(path) => {
                f.debug_tuple("AtWalletDerivationPath").field(path).finish()
            }
        }
    }
}

/// Non-reversible fingerprint of secret key bytes for redacted `Debug`:
/// the first 8 bytes of their SHA-256, hex-encoded. Lets two distinct keys
/// be told apart in logs without exposing either.
fn fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    hex::encode(&digest[..8])
}

impl fmt::Display for PrivateKeyData {
    /// Redacting `Display`: mirrors the `Debug` impl and never prints raw
    /// plaintext private-key bytes for the `Clear`/`AlwaysClear` variants.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrivateKeyData::Clear(data) => {
                write!(f, "Clear(fingerprint={})", fingerprint(data))
            }
            PrivateKeyData::Encrypted(data) => {
                write!(f, "Encrypted({} bytes)", data.len())
            }
            PrivateKeyData::AlwaysClear(data) => {
                write!(f, "AlwaysClear(fingerprint={})", fingerprint(data))
            }
            PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                wallet_seed_hash: wallet_seed,
                derivation_path,
            }) => {
                write!(
                    f,
                    "AtWalletDerivationPath({}/{})",
                    hex::encode(wallet_seed),
                    derivation_path
                )
            }
        }
    }
}

#[derive(Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct KeyStorage {
    pub private_keys:
        BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, PrivateKeyData)>,
}

impl From<BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, PrivateKeyData)>>
    for KeyStorage
{
    fn from(
        value: BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, PrivateKeyData)>,
    ) -> Self {
        Self {
            private_keys: value,
        }
    }
}

impl From<BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, [u8; 32])>>
    for KeyStorage
{
    fn from(
        value: BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, [u8; 32])>,
    ) -> Self {
        Self {
            private_keys: value
                .into_iter()
                .map(|(key, (qualified_identity_public_key, clear_key))| {
                    if qualified_identity_public_key
                        .identity_public_key
                        .security_level()
                        == SecurityLevel::MEDIUM
                    {
                        (
                            key,
                            (
                                qualified_identity_public_key,
                                PrivateKeyData::AlwaysClear(clear_key),
                            ),
                        )
                    } else {
                        (
                            key,
                            (
                                qualified_identity_public_key,
                                PrivateKeyData::Clear(clear_key),
                            ),
                        )
                    }
                })
                .collect(),
        }
    }
}

impl From<BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, WalletDerivationPath)>>
    for KeyStorage
{
    fn from(
        value: BTreeMap<
            (PrivateKeyTarget, KeyID),
            (QualifiedIdentityPublicKey, WalletDerivationPath),
        >,
    ) -> Self {
        Self {
            private_keys: value
                .into_iter()
                .map(
                    |(key, (qualified_identity_public_key, wallet_derivation_path))| {
                        (
                            key,
                            (
                                qualified_identity_public_key,
                                PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path),
                            ),
                        )
                    },
                )
                .collect(),
        }
    }
}

impl KeyStorage {
    // Allow dead_code: This method provides direct key access without password resolution,
    // useful for cases where keys are already decrypted or for debugging purposes
    #[allow(dead_code)]
    pub fn get(
        &self,
        key: &(PrivateKeyTarget, KeyID),
    ) -> Result<Option<(&QualifiedIdentityPublicKey, [u8; 32])>, String> {
        self.private_keys
            .get(key)
            .map(
                |(qualified_identity_public_key_data, private_key_data)| match private_key_data {
                    PrivateKeyData::AlwaysClear(clear) | PrivateKeyData::Clear(clear) => {
                        Ok((qualified_identity_public_key_data, *clear))
                    }
                    PrivateKeyData::Encrypted(_) => {
                        Err("Key is encrypted, please enter password".to_string())
                    }
                    PrivateKeyData::AtWalletDerivationPath(_) => {
                        Err("Key is not resolved, please enter password".to_string())
                    }
                },
            )
            .transpose()
    }

    /// Seed-free resolution for keys that carry their own plaintext
    /// ([`PrivateKeyData::Clear`] / [`PrivateKeyData::AlwaysClear`]).
    ///
    /// `Ok(None)` when the key is absent. Errors for keys that need a secret to
    /// resolve ([`PrivateKeyData::Encrypted`], wallet-derived
    /// [`PrivateKeyData::AtWalletDerivationPath`]); wallet-derived keys go
    /// through the JIT chokepoint via
    /// [`get_resolve_with_seed`](Self::get_resolve_with_seed), gated by
    /// [`wallet_seed_hash_for`](Self::wallet_seed_hash_for). Never reads a
    /// wallet's parked seed.
    pub fn get_resolve_local(
        &self,
        key: &(PrivateKeyTarget, KeyID),
    ) -> Result<Option<(QualifiedIdentityPublicKey, [u8; 32])>, String> {
        self.private_keys
            .get(key)
            .map(
                |(qualified_identity_public_key_data, private_key_data)| match private_key_data {
                    PrivateKeyData::AlwaysClear(clear) | PrivateKeyData::Clear(clear) => {
                        Ok((qualified_identity_public_key_data.clone(), *clear))
                    }
                    PrivateKeyData::Encrypted(_) => {
                        Err("Key is encrypted, please enter password".to_string())
                    }
                    PrivateKeyData::AtWalletDerivationPath(_) => {
                        Err("Key is not resolved, please unlock the wallet".to_string())
                    }
                },
            )
            .transpose()
    }

    /// The wallet seed hash a key would derive from, or `None` for keys that
    /// carry their own plaintext ([`PrivateKeyData::Clear`] /
    /// [`PrivateKeyData::AlwaysClear`]) or are still encrypted.
    ///
    /// Pure, secret-free probe: an async caller uses it to decide whether to
    /// open a [`with_secret`](crate::wallet_backend::SecretAccess::with_secret)
    /// scope at all, so [`get_resolve_with_seed`](Self::get_resolve_with_seed)
    /// only prompts for genuinely wallet-derived keys.
    pub fn wallet_seed_hash_for(&self, key: &(PrivateKeyTarget, KeyID)) -> Option<WalletSeedHash> {
        match self.private_keys.get(key) {
            Some((_, PrivateKeyData::AtWalletDerivationPath(wdp))) => Some(wdp.wallet_seed_hash),
            _ => None,
        }
    }

    /// Seed-as-parameter resolver, the JIT counterpart of
    /// [`get_resolve_local`](Self::get_resolve_local).
    ///
    /// For a [`PrivateKeyData::AtWalletDerivationPath`] key, derives from the
    /// `seed` borrowed by the caller (resolved once through the JIT chokepoint
    /// for the key's wallet seed hash — see
    /// [`wallet_seed_hash_for`](Self::wallet_seed_hash_for)) instead of reading
    /// the wallet's parked seed. Plaintext-carrying variants
    /// ([`PrivateKeyData::Clear`] / [`PrivateKeyData::AlwaysClear`]) defer to
    /// `get_resolve_local` and ignore the seed. The derivation path, network,
    /// and resulting key are unchanged — only the seed source differs.
    pub fn get_resolve_with_seed(
        &self,
        key: &(PrivateKeyTarget, KeyID),
        wallets: &[Arc<RwLock<Wallet>>],
        seed: &[u8; 64],
        network: Network,
    ) -> Result<Option<(QualifiedIdentityPublicKey, [u8; 32])>, String> {
        match self.private_keys.get(key) {
            None => Ok(None),
            Some((
                qualified_identity_public_key_data,
                PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                    wallet_seed_hash,
                    derivation_path,
                }),
            )) => {
                let derived_key = Wallet::derive_private_key_in_arc_rw_lock_slice_with_seed(
                    wallets,
                    *wallet_seed_hash,
                    seed,
                    derivation_path,
                    network,
                )?
                .ok_or(format!(
                    "Wallet for key at derivation path {} not present, we have {} wallets",
                    derivation_path,
                    wallets.len()
                ))?;
                Ok(Some((
                    qualified_identity_public_key_data.clone(),
                    derived_key,
                )))
            }
            // Plaintext-carrying / encrypted variants need no seed.
            Some(_) => self.get_resolve_local(key),
        }
    }

    // Allow dead_code: This method provides access to raw private key data,
    // useful for inspecting key states and encryption status
    #[allow(dead_code)]
    pub fn get_private_key_data(&self, key: &(PrivateKeyTarget, KeyID)) -> Option<&PrivateKeyData> {
        self.private_keys
            .get(key)
            .map(|(_, private_key_data)| private_key_data)
    }

    // Allow dead_code: This method provides combined access to private key data and wallet info,
    // useful for advanced key management and wallet integration scenarios
    #[allow(dead_code)]
    pub fn get_private_key_data_and_wallet_info(
        &self,
        key: &(PrivateKeyTarget, KeyID),
    ) -> Option<(&PrivateKeyData, &Option<WalletDerivationPath>)> {
        self.private_keys
            .get(key)
            .map(|(qualified_identity_public_key_data, private_key_data)| {
                (
                    private_key_data,
                    &qualified_identity_public_key_data.in_wallet_at_derivation_path,
                )
            })
    }

    pub fn get_cloned_private_key_data_and_wallet_info(
        &self,
        key: &(PrivateKeyTarget, KeyID),
    ) -> Option<(PrivateKeyData, Option<WalletDerivationPath>)> {
        self.private_keys
            .get(key)
            .map(|(qualified_identity_public_key_data, private_key_data)| {
                (
                    private_key_data.clone(),
                    qualified_identity_public_key_data
                        .in_wallet_at_derivation_path
                        .clone(),
                )
            })
    }

    pub fn find_master_key(&self) -> Option<&QualifiedIdentityPublicKey> {
        self.private_keys
            .values()
            .find(|(public_key, _)| {
                public_key.identity_public_key.purpose() == Purpose::AUTHENTICATION
                    && public_key.identity_public_key.security_level() == SecurityLevel::MASTER
            })
            .map(|(public_key, _)| public_key)
    }

    pub fn has(&self, key: &(PrivateKeyTarget, KeyID)) -> bool {
        self.private_keys.contains_key(key)
    }

    // Allow dead_code: This method returns all stored key identifiers,
    // useful for key enumeration and management operations
    #[allow(dead_code)]
    pub fn keys_set(&self) -> BTreeSet<(PrivateKeyTarget, KeyID)> {
        self.private_keys.keys().cloned().collect()
    }

    pub fn identity_public_keys(&self) -> Vec<(&PrivateKeyTarget, &QualifiedIdentityPublicKey)> {
        self.private_keys
            .iter()
            .map(|((target, _), (key, _))| (target, key))
            .collect()
    }

    /// Inserts an unencrypted key into `ClearKeyStorage`. Returns an error if the storage is closed.
    pub fn insert_non_encrypted(
        &mut self,
        key: (PrivateKeyTarget, KeyID),
        value: (QualifiedIdentityPublicKey, [u8; 32]),
    ) {
        match value.0.identity_public_key.security_level() {
            SecurityLevel::MEDIUM => {
                self.private_keys
                    .insert(key, (value.0, PrivateKeyData::AlwaysClear(value.1)));
            }
            _ => {
                self.private_keys
                    .insert(key, (value.0, PrivateKeyData::Clear(value.1)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, IdentityPublicKey};
    use std::collections::BTreeMap;

    /// A recognizable 32-byte secret. A full 32-byte collision with random
    /// public-key bytes is astronomically improbable, so finding it anywhere
    /// in a rendering means the raw key bytes leaked.
    fn distinctive_secret() -> [u8; 32] {
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = 0xA0 ^ (i as u8).wrapping_mul(7);
        }
        bytes
    }

    /// Assert `rendered` exposes the secret in none of the forms a sink could
    /// leak it: lowercase hex (a hex-printing sink) and the `[160, 167, …]`
    /// decimal-array form a `#[derive(Debug)]` on `[u8; 32]` would emit. The
    /// decimal form is the shape the pre-fix derived `Debug` actually leaked,
    /// so checking only hex would falsely pass against the original bug.
    fn assert_no_leak(rendered: &str, secret: &[u8; 32], context: &str) {
        let hex = hex::encode(secret);
        let decimal_array = format!(
            "[{}]",
            secret
                .iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(
            !rendered.contains(&hex),
            "{context} leaked the raw private key (hex): {rendered}"
        );
        assert!(
            !rendered.contains(&decimal_array),
            "{context} leaked the raw private key (byte array): {rendered}"
        );
    }

    /// QA-001 — the redacting `Debug` (and `Display`) on `PrivateKeyData` must
    /// never emit raw plaintext private-key bytes, and that guarantee must hold
    /// transitively through the derived-`Debug` chain
    /// `QualifiedIdentity -> KeyStorage -> PrivateKeyData`.
    #[test]
    fn debug_output_never_leaks_plaintext_private_key() {
        let secret = distinctive_secret();

        // 1. The two raw-byte variants directly.
        for variant in [
            PrivateKeyData::Clear(secret),
            PrivateKeyData::AlwaysClear(secret),
        ] {
            assert_no_leak(&format!("{variant:?}"), &secret, "PrivateKeyData Debug");
            assert_no_leak(&format!("{variant}"), &secret, "PrivateKeyData Display");
        }

        // 2. Through KeyStorage, which derives Debug and holds the variant.
        let platform_version = PlatformVersion::latest();
        let public_key = IdentityPublicKey::random_key(0, Some(42), platform_version);
        let mut key_storage = KeyStorage::default();
        key_storage.private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, public_key.id()),
            (
                QualifiedIdentityPublicKey::from(public_key),
                PrivateKeyData::Clear(secret),
            ),
        );
        assert_no_leak(&format!("{key_storage:?}"), &secret, "KeyStorage Debug");

        // 3. Through QualifiedIdentity, which derives Debug and holds KeyStorage.
        let identity = Identity::create_basic_identity(Identifier::default(), platform_version)
            .expect("basic identity");
        let qualified = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: key_storage,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::PendingCreation,
            network: Network::Testnet,
        };
        assert_no_leak(
            &format!("{qualified:?}"),
            &secret,
            "QualifiedIdentity Debug",
        );
    }
}
