pub mod encrypted_key_storage;
pub mod identity_meta;
pub mod qualified_identity_public_key;

// TODO(det): this upward edge is fixed by the `SecretAccess::with_secret`
// contract, whose closures must return `Result<_, TaskError>`. Removing it
// requires making that secret-seam chokepoint generic over the closure error
// type — a wallet_backend change out of scope here.
use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, ResolvedPrivateKey};
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::wallet::{Wallet, WalletSeedHash};
use bincode::{Decode, Encode};
use dash_sdk::dashcore_rpc::dashcore::{PubkeyHash, signer};
use dash_sdk::dpp::async_trait::async_trait;
use dash_sdk::dpp::bls_signatures::{Bls12381G2Impl, SignatureSchemes};
use dash_sdk::dpp::dashcore::address::Payload;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Address, Network, ScriptHash};
use dash_sdk::dpp::data_contract::document_type::DocumentTypeRef;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::ed25519_dalek::Signer as EDDSASigner;
use dash_sdk::dpp::identity::KeyType::{BIP13_SCRIPT_HASH, ECDSA_HASH160};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::dpp::identity::{Identity, KeyID, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::key_wallet::bip32::ChildNumber;
use dash_sdk::dpp::platform_value::BinaryData;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::state_transition::errors::InvalidIdentityPublicKeyTypeError;
use dash_sdk::dpp::{ProtocolError, bls_signatures, ed25519_dalek};
use dash_sdk::platform::IdentityPublicKey;
use std::collections::{BTreeMap, HashSet};
use std::fmt::{Display, Formatter};
use std::sync::{Arc, RwLock};
use zeroize::Zeroizing;

#[derive(Debug, Encode, Decode, PartialEq, Clone, Copy)]
pub enum IdentityType {
    User,
    Masternode,
    Evonode,
}

impl IdentityType {
    pub fn default_encoding(&self) -> Encoding {
        match self {
            IdentityType::User => Encoding::Base58,
            IdentityType::Masternode => Encoding::Hex,
            IdentityType::Evonode => Encoding::Hex,
        }
    }

    /// Stable persistence tag, decoupled from the derived `Debug`
    /// representation. Stored blobs and their filters share this mapping, so a
    /// variant rename can never silently change a discriminator on disk.
    pub const fn as_tag(&self) -> &'static str {
        match self {
            IdentityType::User => "User",
            IdentityType::Masternode => "Masternode",
            IdentityType::Evonode => "Evonode",
        }
    }

    /// Inverse of [`IdentityType::as_tag`]. Returns `None` for an unknown tag.
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "User" => Some(IdentityType::User),
            "Masternode" => Some(IdentityType::Masternode),
            "Evonode" => Some(IdentityType::Evonode),
            _ => None,
        }
    }
}

impl Display for IdentityType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityType::User => write!(f, "User"),
            IdentityType::Masternode => write!(f, "Masternode"),
            IdentityType::Evonode => write!(f, "Evonode"),
        }
    }
}

#[derive(Debug, Encode, Decode, Clone, Hash, Ord, PartialOrd, Eq, PartialEq)]
#[allow(clippy::enum_variant_names)]
pub enum PrivateKeyTarget {
    PrivateKeyOnMainIdentity,
    PrivateKeyOnVoterIdentity,
    PrivateKeyOnOperatorIdentity,
}

impl From<Purpose> for PrivateKeyTarget {
    fn from(value: Purpose) -> Self {
        match value {
            Purpose::VOTING => PrivateKeyTarget::PrivateKeyOnVoterIdentity,
            _ => PrivateKeyTarget::PrivateKeyOnMainIdentity,
        }
    }
}

#[derive(Debug, Encode, Decode, Clone, PartialEq)]
pub struct DPNSNameInfo {
    pub name: String,
    pub acquired_at: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum IdentityStatus {
    /// Identity status is unknown, refresh is required.
    #[default]
    Unknown = 0,
    /// Identity creation is in progress, but not yet completed. It can be also an error condition.
    PendingCreation = 1,
    /// Identity is in a normal state, fully functional.
    Active = 2,
    /// Identity not found on the platform, either failed creation or invalid.
    NotFound = 3,
    /// Identity creation failed, it can be due to various reasons.
    FailedCreation = 4,
}
impl From<u8> for IdentityStatus {
    fn from(value: u8) -> Self {
        match value {
            0 => IdentityStatus::Unknown,
            1 => IdentityStatus::PendingCreation,
            2 => IdentityStatus::Active,
            3 => IdentityStatus::NotFound,
            4 => IdentityStatus::FailedCreation,
            _ => IdentityStatus::Unknown, // Default to Unknown for any other value
        }
    }
}

impl From<IdentityStatus> for u8 {
    fn from(status: IdentityStatus) -> Self {
        match status {
            IdentityStatus::Unknown => 0,
            IdentityStatus::PendingCreation => 1,
            IdentityStatus::Active => 2,
            IdentityStatus::NotFound => 3,
            IdentityStatus::FailedCreation => 4,
        }
    }
}

impl Display for IdentityStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityStatus::Unknown => write!(f, "Unknown"),
            IdentityStatus::PendingCreation => write!(f, "Pending Creation"),
            IdentityStatus::Active => write!(f, "Active"),
            IdentityStatus::NotFound => write!(f, "Not Found"),
            IdentityStatus::FailedCreation => write!(f, "Creation Failed"),
        }
    }
}

