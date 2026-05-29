//! Cold-boot wallet rehydration (T-W-01).
//!
//! Before T-W-01, the in-memory `BTreeMap<WalletSeedHash, Wallet>` was
//! populated from the legacy DET `wallet` SQLite table. After T-W-00
//! (alias / `is_main` / `core_wallet_name` / `xpub_encoded` sidecar) and
//! T-W-00.5-v2 (encrypted-seed envelope inside the upstream
//! [`SecretStore`]), the same map is reconstructed from those two
//! sidecars instead — the legacy table is no longer read.
//!
//! The reconstruction is intentionally cheap: it does NOT touch the
//! `wallet_addresses` table, derive any addresses, or unlock the seed.
//! The wallet starts in `Closed` state when the envelope is
//! password-protected; address bootstrap and identity discovery happen
//! later, through the same paths a freshly-imported wallet uses
//! ([`AppContext::bootstrap_wallet_addresses`] /
//! [`AppContext::handle_wallet_unlocked`]).
//!
//! Locking caveat: every error path here is a "skip" — a single corrupt
//! row must not block the picker from listing the remaining wallets.
//! The migration banner is the recovery surface for full-vault failure;
//! per-row failure is logged and swallowed.

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey;

use crate::backend_task::error::TaskError;
use crate::model::wallet::meta::WalletMeta;
use crate::model::wallet::seed_envelope::StoredSeedEnvelope;
use crate::model::wallet::{ClosedKeyItem, OpenWalletSeed, Wallet, WalletSeed, WalletSeedHash};
use std::collections::{BTreeMap, HashMap};

use super::WalletBackend;

impl WalletBackend {
    /// Rebuild the in-memory `BTreeMap<WalletSeedHash, Wallet>` for one
    /// network from the sidecars. Returns `(seed_hash, Wallet)` pairs in
    /// `WalletMeta::list` order (base58 seed-hash ascending).
    ///
    /// Wallets whose seed envelope is missing are skipped (the metadata
    /// is orphaned; the picker has nothing to unlock). Wallets whose
    /// `xpub_encoded` is empty or fails to decode are also skipped — the
    /// picker cannot render addresses without a valid xpub.
    ///
    /// Per-wallet errors are logged and skipped; the function only
    /// surfaces a `TaskError` when the sidecar accessor itself is wedged
    /// (e.g. seed-store I/O failure). All other classes degrade
    /// gracefully so the wallet picker can show whatever does parse.
    pub fn hydrate_wallets_for_network(
        &self,
        network: Network,
    ) -> Result<Vec<(WalletSeedHash, Wallet)>, TaskError> {
        let meta_view = self.wallet_meta();
        let seed_view = self.wallet_seeds();

        let entries = meta_view.list(network);
        let mut out = Vec::with_capacity(entries.len());
        for (seed_hash, meta) in entries {
            match reconstruct_wallet(&seed_view, &seed_hash, &meta) {
                Ok(Some(wallet)) => out.push((seed_hash, wallet)),
                Ok(None) => {
                    // Logged inside `reconstruct_wallet` — orphaned meta
                    // or empty xpub is a "skip and continue" path.
                }
                Err(e) => {
                    tracing::warn!(
                        target = "wallet_backend::hydration",
                        seed_hash = %hex::encode(seed_hash),
                        error = ?e,
                        "Failed to reconstruct wallet from sidecars; skipping",
                    );
                }
            }
        }
        Ok(out)
    }
}

