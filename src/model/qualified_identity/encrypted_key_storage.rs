use crate::backend_task::error::TaskError;
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
use dash_sdk::platform::IdentityPublicKey;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, RwLock};
use zeroize::Zeroizing;

/// A resolved private key paired with its public-key metadata. The key bytes
/// are held in [`Zeroizing`] so they are wiped when the resolver's result is
/// dropped.
pub type ResolvedPrivateKey = (QualifiedIdentityPublicKey, Zeroizing<[u8; 32]>);

/// Whether a `stored` public half is the same key as the `live` one, ignoring
/// only `disabled_at`.
///
/// Every field of an `IdentityPublicKey` is immutable once the key is added, with
/// that single exception: disabling a key rewrites it. The stored copy is a
/// snapshot taken when the private half was saved, so plain `==` stops matching
/// as soon as a key is disabled or rotated, and a key this device demonstrably
/// holds gets reported as missing.
///
/// Comparing only the id and the key material would fix that and open a worse
/// hole the other way. A main identity's voting key and a linked voter identity's
/// key can carry identical `data` under the same `id`, leaving `purpose` as the
/// only thing telling them apart — and a lookup that conflates them can hand out,
/// or delete, material the requested key does not own. So this excludes the one
/// field that legitimately moves and nothing else.
pub(crate) fn same_key(stored: &IdentityPublicKey, live: &IdentityPublicKey) -> bool {
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;

    let IdentityPublicKey::V0(stored) = stored;
    let IdentityPublicKey::V0(live) = live;
    // Destructured exhaustively, and without `..`, on purpose: a field added
    // upstream must break this build rather than be silently ignored. A new field
    // that distinguishes two keys would otherwise leave this reporting a match
    // where there is none — which is how one key's private material ends up
    // attributed to another. Whoever adds it decides here whether it identifies a
    // key or, like `disabled_at`, only describes its state.
    let IdentityPublicKeyV0 {
        id,
        purpose,
        security_level,
        contract_bounds,
        key_type,
        read_only,
        data,
        // The one field Platform lets move after a key is added: disabling a key
        // rewrites it, and the stored snapshot predates that.
        disabled_at: _,
    } = stored;

    *id == live.id
        && *purpose == live.purpose
        && *security_level == live.security_level
        && *contract_bounds == live.contract_bounds
        && *key_type == live.key_type
        && *read_only == live.read_only
        && *data == live.data
}

/// A `(target, key_id)` map key paired with the raw 32-byte private key the
/// migration must store in the vault — see
/// [`KeyStorage::take_plaintext_for_vault`]. Bytes are [`Zeroizing`].
pub type VaultBoundKey = ((PrivateKeyTarget, KeyID), Zeroizing<[u8; 32]>);

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
        Self::decode(decoder)
    }
}

#[derive(Clone, Encode, Decode, PartialEq)]
pub enum PrivateKeyData {
    AlwaysClear([u8; 32]), // This is for keys that are MEDIUM security level
    Clear([u8; 32]),
    Encrypted(Vec<u8>),
    AtWalletDerivationPath(WalletDerivationPath),
    /// The key's raw bytes live in the secret vault, fetched per-use through
    /// the seam — never resident in this blob. A permanent in-memory
    /// placeholder: the resolver reads the vault at sign time keyed by the
    /// identity scope + the `(target, key_id)` BTreeMap key, so this variant
    /// carries no bytes.
    ///
    /// Appended at the highest bincode index so blobs written before it
    /// (discriminants 0–3) still decode unchanged (TS-RESID-02).
    InVault,
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
            PrivateKeyData::InVault => f.debug_tuple("InVault").finish(),
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
            PrivateKeyData::InVault => write!(f, "InVault"),
        }
    }
}

