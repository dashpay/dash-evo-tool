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
use crate::model::user_role::UserRole;
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

/// Presence of the three masternode/evonode key roles on a loaded node.
///
/// A node loads read-only without any keys; each role can be present or absent
/// independently. Used by the Masternodes card grid to render the compact
/// `V O P` key-status indicator (present roles emphasised, absent roles dimmed)
/// — never colour-only (NFR-6).
///
/// Role → purpose mapping (see `verify_*_key_exists_on_identity` in
/// `backend_task/identity/mod.rs`):
/// * Voting  → a `PrivateKeyOnVoterIdentity` key / `associated_voter_identity`
/// * Owner   → a main-identity key with [`Purpose::OWNER`]
/// * Payout  → a main-identity key with [`Purpose::TRANSFER`]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MasternodeKeyPresence {
    pub voting: bool,
    pub owner: bool,
    pub payout: bool,
}

/// An `OWNER`-key withdrawal was asked to pay an address other than the
/// identity's registered payout address, which Platform does not permit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerKeyWithdrawalNotAllowed;

/// No key could be resolved to sign an identity credit withdrawal: the
/// explicitly requested key is unknown, disabled, or not one this app can sign
/// with, or — when no key was requested — the identity holds no active,
/// locally-signable `TRANSFER`/`OWNER` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoUsableWithdrawalKey;

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

/// Cap on any single allocation `from_bytes` will make while decoding a
/// `QualifiedIdentity` blob. A real identity — including its private keys,
/// DPNS names, and wallet links — is far under this. The cap exists only as a
/// decode-time safety net: bincode's default `NoLimit` config trusts a
/// length-prefixed field's claimed size and pre-allocates it *before* reading
/// anything, so a single flipped bit or a truncated blob can claim gigabytes
/// and abort the process (`handle_alloc_error`, uncatchable, not a
/// `Result::Err`) rather than fail the decode gracefully. `Limit` makes
/// bincode check the claimed size against this cap first and return
/// `DecodeError::LimitExceeded` instead — see
/// `a_length_inflated_collection_prefix_is_rejected_not_preallocated` below
/// for the regression coverage. Encoding is unaffected: this only bounds
/// decode-time allocation and does not change the wire format, so it stays
/// compatible with blobs `to_bytes` already wrote.
const IDENTITY_BLOB_DECODE_LIMIT: usize = 16 * 1024 * 1024; // 16 MiB