impl IdentityStatus {
    /// Returns identity status as a u8 value, for serialization
    pub fn as_u8(&self) -> u8 {
        (*self).into()
    }
    /// Constructs identity status from an u8 value, for deserialization
    pub fn from_u8(x: u8) -> Self {
        Self::from(x)
    }

    /// Returns true if the identity status can be updated to the `to` status.
    pub fn can_update(&self, to: &Self) -> bool {
        use IdentityStatus::*;
        let from = self;
        if from == to {
            return true; // No change needed
        }

        match (from, to) {
            // PendingCreation can be updated to FailedCreation or Active
            (PendingCreation, FailedCreation) => true,
            (PendingCreation, Active) => true,

            // FailedCreation can be updated to Active (but it's unlikely)
            (FailedCreation, Active) => true,

            // Active might disappear - update to NotFound
            (Active, NotFound) => true,

            // Unknown can be updated to Active or NotFound
            (Unknown, Active) => true,
            (Unknown, NotFound) => true,

            // NotFound can be updated to Active or Unknown
            (NotFound, Active) => true,

            _ => false,
        }
    }

    /// Update identity status to the `to` status if the update is allowed.
    ///
    /// See [`IdentityStatus::can_update`] for the rules of updating.
    pub fn update(&mut self, to: Self) {
        if self.can_update(&to) {
            *self = to;
        } else {
            tracing::trace!(
                "Invalid attempt to  update identity status from {:?} to {:?}",
                self,
                to
            );
        }
    }
}

#[derive(Debug, Clone)]
pub struct QualifiedIdentity {
    pub identity: Identity,
    pub associated_voter_identity: Option<(Identity, IdentityPublicKey)>,
    pub associated_operator_identity: Option<(Identity, IdentityPublicKey)>,
    pub associated_owner_key_id: Option<KeyID>,
    pub identity_type: IdentityType,
    pub alias: Option<String>,
    pub private_keys: KeyStorage,
    pub dpns_names: Vec<DPNSNameInfo>,
    pub associated_wallets: BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>,
    /// The JIT secret chokepoint, attached alongside `associated_wallets` when
    /// the identity is hydrated. Lets the async `sign` path fetch the HD seed
    /// just-in-time (no parked seed read) for the ECDSA_HASH160 recovery scan.
    /// Skipped by Encode/Decode and excluded from `PartialEq`, exactly like
    /// `associated_wallets` — it is a runtime wiring handle, not identity data.
    pub secret_access: Option<crate::wallet_backend::SecretAccess>,
    /// The index used to register the identity
    pub wallet_index: Option<u32>,
    pub top_ups: BTreeMap<u32, u64>,
    pub status: IdentityStatus,
    pub network: Network,
}

impl AsRef<QualifiedIdentity> for QualifiedIdentity {
    fn as_ref(&self) -> &QualifiedIdentity {
        self
    }
}

impl PartialEq for QualifiedIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.associated_voter_identity == other.associated_voter_identity
            && self.associated_operator_identity == other.associated_operator_identity
            && self.associated_owner_key_id == other.associated_owner_key_id
            && self.identity_type == other.identity_type
            && self.wallet_index == other.wallet_index
            && self.alias == other.alias
            && self.private_keys == other.private_keys
            && self.dpns_names == other.dpns_names
        // `associated_wallets` is ignored in this comparison
    }
}

// Implement Encode manually for QualifiedIdentity, excluding decrypted_wallets
impl Encode for QualifiedIdentity {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        self.identity.encode(encoder)?;
        self.associated_voter_identity.encode(encoder)?;
        self.associated_operator_identity.encode(encoder)?;
        self.associated_owner_key_id.encode(encoder)?;
        self.identity_type.encode(encoder)?;
        self.alias.encode(encoder)?;
        self.private_keys.encode(encoder)?;
        self.dpns_names.encode(encoder)?;
        // `decrypted_wallets` is skipped

