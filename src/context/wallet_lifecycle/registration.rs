//! Wallet registration and persistence: importing single-key wallets,
//! registering HD wallets, and writing the seed envelope / wallet meta.

use super::*;

impl AppContext {
    /// Single import path for an imported private key (#192). Parses the
    /// WIF, writes the encrypted vault entry + enumerable sidecar through
    /// [`SingleKeyView::import_wif_with_passphrase`], then mirrors the
    /// result into the in-memory `single_key_wallets` map the wallet
    /// screens render from (without which the key stays invisible until
    /// the next cold-boot hydration).
    ///
    /// The in-memory mirror is rebuilt through
    /// [`SingleKeyView::rebuild_display_wallet`] — the same vault-backed path
    /// cold boot uses — so a passphrase-protected key is mirrored **closed**
    /// (no plaintext private key retained in the long-lived map; signing
    /// decrypts just-in-time through the secret chokepoint), while an
    /// unprotected key is mirrored open as before. Rebuilding from the WIF
    /// with `from_wif(.., None, ..)` would have parked the decrypted key in
    /// the session map for the whole session, defeating the per-key
    /// passphrase.
    ///
    /// Every UI entry point — the import dialog, the import-wallet screen,
    /// and the test seam — routes through here so vault write and
    /// in-memory mirror can never diverge. Returns the rebuilt display
    /// wallet so the caller can select it.
    pub fn import_single_key_wif(
        &self,
        wif: &str,
        alias: Option<String>,
        passphrase: crate::wallet_backend::single_key::ImportPassphrase,
    ) -> Result<
        (
            crate::model::single_key::ImportedKey,
            Arc<RwLock<SingleKeyWallet>>,
        ),
        TaskError,
    > {
        let backend = self.wallet_backend()?;
        let single_key = backend.single_key();
        let imported = single_key.import_wif_with_passphrase(wif, alias, passphrase)?;

        // Rebuild the in-memory display wallet from the just-written vault
        // entry so the map matches the shape `hydrate_context_wallets`
        // produces on the next cold boot. For a passphrase-protected entry
        // this yields a closed wallet with no plaintext; for an unprotected
        // entry it yields the open wallet the legacy path produced.
        let wallet = single_key
            .rebuild_display_wallet(&imported)?
            .ok_or(TaskError::ImportedKeyNotFound)?;
        let key_hash = wallet.key_hash();
        let wallet_arc = Arc::new(RwLock::new(wallet));

        if let Ok(mut single_key_wallets) = self.single_key_wallets.write() {
            single_key_wallets.insert(key_hash, wallet_arc.clone());
            self.has_wallet.store(true, Ordering::Relaxed);
        }
        Ok((imported, wallet_arc))
    }

    /// Confirm that `passphrase` unlocks the protected imported key at
    /// `address` against the encrypted vault, without leaving any plaintext in
    /// the long-lived `single_key_wallets` map. Used by the wallets-screen
    /// "Unlock" gesture: signing already decrypts just-in-time through the
    /// secret chokepoint, so the map entry can stay closed while the user gets
    /// confirmation that their passphrase is correct. Returns
    /// [`TaskError::SingleKeyPassphraseIncorrect`] on a wrong passphrase.
    pub fn verify_single_key_passphrase(
        self: &Arc<Self>,
        address: &str,
        passphrase: &str,
    ) -> Result<(), TaskError> {
        // The unlock gesture also lazy re-wraps a protected entry to Tier-2
        // (verify_passphrase re-seals it under the same password). Protection is
        // KEPT, so there is no downgrade to disclose — no notice.
        let backend = self.wallet_backend()?;
        backend
            .single_key()
            .verify_passphrase(address, passphrase)?;
        Ok(())
    }