/// Reconstruct one `Wallet` from its `(WalletMeta, StoredSeedEnvelope)`
/// pair. Returns `Ok(None)` when the wallet must be skipped (envelope
/// missing, xpub absent or undecodable) — the call site logs and moves
/// on. Errors propagate only when the sidecar accessor itself errored.
fn reconstruct_wallet(
    seed_view: &super::wallet_seed_store::WalletSeedView<'_>,
    seed_hash: &WalletSeedHash,
    meta: &WalletMeta,
) -> Result<Option<Wallet>, TaskError> {
    let envelope = match seed_view.get(seed_hash)? {
        Some(e) => e,
        None => {
            tracing::warn!(
                target = "wallet_backend::hydration",
                seed_hash = %hex::encode(seed_hash),
                "Wallet meta has no matching seed envelope; skipping",
            );
            return Ok(None);
        }
    };

    // Prefer the envelope's xpub (written by T-W-00.5-v2) over the meta
    // one. The meta copy was carried for the cold-boot picker before the
    // envelope path was wired; in practice they are written together.
    let xpub_bytes: &[u8] = if !envelope.xpub_encoded.is_empty() {
        &envelope.xpub_encoded
    } else {
        &meta.xpub_encoded
    };

    if xpub_bytes.is_empty() {
        tracing::warn!(
            target = "wallet_backend::hydration",
            seed_hash = %hex::encode(seed_hash),
            "Wallet entry has no master xpub; skipping",
        );
        return Ok(None);
    }

    let master_bip44_ecdsa_extended_public_key = match ExtendedPubKey::decode(xpub_bytes) {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!(
                target = "wallet_backend::hydration",
                seed_hash = %hex::encode(seed_hash),
                error = ?e,
                "Failed to decode master xpub for wallet; skipping",
            );
            return Ok(None);
        }
    };

    let wallet = wallet_from_envelope(
        *seed_hash,
        envelope,
        meta,
        master_bip44_ecdsa_extended_public_key,
    );
    Ok(Some(wallet))
}