        // we don't encode/decode status - it's stored in the database
        // self.status.encode(encoder)?;
        Ok(())
    }
}

// Implement Decode manually for QualifiedIdentity, excluding decrypted_wallets
impl<C> Decode<C> for QualifiedIdentity {
    fn decode<D: bincode::de::Decoder<Context = C>>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        Ok(Self {
            identity: Identity::decode(decoder)?,
            associated_voter_identity: Option::<(Identity, IdentityPublicKey)>::decode(decoder)?,
            associated_operator_identity: Option::<(Identity, IdentityPublicKey)>::decode(decoder)?,
            associated_owner_key_id: Option::<KeyID>::decode(decoder)?,
            identity_type: IdentityType::decode(decoder)?,
            alias: Option::<String>::decode(decoder)?,
            private_keys: KeyStorage::decode(decoder)?,
            dpns_names: Vec::<DPNSNameInfo>::decode(decoder)?,
            associated_wallets: BTreeMap::new(), // Initialize with an empty vector
            secret_access: None,                 // Runtime wiring, attached at hydration
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Unknown, // Loaded from the database, not encoded
            network: Network::Mainnet,       // Loaded from the database, not encoded
        })
    }
}

impl Display for QualifiedIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some(alias) = &self.alias {
            write!(f, "{}", alias)
        } else if !self.dpns_names.is_empty() {
            write!(f, "{}", self.dpns_names[0].name)
        } else {
            write!(f, "{}", self.identity.id())
        }
    }
}

#[async_trait]
impl Signer<IdentityPublicKey> for QualifiedIdentity {
    async fn sign(
        &self,
        identity_public_key: &IdentityPublicKey,
        data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        let target: PrivateKeyTarget = identity_public_key.purpose().into();
        let key_id = identity_public_key.id();

        tracing::debug!(
            identity_id = %self.identity.id().to_string(Encoding::Base58),
            key_id = key_id,
            key_purpose = ?identity_public_key.purpose(),
            key_type = ?identity_public_key.key_type(),
            target = ?target,
            "Attempting to sign with key"
        );

        // Resolve the signing key without ever reading a wallet's parked seed
        // (see [`Self::resolve_private_key_bytes`]).
        let resolved = self
            .resolve_private_key_bytes(target.clone(), key_id)
            .await
            .map_err(|e| ProtocolError::Generic(e.to_string()))?;

        let (_, private_key) = resolved.ok_or_else(|| {
            tracing::error!(
                key_id = key_id,
                purpose = ?identity_public_key.purpose(),
                target = ?target,
                "Key not found in identity"
            );
            // Only dump the identity's available keys when resolution failed —
            // this is the diagnostic that actually matters, off the hot path.
            for ((t, id), (pub_key, _)) in self.private_keys.private_keys.iter() {
                tracing::debug!(
                    target = ?t,
                    key_id = id,
                    purpose = ?pub_key.identity_public_key.purpose(),
                    key_type = ?pub_key.identity_public_key.key_type(),
                    "Available key in identity"
                );
            }
            ProtocolError::Generic(format!(
                "Key {} ({}) not found in identity {:?}",
                identity_public_key.id(),
                identity_public_key.purpose(),
                self.identity.id().to_string(Encoding::Base58)
            ))
        })?;

        tracing::debug!("Successfully resolved private key, proceeding to sign");
        match identity_public_key.key_type() {
            KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160 => {
                // For ECDSA_HASH160, verify that the private key matches the public key hash on Platform
                // If there's a mismatch (due to incorrect stored derivation path), regenerate the correct path
                if identity_public_key.key_type() == KeyType::ECDSA_HASH160 {
                    use dash_sdk::dpp::dashcore::PublicKey;
                    use dash_sdk::dpp::dashcore::hashes::{Hash, ripemd160, sha256};
                    use dash_sdk::dpp::dashcore::secp256k1::{Secp256k1, SecretKey};

                    let platform_key_data = identity_public_key.data().as_slice();

                    if let Ok(secret_key) = SecretKey::from_slice(&private_key[..]) {
                        let secp = Secp256k1::new();
                        let derived_pubkey = PublicKey::new(secret_key.public_key(&secp));
                        let pubkey_bytes = derived_pubkey.to_bytes();
                        let sha256_hash = sha256::Hash::hash(&pubkey_bytes);
                        let hash160 = ripemd160::Hash::hash(sha256_hash.as_byte_array());

                        if hash160.as_byte_array() != platform_key_data {
                            // Mismatch detected — scan identity indices to find
                            // the correct derivation path. The HD seed is
                            // fetched just-in-time through the JIT chokepoint
                            // (no parked-seed read) and the scan runs inside the
                            // closure, so the seed never enters this layer.
                            if let Some(found) = self
                                .sign_via_hash160_path_scan(data, key_id, platform_key_data)
                                .await?
                            {
                                return Ok(found);
                            }

                            tracing::error!(
                                derived = %hex::encode(hash160.as_byte_array()),
                                platform = %hex::encode(platform_key_data),
                                "Key mismatch and could not find correct derivation path after scanning"
                            );
                        }
                    }
                }

                let signature = signer::sign(data, &private_key[..])?;
                Ok(signature.to_vec().into())
            }
            KeyType::BLS12_381 => {
                let pk = bls_signatures::SecretKey::<Bls12381G2Impl>::from_be_bytes(&private_key)
                    .into_option()
                    .ok_or(ProtocolError::Generic(
                        "bls private key from bytes isn't correct".to_string(),
                    ))?;
                Ok(pk
                    .sign(SignatureSchemes::Basic, data)?
                    .as_raw_value()
                    .to_compressed()
                    .to_vec()
                    .into())
            }
            KeyType::EDDSA_25519_HASH160 => {
                let pk = ed25519_dalek::SigningKey::from(&*private_key);
                Ok(pk.sign(data).to_vec().into())
            }
            // the default behavior from
            // https://github.com/dashevo/platform/blob/6b02b26e5cd3a7c877c5fdfe40c4a4385a8dda15/packages/js-dpp/lib/stateTransition/AbstractStateTransition.js#L187
            // is to return the error for the BIP13_SCRIPT_HASH
            KeyType::BIP13_SCRIPT_HASH => Err(ProtocolError::InvalidIdentityPublicKeyTypeError(
                InvalidIdentityPublicKeyTypeError::new(identity_public_key.key_type()),
            )),
        }
    }