    /// Persist a wallet to the database and register it in the in-memory map.
    ///
    /// This is the single entry point for adding a wallet to the system.
    /// UI screens should call this after constructing a [`Wallet`] via
    /// [`Wallet::new_from_seed()`].
    ///
    /// `seed` is the freshly-created/imported HD seed the caller already holds
    /// from wallet construction. It is borrowed for the fresh-register
    /// bootstrap (and, for a password wallet, to promote into the JIT session
    /// cache) so registration never reads a parked seed — an open `Wallet`
    /// parks none (R3). The borrow does not outlive this call.
    ///
    /// `origin` records whether the recovery phrase is brand-new
    /// ([`WalletOrigin::Fresh`]) or pre-existing ([`WalletOrigin::Imported`]).
    /// It sets the upstream SPV scan-window floor: a fresh wallet scans from
    /// the current tip, an imported one from genesis so deposits made before
    /// registration are still found.
    pub fn register_wallet(
        self: &Arc<Self>,
        wallet: Wallet,
        seed: &[u8; 64],
        origin: WalletOrigin,
    ) -> Result<(WalletSeedHash, Arc<RwLock<Wallet>>), TaskError> {
        let seed_hash = wallet.seed_hash();
        let uses_password = wallet.uses_password;

        // 1. Reject a duplicate import. The upstream `platform-wallet.sqlite`
        // persistor is the system of record now; DET no longer writes the
        // legacy `data.db.wallet` row (the fresh-install schema gates that
        // table out entirely). Uniqueness is enforced against the wallet-meta
        // sidecar and the in-memory map — the same key (`seed_hash`) the
        // legacy unique constraint used.
        if self.wallets.read()?.contains_key(&seed_hash)
            || WalletMetaView::new(&self.app_kv)
                .get(self.network, &seed_hash)
                .is_some()
        {
            return Err(TaskError::WalletAlreadyImported);
        }

        // 2. Persist the seed-envelope vault entry — FAIL-CLOSED (F62). This is
        // the encrypted seed the W2 cold-boot bridge re-registers from; without
        // it the wallet works in-session but VANISHES with its funds on the next
        // launch. If it cannot be saved, the registration is aborted here (the
        // wallet is NOT inserted in-memory) so the UI tells the user the wallet
        // was not saved and to retry — never a silent loss. The vault is
        // AppContext-owned, so this succeeds even before the backend is wired.
        self.write_seed_envelope(&wallet)?;

        // Persist the wallet-meta sidecar — FAIL-CLOSED. Cold-boot hydration
        // enumerates ONLY this sidecar (`hydrate_wallets_for_network` rebuilds
        // `ctx.wallets` from `WalletMetaView::list`); there is no
        // upstream→meta reconstruction path. A wallet with a seed envelope but
        // no meta row is never hydrated, so its funds become unreachable on the
        // next launch with no self-heal. Both sidecars are required, so a meta
        // write failure aborts the registration here just like the envelope
        // write above. The sidecar is AppContext-owned (app_kv), so this
        // succeeds even before the backend is wired.
        self.write_wallet_meta(&wallet)?;

        // 3. Register in-memory
        let wallet_arc = Arc::new(RwLock::new(wallet));
        let mut wallets = self.wallets.write()?;
        wallets.insert(seed_hash, wallet_arc.clone());
        self.has_wallet.store(true, Ordering::Relaxed);
        drop(wallets);

        // 4. Bootstrap addresses from the seed the caller holds (fresh
        // register), then — for a password wallet — promote that seed into the
        // JIT session cache so the rest of the session does not re-prompt.
        // A no-password wallet needs no promotion: the chokepoint's
        // unprotected fast-path decrypts it without a prompt regardless.
        self.bootstrap_wallet_addresses(&wallet_arc, seed);
        if uses_password {
            self.promote_seed_to_session(seed_hash, seed);
        }

        // 5. Register the wallet with the upstream SPV backend so its addresses
        // are watched and received funds become visible (W1). The
        // upstream `create_wallet_from_seed_bytes` is the only writer to the
        // persistor, so without this the wallet is never watched. Done on a
        // tracked subtask because registration is async and this entry point is
        // synchronous; the seed is moved in zeroized and dropped when the task
        // ends. If the backend is not wired yet, the W2 cold-boot bridge covers
        // it at the next launch.
        self.register_wallet_upstream(seed_hash, seed, origin);

        Ok((seed_hash, wallet_arc))
    }

    /// Spawn the W1 upstream-registration subtask for a just-registered wallet.
    ///
    /// Moves a zeroized copy of `seed` into the subtask; the borrow in
    /// [`Self::register_wallet`] is not extended. The birth height follows the
    /// wallet's [`WalletOrigin`]. Best-effort: a registration failure is logged
    /// and the wallet is retried by the W2 cold-boot bridge at next launch.
    fn register_wallet_upstream(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        seed: &[u8; 64],
        origin: WalletOrigin,
    ) {
        let Ok(backend) = self.wallet_backend() else {
            tracing::debug!(
                wallet = %hex::encode(seed_hash),
                "Wallet backend not wired yet; deferring upstream registration to next cold boot"
            );
            return;
        };
        let seed = zeroize::Zeroizing::new(*seed);
        let birth_height = registration_birth_height(origin);
        let _ = self
            .subtasks
            .spawn_sync("wallet_upstream_registration", async move {
                if let Err(error) = backend
                    .register_wallet_from_seed(&seed_hash, &seed, birth_height)
                    .await
                {
                    tracing::warn!(
                        wallet = %hex::encode(seed_hash),
                        %error,
                        "Upstream wallet registration failed; will retry at next cold boot"
                    );
                }
            });
    }

