//! Cold-boot wallet rehydration (T-W-01).
//!
//! The in-memory `BTreeMap<WalletSeedHash, Wallet>` is reconstructed from the
//! wallet-meta sidecar (alias / `is_main` / `core_wallet_name` /
//! `xpub_encoded`) and the encrypted-seed envelope inside the upstream
//! `SecretStore`.
//!
//! The reconstruction is intentionally cheap: it does NOT touch the
//! `wallet_addresses` table, derive any addresses, or unlock the seed.
//! The wallet starts in `Closed` state when the envelope is
//! password-protected; address bootstrap and identity discovery happen
//! later, through the same paths a freshly-imported wallet uses
//! (`AppContext::bootstrap_wallet_addresses` /
//! `AppContext::handle_wallet_unlocked`).
//!
//! Per-row failure is logged and swallowed: a single corrupt row must not
//! block the picker from listing the remaining wallets. These skips are
//! log-only — there is no reachable egui context here, so the skip reason
//! does not surface to the user. The migration banner is the recovery
//! surface for a full-vault failure.

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey;

use crate::backend_task::error::TaskError;
use crate::model::wallet::meta::{WalletMeta, wallet_label};
use crate::model::wallet::seed_envelope::StoredSeedEnvelope;
use crate::model::wallet::{ClosedKeyItem, OpenWalletSeed, Wallet, WalletSeed, WalletSeedHash};
use crate::wallet_backend::secret_seam::SecretScheme;
use std::collections::{BTreeMap, HashMap};
#[cfg(test)]
use zeroize::Zeroizing;

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
        hydrate_hd_wallets_from_views(&self.wallet_seeds(), &self.wallet_meta(), network)
    }
}