    fn can_sign_with(&self, identity_public_key: &IdentityPublicKey) -> bool {
        self.private_keys.has(&(
            identity_public_key.purpose().into(),
            identity_public_key.id(),
        ))
    }

    async fn sign_create_witness(
        &self,
        identity_public_key: &IdentityPublicKey,
        data: &[u8],
    ) -> Result<dash_sdk::dpp::address_funds::AddressWitness, ProtocolError> {
        use dash_sdk::dpp::address_funds::AddressWitness;

        // First, sign the data to get the signature (compact recoverable signature)
        // The public key will be recovered from the signature during verification
        let signature = self.sign(identity_public_key, data).await?;

        // Create the appropriate AddressWitness based on the key type
        match identity_public_key.key_type() {
            KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160 => {
                // P2PKH witness only needs the recoverable signature
                Ok(AddressWitness::P2pkh { signature })
            }
            KeyType::EDDSA_25519_HASH160 => {
                // Ed25519 keys are not supported for address witnesses (P2PKH requires ECDSA)
                Err(ProtocolError::InvalidIdentityPublicKeyTypeError(
                    InvalidIdentityPublicKeyTypeError::new(identity_public_key.key_type()),
                ))
            }
            KeyType::BIP13_SCRIPT_HASH => {
                // For script hash, we would need the redeem script which isn't available from just the key
                Err(ProtocolError::InvalidIdentityPublicKeyTypeError(
                    InvalidIdentityPublicKeyTypeError::new(identity_public_key.key_type()),
                ))
            }
            KeyType::BLS12_381 => {
                // BLS keys are not supported for address witnesses
                Err(ProtocolError::InvalidIdentityPublicKeyTypeError(
                    InvalidIdentityPublicKeyTypeError::new(identity_public_key.key_type()),
                ))
            }
        }
    }
}

