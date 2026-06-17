//! Cache-hit-else-JIT resolution of identity-authentication public keys (D4b).
//!
//! The identity-auth derivation path is hardened to the leaf, so its
//! public keys cannot be derived from a stored xpub; instead they are
//! memoised in the DET KV sidecar (see
//! [`AuthPubkeyCache`](crate::model::wallet::auth_pubkey_cache::AuthPubkeyCache)).
//! These helpers are the single place the identity load/discover tasks go
//! through to read those keys: a warm cache serves them with zero seed
//! access; a cold miss resolves the seed *once* through the
//! [`SecretAccess`](crate::wallet_backend::SecretAccess) chokepoint,
//! derives the missing keys, writes them back, and returns them.
//!
//! Correctness never depends on the cache being present — an absent or
//! corrupt blob reads as cold and self-heals on first touch.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::{Arc, RwLock};

use dash_sdk::dpp::dashcore::PublicKey;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::wallet_backend::SecretScope;

/// The two identity-auth lookup maps a data-map resolution returns:
/// compressed-pubkey-bytes -> key_index and hash160 -> key_index.
type AuthPubkeyDataMaps = (BTreeMap<Vec<u8>, u32>, BTreeMap<[u8; 20], u32>);

impl AppContext {
    /// Resolve one identity-auth ECDSA public key, cache-first.
    ///
    /// Reads the wallet's [`AuthPubkeyCache`](crate::model::wallet::auth_pubkey_cache::AuthPubkeyCache)
    /// for `(network, identity_index, key_index)`; on a hit returns it with
    /// no seed access. On a miss, opens one `with_secret` scope, derives
    /// the key from the seed, writes it back to the cache, and returns it.
    ///
    /// `allow_prompt` controls the cold-cache path: when `false`, a miss on a
    /// passphrase-protected wallet that is not session-unlocked returns
    /// [`TaskError::AuthKeyUnlockRequired`] instead of opening a `with_secret`
    /// scope — so the background sweep skips a locked wallet rather than popping
    /// a passphrase modal. The interactive search passes `true`.
    pub(super) async fn resolve_identity_auth_pubkey(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
        allow_prompt: bool,
        identity_index: u32,
        key_index: u32,
    ) -> Result<PublicKey, TaskError> {
        let network = self.network;
        let seed_hash = wallet.read()?.seed_hash();

        let backend = self.wallet_backend()?;
        let cache = backend.auth_pubkey_cache().get(network, &seed_hash);

        if let Some(public_key) = wallet
            .read()?
            .identity_authentication_ecdsa_public_key_cached(
                &cache,
                network,
                identity_index,
                key_index,
            )
        {
            return Ok(public_key);
        }

        if !allow_prompt
            && !backend
                .secret_access()
                .can_resolve_without_prompt(&SecretScope::HdSeed { seed_hash })
        {
            return Err(TaskError::AuthKeyUnlockRequired);
        }

        let wallet = Arc::clone(wallet);
        backend
            .secret_access()
            .with_secret(&SecretScope::HdSeed { seed_hash }, |plaintext| {
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::ContactWalletSeedUnavailable)?;
                let public_key = wallet
                    .read()?
                    .identity_authentication_ecdsa_public_key_from_seed(
                        seed,
                        network,
                        identity_index,
                        key_index,
                    )
                    .map_err(|detail| {
                        tracing::warn!(error = %detail, "identity-auth key derivation failed");
                        TaskError::WalletAddressDerivationFailed
                    })?;

                let mut cache = cache;
                if cache.insert(network, identity_index, key_index, &public_key) {
                    backend
                        .auth_pubkey_cache()
                        .put(network, &seed_hash, &cache)?;
                }
                Ok(public_key)
            })
            .await
    }

    /// Resolve the two identity-auth lookup maps for `key_index_range`,
    /// cache-first, partitioning all cache misses into a single
    /// `with_secret` scope (one prompt for the whole request, not one per
    /// key).
    ///
    /// `register_addresses` mirrors the legacy data-map flag: when set,
    /// each key's P2PKH address is registered on the wallet regardless of
    /// whether the key came from the cache or a cold derivation.
    ///
    /// `allow_prompt` controls the cold-cache path exactly as in
    /// [`Self::resolve_identity_auth_pubkey`]: when `false`, any cache miss on a
    /// passphrase-protected wallet that is not session-unlocked returns
    /// [`TaskError::AuthKeyUnlockRequired`] before opening a `with_secret`
    /// scope, so the background sweep never triggers a passphrase modal.
    pub(super) async fn resolve_identity_auth_pubkeys_data_map(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
        register_addresses: bool,
        allow_prompt: bool,
        identity_index: u32,
        key_index_range: Range<u32>,
    ) -> Result<AuthPubkeyDataMaps, TaskError> {
        let network = self.network;
        let seed_hash = wallet.read()?.seed_hash();

        let backend = self.wallet_backend()?;
        let cache = backend.auth_pubkey_cache().get(network, &seed_hash);

        let (mut public_key_map, mut public_key_hash_map, misses) = {
            let mut guard = wallet.write()?;
            guard
                .identity_authentication_ecdsa_public_keys_data_map_cached(
                    self,
                    register_addresses,
                    &cache,
                    network,
                    identity_index,
                    key_index_range,
                )
                .map_err(|detail| {
                    tracing::warn!(error = %detail, "identity-auth key map derivation failed");
                    TaskError::WalletAddressDerivationFailed
                })?
        };

        if misses.is_empty() {
            return Ok((public_key_map, public_key_hash_map));
        }

        if !allow_prompt
            && !backend
                .secret_access()
                .can_resolve_without_prompt(&SecretScope::HdSeed { seed_hash })
        {
            return Err(TaskError::AuthKeyUnlockRequired);
        }

        let wallet = Arc::clone(wallet);
        backend
            .secret_access()
            .with_secret(&SecretScope::HdSeed { seed_hash }, |plaintext| {
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::ContactWalletSeedUnavailable)?;
                let (miss_key_map, miss_hash_map, derived) = {
                    let mut guard = wallet.write()?;
                    guard
                        .identity_authentication_ecdsa_public_keys_data_map_from_seed(
                            self,
                            register_addresses,
                            seed,
                            network,
                            identity_index,
                            &misses,
                        )
                        .map_err(|detail| {
                            tracing::warn!(error = %detail, "identity-auth key map derivation failed");
                            TaskError::WalletAddressDerivationFailed
                        })?
                };
                public_key_map.extend(miss_key_map);
                public_key_hash_map.extend(miss_hash_map);

                if !derived.is_empty() {
                    let mut cache = cache;
                    let mut changed = false;
                    for (key_index, public_key) in &derived {
                        changed |= cache.insert(network, identity_index, *key_index, public_key);
                    }
                    if changed {
                        backend
                            .auth_pubkey_cache()
                            .put(network, &seed_hash, &cache)?;
                    }
                }
                Ok((public_key_map, public_key_hash_map))
            })
            .await
    }
}