/// Free-function version of [`WalletBackend::hydrate_wallets_for_network`]
/// driven by borrowed views. Exposed so benches and integration tests can
/// time cold-start hydration without standing up a full backend (no SDK,
/// no `PlatformWalletManager`, no per-network sync dir).
pub fn hydrate_hd_wallets_from_views(
    seed_view: &super::wallet_seed_store::WalletSeedView<'_>,
    meta_view: &super::wallet_meta::WalletMetaView<'_>,
    network: Network,
) -> Result<Vec<(WalletSeedHash, Wallet)>, TaskError> {
    let entries = meta_view.list(network);
    let mut out = Vec::with_capacity(entries.len());
    for (seed_hash, meta) in entries {
        match reconstruct_wallet(seed_view, &seed_hash, &meta) {
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

/// Reconstruct one `Wallet` from its `(WalletMeta, StoredSeedEnvelope)`
/// pair. Returns `Ok(None)` when the wallet must be skipped (envelope
/// missing, xpub absent or undecodable) — the call site logs and moves
/// on. Errors propagate only when the sidecar accessor itself errored.
fn reconstruct_wallet(
    seed_view: &super::wallet_seed_store::WalletSeedView<'_>,
    seed_hash: &WalletSeedHash,
    meta: &WalletMeta,
) -> Result<Option<Wallet>, TaskError> {
    // Branch on the raw-seam at-rest scheme BEFORE reading the seed. A Tier-2
    // protected seed is intentionally unreadable at cold boot (the object
    // password is not available without a prompt), so probing the scheme first
    // keeps a `get_raw` (which would surface `NeedsPassword`) off the protected
    // path and lets the wallet still render closed.
    match seed_view.scheme(seed_hash)? {
        // Tier-2 protected: reconstruct a closed (watch-only) wallet from the
        // public master xpub in `WalletMeta` — never read the seed. The unlock
        // gesture later supplies the password through the JIT chokepoint.
        SecretScheme::Protected => {
            seed_view.delete_legacy_best_effort(seed_hash);
            let envelope = StoredSeedEnvelope {
                encrypted_seed: zeroize::Zeroizing::new(Vec::new()),
                salt: Vec::new(),
                nonce: Vec::new(),
                password_hint: meta.password_hint.clone(),
                uses_password: true,
                xpub_encoded: meta.xpub_encoded.clone(),
            };
            return reconstruct_from_envelope(seed_hash, envelope, meta);
        }
        // Tier-1 raw seed present (precedence raw > legacy). A migrated
        // no-password wallet has no envelope — its seed rides raw under
        // `seed.raw.v1` and its non-secret metadata (xpub) lives in `WalletMeta`.
        SecretScheme::Unprotected => {
            seed_view.delete_legacy_best_effort(seed_hash);
            let raw = seed_view
                .get_raw(seed_hash)?
                .ok_or(TaskError::SecretSeamMissing)?;
            let envelope = StoredSeedEnvelope {
                encrypted_seed: zeroize::Zeroizing::new(raw.to_vec()),
                salt: Vec::new(),
                nonce: Vec::new(),
                password_hint: meta.password_hint.clone(),
                uses_password: false,
                xpub_encoded: meta.xpub_encoded.clone(),
            };
            return reconstruct_from_envelope(seed_hash, envelope, meta);
        }
        // No raw value yet — fall through to the legacy `envelope.v1` reader and
        // its eager/lazy migration below.
        SecretScheme::Absent => {}
    }

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

    // EAGER migration (dialog-free): a no-password legacy envelope holds the
    // raw seed verbatim. Re-store it raw and garbage-collect the redundant
    // envelope. `set_raw` is idempotent, and the raw form is preferred on the
    // next load. A password envelope is left for the lazy unlock update.
    if !envelope.uses_password
        && envelope.encrypted_seed.len() == EXPECTED_SEED_LEN as usize
        && let Ok(seed) = <[u8; 64]>::try_from(envelope.encrypted_seed.as_slice())
    {
        // Keep the extracted raw seed in `Zeroizing` so the stack copy wipes on
        // drop, matching every other raw-seed site introduced by this work.
        let seed = zeroize::Zeroizing::new(seed);
        if let Err(e) = seed_view.set_raw(seed_hash, &seed) {
            tracing::warn!(
                target = "wallet_backend::hydration",
                seed_hash = %hex::encode(seed_hash),
                error = ?e,
                "Eager no-password seed migration deferred (raw write failed)",
            );
        }
    }

    reconstruct_from_envelope(seed_hash, envelope, meta)
}

/// Decode the master xpub (envelope copy preferred, `WalletMeta` fallback) and
/// assemble the `Wallet`. Shared by the raw-seam and legacy-envelope paths in
/// [`reconstruct_wallet`]. `Ok(None)` (skip + log) when the xpub is absent or
/// undecodable.
fn reconstruct_from_envelope(
    seed_hash: &WalletSeedHash,
    envelope: StoredSeedEnvelope,
    meta: &WalletMeta,
) -> Result<Option<Wallet>, TaskError> {
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
    )?;
    Ok(Some(wallet))
}

/// Expected plaintext length of a BIP-39 seed (BIP39 mnemonics produce a
/// 64-byte seed via PBKDF2). Surfaced as a constant so error variants
/// can speak in concrete numbers.
const EXPECTED_SEED_LEN: u32 = 64;

/// Assemble the final `Wallet` from its parts. Mirrors the legacy
/// `db.get_wallets` row → `Wallet` mapping (`encrypted_seed` becomes the
/// plaintext seed when `uses_password = false`; otherwise the closed
/// envelope stays encrypted until the user unlocks it).
fn wallet_from_envelope(
    seed_hash: WalletSeedHash,
    envelope: StoredSeedEnvelope,
    meta: &WalletMeta,
    master_bip44_ecdsa_extended_public_key: ExtendedPubKey,
) -> Result<Wallet, TaskError> {
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
        // Non-password envelopes store the raw 64-byte seed verbatim. A
        // length mismatch is a corrupt envelope: surface a typed error
        // carrying the wallet label and observed length. The caller logs
        // and skips it (log-only — no egui context is reachable here). The
        // envelope is only length-checked to prove it is well-formed; the
        // open wallet parks no plaintext seed (R3).
        if encrypted_seed.len() != EXPECTED_SEED_LEN as usize {
            return Err(TaskError::SeedLengthInvalid {
                wallet_label: wallet_label(&meta.alias, &seed_hash),
                got: encrypted_seed.len() as u32,
                expected: EXPECTED_SEED_LEN,
            });
        }
        WalletSeed::Open(OpenWalletSeed {
            wallet_info: closed,
        })
    };

    Ok(Wallet {
        wallet_seed,
        uses_password,
        master_bip44_ecdsa_extended_public_key,
        // Not persisted: re-derived just-in-time from the seed on first unlock
        // (see `Wallet::ensure_platform_payment_account_xpub`).
        platform_payment_account_xpub: None,
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::wallet::{ClosedKeyItem, Wallet};
    use crate::wallet_backend::kv_test_support::InMemoryKv;
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
            encrypted_seed: Zeroizing::new(seed.to_vec()),
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
            uses_password: false,
            password_hint: None,
        };

        // Stand-in for `WalletSeedView::get` — direct decode of the
        // envelope, no upstream vault required.
        let master = ExtendedPubKey::decode(&envelope.xpub_encoded).expect("xpub decodes");
        let wallet = wallet_from_envelope(seed_hash_for(seed), envelope, &meta, master)
            .expect("wallet rebuilt");

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
            encrypted_seed: Zeroizing::new(vec![0xAB; 80]),
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
            uses_password: false,
            password_hint: None,
        };

        let master = ExtendedPubKey::decode(&envelope.xpub_encoded).expect("xpub decodes");
        let wallet = wallet_from_envelope(seed_hash_for(seed), envelope, &meta, master)
            .expect("wallet rebuilt");

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
            encrypted_seed: Zeroizing::new(seed.to_vec()),
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
            uses_password: false,
            password_hint: None,
        };
        let master = ExtendedPubKey::decode(&envelope.xpub_encoded).expect("xpub decodes");
        let wallet = wallet_from_envelope(seed_hash_for(seed), envelope, &meta, master)
            .expect("wallet rebuilt");
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
            encrypted_seed: Zeroizing::new(seed.to_vec()),
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
            uses_password: false,
            password_hint: None,
        };

        let wallet = reconstruct_wallet(&view, &hash, &meta)
            .expect("no error")
            .expect("rebuilt");
        assert_eq!(wallet.alias.as_deref(), Some("primary"));
        assert!(wallet.is_main);
        assert!(wallet.is_open());
        assert_eq!(wallet.seed_hash(), hash);
    }

    /// TS-EAGER-01 / TS-EAGER-04 — a no-password legacy envelope is eagerly
    /// copied on load: the raw `seed.raw.v1` is written, the redundant legacy
    /// envelope is deleted, and a reload reads via the raw seam. Running the
    /// load twice is idempotent.
    #[test]
    fn ts_eager_01_no_password_seed_migrates_on_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = fresh_secret_store(dir.path());
        let view = WalletSeedView::new(&store);

        let seed = [0x5Au8; 64];
        let network = Network::Testnet;
        let xpub = xpub_bytes_for(seed, network);
        let hash = seed_hash_for(seed);
        view.set(
            &hash,
            &StoredSeedEnvelope {
                encrypted_seed: Zeroizing::new(seed.to_vec()),
                salt: Vec::new(),
                nonce: Vec::new(),
                password_hint: None,
                uses_password: false,
                xpub_encoded: xpub.clone(),
            },
        )
        .expect("seed legacy envelope");
        let meta = WalletMeta {
            alias: "eager".into(),
            is_main: false,
            core_wallet_name: None,
            xpub_encoded: xpub,
            uses_password: false,
            password_hint: None,
        };

        // First load migrates.
        let wallet = reconstruct_wallet(&view, &hash, &meta)
            .expect("no error")
            .expect("rebuilt");
        assert!(wallet.is_open());
        // Raw present and equals the seed; redundant envelope removed.
        assert_eq!(*view.get_raw(&hash).unwrap().unwrap(), seed);
        assert!(
            view.legacy_envelope_get(&hash).unwrap().is_none(),
            "a no-password seed must have exactly one vault copy after eager migration"
        );

        // Second load is idempotent — reads via the raw seam, no error.
        let wallet2 = reconstruct_wallet(&view, &hash, &meta)
            .expect("no error")
            .expect("rebuilt again");
        assert!(wallet2.is_open());
        assert_eq!(*view.get_raw(&hash).unwrap().unwrap(), seed);
        assert!(view.legacy_envelope_get(&hash).unwrap().is_none());
    }

    /// TS-CRASH-01 (read half) — the legal mid-migration state (raw present
    /// AND legacy still present) loads from the RAW value and finishes legacy
    /// garbage collection. No key loss, no error.
    #[test]
    fn ts_crash_01_raw_wins_and_legacy_is_collected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = fresh_secret_store(dir.path());
        let view = WalletSeedView::new(&store);

        let seed = [0x6Bu8; 64];
        let network = Network::Testnet;
        let xpub = xpub_bytes_for(seed, network);
        let hash = seed_hash_for(seed);
        // Both forms present by design.
        view.set_raw(&hash, &seed).expect("raw");
        view.set(
            &hash,
            &StoredSeedEnvelope {
                encrypted_seed: Zeroizing::new(seed.to_vec()),
                salt: Vec::new(),
                nonce: Vec::new(),
                password_hint: None,
                uses_password: false,
                xpub_encoded: xpub.clone(),
            },
        )
        .expect("legacy too");
        let meta = WalletMeta {
            alias: "midmig".into(),
            is_main: false,
            core_wallet_name: None,
            xpub_encoded: xpub,
            uses_password: false,
            password_hint: None,
        };

        let wallet = reconstruct_wallet(&view, &hash, &meta)
            .expect("no error")
            .expect("rebuilt");
        assert!(wallet.is_open());
        assert_eq!(*view.get_raw(&hash).unwrap().unwrap(), seed);
        assert!(view.legacy_envelope_get(&hash).unwrap().is_none());
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
            uses_password: false,
            password_hint: None,
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
            encrypted_seed: Zeroizing::new(seed.to_vec()),
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
            uses_password: false,
            password_hint: None,
        };
        let result = reconstruct_wallet(&view, &hash, &meta).expect("no error");
        assert!(result.is_none(), "empty xpub must collapse to None");
    }

    /// A non-password envelope whose `encrypted_seed` is not 64 bytes
    /// surfaces [`TaskError::SeedLengthInvalid`] with the alias-as-label
    /// and the observed length, instead of silently degrading to a
    /// closed wallet.
    #[test]
    fn sec_008_non_64_byte_seed_surfaces_typed_error() {
        let seed = [0xBEu8; 64];
        let xpub = xpub_bytes_for(seed, Network::Testnet);
        let envelope = StoredSeedEnvelope {
            encrypted_seed: Zeroizing::new(vec![0x33; 16]), // wrong length on purpose
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: xpub.clone(),
        };
        let meta = WalletMeta {
            alias: "shorty".into(),
            is_main: false,
            core_wallet_name: None,
            xpub_encoded: xpub.clone(),
            uses_password: false,
            password_hint: None,
        };
        let master = ExtendedPubKey::decode(&xpub).expect("xpub decodes");
        let err = wallet_from_envelope(seed_hash_for(seed), envelope, &meta, master)
            .expect_err("typed error");
        match err {
            TaskError::SeedLengthInvalid {
                wallet_label,
                got,
                expected,
            } => {
                assert_eq!(wallet_label, "shorty");
                assert_eq!(got, 16);
                assert_eq!(expected, 64);
            }
            other => panic!("expected SeedLengthInvalid, got {other:?}"),
        }
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
            encrypted_seed: Zeroizing::new(seed.to_vec()),
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
            uses_password: false,
            password_hint: None,
        };
        let master = ExtendedPubKey::decode(&xpub).expect("xpub decodes");
        let mut wallet = wallet_from_envelope(seed_hash_for(seed), envelope, &meta, master)
            .expect("wallet rebuilt");
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

    /// Regression for the cold-boot disappearance of a Tier-2-protected HD
    /// seed: a seed re-wrapped under its own object password (keep-protection)
    /// must rehydrate as a CLOSED wallet, not be skipped. Before the
    /// scheme-first branch, `reconstruct_wallet` called `get_raw` on the
    /// protected label, which surfaced `NeedsPassword`; the `?` then dropped the
    /// wallet from the picker on every launch.
    #[test]
    fn tier2_protected_seed_reconstructs_closed_and_is_listed() {
        use crate::wallet_backend::wallet_meta::WalletMetaView;
        use platform_wallet_storage::secrets::SecretString;

        let dir = tempfile::tempdir().expect("tempdir");
        let store = fresh_secret_store(dir.path());
        let view = WalletSeedView::new(&store);

        let seed = [0x91u8; 64];
        let network = Network::Testnet;
        let xpub = xpub_bytes_for(seed, network);
        let hash = seed_hash_for(seed);

        // Keep-protection storage-update shape: the seed lives Tier-2 under its
        // own object password at `seed.raw.v1`.
        let password = SecretString::new("correct-horse-battery");
        view.set_protected(&hash, &seed, &password)
            .expect("set_protected");
        assert_eq!(
            view.scheme(&hash).expect("scheme"),
            SecretScheme::Protected,
            "seed must read back as Tier-2 protected without a password",
        );

        let meta = WalletMeta {
            alias: "savings".into(),
            is_main: false,
            core_wallet_name: None,
            xpub_encoded: xpub,
            uses_password: true,
            password_hint: Some("the usual".into()),
        };

        // Direct reconstruction returns Ok(Some(closed)) — never an Err.
        let wallet = reconstruct_wallet(&view, &hash, &meta)
            .expect("no error from a protected seed")
            .expect("protected wallet must rehydrate, not be skipped");
        assert!(!wallet.is_open(), "protected seed must rehydrate closed");
        assert!(wallet.uses_password);
        assert_eq!(wallet.seed_hash(), hash);
        assert_eq!(wallet.password_hint().as_deref(), Some("the usual"));

        // And it appears in the cold-boot enumeration (not skipped).
        let meta_kv = std::sync::Arc::new(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(InMemoryKv::default()),
        ));
        let meta_view = WalletMetaView::new(&meta_kv);
        meta_view.set(network, &hash, &meta).expect("persist meta");
        let listed =
            hydrate_hd_wallets_from_views(&view, &meta_view, network).expect("hydration ok");
        assert!(
            listed.iter().any(|(h, _)| *h == hash),
            "the protected wallet must appear in the cold-boot listing",
        );
    }
}