impl QualifiedIdentity {
    /// Serializes the QualifiedIdentity to a vector of bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::encode_to_vec(self, bincode::config::standard())
            .expect("Failed to encode QualifiedIdentity")
    }

    /// Deserializes a `QualifiedIdentity` from a vector of bytes.
    ///
    /// Returns an error if the blob is corrupted or cannot be decoded.
    /// Callers must stop processing on the first deserialization error rather
    /// than skipping corrupted entries, because identities hold private keys
    /// and balance information — silently ignoring a corrupted identity could
    /// lead to loss of funds.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        bincode::decode_from_slice(bytes, bincode::config::standard())
            .map(|(identity, _)| identity)
            .map_err(|e| format!("Failed to decode QualifiedIdentity: {}", e))
    }

    /// Resolve the 32-byte private key for `(target, key_id)` without ever
    /// reading a wallet's parked seed.
    ///
    /// A wallet-derived key ([`PrivateKeyData::AtWalletDerivationPath`]) pulls
    /// its HD seed just-in-time through the [`SecretAccess`] chokepoint and
    /// derives inside the scope; a key that carries its own plaintext
    /// (`Clear`/`AlwaysClear`) resolves with no seed access and no prompt. The
    /// pure [`wallet_seed_hash_for`](KeyStorage::wallet_seed_hash_for) probe
    /// decides which path applies, so the prompt fires only for genuinely
    /// wallet-derived keys.
    ///
    /// Returns `Ok(None)` when the key is absent.
    ///
    /// [`PrivateKeyData::AtWalletDerivationPath`]: encrypted_key_storage::PrivateKeyData::AtWalletDerivationPath
    /// [`SecretAccess`]: crate::wallet_backend::SecretAccess
    pub async fn resolve_private_key_bytes(
        &self,
        target: PrivateKeyTarget,
        key_id: KeyID,
    ) -> Result<Option<ResolvedPrivateKey>, TaskError> {
        let resolve_key = (target.clone(), key_id);

        // Vault-backed identity key: fetch the raw bytes per-use through the
        // chokepoint (unprotected fast-path, no prompt). Requires the
        // chokepoint to be wired; without it the key cannot be resolved (the
        // bytes are not resident), so fail closed.
        if self.private_keys.is_in_vault(&resolve_key) {
            let Some(secret_access) = self.secret_access.as_ref() else {
                return Err(TaskError::WalletLocked);
            };
            let Some(public_key) = self.private_keys.public_key_for(&resolve_key).cloned() else {
                return Ok(None);
            };
            let scope = crate::wallet_backend::SecretScope::IdentityKey {
                identity_id: self.identity.id().to_buffer(),
                target,
                key_id,
            };
            return secret_access
                .with_secret(&scope, move |plaintext| {
                    let key = plaintext
                        .expose_identity_key()
                        .ok_or(TaskError::IdentityKeyMissing)?;
                    Ok(Some((public_key, Zeroizing::new(*key))))
                })
                .await;
        }

        match (
            self.secret_access.as_ref(),
            self.private_keys.wallet_seed_hash_for(&resolve_key),
        ) {
            (Some(secret_access), Some(seed_hash)) => {
                let network = self.network;
                let wallets = self
                    .associated_wallets
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                secret_access
                    .with_secret(
                        &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                        |plaintext| {
                            let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                            self.private_keys
                                .get_resolve_with_seed(&resolve_key, &wallets, seed, network)
                                .map_err(|detail| {
                                    tracing::warn!(error = %detail, "Wallet key lookup failed");
                                    TaskError::WalletKeyLookupFailed
                                })
                        },
                    )
                    .await
            }
            // No chokepoint, or a key that carries its own plaintext: resolve
            // seed-free. A wallet-derived key with no chokepoint fails closed
            // inside `get_resolve_local`.
            _ => self
                .private_keys
                .get_resolve_local(&resolve_key)
                .map_err(|detail| {
                    tracing::warn!(error = %detail, "Local key resolution failed");
                    TaskError::WalletKeyLookupFailed
                }),
        }
    }

    /// The seed hash of the wallet DashPay derives contact keys against.
    ///
    /// When an identity has more than one associated wallet, both the
    /// send-side (contact-request xpub) and the receive-side (incoming
    /// address scan) MUST select the *same* wallet, or a contact would pay
    /// into addresses the recipient never scans. `associated_wallets` is a
    /// `BTreeMap<WalletSeedHash, _>`, so the first key is the lowest seed
    /// hash — a stable, content-derived choice that does not depend on
    /// insertion order. Both sides call this one helper so the rule lives in
    /// exactly one place (SEC-W-001).
    pub fn dashpay_wallet_seed_hash(&self) -> Option<WalletSeedHash> {
        self.associated_wallets.keys().next().copied()
    }

    /// The wallet DashPay derives contact keys against (see
    /// [`Self::dashpay_wallet_seed_hash`] for the selection rule). The
    /// receive side needs the wallet handle to register scanned addresses;
    /// the send side needs only the seed hash. Both resolve to the same
    /// wallet by construction.
    pub fn dashpay_wallet(&self) -> Option<(WalletSeedHash, &Arc<RwLock<Wallet>>)> {
        self.associated_wallets
            .iter()
            .next()
            .map(|(hash, wallet)| (*hash, wallet))
    }

    /// ECDSA_HASH160 recovery scan: when the stored derivation path produces a
    /// public-key hash that disagrees with Platform's, scan identity indices
    /// 0..10 for the path whose derived key matches `platform_key_data`, and
    /// sign `data` with it.
    ///
    /// The HD seed is fetched just-in-time through the [`SecretAccess`]
    /// chokepoint (keyed by the identity's first associated wallet seed hash)
    /// and the whole scan runs inside the closure — the seed is borrowed for
    /// this one operation and zeroizes when the closure returns; it never
    /// enters the model layer by value.
    ///
    /// Returns `Ok(None)` when the chokepoint is not wired or no associated
    /// wallet exists (best-effort recovery — the caller falls back to the
    /// originally resolved key). Chokepoint failures (e.g. a cancelled
    /// passphrase prompt) surface as a [`ProtocolError`].
    async fn sign_via_hash160_path_scan(
        &self,
        data: &[u8],
        key_id: KeyID,
        platform_key_data: &[u8],
    ) -> Result<Option<BinaryData>, ProtocolError> {
        use dash_sdk::dpp::dashcore::PublicKey;
        use dash_sdk::dpp::dashcore::hashes::{Hash, ripemd160, sha256};
        use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
        use dash_sdk::dpp::key_wallet::bip32::{DerivationPath as DP, KeyDerivationType};

        let (Some(secret_access), Some(seed_hash)) =
            (self.secret_access.as_ref(), self.dashpay_wallet_seed_hash())
        else {
            return Ok(None);
        };

        let network = self.network;
        // Owned so the closure (`'static`-friendly capture) needs no borrow of
        // `data`/`platform_key_data` across the await.
        let data = data.to_vec();
        let platform_key_data = platform_key_data.to_vec();

        secret_access
            .with_secret(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                |plaintext| {
                    let Some(seed) = plaintext.expose_hd_seed() else {
                        return Ok(None);
                    };
                    let secp = Secp256k1::new();
                    for identity_index in 0..10u32 {
                        let correct_path = DP::identity_authentication_path(
                            network,
                            KeyDerivationType::ECDSA,
                            identity_index,
                            key_id,
                        );
                        let Ok(extended_key) =
                            correct_path.derive_priv_ecdsa_for_master_seed(seed, network)
                        else {
                            continue;
                        };
                        let correct_pubkey =
                            PublicKey::new(extended_key.private_key.public_key(&secp));
                        let correct_hash = ripemd160::Hash::hash(
                            sha256::Hash::hash(&correct_pubkey.to_bytes()).as_byte_array(),
                        );
                        if correct_hash.as_byte_array() == platform_key_data.as_slice() {
                            tracing::info!(
                                identity_index = identity_index,
                                key_id = key_id,
                                path = %correct_path,
                                "Using corrected derivation path for signing (found via scan)"
                            );
                            let signature =
                                signer::sign(&data, &extended_key.private_key.secret_bytes())
                                    .map_err(|_| TaskError::EncryptionError {
                                        detail:
                                            "Failed to sign with the recovered derivation path."
                                                .to_string(),
                                    })?;
                            return Ok(Some(BinaryData::from(signature.to_vec())));
                        }
                    }
                    Ok(None)
                },
            )
            .await
            .map_err(|e| ProtocolError::Generic(format!("HASH160 recovery scan failed: {e}")))
    }

    pub fn display_string(&self) -> String {
        self.alias
            .clone()
            .unwrap_or(self.identity.id().to_string(Encoding::Base58))
    }

    pub fn masternode_payout_address(&self, network: Network) -> Option<Address> {
        self.identity
            .get_first_public_key_matching(
                Purpose::TRANSFER,
                [SecurityLevel::CRITICAL].into(),
                [ECDSA_HASH160, BIP13_SCRIPT_HASH].into(),
                false,
            )
            .and_then(|identity_public_key| {
                let key = identity_public_key.public_key_hash().ok()?;
                if identity_public_key.key_type() == BIP13_SCRIPT_HASH {
                    Some(Address::new(
                        network,
                        Payload::ScriptHash(ScriptHash::from_byte_array(key)),
                    ))
                } else {
                    Some(Address::new(
                        network,
                        Payload::PubkeyHash(PubkeyHash::from_byte_array(key)),
                    ))
                }
            })
    }

    pub fn can_sign_with_master_key(&self) -> Option<&QualifiedIdentityPublicKey> {
        if self.identity_type != IdentityType::User {
            return None;
        }

        // Iterate through the encrypted private keys to check for a valid master key
        for (target, public_key) in self.private_keys.identity_public_keys() {
            if *target == PrivateKeyTarget::PrivateKeyOnMainIdentity
                && public_key.identity_public_key.purpose() == Purpose::AUTHENTICATION
                && public_key.identity_public_key.security_level() == SecurityLevel::MASTER
            {
                return Some(public_key);
            }
        }

        None
    }

    pub fn document_signing_key(
        &self,
        document_type: &DocumentTypeRef,
    ) -> Option<&IdentityPublicKey> {
        self.identity.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([document_type.security_level_requirement()]),
            HashSet::from(KeyType::all_key_types()),
            false,
        )
    }

    pub fn available_withdrawal_keys(&self) -> Vec<&QualifiedIdentityPublicKey> {
        let mut keys = vec![];

        // Check the main identity's public keys
        for (target, public_key) in self.private_keys.identity_public_keys() {
            match (self.identity_type, target) {
                (IdentityType::User, PrivateKeyTarget::PrivateKeyOnMainIdentity) => {
                    if public_key.identity_public_key.purpose() == Purpose::TRANSFER {
                        keys.push(public_key);
                    }
                }
                (IdentityType::Masternode | IdentityType::Evonode, target_type) => {
                    if target_type == &PrivateKeyTarget::PrivateKeyOnMainIdentity {
                        if public_key.identity_public_key.purpose() == Purpose::OWNER {
                            keys.push(public_key);
                        }
                        if public_key.identity_public_key.purpose() == Purpose::TRANSFER {
                            keys.push(public_key);
                        }
                    }
                }
                _ => {}
            }
        }

        keys
    }

    /// Returns the key to pre-select for signing a withdrawal.
    ///
    /// Only keys whose private material is held locally are considered (via
    /// [`available_withdrawal_keys`](Self::available_withdrawal_keys)). A
    /// `TRANSFER` key is preferred, falling back to an `OWNER` key — mirroring
    /// Platform's `TransferPreferred` signing-key selection. Returns `None` when
    /// no locally-signable withdrawal key exists, so callers never pre-select an
    /// on-chain key the signer cannot actually use.
    pub fn default_withdrawal_key(&self) -> Option<&QualifiedIdentityPublicKey> {
        let keys = self.available_withdrawal_keys();
        keys.iter()
            .find(|qk| qk.identity_public_key.purpose() == Purpose::TRANSFER)
            .or_else(|| {
                keys.iter()
                    .find(|qk| qk.identity_public_key.purpose() == Purpose::OWNER)
            })
            .copied()
    }

    pub fn available_transfer_keys(&self) -> Vec<&QualifiedIdentityPublicKey> {
        let mut keys = vec![];

        // Check the main identity's public keys
        for (_, public_key) in self.private_keys.identity_public_keys() {
            if public_key.identity_public_key.purpose() == Purpose::TRANSFER {
                keys.push(public_key);
            }
        }

        keys
    }

    /// Authentication-purpose keys whose security level satisfies `predicate`.
    fn authentication_keys_matching(
        &self,
        predicate: impl Fn(SecurityLevel) -> bool,
    ) -> Vec<&QualifiedIdentityPublicKey> {
        self.private_keys
            .identity_public_keys()
            .into_iter()
            .map(|(_, public_key)| public_key)
            .filter(|public_key| {
                public_key.identity_public_key.purpose() == Purpose::AUTHENTICATION
                    && predicate(public_key.identity_public_key.security_level())
            })
            .collect()
    }

    pub fn available_authentication_keys_non_master(&self) -> Vec<&QualifiedIdentityPublicKey> {
        self.authentication_keys_matching(|level| level != SecurityLevel::MASTER)
    }

    pub fn available_authentication_keys_with_critical_security_level(
        &self,
    ) -> Vec<&QualifiedIdentityPublicKey> {
        self.authentication_keys_matching(|level| level == SecurityLevel::CRITICAL)
    }

    pub fn available_authentication_keys(&self) -> Vec<&QualifiedIdentityPublicKey> {
        self.authentication_keys_matching(|_| true)
    }

    /// Returns the wallet info for the first public key that is in a wallet.
    ///
    /// If more than one public key is in a wallet, it returns the first one found.
    ///
    /// ## Returns
    /// A tuple containing the wallet seed hash and the index of the identity in the wallet.
    pub fn determine_wallet_info(&self) -> Result<Option<(WalletSeedHash, u32)>, String> {
        let wallet_info = self
            .private_keys
            .identity_public_keys()
            .into_iter()
            .filter_map(|(_, public_key)| {
                if let Some(wallet_derivation_path) = &public_key.in_wallet_at_derivation_path {
                    let seed_hash = wallet_derivation_path.wallet_seed_hash;
                    let derivation_path = &wallet_derivation_path.derivation_path;
                    // second to last element is the wallet index
                    if derivation_path.len() < 2 {
                        return None; // Not enough elements to get wallet index
                    }
                    // Get the wallet index from the second to last element
                    let wallet_index = match derivation_path[derivation_path.len() - 2] {
                        ChildNumber::Hardened { index } => Some(index),
                        ChildNumber::Normal { index } => Some(index),
                        _ => {
                            tracing::debug!(
                                ?derivation_path,
                                "determine wallet: unexpected derivation path format, skipping key"
                            );
                            None
                        }
                    }?;
                    // consistency check; if we get a different index here, this is non-recoverable error and we should panic
                    // to avoid unexpected behavior and loss of access to private keys
                    if self.wallet_index.is_some_and(|v| v != wallet_index) {
                        panic!(
                            "Inconsistent wallet index found: {:?} vs {:?}",
                            self.wallet_index, wallet_index
                        );
                    };
                    Some((seed_hash, wallet_index))
                } else {
                    None
                }
            })
            .next();

        Ok(wallet_info)
    }
}

