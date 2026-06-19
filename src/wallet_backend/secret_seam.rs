//! The single chokepoint for storing/loading raw wallet secret bytes.
//!
//! All three secret classes (HD seed, imported single key, identity private
//! key) route their RAW bytes through this one seam into the upstream
//! [`SecretStore`] vault. No DET-side serialization wraps the secret: a
//! [`SecretBytes`] is written verbatim and read back verbatim.
//!
//! TODAY this is a no-encryption pass-through to the vault. This is the exact
//! place per-secret encryption wires in later — every put/get body is tagged
//! with the greppable string `TODO(per-secret-encryption):` so a reviewer-side
//! grep is the wiring checklist.
//!
//! The seam is **prompt-free**: it never builds a passphrase request. The only
//! place a passphrase is needed is the retained legacy-envelope decrypt during
//! migration, which lives in the legacy reader, not here.
//!
//! No-serialization invariant: secrets are passed as [`SecretBytes`], which
//! deliberately has no `Serialize`/`Encode` (verified upstream). Any struct
//! embedding a `SecretBytes` therefore cannot derive those traits — the
//! compiler enforces the rule. The seam never constructs an intermediate
//! serializable wrapper around the secret.
//!
//! TS-INV-01 — the invariant guard, enforced by the compiler. A newtype that
//! tries to derive `serde::Serialize` over a [`SecretBytes`] does NOT compile,
//! because `SecretBytes: !Serialize`. If a future upstream adds `Serialize` to
//! `SecretBytes`, this doctest starts compiling and the failing test flags that
//! the invariant has silently weakened.
//!
//! ```compile_fail
//! use platform_wallet_storage::secrets::SecretBytes;
//! #[derive(serde::Serialize)]
//! struct Leaky(SecretBytes);
//! ```
//!
//! Serializing a `SecretBytes` directly is likewise rejected:
//!
//! ```compile_fail
//! use platform_wallet_storage::secrets::SecretBytes;
//! let secret = SecretBytes::from_slice(&[0u8; 32]);
//! let _ = serde_json::to_string(&secret).unwrap();
//! ```

use std::sync::Arc;

use platform_wallet_storage::secrets::{
    SecretBytes, SecretStore, SecretStoreError, WalletId as SecretWalletId,
};

use crate::backend_task::error::TaskError;

/// The single doorway through which raw wallet secret bytes enter and leave
/// the vault. Cheap to construct — callers build one per operation over the
/// shared [`SecretStore`] handle.
pub struct SecretSeam<'a> {
    secret_store: &'a Arc<SecretStore>,
}

impl<'a> SecretSeam<'a> {
    /// Borrow the shared [`SecretStore`] as the raw-secret seam.
    pub fn new(secret_store: &'a Arc<SecretStore>) -> Self {
        Self { secret_store }
    }

    /// Store `secret` raw under `(scope, label)`, overwriting any prior value.
    /// Idempotent — the upstream `set` upserts.
    ///
    /// TODAY the [`SecretBytes`] is written verbatim with no DET-side
    /// encryption; the upstream vault adds its own at-rest layer.
    // TODO(per-secret-encryption): encrypt `secret` here before set() once the
    // upstream per-secret key layer lands (see platform /todo).
    pub fn put_secret(
        &self,
        scope: &SecretWalletId,
        label: &str,
        secret: &SecretBytes,
    ) -> Result<(), TaskError> {
        self.secret_store.set(scope, label, secret).map_err(map_err)
    }

    /// Load the raw bytes stored under `(scope, label)`, or `Ok(None)` if
    /// nothing is stored there. No prompt — an already-migrated raw secret
    /// needs none.
    ///
    /// TODAY the vault bytes are returned verbatim.
    // TODO(per-secret-encryption): decrypt the loaded bytes here once the
    // upstream per-secret key layer lands.
    pub fn get_secret(
        &self,
        scope: &SecretWalletId,
        label: &str,
    ) -> Result<Option<SecretBytes>, TaskError> {
        self.secret_store.get(scope, label).map_err(map_err)
    }

    /// Idempotent delete of `(scope, label)`. A missing entry is `Ok(())`.
    // No `TODO(per-secret-encryption)` here — delete is metadata-free, there is
    // no secret to (de)crypt.
    pub fn delete_secret(&self, scope: &SecretWalletId, label: &str) -> Result<(), TaskError> {
        self.secret_store.delete(scope, label).map_err(map_err)
    }
}