    /// Persist a newly-registered wallet's encrypted seed envelope to the
    /// vault. **Fail-closed** (F62): this is the must-succeed write — the
    /// envelope is the encrypted seed the W2 cold-boot bridge re-registers the
    /// wallet from, so a failure here means the wallet would silently disappear
    /// with its funds at the next launch. The caller propagates the error so
    /// the wallet is not kept.
    ///
    /// Writes through the shared `secret_store` vault that `AppContext` opens at
    /// boot, so it succeeds even before the wallet backend is wired:
    /// the backend, once built, reuses the very same vault handle.
    fn write_seed_envelope(&self, wallet: &Wallet) -> Result<(), TaskError> {
        let seed_hash = wallet.seed_hash();
        let view = WalletSeedView::new(&self.secret_store);
        // No-password wallets store the raw 64-byte seed directly through the
        // seam: `encrypted_seed_slice()` is the verbatim seed (no DET AES-GCM).
        // The non-secret metadata rides in `WalletMeta` (write_wallet_meta).
        if !wallet.uses_password {
            let seed: [u8; 64] = wallet.encrypted_seed_slice().try_into().map_err(|_| {
                TaskError::WalletSeedStorage {
                    source: Box::new(
                        platform_wallet_storage::secrets::SecretStoreError::MalformedVault,
                    ),
                }
            })?;
            return view.set_raw(&seed_hash, &seed);
        }
        // Password wallets keep the legacy AES-GCM envelope at creation; they
        // migrate to the raw seam lazily at the next unlock (one prompt the
        // user already does).
        let envelope = StoredSeedEnvelope {
            encrypted_seed: zeroize::Zeroizing::new(wallet.encrypted_seed_slice().to_vec()),
            salt: wallet.salt().to_vec(),
            nonce: wallet.nonce().to_vec(),
            password_hint: wallet.password_hint().clone(),
            uses_password: wallet.uses_password,
            xpub_encoded: wallet
                .master_bip44_ecdsa_extended_public_key
                .encode()
                .to_vec(),
        };
        view.set(&seed_hash, &envelope)
    }

    /// Persist a newly-registered wallet's metadata (alias / is_main /
    /// core_wallet_name + master xpub) to the wallet-meta sidecar.
    /// **Fail-closed**: cold-boot hydration enumerates ONLY this
    /// sidecar (`hydrate_wallets_for_network` lists `WalletMetaView`), and
    /// nothing reconstructs the meta from the upstream persistor — so a wallet
    /// with no meta row never rehydrates and its funds become unreachable. The
    /// caller propagates the error so the wallet is not kept.
    fn write_wallet_meta(&self, wallet: &Wallet) -> Result<(), TaskError> {
        let seed_hash = wallet.seed_hash();
        let meta = WalletMeta {
            alias: wallet.alias.clone().unwrap_or_default(),
            is_main: wallet.is_main,
            core_wallet_name: wallet.core_wallet_name.clone(),
            xpub_encoded: wallet
                .master_bip44_ecdsa_extended_public_key
                .encode()
                .to_vec(),
            uses_password: wallet.uses_password,
            password_hint: wallet.password_hint().clone(),
        };
        WalletMetaView::new(&self.app_kv).set(self.network, &seed_hash, &meta)
    }

    /// Promote a known HD seed into the JIT chokepoint's session cache
    /// (`UntilAppClose`), so the rest of the session does not re-prompt for
    /// this wallet.
    ///
    /// Used by the fresh-register path, which holds the seed from wallet
    /// construction. Best-effort: if the backend is not wired yet the promotion
    /// is skipped — signing still resolves the seed just-in-time from the vault.
    fn promote_seed_to_session(self: &Arc<Self>, seed_hash: WalletSeedHash, seed: &[u8; 64]) {
        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        let seed = zeroize::Zeroizing::new(*seed);
        backend.secret_access().remember_session(
            &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
            crate::wallet_backend::SecretPlaintext::HdSeed(&seed),
            crate::wallet_backend::RememberPolicy::UntilAppClose,
        );
        tracing::trace!(
            wallet = %hex::encode(seed_hash),
            "Freshly-registered seed promoted to the session cache"
        );
    }
}