#[cfg(test)]
mod withdrawal_key_tests {
    use super::*;
    use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;

    fn key(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
        let mut k = IdentityPublicKey::random_key(id, Some(id as u64), PlatformVersion::latest());
        k.set_id(id);
        k.set_purpose(purpose);
        k.set_security_level(SecurityLevel::CRITICAL);
        k
    }

    fn build_identity(
        identity_type: IdentityType,
        on_chain: Vec<IdentityPublicKey>,
        with_private: Vec<IdentityPublicKey>,
    ) -> QualifiedIdentity {
        let public_keys: BTreeMap<KeyID, IdentityPublicKey> =
            on_chain.into_iter().map(|k| (k.id(), k)).collect();
        let identity = Identity::new_with_id_and_keys(
            Identifier::random(),
            public_keys,
            PlatformVersion::latest(),
        )
        .expect("identity");

        let mut private_keys = BTreeMap::new();
        for k in with_private {
            private_keys.insert(
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, k.id()),
                (
                    QualifiedIdentityPublicKey::from(k),
                    PrivateKeyData::Clear([0u8; 32]),
                ),
            );
        }

        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type,
            alias: None,
            private_keys: KeyStorage { private_keys },
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    /// Repro for the withdraw key-selection bug: a TRANSFER key that exists
    /// on-chain but whose private material is not held locally must never be
    /// pre-selected — the signer cannot use it.
    #[test]
    fn ghost_transfer_key_is_not_selected() {
        let transfer = key(1, Purpose::TRANSFER);
        let qi = build_identity(IdentityType::User, vec![transfer], vec![]);
        assert!(qi.default_withdrawal_key().is_none());
    }