/// The bincode configuration [`QualifiedIdentity::from_bytes`] decodes under.
/// Pulled into its own function (rather than inlined at the one call site) so
/// the decode-limit regression test below exercises the *exact* configuration
/// production uses — sharing this function, not a second hand-typed copy of
/// `.with_limit()` — so a future edit that weakens or drops the limit here is
/// caught by that test rather than silently diverging from it.
fn identity_blob_decode_config() -> impl bincode::config::Config {
    bincode::config::standard().with_limit::<{ IDENTITY_BLOB_DECODE_LIMIT }>()
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
    ///
    /// Decodes under [`identity_blob_decode_config`] rather than bincode's
    /// unbounded default, so a corrupted or length-inflated blob returns this
    /// `Err` instead of aborting the process.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        bincode::decode_from_slice(bytes, identity_blob_decode_config())
            .map(|(identity, _)| identity)
            .map_err(|e| format!("Failed to decode QualifiedIdentity: {}", e))
    }

    /// Which masternode/evonode key roles are loaded for this identity.
    ///
    /// Voting presence is signalled by a loaded voter identity
    /// (`associated_voter_identity`) OR any [`Purpose::VOTING`] key; owner by a
    /// [`Purpose::OWNER`] key; payout by a [`Purpose::TRANSFER`] key. Intended
    /// for masternode/evonode identities — a `User` identity may carry a
    /// `TRANSFER` key for withdrawals, which this method would report as
    /// `payout`, so callers must scope it to the Masternodes surface.
    pub fn masternode_key_presence(&self) -> MasternodeKeyPresence {
        let mut presence = MasternodeKeyPresence {
            voting: self.associated_voter_identity.is_some(),
            owner: false,
            payout: false,
        };
        for (public_key, _) in self.private_keys.private_keys.values() {
            match public_key.identity_public_key.purpose() {
                Purpose::VOTING => presence.voting = true,
                Purpose::OWNER => presence.owner = true,
                Purpose::TRANSFER => presence.payout = true,
                _ => {}
            }
        }
        presence
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
            // Platform rejects signing with a disabled key. Rotating a masternode
            // payout address disables the old TRANSFER key and appends a new active
            // one, so a rotated identity holds both — only the active key is signable.
            if public_key.identity_public_key.is_disabled() {
                continue;
            }
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

    /// Whether the active role can attempt a withdrawal with this identity.
    ///
    /// Uniform across every role: a withdrawal is only ever executable with a
    /// locally-held TRANSFER or OWNER key (see
    /// [`resolve_withdrawal_signing_key`](Self::resolve_withdrawal_signing_key),
    /// the backend's unconditional enforcement layer — no role currently
    /// relaxes it). A Developer-only "any on-chain key" carve-out previously
    /// existed here and in [`default_withdrawal_key`](Self::default_withdrawal_key)'s
    /// caller, but nothing downstream ever accepted an on-chain-only key for
    /// signing, so that carve-out only produced an enabled button that led to
    /// a blank or failing withdrawal screen. Gate on the same invariant the
    /// backend enforces, so this predicate never lies about what the screen
    /// can actually do.
    pub fn can_attempt_withdrawal(&self, _role: UserRole) -> bool {
        !self.available_withdrawal_keys().is_empty()
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

    /// Resolves the effective key to sign an identity credit withdrawal,
    /// returning its [`KeyID`].
    ///
    /// `requested` is the caller's explicit key choice, or `None` to auto-select.
    /// Resolution runs against `self.identity`, which the caller must have
    /// refreshed from Platform first: a key disabled on-chain since the identity
    /// was last loaded is rejected here rather than reaching signing. Only keys
    /// whose private material this app holds and whose purpose is `TRANSFER`
    /// (preferred) or `OWNER` are eligible, mirroring Platform's
    /// `TransferPreferred` selection.
    ///
    /// An explicit `requested` id that is unknown, disabled, or not locally
    /// signable is rejected — never silently replaced with a different key.
    /// Returns [`NoUsableWithdrawalKey`] when nothing eligible remains, so the
    /// caller can surface a clear error instead of letting the SDK pick a key
    /// (which would allow a disabled key or an unintended `OWNER` fallback).
    pub fn resolve_withdrawal_signing_key(
        &self,
        requested: Option<KeyID>,
    ) -> Result<KeyID, NoUsableWithdrawalKey> {
        // Locally-held TRANSFER/OWNER keys that are also active in the current
        // (refreshed) identity snapshot. `available_withdrawal_keys` filters on
        // the stored copy's disabled flag; the extra check catches a key that was
        // disabled on-chain after this identity was loaded.
        let usable: Vec<&QualifiedIdentityPublicKey> = self
            .available_withdrawal_keys()
            .into_iter()
            .filter(|qk| {
                matches!(
                    self.identity.get_public_key_by_id(qk.identity_public_key.id()),
                    Some(current) if !current.is_disabled()
                )
            })
            .collect();

        if let Some(requested_id) = requested {
            return usable
                .iter()
                .any(|qk| qk.identity_public_key.id() == requested_id)
                .then_some(requested_id)
                .ok_or(NoUsableWithdrawalKey);
        }

        usable
            .iter()
            .find(|qk| qk.identity_public_key.purpose() == Purpose::TRANSFER)
            .or_else(|| {
                usable
                    .iter()
                    .find(|qk| qk.identity_public_key.purpose() == Purpose::OWNER)
            })
            .map(|qk| qk.identity_public_key.id())
            .ok_or(NoUsableWithdrawalKey)
    }

    /// Resolves the output address to pass to a withdrawal, enforcing Platform's
    /// rule that an `OWNER`-key withdrawal must carry no output script and is
    /// routed to the identity's registered payout address.
    ///
    /// For a non-`OWNER` signing key the requested address passes through
    /// unchanged. For an `OWNER` key: a `None` request, or one that already
    /// equals the registered payout address, resolves to `None` (Platform pays
    /// the registered payout address). A request for any other address returns
    /// [`OwnerKeyWithdrawalNotAllowed`] — the owner key cannot pay it, and
    /// silently redirecting to the payout address would send funds elsewhere
    /// than the user asked.
    pub fn resolve_withdrawal_output(
        &self,
        signing_key_purpose: Option<Purpose>,
        requested: Option<Address>,
        network: Network,
    ) -> Result<Option<Address>, OwnerKeyWithdrawalNotAllowed> {
        if signing_key_purpose != Some(Purpose::OWNER) {
            return Ok(requested);
        }
        match requested {
            None => Ok(None),
            Some(address) => {
                if self.masternode_payout_address(network).as_ref() == Some(&address) {
                    Ok(None)
                } else {
                    Err(OwnerKeyWithdrawalNotAllowed)
                }
            }
        }
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
mod masternode_key_presence_tests {
    use super::*;
    use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::platform_value::BinaryData;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;

    /// Build a main-identity public key with an explicit purpose. Only the
    /// purpose is read by [`QualifiedIdentity::masternode_key_presence`]; the
    /// key type and data are inert placeholders.
    fn key_with_purpose(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
        IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id,
            purpose,
            security_level: SecurityLevel::HIGH,
            contract_bounds: None,
            key_type: KeyType::ECDSA_HASH160,
            read_only: false,
            data: BinaryData::new(vec![0u8; 20]),
            disabled_at: None,
        })
    }

    /// Assemble a masternode-shaped `QualifiedIdentity`: `voting` attaches a
    /// voter identity; each purpose in `main_key_purposes` becomes a
    /// main-identity key.
    fn qi_with(voting: bool, main_key_purposes: &[Purpose]) -> QualifiedIdentity {
        let pv = PlatformVersion::latest();
        let identity =
            Identity::create_basic_identity(Identifier::from([1u8; 32]), pv).expect("identity");

        let mut ks = KeyStorage::default();
        for (i, purpose) in main_key_purposes.iter().enumerate() {
            let key = key_with_purpose(i as KeyID, *purpose);
            ks.private_keys.insert(
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, key.id()),
                (
                    QualifiedIdentityPublicKey::from(key),
                    PrivateKeyData::Clear([0u8; 32]),
                ),
            );
        }

        let associated_voter_identity = voting.then(|| {
            let voter = Identity::create_basic_identity(Identifier::from([2u8; 32]), pv)
                .expect("voter identity");
            let voting_key = key_with_purpose(0, Purpose::VOTING);
            (voter, voting_key)
        });

        QualifiedIdentity {
            identity,
            associated_voter_identity,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: None,
            private_keys: ks,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    /// TC-FR3-08 — all eight bit-combinations of {Voting, Owner, Payout} are
    /// reported exactly, with all-off and all-on distinct from partial states.
    #[test]
    fn tc_fr3_08_all_vop_combinations() {
        for mask in 0u8..8 {
            let voting = mask & 0b100 != 0;
            let owner = mask & 0b010 != 0;
            let payout = mask & 0b001 != 0;

            let mut purposes = Vec::new();
            if owner {
                purposes.push(Purpose::OWNER);
            }
            if payout {
                purposes.push(Purpose::TRANSFER);
            }

            let presence = qi_with(voting, &purposes).masternode_key_presence();
            assert_eq!(
                presence,
                MasternodeKeyPresence {
                    voting,
                    owner,
                    payout,
                },
                "mask {mask:03b} (V={voting} O={owner} P={payout}) misreported"
            );
        }
    }

    /// A `Purpose::VOTING` key on the main identity signals voting readiness
    /// even without a separately loaded voter identity.
    #[test]
    fn voting_purpose_key_counts_as_voting_present() {
        let presence = qi_with(false, &[Purpose::VOTING]).masternode_key_presence();
        assert!(presence.voting);
        assert!(!presence.owner);
        assert!(!presence.payout);
    }

    /// A node loaded read-only (no keys, no voter identity) reports every role
    /// absent.
    #[test]
    fn read_only_node_has_no_keys() {
        let presence = qi_with(false, &[]).masternode_key_presence();
        assert_eq!(presence, MasternodeKeyPresence::default());
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

    fn disabled_key(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
        let mut k = key(id, purpose);
        k.set_disabled_at(1);
        k
    }

    /// Repro for the withdrawal disabled-key bug: rotating a masternode payout
    /// address disables the original `TRANSFER` key (id 0) and appends a new
    /// active one at a higher id. The disabled key must be skipped so the
    /// withdrawal signs with the active key Platform still accepts.
    #[test]
    fn disabled_transfer_key_is_skipped_for_active() {
        let disabled = disabled_key(0, Purpose::TRANSFER);
        let active = key(1, Purpose::TRANSFER);
        let qi = build_identity(
            IdentityType::Masternode,
            vec![disabled.clone(), active.clone()],
            vec![disabled, active],
        );
        let selected = qi.default_withdrawal_key().expect("a key");
        assert_eq!(selected.identity_public_key.id(), 1);
        assert!(!selected.identity_public_key.is_disabled());
    }

    #[test]
    fn available_withdrawal_keys_excludes_disabled() {
        let disabled = disabled_key(0, Purpose::TRANSFER);
        let active = key(1, Purpose::TRANSFER);
        let qi = build_identity(
            IdentityType::Masternode,
            vec![disabled.clone(), active.clone()],
            vec![disabled, active],
        );
        let ids: Vec<KeyID> = qi
            .available_withdrawal_keys()
            .iter()
            .map(|qk| qk.identity_public_key.id())
            .collect();
        assert_eq!(ids, vec![1]);
    }

    /// An on-chain-only key (no local private material) can never be used to
    /// sign a withdrawal (see `resolve_withdrawal_signing_key`), for any
    /// role — including Developer. Enabling the gate for it previously led
    /// `WithdrawalScreen` to a blank or failing screen.
    #[test]
    fn can_attempt_withdrawal_ignores_on_chain_only_keys_for_every_role() {
        let without_keys = build_identity(IdentityType::User, vec![], vec![]);
        let authentication = key(1, Purpose::AUTHENTICATION);
        let on_chain_only = build_identity(IdentityType::User, vec![authentication], vec![]);

        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            assert!(!without_keys.can_attempt_withdrawal(role));
            assert!(
                !on_chain_only.can_attempt_withdrawal(role),
                "on-chain-only AUTHENTICATION key must not enable withdrawal for {role:?}"
            );
        }
    }

    #[test]
    fn can_attempt_withdrawal_uses_local_withdrawal_keys_for_every_role() {
        let transfer = key(2, Purpose::TRANSFER);
        let locally_signable =
            build_identity(IdentityType::User, vec![transfer.clone()], vec![transfer]);

        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            assert!(locally_signable.can_attempt_withdrawal(role));
        }
    }

    /// A wholly disabled key set leaves no signable withdrawal key.
    #[test]
    fn all_disabled_yields_no_withdrawal_key() {
        let disabled = disabled_key(0, Purpose::TRANSFER);
        let qi = build_identity(
            IdentityType::Masternode,
            vec![disabled.clone()],
            vec![disabled],
        );
        assert!(qi.default_withdrawal_key().is_none());
        assert!(qi.available_withdrawal_keys().is_empty());
    }

    /// A loaded enabled TRANSFER key is preferred over a lower-id OWNER key, so
    /// an arbitrary-address withdrawal never defaults to the OWNER key (which
    /// Platform rejects when an output script is present).
    #[test]
    fn active_transfer_preferred_over_lower_id_owner() {
        let disabled_transfer = disabled_key(0, Purpose::TRANSFER);
        let owner = key(1, Purpose::OWNER);
        let active_transfer = key(2, Purpose::TRANSFER);
        let qi = build_identity(
            IdentityType::Masternode,
            vec![
                disabled_transfer.clone(),
                owner.clone(),
                active_transfer.clone(),
            ],
            vec![disabled_transfer, owner, active_transfer],
        );
        let selected = qi.default_withdrawal_key().expect("a key");
        assert_eq!(selected.identity_public_key.id(), 2);
        assert_eq!(selected.identity_public_key.purpose(), Purpose::TRANSFER);
    }

    /// Auto-select (no explicit id) resolves the active TRANSFER key, matching
    /// the SDK's `TransferPreferred` intent but restricted to keys the signer
    /// can actually use.
    #[test]
    fn resolve_signing_key_auto_selects_active_transfer() {
        let transfer = key(1, Purpose::TRANSFER);
        let qi = build_identity(IdentityType::User, vec![transfer.clone()], vec![transfer]);
        assert_eq!(qi.resolve_withdrawal_signing_key(None), Ok(1));
    }

    /// A key active in the stored copy but disabled in the refreshed on-chain
    /// identity must not be auto-selected — the exact case the SDK's own
    /// selection would sign with and fail. The active key is chosen instead.
    #[test]
    fn resolve_signing_key_skips_key_disabled_after_refresh() {
        // On-chain (refreshed): id 1 disabled, id 2 active.
        // Stored copy: both look active (stale) — the resolver must consult the
        // refreshed identity, not the stored disabled flag.
        let qi = build_identity(
            IdentityType::User,
            vec![
                disabled_key(1, Purpose::TRANSFER),
                key(2, Purpose::TRANSFER),
            ],
            vec![key(1, Purpose::TRANSFER), key(2, Purpose::TRANSFER)],
        );
        assert_eq!(qi.resolve_withdrawal_signing_key(None), Ok(2));
    }

    /// An explicitly requested key that is disabled on-chain after refresh is
    /// rejected, never silently swapped for another key.
    #[test]
    fn resolve_signing_key_rejects_explicit_disabled_key() {
        let qi = build_identity(
            IdentityType::User,
            vec![
                disabled_key(1, Purpose::TRANSFER),
                key(2, Purpose::TRANSFER),
            ],
            vec![key(1, Purpose::TRANSFER), key(2, Purpose::TRANSFER)],
        );
        assert_eq!(
            qi.resolve_withdrawal_signing_key(Some(1)),
            Err(NoUsableWithdrawalKey)
        );
    }

    /// An explicitly requested key the signer does not hold locally is rejected.
    #[test]
    fn resolve_signing_key_rejects_unknown_explicit_key() {
        let transfer = key(1, Purpose::TRANSFER);
        let qi = build_identity(IdentityType::User, vec![transfer.clone()], vec![transfer]);
        assert_eq!(
            qi.resolve_withdrawal_signing_key(Some(99)),
            Err(NoUsableWithdrawalKey)
        );
    }

    /// A valid explicit request resolves to that same key.
    #[test]
    fn resolve_signing_key_honors_valid_explicit_request() {
        let owner = key(1, Purpose::OWNER);
        let transfer = key(2, Purpose::TRANSFER);
        let qi = build_identity(
            IdentityType::Masternode,
            vec![owner.clone(), transfer.clone()],
            vec![owner, transfer],
        );
        assert_eq!(qi.resolve_withdrawal_signing_key(Some(1)), Ok(1));
    }

    /// With no usable key at all, resolution fails so the caller surfaces a
    /// clear error instead of the SDK picking a key that cannot sign.
    #[test]
    fn resolve_signing_key_errors_when_none_usable() {
        let ghost = key(1, Purpose::TRANSFER);
        // On-chain only; no private material held.
        let qi = build_identity(IdentityType::User, vec![ghost], vec![]);
        assert_eq!(
            qi.resolve_withdrawal_signing_key(None),
            Err(NoUsableWithdrawalKey)
        );
    }

    fn payout_key(id: KeyID, hash: [u8; 20]) -> IdentityPublicKey {
        let mut k = key(id, Purpose::TRANSFER);
        k.set_key_type(KeyType::ECDSA_HASH160);
        k.set_data(BinaryData::new(hash.to_vec()));
        k
    }

    fn addr(hash: [u8; 20]) -> Address {
        Address::new(
            Network::Testnet,
            Payload::PubkeyHash(PubkeyHash::from_byte_array(hash)),
        )
    }

    /// Repro for the owner-key withdrawal bug: Platform rejects an owner-key
    /// withdrawal that carries an output script. When only the OWNER key is
    /// loaded and the user targets the registered payout address, the output
    /// script must be omitted so Platform routes to the payout address.
    #[test]
    fn owner_key_withdrawal_to_payout_omits_output_script() {
        let owner = key(1, Purpose::OWNER);
        let payout = payout_key(2, [0x11; 20]);
        let qi = build_identity(
            IdentityType::Masternode,
            vec![owner.clone(), payout],
            vec![owner],
        );
        let payout_addr = qi
            .masternode_payout_address(Network::Testnet)
            .expect("payout address");

        assert_eq!(
            qi.resolve_withdrawal_output(
                Some(Purpose::OWNER),
                Some(payout_addr),
                Network::Testnet,
            ),
            Ok(None),
        );
        assert_eq!(
            qi.resolve_withdrawal_output(Some(Purpose::OWNER), None, Network::Testnet),
            Ok(None),
        );
    }

    #[test]
    fn owner_key_withdrawal_to_other_address_is_rejected() {
        let owner = key(1, Purpose::OWNER);
        let payout = payout_key(2, [0x11; 20]);
        let qi = build_identity(
            IdentityType::Masternode,
            vec![owner.clone(), payout],
            vec![owner],
        );
        assert_eq!(
            qi.resolve_withdrawal_output(
                Some(Purpose::OWNER),
                Some(addr([0x22; 20])),
                Network::Testnet,
            ),
            Err(OwnerKeyWithdrawalNotAllowed),
        );
    }

    #[test]
    fn transfer_key_withdrawal_passes_requested_address_through() {
        let transfer = payout_key(2, [0x11; 20]);
        let qi = build_identity(
            IdentityType::Masternode,
            vec![transfer.clone()],
            vec![transfer],
        );
        let requested = addr([0x22; 20]);
        assert_eq!(
            qi.resolve_withdrawal_output(
                Some(Purpose::TRANSFER),
                Some(requested.clone()),
                Network::Testnet,
            ),
            Ok(Some(requested)),
        );
    }
}

/// Regression coverage for the `from_bytes` decode-limit fix (SEC-001 from the
/// PR #885 grumpy-review): a corrupted or length-inflated blob must decode to
/// a graceful `Err`, never abort the process.
///
/// This deliberately does NOT decode a full `QualifiedIdentity` blob. Crafting
/// a byte-exact corruption of a real encoded identity is fragile — it would
/// tie the test to the current field order of a struct with many nested
/// types, and it does not need to succeed through `Identity`'s own encoding
/// to prove the point. `from_bytes`'s vulnerability lived entirely in its
/// bincode *configuration*, not in `QualifiedIdentity`'s shape: any
/// length-prefixed collection decoded under that configuration was exposed.
/// Exercising the exact same configuration directly against a minimal,
/// hand-built length-inflated prefix pins the actual fix (the config change)
/// precisely, and stays valid regardless of future changes to
/// `QualifiedIdentity`'s fields.
///
/// The prefix construction mirrors the live reproduction from the review: a
/// `u64` varint length header (bincode's `U64_BYTE` marker, 253) claiming an
/// enormous element count, followed by only a couple of trailing bytes --
/// exactly what a single flipped continuation bit or a truncated file
/// produces on a real blob. Before the fix (decoding under
/// `bincode::config::standard()`, i.e. `NoLimit`), decoding this buffer as
/// `Vec<u8>` pre-allocates the claimed length and aborts the process --
/// confirmed by a standalone probe run outside the test harness during
/// review, since an in-process abort cannot be asserted as a normal test
/// failure (it takes the whole test binary down with it). After the fix
/// (decoding under `IDENTITY_BLOB_DECODE_LIMIT`), the same buffer must return
/// `DecodeError::LimitExceeded` instead.
#[cfg(test)]
mod decode_limit_tests {
    use super::identity_blob_decode_config;

    #[test]
    fn a_length_inflated_collection_prefix_is_rejected_not_preallocated() {
        // bincode 2.0.1's varint scheme: 253 (`U64_BYTE`) marks "the next 8
        // bytes are a little-endian u64 length". Claim far more than the
        // configured limit, then supply only 2 trailing bytes -- ordinary
        // bit-flip/truncation corruption never has the claimed payload
        // actually present.
        const U64_VARINT_MARKER: u8 = 253;
        let claimed_len: u64 = 1 << 40; // 1 TiB -- larger than any real identity blob
        let mut corrupted = vec![U64_VARINT_MARKER];
        corrupted.extend_from_slice(&claimed_len.to_le_bytes());
        corrupted.extend_from_slice(&[0xAA, 0xBB]);

        // Uses the SAME config function `from_bytes` calls -- not a second
        // hand-typed `.with_limit()` -- so a regression in that shared
        // function is what this test actually catches.
        let result: Result<(Vec<u8>, usize), bincode::error::DecodeError> =
            bincode::decode_from_slice(&corrupted, identity_blob_decode_config());

        match result {
            Err(bincode::error::DecodeError::LimitExceeded) => {}
            other => panic!(
                "expected DecodeError::LimitExceeded for a length-inflated prefix, got \
                 {other:?} -- if from_bytes's decode config regresses to NoLimit this \
                 same buffer would instead pre-allocate 1 TiB and abort the process"
            ),
        }
    }

    /// Sanity check that the limit is not so tight it rejects ordinary
    /// legitimate data -- a real `QualifiedIdentity` with keys is far under
    /// 16 MiB (the golden v0.9.3 fixture blob decoded elsewhere in this crate
    /// is a few hundred bytes), so a plain in-bounds `Vec<u8>` must still
    /// round-trip under the same limited config.
    #[test]
    fn an_ordinary_small_payload_still_decodes_under_the_limit() {
        let payload = vec![0xABu8; 4096];
        let encoded = bincode::encode_to_vec(&payload, identity_blob_decode_config())
            .expect("encode under the limit");
        let (decoded, _): (Vec<u8>, usize) =
            bincode::decode_from_slice(&encoded, identity_blob_decode_config())
                .expect("decode under the limit");
        assert_eq!(decoded, payload);
    }
}