fn map_err(source: SecretStoreError) -> TaskError {
    TaskError::SecretSeam {
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::single_key::open_secret_store;

    fn fresh_store(dir: &std::path::Path) -> Arc<SecretStore> {
        let path = dir.join("secrets.pwsvault");
        Arc::new(open_secret_store(&path).expect("open vault"))
    }

    /// TS-RT-01 — HD seed raw round-trip. A known 64-byte seed stored under
    /// `seed.raw.v1` comes back byte-for-byte. A missing label and a foreign
    /// scope both return `Ok(None)` (scope/label partition).
    #[test]
    fn ts_rt_01_hd_seed_raw_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seam = SecretSeam::new(&store);
        let scope = SecretWalletId::from([0x11u8; 32]);
        let mut seed = [0u8; 64];
        for (i, b) in seed.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(3).wrapping_add(7);
        }

        seam.put_secret(&scope, "seed.raw.v1", &SecretBytes::from_slice(&seed))
            .expect("put");
        let got = seam
            .get_secret(&scope, "seed.raw.v1")
            .expect("get")
            .expect("present");
        assert_eq!(
            got.expose_secret(),
            &seed[..],
            "round-tripped seed must equal the exact 64 input bytes"
        );

        // Missing label and foreign scope both miss.
        assert!(
            seam.get_secret(&scope, "single_key_priv.x")
                .expect("get missing label")
                .is_none()
        );
        let other = SecretWalletId::from([0x22u8; 32]);
        assert!(
            seam.get_secret(&other, "seed.raw.v1")
                .expect("get foreign scope")
                .is_none(),
            "a different scope must not see the seed"
        );
    }

    /// TS-RT-02 — single-key raw round-trip. The stored value is exactly 32
    /// bytes (raw, NOT a `SingleKeyEntry` envelope). A foreign scope misses.
    #[test]
    fn ts_rt_02_single_key_raw_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seam = SecretSeam::new(&store);
        let scope = crate::wallet_backend::single_key::single_key_namespace_id();
        let key = [0xABu8; 32];
        let label = "single_key_priv.yTestAddress";

        seam.put_secret(&scope, label, &SecretBytes::from_slice(&key))
            .expect("put");
        let got = seam
            .get_secret(&scope, label)
            .expect("get")
            .expect("present");
        assert_eq!(got.expose_secret(), &key[..]);
        assert_eq!(
            got.expose_secret().len(),
            32,
            "raw single key is exactly 32 bytes, not a versioned envelope"
        );

        let other = SecretWalletId::from([0u8; 32]);
        assert!(seam.get_secret(&other, label).expect("foreign").is_none());
    }

    /// TS-RT-03 — identity-key raw round-trip. Two `(target, key_id)` labels
    /// under one identity scope do not collide; two identities (distinct
    /// scopes) with the same `key_id` do not collide.
    #[test]
    fn ts_rt_03_identity_key_raw_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seam = SecretSeam::new(&store);
        let identity_a = SecretWalletId::from([0xA1u8; 32]);
        let identity_b = SecretWalletId::from([0xB2u8; 32]);
        let key0 = [0x01u8; 32];
        let key1 = [0x02u8; 32];

        seam.put_secret(
            &identity_a,
            "identity_key_priv.0.0",
            &SecretBytes::from_slice(&key0),
        )
        .unwrap();
        seam.put_secret(
            &identity_a,
            "identity_key_priv.0.1",
            &SecretBytes::from_slice(&key1),
        )
        .unwrap();
        // Same key_id (0) under a different identity scope — distinct value.
        let key_other = [0x99u8; 32];
        seam.put_secret(
            &identity_b,
            "identity_key_priv.0.0",
            &SecretBytes::from_slice(&key_other),
        )
        .unwrap();

        assert_eq!(
            seam.get_secret(&identity_a, "identity_key_priv.0.0")
                .unwrap()
                .unwrap()
                .expose_secret(),
            &key0[..]
        );
        assert_eq!(
            seam.get_secret(&identity_a, "identity_key_priv.0.1")
                .unwrap()
                .unwrap()
                .expose_secret(),
            &key1[..],
            "distinct (target,key_id) labels under one identity do not collide"
        );
        assert_eq!(
            seam.get_secret(&identity_b, "identity_key_priv.0.0")
                .unwrap()
                .unwrap()
                .expose_secret(),
            &key_other[..],
            "same key_id under a different identity scope does not collide"
        );
    }

    /// TS-INV-02 — the seam accepts/returns `SecretBytes`, never a serde
    /// struct. The compiler is the assertion: this round-trips a `SecretBytes`
    /// through the real signatures with no intermediate serializable wrapper.
    #[test]
    fn ts_inv_02_seam_uses_secret_bytes_not_serde_struct() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seam = SecretSeam::new(&store);
        let scope = SecretWalletId::from([0x33u8; 32]);
        let secret: SecretBytes = SecretBytes::from_slice(&[0x42u8; 32]);
        seam.put_secret(&scope, "seed.raw.v1", &secret).unwrap();
        let _back: Option<SecretBytes> = seam.get_secret(&scope, "seed.raw.v1").unwrap();
    }

    /// Idempotent delete — removing an absent entry succeeds, and a delete
    /// after `put_secret` clears the value.
    #[test]
    fn delete_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seam = SecretSeam::new(&store);
        let scope = SecretWalletId::from([0x44u8; 32]);
        seam.delete_secret(&scope, "seed.raw.v1").expect("absent");
        seam.put_secret(&scope, "seed.raw.v1", &SecretBytes::from_slice(&[1u8; 64]))
            .unwrap();
        seam.delete_secret(&scope, "seed.raw.v1").expect("first");
        seam.delete_secret(&scope, "seed.raw.v1").expect("second");
        assert!(seam.get_secret(&scope, "seed.raw.v1").unwrap().is_none());
    }

    /// TS-NOLEAK-01 — the on-disk vault file holds the raw secret in neither
    /// hex nor decimal-array form (the upstream file backend encrypts at rest
    /// even under an empty global passphrase). The in-memory `get_secret`
    /// return is legitimately plaintext by design — this asserts the persisted
    /// file, not the return value.
    #[test]
    fn ts_noleak_01_on_disk_vault_does_not_contain_raw_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.pwsvault");
        let store = Arc::new(open_secret_store(&path).expect("open vault"));
        let seam = SecretSeam::new(&store);
        let scope = SecretWalletId::from([0x55u8; 32]);
        let secret = crate::wallet_backend::leak_test_support::distinctive_secret_64();
        seam.put_secret(&scope, "seed.raw.v1", &SecretBytes::from_slice(&secret))
            .unwrap();
        drop(store);

        let on_disk = std::fs::read(&path).expect("read vault file");
        let rendered = String::from_utf8_lossy(&on_disk);
        crate::wallet_backend::leak_test_support::assert_no_leak_bytes(
            &rendered,
            &secret,
            "seam on-disk vault",
        );
    }

    /// TS-INV-03 — source-text audit over the changed secret-path modules: no
    /// `#[derive(...Serialize...)]` / `Encode` struct may name a plaintext-key
    /// field. Catches the bare-`Vec<u8>`/`[u8; 32]` plaintext bypass the
    /// `compile_fail` doctests (which only catch an embedded `SecretBytes`)
    /// cannot. The module list must track the blast-radius table — a stale list
    /// silently shrinks the surface, itself a finding.
    #[test]
    fn ts_inv_03_no_serializable_struct_embeds_a_plaintext_field() {
        // Files under the secret-path blast radius.
        const MODULES: &[&str] = &[
            "src/wallet_backend/secret_seam.rs",
            "src/wallet_backend/secret_access.rs",
            "src/wallet_backend/identity_key_store.rs",
            "src/wallet_backend/wallet_seed_store.rs",
            "src/wallet_backend/single_key.rs",
            "src/wallet_backend/single_key_entry.rs",
            "src/model/qualified_identity/encrypted_key_storage.rs",
            "src/model/wallet/meta.rs",
            "src/model/single_key.rs",
            "src/model/wallet/seed_envelope.rs",
        ];
        // Field-shape needles that name plaintext key/seed material by type.
        // (`ciphertext`/`encrypted_seed`/`encrypted_private_key` are NOT here —
        // they hold AES-GCM ciphertext or migration-reader bytes, not plaintext;
        // the no-serialization invariant is about embedding live plaintext.)
        const PLAINTEXT_NEEDLES: &[&str] = &["SecretBytes", "Zeroizing<[u8", ": [u8; 32]", ": [u8; 64]"];

        let manifest = env!("CARGO_MANIFEST_DIR");
        let mut offenders = Vec::new();
        for rel in MODULES {
            let path = std::path::Path::new(manifest).join(rel);
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("audit must read {rel}: {e} (stale module list?)"));

            // Track whether the most recent derive line opted into a serializer.
            let mut in_serializable_struct = false;
            let mut brace_depth_at_struct: Option<usize> = None;
            let mut depth = 0usize;
            for line in src.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("#[derive(")
                    && (trimmed.contains("Serialize") || trimmed.contains("Encode"))
                {
                    in_serializable_struct = true;
                    brace_depth_at_struct = None;
                    continue;
                }
                if in_serializable_struct && brace_depth_at_struct.is_none() {
                    // The struct opener for the pending derive.
                    if line.contains('{') {
                        brace_depth_at_struct = Some(depth);
                    }
                }
                // Inside the serializable struct body, look for plaintext fields.
                if in_serializable_struct
                    && brace_depth_at_struct.is_some()
                    && PLAINTEXT_NEEDLES.iter().any(|n| line.contains(n))
                {
                    offenders.push(format!("{rel}: {}", line.trim()));
                }
                depth += line.matches('{').count();
                depth = depth.saturating_sub(line.matches('}').count());
                if let Some(start) = brace_depth_at_struct
                    && depth <= start
                    && line.contains('}')
                {
                    in_serializable_struct = false;
                    brace_depth_at_struct = None;
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "a Serialize/Encode struct names a plaintext-key field (no-serialization invariant): {offenders:#?}",
        );
    }
}