    #[test]
    fn private_backed_transfer_key_is_selected() {
        let transfer = key(1, Purpose::TRANSFER);
        let qi = build_identity(IdentityType::User, vec![transfer.clone()], vec![transfer]);
        let selected = qi.default_withdrawal_key().expect("a key");
        assert_eq!(selected.identity_public_key.id(), 1);
        assert_eq!(selected.identity_public_key.purpose(), Purpose::TRANSFER);
    }

    #[test]
    fn owner_key_is_used_as_fallback_when_no_transfer() {
        let owner = key(2, Purpose::OWNER);
        let qi = build_identity(IdentityType::Masternode, vec![owner.clone()], vec![owner]);
        let selected = qi.default_withdrawal_key().expect("a key");
        assert_eq!(selected.identity_public_key.id(), 2);
        assert_eq!(selected.identity_public_key.purpose(), Purpose::OWNER);
    }

    #[test]
    fn transfer_key_is_preferred_over_owner() {
        let owner = key(2, Purpose::OWNER);
        let transfer = key(1, Purpose::TRANSFER);
        let qi = build_identity(
            IdentityType::Masternode,
            vec![owner.clone(), transfer.clone()],
            vec![owner, transfer],
        );
        let selected = qi.default_withdrawal_key().expect("a key");
        assert_eq!(selected.identity_public_key.purpose(), Purpose::TRANSFER);
    }
}