/// Assemble the final `Wallet` from its parts. Mirrors the legacy
/// `db.get_wallets` row → `Wallet` mapping (`encrypted_seed` becomes the
/// plaintext seed when `uses_password = false`; otherwise the closed
/// envelope stays encrypted until the user unlocks it).
fn wallet_from_envelope(
    seed_hash: WalletSeedHash,
    envelope: StoredSeedEnvelope,
    meta: &WalletMeta,
    master_bip44_ecdsa_extended_public_key: ExtendedPubKey,
) -> Wallet {
    let StoredSeedEnvelope {
        encrypted_seed,
        salt,
        nonce,
        password_hint,
        uses_password,
        xpub_encoded: _,
    } = envelope;

    let closed = ClosedKeyItem {
        seed_hash,
        encrypted_seed: encrypted_seed.clone(),
        salt,
        nonce,
        password_hint,
    };

    let wallet_seed = if uses_password {
        WalletSeed::Closed(closed)
    } else {
        // Non-password envelopes store the raw 64-byte seed verbatim;
        // mirror the legacy DB reader behaviour. A length mismatch
        // collapses to `Closed` so the wallet still appears in the
        // picker even if the seed bytes are unusable.
        match encrypted_seed.clone().try_into() {
            Ok(seed) => WalletSeed::Open(OpenWalletSeed {
                seed,
                wallet_info: closed,
            }),
            Err(bytes) => {
                tracing::warn!(
                    target = "wallet_backend::hydration",
                    seed_hash = %hex::encode(seed_hash),
                    blob_len = bytes.len(),
                    "Non-password seed envelope is not 64 bytes; falling back to closed wallet",
                );
                WalletSeed::Closed(closed)
            }
        }
    };

    Wallet {
        wallet_seed,
        uses_password,
        master_bip44_ecdsa_extended_public_key,
        known_addresses: BTreeMap::new(),
        watched_addresses: BTreeMap::new(),
        alias: if meta.alias.is_empty() {
            None
        } else {
            Some(meta.alias.clone())
        },
        identities: HashMap::new(),
        is_main: meta.is_main,
        platform_address_info: BTreeMap::new(),
        core_wallet_name: meta.core_wallet_name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::wallet::{ClosedKeyItem, Wallet};
    use crate::wallet_backend::wallet_seed_store::WalletSeedView;
    use dash_sdk::dpp::dashcore::Network;
    use platform_wallet_storage::secrets::SecretStore;
    use std::sync::Arc;

    /// Build a real BIP44 master pubkey from a seed so the encoded bytes
    /// survive the `ExtendedPubKey::decode` round trip.
    fn xpub_bytes_for(seed: [u8; 64], network: Network) -> Vec<u8> {
        let w = Wallet::new_from_seed(seed, network, None, None).expect("wallet");
        w.master_bip44_ecdsa_extended_public_key.encode().to_vec()
    }

    fn seed_hash_for(seed: [u8; 64]) -> WalletSeedHash {
        ClosedKeyItem::compute_seed_hash(&seed)
    }

    /// TC-W-001 — a wallet whose `WalletMeta` + `StoredSeedEnvelope` sit
    /// in the sidecars is rebuilt verbatim: alias, is_main,
    /// core_wallet_name, master xpub, and the un-encrypted seed all match.
    #[test]
    fn tc_w_001_reconstructs_non_password_wallet_from_sidecars() {
        let seed = [0x42u8; 64];
        let network = Network::Testnet;
        let xpub = xpub_bytes_for(seed, network);

        let envelope = StoredSeedEnvelope {
            encrypted_seed: seed.to_vec(),
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: xpub.clone(),
        };
        let meta = WalletMeta {
            alias: "paycheque".into(),
            is_main: true,
            core_wallet_name: Some("local-dashd".into()),
            xpub_encoded: xpub,
        };

        // Stand-in for `WalletSeedView::get` — direct decode of the
        // envelope, no upstream vault required.
        let master = ExtendedPubKey::decode(&envelope.xpub_encoded).expect("xpub decodes");
        let wallet = wallet_from_envelope(seed_hash_for(seed), envelope, &meta, master);

        assert_eq!(wallet.alias.as_deref(), Some("paycheque"));
        assert!(wallet.is_main);
        assert_eq!(wallet.core_wallet_name.as_deref(), Some("local-dashd"));
        assert!(
            wallet.is_open(),
            "non-password envelope must rehydrate open"
        );
        assert_eq!(wallet.seed_hash(), seed_hash_for(seed));
    }

    /// TC-W-009 — a password-protected wallet's alias and `is_main`
    /// survive reconstruction without unlocking; the wallet stays closed
    /// so the unlock UI still has work to do.
    #[test]
    fn tc_w_009_password_wallet_metadata_preserved_locked() {
        let seed = [0x77u8; 64];
        let network = Network::Mainnet;
        let xpub = xpub_bytes_for(seed, network);

        let envelope = StoredSeedEnvelope {
            encrypted_seed: vec![0xAB; 80],
            salt: vec![0x01; 16],
            nonce: vec![0x02; 12],
            password_hint: Some("granny's birthday".into()),
            uses_password: true,
            xpub_encoded: xpub.clone(),
        };
        let meta = WalletMeta {
            alias: "savings".into(),
            is_main: false,
            core_wallet_name: None,
            xpub_encoded: xpub,
        };

        let master = ExtendedPubKey::decode(&envelope.xpub_encoded).expect("xpub decodes");
        let wallet = wallet_from_envelope(seed_hash_for(seed), envelope, &meta, master);

        assert_eq!(wallet.alias.as_deref(), Some("savings"));
        assert!(!wallet.is_main);
        assert!(!wallet.is_open(), "password envelope must stay closed");
        assert!(wallet.uses_password);
        assert_eq!(wallet.password_hint().as_deref(), Some("granny's birthday"));
    }

    /// Empty alias on `WalletMeta` (the "fresh install, never named"
    /// shape from the migration writer) maps back to `None` — matches
    /// the legacy `Option<String>` column shape that downstream code
    /// branches on.
    #[test]
    fn empty_alias_maps_to_none() {
        let seed = [0xDDu8; 64];
        let xpub = xpub_bytes_for(seed, Network::Testnet);
        let envelope = StoredSeedEnvelope {
            encrypted_seed: seed.to_vec(),
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: xpub.clone(),
        };
        let meta = WalletMeta {
            alias: String::new(),
            is_main: false,
            core_wallet_name: None,
            xpub_encoded: xpub,
        };
        let master = ExtendedPubKey::decode(&envelope.xpub_encoded).expect("xpub decodes");
        let wallet = wallet_from_envelope(seed_hash_for(seed), envelope, &meta, master);
        assert!(wallet.alias.is_none());
    }

    fn fresh_secret_store(dir: &std::path::Path) -> Arc<SecretStore> {
        let path = dir.join("secrets.pwsvault");
        Arc::new(crate::wallet_backend::single_key::open_secret_store(&path).expect("open vault"))
    }

    /// TC-W-001 (sidecar round-trip) — `reconstruct_wallet` returns a
    /// `Wallet` with the same shape as the per-field assembly path,
    /// when fed a `WalletSeedView` that read what the migration writer
    /// would have produced.
    #[test]
    fn tc_w_001_round_trip_through_real_seed_view() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = fresh_secret_store(dir.path());
        let view = WalletSeedView::new(&store);

        let seed = [0xABu8; 64];
        let network = Network::Testnet;
        let xpub = xpub_bytes_for(seed, network);
        let envelope = StoredSeedEnvelope {
            encrypted_seed: seed.to_vec(),
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: xpub.clone(),
        };
        let hash = seed_hash_for(seed);
        view.set(&hash, &envelope).expect("set");

        let meta = WalletMeta {
            alias: "primary".into(),
            is_main: true,
            core_wallet_name: None,
            xpub_encoded: xpub,
        };

        let wallet = reconstruct_wallet(&view, &hash, &meta)
            .expect("no error")
            .expect("rebuilt");
        assert_eq!(wallet.alias.as_deref(), Some("primary"));
        assert!(wallet.is_main);
        assert!(wallet.is_open());
        assert_eq!(wallet.seed_hash(), hash);
    }

    /// Orphan path — a `WalletMeta` entry whose envelope is missing is
    /// returned as `Ok(None)` from `reconstruct_wallet` so the picker
    /// can keep listing the survivors.
    #[test]
    fn orphan_meta_without_envelope_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = fresh_secret_store(dir.path());
        let view = WalletSeedView::new(&store);

        let seed = [0xCDu8; 64];
        let xpub = xpub_bytes_for(seed, Network::Testnet);
        let meta = WalletMeta {
            alias: "orphan".into(),
            is_main: false,
            core_wallet_name: None,
            xpub_encoded: xpub,
        };
        let result = reconstruct_wallet(&view, &seed_hash_for(seed), &meta).expect("no error");
        assert!(result.is_none(), "missing envelope must collapse to None");
    }

    /// Empty xpub (legacy entry written before T-W-00.5) collapses to
    /// `None` — the picker has nothing to derive from.
    #[test]
    fn empty_xpub_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = fresh_secret_store(dir.path());
        let view = WalletSeedView::new(&store);

        let seed = [0xEEu8; 64];
        let hash = seed_hash_for(seed);
        let envelope = StoredSeedEnvelope {
            encrypted_seed: seed.to_vec(),
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: Vec::new(),
        };
        view.set(&hash, &envelope).expect("set");
        let meta = WalletMeta {
            alias: "no-xpub".into(),
            is_main: false,
            core_wallet_name: None,
            xpub_encoded: Vec::new(),
        };
        let result = reconstruct_wallet(&view, &hash, &meta).expect("no error");
        assert!(result.is_none(), "empty xpub must collapse to None");
    }

    /// TC-W-008 (rename-shape half) — assigning a new alias to a
    /// reconstructed wallet preserves seed-hash / xpub / is_main so the
    /// only observable diff after the rename is the alias itself.
    /// Locks the "rename does not invalidate the wallet" invariant.
    #[test]
    fn tc_w_008_rename_only_touches_alias() {
        let seed = [0x55u8; 64];
        let xpub = xpub_bytes_for(seed, Network::Testnet);
        let envelope = StoredSeedEnvelope {
            encrypted_seed: seed.to_vec(),
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: xpub.clone(),
        };
        let meta = WalletMeta {
            alias: "old".into(),
            is_main: true,
            core_wallet_name: None,
            xpub_encoded: xpub.clone(),
        };
        let master = ExtendedPubKey::decode(&xpub).expect("xpub decodes");
        let mut wallet = wallet_from_envelope(seed_hash_for(seed), envelope, &meta, master);
        let original_hash = wallet.seed_hash();
        let original_xpub = wallet.master_bip44_ecdsa_extended_public_key.encode();

        wallet.alias = Some("new".to_string());

        assert_eq!(wallet.seed_hash(), original_hash);
        assert_eq!(
            wallet.master_bip44_ecdsa_extended_public_key.encode(),
            original_xpub
        );
        assert!(wallet.is_main);
        assert_eq!(wallet.alias.as_deref(), Some("new"));
    }
}