/// Every private half this install holds for one identity, keyed by the store it
/// is filed under and the key's id.
///
/// The map is private on purpose. Which store a key is filed under is not
/// something a caller should be deriving for itself — that is what produced a
/// saved key no signing path could find — so reads go through
/// [`candidates`](Self::candidates), which selects on key material.
///
/// Every accessor that *names* a placement is `pub(crate)`, so the callers that
/// legitimately know one stay enumerable: the loader folding a previously-loaded
/// record into a fresh one, and legacy recovery, which is *about* the placements
/// an old blob recorded. [`insert_at`](Self::insert_at) and
/// [`entry_at`](Self::entry_at) have no production caller left at all and are
/// `#[cfg(test)]`. What stays `pub` cannot quietly file a key wrongly: the
/// target-blind enumerators, [`insert_non_encrypted`](Self::insert_non_encrypted),
/// which refuses an occupied slot, and the whole-map `From` conversions, where
/// the map's own keys are the placements.
// TODO(placement-named-pub-surface): the narrowing above is overstated — several
// pub methods still take a caller-named placement (get_resolve_local,
// get_resolve_with_seed, get_cloned_private_key_data_and_wallet_info,
// mark_in_vault, is_in_vault, public_key_for, wallet_seed_hash_for), and
// mark_in_vault zeroizes the occupant with no same_key guard. Narrow them to
// pub(crate) or guard them, then align this rustdoc and restore design.md §7's
// stronger claim (§8 tracks this residual).
#[derive(Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct KeyStorage {
    private_keys: BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, PrivateKeyData)>,
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
    ) -> Result<Option<ResolvedPrivateKey>, String> {
        self.private_keys
            .get(key)
            .map(
                |(qualified_identity_public_key_data, private_key_data)| match private_key_data {
                    PrivateKeyData::AlwaysClear(clear) | PrivateKeyData::Clear(clear) => Ok((
                        qualified_identity_public_key_data.clone(),
                        Zeroizing::new(*clear),
                    )),
                    PrivateKeyData::Encrypted(_) => {
                        Err("Key is encrypted, please enter password".to_string())
                    }
                    PrivateKeyData::AtWalletDerivationPath(_) => {
                        Err("Key is not resolved, please unlock the wallet".to_string())
                    }
                    PrivateKeyData::InVault => {
                        Err("Key is stored securely, resolve it through the vault".to_string())
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
    ) -> Result<Option<ResolvedPrivateKey>, String> {
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
                )
                .map_err(|e| e.to_string())?
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

    pub(crate) fn has(&self, key: &(PrivateKeyTarget, KeyID)) -> bool {
        self.private_keys.contains_key(key)
    }

    /// Every map key whose stored public half *is* `key`, in
    /// [`PROBE_ORDER`](crate::model::qualified_identity::key_placement::PROBE_ORDER).
    ///
    /// This is how a read finds a private half. It probes each store at `key`'s
    /// own id and keeps only entries [`same_key`] accepts, so it finds the key
    /// wherever an older build filed it and never returns a different key that
    /// merely shares the id — the voter and main id spaces overlap, so matching
    /// on the id alone is what lets a delete land on the wrong key.
    ///
    /// Three `BTreeMap` probes, not a scan. Yields more than one entry only when
    /// the same key is genuinely filed under several stores; a caller that needs
    /// bytes should take the first that *resolves* rather than the first that
    /// matches, since a match can name a vault placeholder whose secret is gone
    /// (see
    /// [`resolve_private_key_bytes`](crate::model::qualified_identity::QualifiedIdentity::resolve_private_key_bytes)).
    pub fn candidates<'a>(
        &'a self,
        key: &'a IdentityPublicKey,
    ) -> impl Iterator<Item = (PrivateKeyTarget, KeyID)> + 'a {
        let key_id = key.id();
        crate::model::qualified_identity::key_placement::PROBE_ORDER
            .iter()
            .filter_map(move |target| {
                let map_key = (target.clone(), key_id);
                let (stored, _) = self.private_keys.get(&map_key)?;
                same_key(&stored.identity_public_key, key).then_some(map_key)
            })
    }

    /// The placement a synchronous caller should name for `key`: the first
    /// candidate whose bytes are resident, or the first candidate at all.
    ///
    /// [`candidates`](Self::candidates) can name a [`PrivateKeyData::InVault`]
    /// placeholder ahead of an entry that carries its own material. Resolving
    /// bytes is the honest way to choose between them, but that is async and a
    /// UI frame cannot await, so this picks the placement whose material is
    /// visible without opening the vault and falls back to the first match when
    /// none is. Every synchronous site shares this one rule, so the placement a
    /// screen names and the material it displays can never come from two
    /// different stores.
    pub fn first_live_candidate(
        &self,
        key: &IdentityPublicKey,
    ) -> Option<(PrivateKeyTarget, KeyID)> {
        // Two lazy passes rather than one collected Vec: this runs per key per
        // frame, and the fallback re-probe costs at most three map reads.
        self.candidates(key)
            .find(|placement| !self.is_in_vault(placement))
            .or_else(|| self.candidates(key).next())
    }

    /// What this identity holds for `key` — its stored [`PrivateKeyData`] and
    /// the wallet derivation info recorded with it — taken from the placement
    /// [`first_live_candidate`](Self::first_live_candidate) names.
    ///
    /// Clones the entry, so it copies raw key bytes for a plaintext-carrying
    /// key: call it when the material is actually needed, not to answer whether
    /// a key is held.
    pub fn held_private_key_data(
        &self,
        key: &IdentityPublicKey,
    ) -> Option<(PrivateKeyData, Option<WalletDerivationPath>)> {
        let placement = self.first_live_candidate(key)?;
        self.get_cloned_private_key_data_and_wallet_info(&placement)
    }

    /// The wallet derivation path `key` is filed at, under any placement.
    ///
    /// A typed probe, not a liveness one: it answers "is this a wallet-derived
    /// key, and from which wallet", so a key filed under two placements where
    /// the first is not wallet-derived still finds its wallet.
    pub fn wallet_derived_at(&self, key: &IdentityPublicKey) -> Option<&WalletDerivationPath> {
        self.candidates(key)
            .find_map(|placement| match self.private_keys.get(&placement) {
                Some((_, PrivateKeyData::AtWalletDerivationPath(path))) => Some(path),
                _ => None,
            })
    }

    /// Returns all stored key identifiers.
    pub fn keys_set(&self) -> BTreeSet<(PrivateKeyTarget, KeyID)> {
        self.private_keys.keys().cloned().collect()
    }

    /// How many private halves are held.
    pub fn len(&self) -> usize {
        self.private_keys.len()
    }

    /// Whether no private half is held at all.
    pub fn is_empty(&self) -> bool {
        self.private_keys.is_empty()
    }

    /// Every entry, keyed by placement. Target-blind, for callers that need to
    /// walk the whole store rather than find one key.
    pub fn iter(
        &self,
    ) -> impl Iterator<
        Item = (
            &(PrivateKeyTarget, KeyID),
            &(QualifiedIdentityPublicKey, PrivateKeyData),
        ),
    > {
        self.private_keys.iter()
    }

    /// Every stored entry, without its placement. Target-blind, for callers
    /// asking about the keys themselves rather than where they are filed.
    pub fn values(&self) -> impl Iterator<Item = &(QualifiedIdentityPublicKey, PrivateKeyData)> {
        self.private_keys.values()
    }

    /// The stored entry at exactly `key`, if any.
    ///
    /// Names a placement directly, so it answers "is *this* slot occupied", not
    /// "where is this key". Prefer [`candidates`](Self::candidates) for the
    /// latter: this cannot tell a key from a different one sharing its id.
    #[cfg(test)]
    pub(crate) fn entry_at(
        &self,
        key: &(PrivateKeyTarget, KeyID),
    ) -> Option<&(QualifiedIdentityPublicKey, PrivateKeyData)> {
        self.private_keys.get(key)
    }

    /// Store `value` at exactly `key`, replacing whatever was there.
    ///
    /// For callers that know a placement structurally — a loader walking the
    /// identity list it read a key from, or legacy recovery restoring an entry
    /// to the placement the old blob recorded. A caller that has to *choose* one
    /// for new material asks
    /// [`QualifiedIdentity::placement_of`](crate::model::qualified_identity::QualifiedIdentity::placement_of),
    /// unless construction already fixes it: `add_key_to_identity` mints its key
    /// at `max_id + 1` on the main identity, so only the main store can hold it.
    #[cfg(test)]
    pub(crate) fn insert_at(
        &mut self,
        key: (PrivateKeyTarget, KeyID),
        value: (QualifiedIdentityPublicKey, PrivateKeyData),
    ) -> Option<(QualifiedIdentityPublicKey, PrivateKeyData)> {
        self.private_keys.insert(key, value)
    }

    /// Store `value` at `key` only if nothing is there — the merge-preserving
    /// write. Used when folding a previously-loaded record into a fresh one, so
    /// a key the new load did not resupply is kept rather than dropped, and one
    /// it did resupply is not overwritten with the stale copy.
    pub(crate) fn insert_if_absent(
        &mut self,
        key: (PrivateKeyTarget, KeyID),
        value: (QualifiedIdentityPublicKey, PrivateKeyData),
    ) {
        self.private_keys.entry(key).or_insert(value);
    }

    /// Consume the store, yielding every entry with its placement.
    pub fn into_entries(
        self,
    ) -> impl Iterator<
        Item = (
            (PrivateKeyTarget, KeyID),
            (QualifiedIdentityPublicKey, PrivateKeyData),
        ),
    > {
        self.private_keys.into_iter()
    }

    /// Remove the entry at exactly `key`, returning it if it was there.
    ///
    /// Removing *a key* rather than a slot means removing every placement that
    /// holds it — see [`candidates`](Self::candidates), which selects on key
    /// material so a removal cannot land on a different key sharing the id.
    pub(crate) fn remove_at(
        &mut self,
        key: &(PrivateKeyTarget, KeyID),
    ) -> Option<(QualifiedIdentityPublicKey, PrivateKeyData)> {
        self.private_keys.remove(key)
    }

    pub fn identity_public_keys(&self) -> Vec<(&PrivateKeyTarget, &QualifiedIdentityPublicKey)> {
        self.private_keys
            .iter()
            .map(|((target, _), (key, _))| (target, key))
            .collect()
    }

    /// File a key's plaintext private half at exactly `key`, replacing that
    /// key's own earlier entry if it has one.
    ///
    /// A `MEDIUM`-security key is stored as [`PrivateKeyData::AlwaysClear`],
    /// anything else as [`PrivateKeyData::Clear`].
    ///
    /// # Errors
    ///
    /// [`TaskError::IdentityKeySlotOccupied`] when the slot holds a *different*
    /// key — one [`same_key`] rejects. The voter and main id spaces overlap, so
    /// a slot can legitimately belong to another key, whose private half is
    /// often the only copy there is; taking the slot would destroy it silently.
    pub fn insert_non_encrypted(
        &mut self,
        key: (PrivateKeyTarget, KeyID),
        value: (QualifiedIdentityPublicKey, [u8; 32]),
    ) -> Result<(), TaskError> {
        let (public_key, private_key_bytes) = value;
        if let Some((occupant, _)) = self.private_keys.get(&key)
            && !same_key(
                &occupant.identity_public_key,
                &public_key.identity_public_key,
            )
        {
            return Err(TaskError::IdentityKeySlotOccupied);
        }

        let data = match public_key.identity_public_key.security_level() {
            SecurityLevel::MEDIUM => PrivateKeyData::AlwaysClear(private_key_bytes),
            _ => PrivateKeyData::Clear(private_key_bytes),
        };
        self.private_keys.insert(key, (public_key, data));
        Ok(())
    }

    /// Mark `key` as a vault placeholder ([`PrivateKeyData::InVault`]), wiping
    /// any resident plaintext bytes. Used after a freshly-added key has been
    /// sealed into the secret vault so the at-rest encode path stores
    /// no plaintext for it. Returns `true` if the key was present.
    pub fn mark_in_vault(&mut self, key: &(PrivateKeyTarget, KeyID)) -> bool {
        use zeroize::Zeroize;
        match self.private_keys.get_mut(key) {
            Some((_pub_key, data)) => {
                if let PrivateKeyData::Clear(bytes) | PrivateKeyData::AlwaysClear(bytes) = data {
                    bytes.zeroize();
                }
                *data = PrivateKeyData::InVault;
                true
            }
            None => false,
        }
    }

    /// Whether the key at `key` is a vault placeholder
    /// ([`PrivateKeyData::InVault`]) — its bytes live in the secret vault and
    /// are fetched per-use, never resident here.
    pub fn is_in_vault(&self, key: &(PrivateKeyTarget, KeyID)) -> bool {
        matches!(
            self.private_keys.get(key),
            Some((_, PrivateKeyData::InVault))
        )
    }

    /// The public-key metadata for `key`, regardless of how its private bytes
    /// are stored. Lets a vault-placeholder key still surface its public key
    /// for display and signing-key selection without touching the secret.
    pub fn public_key_for(
        &self,
        key: &(PrivateKeyTarget, KeyID),
    ) -> Option<&QualifiedIdentityPublicKey> {
        self.private_keys.get(key).map(|(pub_key, _)| pub_key)
    }

    /// Whether any key still carries resident plaintext bytes
    /// ([`PrivateKeyData::Clear`] / [`PrivateKeyData::AlwaysClear`]) that
    /// [`Self::take_plaintext_for_vault`] would move into the vault. A cheap,
    /// non-mutating probe so callers can skip a full `KeyStorage` clone when
    /// there is nothing to migrate (the steady-state, already-`InVault` case).
    pub fn has_plaintext_for_vault(&self) -> bool {
        self.private_keys.values().any(|(_, data)| {
            matches!(
                data,
                PrivateKeyData::Clear(_) | PrivateKeyData::AlwaysClear(_)
            )
        })
    }

    /// Whether any key uses the legacy [`PrivateKeyData::Encrypted`] variant.
    ///
    /// `Encrypted` is **decode-only** — no current producer creates these keys
    /// in new installations. They cannot be migrated to the vault without the
    /// decryption password, so [`Self::take_plaintext_for_vault`] leaves them
    /// untouched. The protect-identity guard calls this to fail-closed: an
    /// `Encrypted` key has vault scheme `Absent` and would be silently skipped
    /// by the seal step, causing a false-protected report.
    // TODO: when a migration path for Encrypted keys is available,
    // replace this with a proper re-seal that moves them into the new password
    // envelope instead of blocking the protect operation.
    pub fn has_encrypted_legacy_keys(&self) -> bool {
        self.private_keys
            .values()
            .any(|(_, data)| matches!(data, PrivateKeyData::Encrypted(_)))
    }

    /// Rewrite every plaintext-carrying identity key
    /// ([`PrivateKeyData::Clear`] / [`PrivateKeyData::AlwaysClear`]) to an
    /// [`PrivateKeyData::InVault`] placeholder, returning the raw bytes that
    /// must be stored in the vault under each `(target, key_id)` BEFORE the
    /// blob is persisted (migration order: vault first, then blob rewrite).
    ///
    /// Wallet-derived ([`PrivateKeyData::AtWalletDerivationPath`]) and already
    /// vault-backed / encrypted keys are left untouched — they were never
    /// plaintext-at-rest.
    pub fn take_plaintext_for_vault(&mut self) -> Vec<VaultBoundKey> {
        use zeroize::Zeroize;
        let mut out = Vec::new();
        for (map_key, (_pub_key, data)) in self.private_keys.iter_mut() {
            let raw = match data {
                PrivateKeyData::Clear(bytes) | PrivateKeyData::AlwaysClear(bytes) => {
                    let raw = *bytes;
                    // Wipe the resident array before the `InVault` overwrite
                    // drops it — de-residenting the key is this fn's whole job.
                    bytes.zeroize();
                    raw
                }
                _ => continue,
            };
            out.push((map_key.clone(), Zeroizing::new(raw)));
            *data = PrivateKeyData::InVault;
        }
        out
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

    use crate::wallet_backend::leak_test_support::{assert_no_leak_bytes, distinctive_secret_32};

    /// A recognizable 32-byte secret. Delegates to the shared
    /// [`distinctive_secret_32`] so the seam / sidecar / QI-blob leak cases
    /// share one definition rather than forking it.
    fn distinctive_secret() -> [u8; 32] {
        distinctive_secret_32()
    }

    /// Assert `rendered` exposes the secret in none of the forms a sink could
    /// leak it. Thin wrapper over the shared [`assert_no_leak_bytes`] so the
    /// existing call sites keep their `&[u8; 32]` ergonomics.
    fn assert_no_leak(rendered: &str, secret: &[u8; 32], context: &str) {
        assert_no_leak_bytes(rendered, secret, context);
    }

    /// The redacting `Debug` (and `Display`) on `PrivateKeyData` must
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

    /// Helper: a `KeyStorage` carrying one `Clear` (HIGH) and one `AlwaysClear`
    /// (MEDIUM) plaintext key plus one `AtWalletDerivationPath` key, used by
    /// the migration / residency cases.
    fn storage_with_plaintext_and_derived(
        secret_high: [u8; 32],
        secret_medium: [u8; 32],
    ) -> KeyStorage {
        let pv = PlatformVersion::latest();
        let mut ks = KeyStorage::default();

        let high = IdentityPublicKey::random_key(1, Some(1), pv);
        ks.private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, high.id()),
            (
                QualifiedIdentityPublicKey::from(high),
                PrivateKeyData::Clear(secret_high),
            ),
        );
        let medium = IdentityPublicKey::random_key(2, Some(2), pv);
        ks.private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, medium.id()),
            (
                QualifiedIdentityPublicKey::from(medium),
                PrivateKeyData::AlwaysClear(secret_medium),
            ),
        );
        let derived = IdentityPublicKey::random_key(3, Some(3), pv);
        ks.private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, derived.id()),
            (
                QualifiedIdentityPublicKey::from(derived),
                PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                    wallet_seed_hash: [0x07; 32],
                    derivation_path: DerivationPath::from(vec![]),
                }),
            ),
        );
        ks
    }

    /// TS-RESID-02 — a bincode blob written BEFORE `InVault` was appended
    /// (discriminants 0–3 only) still decodes into the extended enum, and the
    /// new highest-index variant round-trips. Guards the bincode-discriminant
    /// trap: appending at index 4 must not shift 0–3.
    #[test]
    fn ts_resid_02_old_blob_decodes_after_appending_in_vault() {
        let cfg = bincode::config::standard();
        // Each of the four pre-existing variants must round-trip unchanged.
        for original in [
            PrivateKeyData::AlwaysClear([0x11; 32]),
            PrivateKeyData::Clear([0x22; 32]),
            PrivateKeyData::Encrypted(vec![0x33; 48]),
            PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                wallet_seed_hash: [0x44; 32],
                derivation_path: DerivationPath::from(vec![]),
            }),
        ] {
            let bytes = bincode::encode_to_vec(&original, cfg).expect("encode");
            let (decoded, _): (PrivateKeyData, _) =
                bincode::decode_from_slice(&bytes, cfg).expect("decode old variant");
            assert!(
                decoded == original,
                "pre-InVault variant must decode unchanged"
            );
        }
        // The new variant round-trips too.
        let bytes = bincode::encode_to_vec(PrivateKeyData::InVault, cfg).expect("encode");
        let (decoded, _): (PrivateKeyData, _) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode InVault");
        assert!(decoded == PrivateKeyData::InVault);
    }

    /// TS-RESID-01 / TS-NOLEAK-03 — after `take_plaintext_for_vault`, every
    /// plaintext-carrying key is an `InVault` placeholder (zero Clear /
    /// AlwaysClear remain), the wallet-derived key is untouched, and the
    /// returned raw bytes match the originals. The re-encoded blob leaks
    /// neither secret in hex nor decimal-array form.
    #[test]
    fn ts_resid_01_migration_leaves_only_in_vault_and_blob_has_no_plaintext() {
        let high = distinctive_secret_32();
        let mut medium = high;
        medium[0] ^= 0xFF; // distinct from `high`
        let mut ks = storage_with_plaintext_and_derived(high, medium);

        let taken = ks.take_plaintext_for_vault();
        assert_eq!(taken.len(), 2, "both plaintext keys are extracted");
        let taken_bytes: Vec<[u8; 32]> = taken.iter().map(|(_, b)| **b).collect();
        assert!(taken_bytes.contains(&high) && taken_bytes.contains(&medium));

        let mut in_vault = 0;
        let mut derived = 0;
        for (_, data) in ks.private_keys.values() {
            match data {
                PrivateKeyData::InVault => in_vault += 1,
                PrivateKeyData::AtWalletDerivationPath(_) => derived += 1,
                PrivateKeyData::Clear(_) | PrivateKeyData::AlwaysClear(_) => {
                    panic!("plaintext key survived migration")
                }
                PrivateKeyData::Encrypted(_) => {}
            }
        }
        assert_eq!(in_vault, 2, "both plaintext keys became InVault");
        assert_eq!(derived, 1, "wallet-derived key untouched");

        // The persisted blob carries InVault markers, never plaintext.
        let blob = bincode::encode_to_vec(&ks, bincode::config::standard()).expect("encode");
        let rendered = format!("{blob:?}");
        assert_no_leak_bytes(&rendered, &high, "migrated KeyStorage blob (high)");
        assert_no_leak_bytes(&rendered, &medium, "migrated KeyStorage blob (medium)");
    }

    const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;

    /// A slot already holding a *different* key is not free. The voter and main
    /// id spaces overlap, so two keys sharing an id is an ordinary masternode
    /// shape — and overwriting the occupant destroys the only copy of its
    /// private half, in this map and in the vault it is the sole pointer to.
    #[test]
    fn a_different_key_cannot_take_an_occupied_slot() {
        let pv = PlatformVersion::latest();
        let occupant = IdentityPublicKey::random_key(0, Some(1), pv);
        let intruder = IdentityPublicKey::random_key(0, Some(2), pv);
        assert_ne!(
            occupant.data(),
            intruder.data(),
            "the fixture holds two distinct keys under one id"
        );

        let mut ks = KeyStorage::default();
        ks.insert_non_encrypted(
            (MAIN, 0),
            (QualifiedIdentityPublicKey::from(occupant), [0x11; 32]),
        )
        .expect("an empty slot accepts the first key");
        let before = ks.entry_at(&(MAIN, 0)).cloned().expect("occupant stored");

        let refused = ks.insert_non_encrypted(
            (MAIN, 0),
            (QualifiedIdentityPublicKey::from(intruder), [0x22; 32]),
        );

        assert!(
            matches!(refused, Err(TaskError::IdentityKeySlotOccupied)),
            "a foreign occupant must refuse the write, got {refused:?}"
        );
        assert_eq!(
            ks.entry_at(&(MAIN, 0)),
            Some(&before),
            "the occupant's entry survives the refused write unchanged"
        );
    }

    /// The refusal's remedy must be one the user can actually perform. The
    /// occupant is a *locally saved* private half, so removing it is the exit;
    /// a refresh updates published keys but evicts no local private half — and
    /// the backend add path has refetched the identity moments before this
    /// refusal fires, so advising a refresh there sends the user around a loop
    /// that recomputes the identical collision.
    #[test]
    fn the_occupied_slot_refusal_names_a_performable_remedy() {
        let message = TaskError::IdentityKeySlotOccupied.to_string();
        assert!(
            message.contains("remove its saved private key"),
            "the remedy is removing the occupant's local private half, got: {message}"
        );
        assert!(
            !message.contains("Refresh"),
            "refreshing cannot evict the occupying private half, got: {message}"
        );
    }

    /// Re-entering the same key at its own slot replaces it. The paste path
    /// relies on this to let a user correct a key already saved, rather than
    /// growing a second copy under another store.
    #[test]
    fn the_same_key_still_overwrites_itself() {
        let pv = PlatformVersion::latest();
        let key = IdentityPublicKey::random_key(0, Some(1), pv);
        let mut ks = KeyStorage::default();
        ks.insert_non_encrypted(
            (MAIN, 0),
            (QualifiedIdentityPublicKey::from(key.clone()), [0x11; 32]),
        )
        .expect("an empty slot accepts the key");

        ks.insert_non_encrypted(
            (MAIN, 0),
            (QualifiedIdentityPublicKey::from(key), [0x22; 32]),
        )
        .expect("a key may replace itself at its own slot");

        assert_eq!(ks.len(), 1, "replacing in place adds no second entry");
        match ks.entry_at(&(MAIN, 0)).expect("entry present") {
            (_, PrivateKeyData::Clear(bytes) | PrivateKeyData::AlwaysClear(bytes)) => {
                assert_eq!(bytes, &[0x22; 32], "the re-entered bytes replaced the old")
            }
            (_, other) => panic!("expected plaintext bytes, got {other:?}"),
        }
    }

    const VOTER: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;

    /// `disabled_at` is the one field Platform lets move after a key is added,
    /// so a key disabled on chain since its private half was saved must still
    /// match the stored snapshot — otherwise a key the device demonstrably
    /// holds is reported missing the moment it is retired.
    #[test]
    fn same_key_ignores_a_key_being_disabled() {
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;

        let stored = IdentityPublicKey::random_key(0, Some(1), PlatformVersion::latest());
        let mut disabled_since = stored.clone();
        disabled_since.set_disabled_at(1_700_000_000);

        assert!(
            same_key(&stored, &disabled_since),
            "a snapshot taken before the key was disabled still names that key"
        );
        assert!(
            same_key(&disabled_since, &stored),
            "and the comparison does not depend on which side moved"
        );
    }

    /// Every other field identifies the key. `read_only` stands for the six:
    /// disagreeing on any of them makes this a different key, and treating it
    /// as the same one hands out or deletes material it does not own.
    #[test]
    fn same_key_rejects_a_disagreement_anywhere_else() {
        let stored = IdentityPublicKey::random_key(0, Some(1), PlatformVersion::latest());
        let IdentityPublicKey::V0(mut altered) = stored.clone();
        altered.read_only = !altered.read_only;

        assert!(
            !same_key(&stored, &IdentityPublicKey::V0(altered)),
            "a key differing outside disabled_at is not this key"
        );
    }

    /// A storage filing one key under several placements, each with the given
    /// stored data.
    fn filed_under(
        key: &IdentityPublicKey,
        placements: &[(PrivateKeyTarget, PrivateKeyData)],
    ) -> KeyStorage {
        let mut ks = KeyStorage::default();
        for (target, data) in placements {
            ks.insert_at(
                (target.clone(), key.id()),
                (QualifiedIdentityPublicKey::from(key.clone()), data.clone()),
            );
        }
        ks
    }

    fn derivation_path(seed_hash: u8) -> WalletDerivationPath {
        WalletDerivationPath {
            wallet_seed_hash: [seed_hash; 32],
            derivation_path: DerivationPath::from(vec![]),
        }
    }

    /// A vault placeholder probed first must not hide resident material for the
    /// same key under a later placement — a screen that named the placeholder
    /// would show a key it holds as unusable, and would send the vault a label
    /// its bytes were never stored under.
    #[test]
    fn a_resident_placement_wins_over_a_vault_placeholder() {
        let key = IdentityPublicKey::random_key(0, Some(1), PlatformVersion::latest());
        let ks = filed_under(
            &key,
            &[
                (MAIN, PrivateKeyData::InVault),
                (VOTER, PrivateKeyData::Clear([0x33; 32])),
            ],
        );

        assert_eq!(
            ks.first_live_candidate(&key),
            Some((VOTER, key.id())),
            "the resident placement is the one to name"
        );
        assert!(
            matches!(
                ks.held_private_key_data(&key),
                Some((PrivateKeyData::Clear(bytes), None)) if bytes == [0x33; 32]
            ),
            "the material comes from the placement that was named"
        );
    }

    /// With one placement there is nothing to prefer: it is named whether its
    /// bytes are resident or in the vault, so a vault-backed key stays usable.
    #[test]
    fn a_lone_placement_is_named_whatever_it_holds() {
        let key = IdentityPublicKey::random_key(0, Some(1), PlatformVersion::latest());

        for data in [PrivateKeyData::InVault, PrivateKeyData::Clear([0x44; 32])] {
            let ks = filed_under(&key, &[(MAIN, data.clone())]);
            assert_eq!(
                ks.first_live_candidate(&key),
                Some((MAIN, key.id())),
                "a lone {data:?} placement is still the answer"
            );
        }
    }

    /// A key nothing is filed for has no placement to name and no material.
    #[test]
    fn an_unheld_key_has_no_placement_and_no_material() {
        let key = IdentityPublicKey::random_key(0, Some(1), PlatformVersion::latest());
        let ks = KeyStorage::default();

        assert_eq!(ks.first_live_candidate(&key), None);
        assert!(ks.held_private_key_data(&key).is_none());
    }

    /// `wallet_derived_at` asks a typed question, not a liveness one: a key
    /// whose first placement carries its own plaintext still finds the wallet
    /// it is derived from under a later placement.
    #[test]
    fn wallet_derived_at_looks_past_a_placement_that_is_not_derived() {
        let key = IdentityPublicKey::random_key(0, Some(1), PlatformVersion::latest());
        let ks = filed_under(
            &key,
            &[
                (MAIN, PrivateKeyData::Clear([0x55; 32])),
                (
                    VOTER,
                    PrivateKeyData::AtWalletDerivationPath(derivation_path(0x66)),
                ),
            ],
        );

        assert_eq!(
            ks.wallet_derived_at(&key).map(|path| path.wallet_seed_hash),
            Some([0x66; 32]),
            "the wallet is found under the placement that names it"
        );

        let plaintext_only = filed_under(&key, &[(MAIN, PrivateKeyData::Clear([0x55; 32]))]);
        assert!(
            plaintext_only.wallet_derived_at(&key).is_none(),
            "a key no placement derives has no wallet"
        );
    }

    /// `is_in_vault` and `public_key_for` probes: a vault placeholder reports
    /// `true` and still surfaces its public key; a plaintext key reports
    /// `false`.
    #[test]
    fn in_vault_and_public_key_probes() {
        let mut ks = storage_with_plaintext_and_derived([0x01; 32], [0x02; 32]);
        let keys: Vec<_> = ks.private_keys.keys().cloned().collect();
        ks.take_plaintext_for_vault();
        // The two plaintext keys are now InVault; the derived one is not.
        let mut in_vault_count = 0;
        for k in &keys {
            assert!(
                ks.public_key_for(k).is_some(),
                "public key always available"
            );
            if ks.is_in_vault(k) {
                in_vault_count += 1;
            }
        }
        assert_eq!(in_vault_count, 2);
    }
}
