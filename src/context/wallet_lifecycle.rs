use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::database::is_unique_constraint_violation;
use crate::model::spv_status::SpvStatus;
use crate::model::wallet::meta::WalletMeta;
use crate::model::wallet::seed_envelope::StoredSeedEnvelope;
use crate::model::wallet::{DerivationPathReference, DerivationPathType, Wallet, WalletSeedHash};
use crate::wallet_backend::{DetScope, WalletBackend};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

/// Number of identity-authentication keys warmed per known identity index
/// during the JIT bootstrap (D4b). Matches the readers' auth-key lookup
/// window so the common identity-load path serves entirely from cache.
const AUTH_PUBKEY_WARM_KEY_COUNT: u32 = 12;

impl AppContext {
    /// Clear the SPV data directory.
    ///
    /// No-op: chain sync is owned by upstream `platform-wallet`; DET no longer
    /// maintains an SPV data directory. P2 wires this to the upstream runtime.
    pub fn clear_spv_data(&self) -> Result<(), TaskError> {
        Ok(())
    }

    pub fn clear_network_database(&self) -> Result<(), TaskError> {
        self.db.clear_network_data(self.network)?;

        // D4d: drain the DashPay k/v sidecar. The Global-scoped overlays
        // (blocked / rejected markers, timestamps, reverse address map)
        // share the `det:dashpay:` prefix and come out in one sweep. The
        // per-contact private memos and address-index cursors now live in
        // each owner's `DetScope::Identity` scope (Wave 2 promotion), which
        // the Global sweep cannot reach — so fan the per-owner clear out
        // over the identity index. Best-effort when the wallet backend has
        // not been wired yet (clear at first run before any wallet exists)
        // — there is nothing to drain in that case.
        if let Ok(backend) = self.wallet_backend() {
            let kv = backend.kv();
            match kv.list(DetScope::Global, Some("det:dashpay:")) {
                Ok(keys) => {
                    for k in keys {
                        if let Err(e) = kv.delete(DetScope::Global, &k) {
                            tracing::warn!(key = %k, "DashPay sidecar delete failed: {e:?}");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("DashPay sidecar listing failed: {e:?}");
                }
            }
            match self.local_identity_ids() {
                Ok(owners) => {
                    for owner in owners {
                        if let Err(e) = backend.dashpay_clear_owner_overlays(&owner) {
                            tracing::warn!(
                                owner = %owner,
                                "DashPay per-owner overlay clear failed: {e:?}"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Identity index listing for DashPay clear failed: {e:?}");
                }
            }
        }

        // Drop the per-network shielded commitment-tree SQLite sidecar
        // (replaces the legacy in-place table truncation on `data.db`).
        // Missing file is the expected state on fresh installs and is
        // tolerated. Backend-not-initialised is also fine — the file
        // cannot exist without the backend having opened it.
        if let Ok(tree_path) = self.shielded_commitment_tree_path()
            && let Err(e) = std::fs::remove_file(&tree_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(TaskError::FileSystem { source: e });
        }

        if let Ok(mut wallets) = self.wallets.write() {
            wallets.clear();
        }

        if let Ok(mut single_key_wallets) = self.single_key_wallets.write() {
            single_key_wallets.clear();
        }

        self.has_wallet.store(false, Ordering::Relaxed);

        Ok(())
    }

    /// Start chain sync against an already-wired wallet backend.
    ///
    /// Delegates to [`WalletBackend::start`], which spawns the upstream
    /// `SpvRuntime` run loop and the platform-address / identity sync
    /// coordinators. The backend's `start` is idempotent, so calling this more
    /// than once (Connect clicked twice, eager-init plus a manual click) is
    /// safe.
    ///
    /// Fails fast with [`TaskError::WalletBackendNotYetWired`] when the wallet
    /// seam has not been built yet. Most callers should reach for
    /// [`Self::ensure_wallet_backend_and_start_spv`] instead, which wires the
    /// backend first and so cannot hit that race; this entry point exists for
    /// the post-wiring paths that already hold a wired backend.
    pub fn start_spv(self: &Arc<Self>) -> Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        self.spawn_backend_start(backend);
        Ok(())
    }

    /// Wire the wallet backend (idempotent) and then start chain sync.
    ///
    /// This is the single chokepoint for "start SPV" across every entry path:
    /// GUI boot auto-start, the manual Connect button, MCP/CLI standalone boot,
    /// and the post-network-switch restart. Wiring happens first, so the
    /// historical `WalletBackendNotYetWired` fast-fail race — callers invoking
    /// [`Self::start_spv`] before [`Self::ensure_wallet_backend`] had a chance
    /// to complete — cannot occur.
    ///
    /// Both steps are idempotent: the backend is wired at most once (first
    /// writer wins) and the upstream run loop is spawned at most once (guarded
    /// by the backend's start latch). Chain sync runs asynchronously — progress
    /// and success arrive via the `EventBridge`.
    ///
    /// On failure the SPV connection indicator is flipped to
    /// [`SpvStatus::Error`] before the error is returned, so every caller — GUI
    /// boot auto-start, the manual Connect button, the network-switch restart,
    /// and the MCP/headless path — gets a consistent error state on the
    /// indicator without each having to remember to set it. GUI callers may
    /// additionally show a banner; headless callers need no egui context for
    /// the indicator flip, which is what this method owns.
    pub async fn ensure_wallet_backend_and_start_spv(
        self: &Arc<Self>,
        sender: crate::utils::egui_mpsc::SenderAsync<crate::app::TaskResult>,
    ) -> Result<(), TaskError> {
        if let Err(e) = self.ensure_wallet_backend(sender).await {
            self.mark_spv_error(&e);
            return Err(e);
        }
        let backend = self.wallet_backend()?;
        self.run_backend_start(backend).await;
        Ok(())
    }

    /// Flip the SPV connection indicator to [`SpvStatus::Error`] and record the
    /// failure detail. Safe in every context (GUI and headless) — it touches
    /// only `ConnectionStatus` atomics, never an egui context.
    fn mark_spv_error(&self, error: &TaskError) {
        tracing::error!(error = %error, "Failed to start chain sync");
        self.connection_status
            .set_spv_last_error(Some(format!("{error}")));
        self.connection_status.set_spv_status(SpvStatus::Error);
        self.connection_status.refresh_state();
    }

    /// Spawn [`WalletBackend::start`] on the subtask runtime, surfacing a
    /// start failure on the SPV connection indicator. Shared by the
    /// synchronous [`Self::start_spv`] entry point.
    fn spawn_backend_start(self: &Arc<Self>, backend: Arc<WalletBackend>) {
        let ctx = Arc::clone(self);
        self.subtasks.spawn_sync("spv_start", async move {
            ctx.run_backend_start(backend).await;
        });
    }

    /// Drive [`WalletBackend::start`] to completion, flipping the SPV indicator
    /// to `Error` if the start fails. Awaited directly by the async chokepoint
    /// and indirectly (via a subtask) by the synchronous one.
    async fn run_backend_start(&self, backend: Arc<WalletBackend>) {
        // Forward-compat: `start()`'s signature is fallible though the current
        // impl is infallible. The reachable start-time failure today is the
        // wiring step, which the chokepoint surfaces via `mark_spv_error`; this
        // branch keeps the start step covered should `start()` begin to fail.
        if let Err(e) = backend.start().await {
            self.mark_spv_error(&e);
        }
    }

    /// Stop chain sync. Inert; see [`Self::start_spv`].
    pub fn stop_spv(&self) {
        self.connection_status.reset_timer();
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
    pub fn register_wallet(
        self: &Arc<Self>,
        wallet: Wallet,
        seed: &[u8; 64],
    ) -> Result<(WalletSeedHash, Arc<RwLock<Wallet>>), TaskError> {
        let seed_hash = wallet.seed_hash();
        let uses_password = wallet.uses_password;

        // 1. Persist to sidecars (T-W-01). The wallet-meta sidecar
        // carries alias/is_main/core_wallet_name plus the pre-computed
        // master xpub the cold-boot picker reads without unlocking the
        // vault; the seed-envelope sidecar carries the encrypted seed
        // bytes plus the matching xpub copy.
        //
        // The legacy `data.db.wallet` row is still written so the
        // in-process migration replay (T-DEV-02) has something to read
        // when an older build is downgraded onto a freshly-imported
        // wallet. The cold-boot READ path no longer touches that row —
        // see the comment in `AppContext::new`.
        if let Err(e) = self.write_wallet_sidecars(&wallet) {
            tracing::warn!(
                wallet = %hex::encode(seed_hash),
                error = ?e,
                "Failed to persist wallet sidecars; rolling forward with legacy DB write",
            );
        }

        let addresses: Vec<_> = wallet
            .known_addresses
            .iter()
            .map(|(address, path)| {
                (
                    address,
                    path,
                    DerivationPathReference::BIP44,
                    DerivationPathType::CLEAR_FUNDS,
                )
            })
            .collect();

        self.db
            .store_wallet_with_addresses(&wallet, &self.network, &addresses)
            .map_err(|e| {
                if is_unique_constraint_violation(&e) {
                    TaskError::WalletAlreadyImported
                } else {
                    TaskError::Database { source: e }
                }
            })?;

        // 2. Register in-memory
        let wallet_arc = Arc::new(RwLock::new(wallet));
        let mut wallets = self.wallets.write()?;
        wallets.insert(seed_hash, wallet_arc.clone());
        self.has_wallet.store(true, Ordering::Relaxed);
        drop(wallets);

        // 3. Bootstrap addresses from the seed the caller holds (fresh
        // register), then — for a password wallet — promote that seed into the
        // JIT session cache so the rest of the session does not re-prompt.
        // A no-password wallet needs no promotion: the chokepoint's
        // unprotected fast-path decrypts it without a prompt regardless.
        self.bootstrap_wallet_addresses(&wallet_arc, seed);
        if uses_password {
            self.promote_seed_to_session(seed_hash, seed);
        }

        Ok((seed_hash, wallet_arc))
    }

    /// Mirror a newly-registered wallet into the wallet-meta +
    /// seed-envelope sidecars. Skipped (logged) when the wallet backend
    /// has not been wired yet — the next `ensure_wallet_backend` boot
    /// then rebuilds the same sidecar entries via T-W-00 / T-W-00.5-v2
    /// migration replay against the legacy row that was written below.
    fn write_wallet_sidecars(&self, wallet: &Wallet) -> Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        let seed_hash = wallet.seed_hash();
        let xpub_encoded = wallet
            .master_bip44_ecdsa_extended_public_key
            .encode()
            .to_vec();

        let envelope = StoredSeedEnvelope {
            encrypted_seed: wallet.encrypted_seed_slice().to_vec(),
            salt: wallet.salt().to_vec(),
            nonce: wallet.nonce().to_vec(),
            password_hint: wallet.password_hint().clone(),
            uses_password: wallet.uses_password,
            xpub_encoded: xpub_encoded.clone(),
        };
        backend.wallet_seeds().set(&seed_hash, &envelope)?;

        let meta = WalletMeta {
            alias: wallet.alias.clone().unwrap_or_default(),
            is_main: wallet.is_main,
            core_wallet_name: wallet.core_wallet_name.clone(),
            xpub_encoded,
        };
        backend.wallet_meta().set(self.network, &seed_hash, &meta)?;

        Ok(())
    }

    /// Whether `wallet` still needs its bootstrap address set derived.
    ///
    /// `true` for a fresh wallet (no known addresses) or one created with only
    /// a Core address (no Platform-payment addresses yet). Idempotent: a
    /// fully-bootstrapped wallet returns `false`.
    fn wallet_needs_bootstrap(guard: &Wallet) -> bool {
        // INTENTIONAL(CODE-006): Bootstrap checks only PlatformPayment address
        // type. Other platform address types may trigger redundant
        // re-derivation, but `bootstrap_known_addresses` is idempotent so this
        // is safe.
        let has_platform_addresses = guard.watched_addresses.values().any(|info| {
            info.path_reference == crate::model::wallet::DerivationPathReference::PlatformPayment
        });
        guard.known_addresses.is_empty() || !has_platform_addresses
    }

    /// Bootstrap a wallet's address set from a borrowed HD seed.
    ///
    /// The sync bridge used by the **fresh-register** path only
    /// ([`Self::register_wallet`]): a just-created or just-imported wallet's
    /// seed is in the caller's hand from construction, so it is passed in by
    /// borrow rather than read from any parked field — an open `Wallet` parks
    /// no seed (R3). The borrow is fanned down into the seed-as-parameter
    /// [`Wallet::bootstrap_known_addresses`]; no `bootstrap_*` child reaches
    /// back into the wallet for a seed. A locked wallet is skipped and
    /// bootstraps later via [`Self::bootstrap_wallet_addresses_jit`] once its
    /// seed is resolvable through the chokepoint.
    pub fn bootstrap_wallet_addresses(&self, wallet: &Arc<RwLock<Wallet>>, seed: &[u8; 64]) {
        if let Ok(mut guard) = wallet.write() {
            if !guard.is_open() {
                tracing::debug!("Skipping address bootstrap for locked wallet");
                return;
            }
            if Self::wallet_needs_bootstrap(&guard) {
                tracing::info!(wallet = %hex::encode(guard.seed_hash()), "Bootstrapping wallet addresses");
                guard.bootstrap_known_addresses(seed, self);
            }
        }
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

    /// Bootstrap a wallet's address set by resolving its HD seed just-in-time
    /// through the [`SecretAccess`](crate::wallet_backend::SecretAccess)
    /// chokepoint, holding one `with_secret_session` for the whole bootstrap
    /// run.
    ///
    /// The async sibling of [`Self::bootstrap_wallet_addresses`] for the
    /// cold-boot path. To preserve the prompt-free startup contract it operates
    /// only on wallets whose seed already resolves without asking the user — an
    /// unprotected wallet (resolved via the chokepoint's no-passphrase
    /// fast-path) or a protected one whose seed the user already promoted to the
    /// session cache on unlock. A still-locked protected wallet is left for its
    /// unlock gesture to bootstrap, exactly as before; this method never forces
    /// a passphrase prompt at startup.
    pub async fn bootstrap_wallet_addresses_jit(&self, wallet: &Arc<RwLock<Wallet>>) {
        let seed_hash = {
            let Ok(guard) = wallet.read() else {
                return;
            };
            // Gate on the open seed being resolvable prompt-free: an open
            // wallet at cold boot is either unprotected (no-prompt fast-path) or
            // already session-cached via the unlock gesture. A locked protected
            // wallet is skipped to avoid a surprise startup prompt.
            if !guard.is_open() || !Self::wallet_needs_bootstrap(&guard) {
                return;
            }
            guard.seed_hash()
        };

        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        let wallet = Arc::clone(wallet);
        let result = backend
            .secret_access()
            .with_secret_session(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                async |session| {
                    let plaintext = session.plaintext();
                    let seed = plaintext
                        .expose_hd_seed()
                        .ok_or(TaskError::WalletLocked)?;
                    if let Ok(mut guard) = wallet.write() {
                        // Re-check under the write lock: a concurrent bootstrap
                        // may have run between the read above and here.
                        if Self::wallet_needs_bootstrap(&guard) {
                            tracing::info!(wallet = %hex::encode(seed_hash), "Bootstrapping wallet addresses (JIT seed)");
                            guard.bootstrap_known_addresses(seed, self);
                        }
                    }
                    // D4b lazy warm: populate the identity-auth public-key
                    // cache for the identities this wallet already knows, in
                    // the same prompt-free seed scope, so the steady-state
                    // identity-auth reads are seed-free. Best-effort — a warm
                    // failure only forgoes the optimisation.
                    if let Ok(guard) = wallet.read() {
                        self.warm_auth_pubkey_cache(&backend, &guard, seed, seed_hash);
                    }
                    Ok(())
                },
            )
            .await;
        if let Err(e) = result {
            tracing::debug!(
                wallet = %hex::encode(seed_hash),
                error = %e,
                "JIT address bootstrap skipped"
            );
        }
    }

    /// Warm the identity-authentication public-key cache (D4b) for the
    /// identities this wallet already knows.
    ///
    /// Called from inside the JIT bootstrap's `with_secret_session` scope,
    /// so the borrowed seed is already in hand and no extra prompt is
    /// raised. Derives the first [`AUTH_PUBKEY_WARM_KEY_COUNT`] auth keys
    /// per known identity index and persists them in one whole-blob write.
    /// Identities discovered later warm lazily on the read path's cold-fill.
    /// Best-effort: a derivation or persist failure is logged and skipped,
    /// because the read path self-heals regardless.
    fn warm_auth_pubkey_cache(
        &self,
        backend: &WalletBackend,
        wallet: &Wallet,
        seed: &[u8; 64],
        seed_hash: WalletSeedHash,
    ) {
        let network = self.network;
        let view = backend.auth_pubkey_cache();
        let mut cache = view.get(network, &seed_hash);
        let mut changed = false;

        for &identity_index in wallet.identities.keys() {
            for key_index in 0..AUTH_PUBKEY_WARM_KEY_COUNT {
                if cache.get(network, identity_index, key_index).is_some() {
                    continue;
                }
                match wallet.identity_authentication_ecdsa_public_key_from_seed(
                    seed,
                    network,
                    identity_index,
                    key_index,
                ) {
                    Ok(public_key) => {
                        changed |= cache.insert(network, identity_index, key_index, &public_key);
                    }
                    Err(error) => {
                        tracing::debug!(
                            wallet = %hex::encode(seed_hash),
                            identity_index,
                            key_index,
                            %error,
                            "Skipping auth-pubkey warm for one key"
                        );
                    }
                }
            }
        }

        if changed && let Err(e) = view.put(network, &seed_hash, &cache) {
            tracing::debug!(
                wallet = %hex::encode(seed_hash),
                error = %e,
                "Failed to persist warmed auth-pubkey cache"
            );
        }
    }

    /// React to a wallet becoming unlocked in the UI.
    ///
    /// Since the JIT migration this is **not** a seed-distribution point —
    /// signing pulls the seed just-in-time from the encrypted vault through
    /// the [`SecretAccess`](crate::wallet_backend::SecretAccess) chokepoint.
    /// Its only job now is to honor the unlock gesture's "keep unlocked"
    /// intent for **password-protected** wallets: re-decrypt the just-verified
    /// seed through the chokepoint with the passphrase the user just entered,
    /// and promote it into the session cache (`UntilAppClose`) so the rest of
    /// the session's operations on this wallet do not re-prompt.
    ///
    /// `passphrase` is the secret the UI just validated via
    /// [`WalletSeed::open`](crate::model::wallet::WalletSeed::open). It is
    /// `None` for the cold-boot bridge ([`Self::bootstrap_loaded_wallets`]) and
    /// for no-password wallets: in both cases there is nothing to promote here
    /// (a password wallet with no passphrase in hand is left for its unlock
    /// gesture, so this never forces a startup prompt), and a no-password
    /// wallet resolves prompt-free through the chokepoint's unprotected
    /// fast-path regardless.
    ///
    /// The seed is obtained ONLY by decrypting the stored envelope through the
    /// chokepoint — no parked seed is read, because an open `Wallet` parks none
    /// (R3). Shielded state is not warmed here: it is derived on the first
    /// shielded operation via the chokepoint.
    pub fn handle_wallet_unlocked(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        passphrase: Option<&str>,
    ) {
        let (seed_hash, uses_password) = match wallet.read() {
            Ok(guard) => (guard.seed_hash(), guard.uses_password),
            Err(_) => return,
        };

        // No-password wallets resolve prompt-free through the chokepoint's
        // unprotected fast-path; promoting them is unnecessary. A password
        // wallet with no passphrase in hand (cold boot) is left for its unlock
        // gesture so we never force a startup prompt.
        if !uses_password {
            return;
        }
        let Some(passphrase) = passphrase else {
            return;
        };

        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        let secret = platform_wallet_storage::secrets::SecretString::new(passphrase);
        match backend.secret_access().promote_hd_seed_with_passphrase(
            &seed_hash,
            Some(&secret),
            crate::wallet_backend::RememberPolicy::UntilAppClose,
        ) {
            Ok(()) => tracing::trace!(
                wallet = %hex::encode(seed_hash),
                "Verified-open seed promoted to the session cache on unlock"
            ),
            Err(error) => tracing::debug!(
                wallet = %hex::encode(seed_hash),
                %error,
                "Unlock seed promotion skipped"
            ),
        }
    }

    pub fn handle_wallet_locked(self: &Arc<Self>, _wallet: &Arc<RwLock<Wallet>>) {}

    /// Initialize shielded state for unlocked wallets that were skipped
    /// because the protocol version wasn't known at unlock time. Called when
    /// the protocol version first crosses the shielded threshold.
    ///
    /// Each init now derives the Orchard keys by pulling the seed just-in-time
    /// through the JIT chokepoint, which is async, so each candidate is
    /// initialized on a tracked subtask (mirroring [`Self::queue_shielded_sync`]).
    /// Only currently-open wallets are candidates, so a no-password wallet
    /// derives silently and a passphrase-protected wallet whose seed the user
    /// already remembered for the session resolves from the session cache
    /// without a surprise background prompt.
    pub(crate) fn init_missing_shielded_wallets(self: &Arc<Self>) {
        // Collect candidate seed hashes while holding locks, then release.
        let candidates: Vec<WalletSeedHash> = (|| {
            let wallets = self.wallets.read().ok()?;
            let existing = self.shielded_states.lock().ok()?;
            Some(
                wallets
                    .iter()
                    .filter(|(hash, wallet_arc)| {
                        !existing.contains_key(*hash)
                            && wallet_arc.read().ok().map(|w| w.is_open()).unwrap_or(false)
                    })
                    .map(|(hash, _)| *hash)
                    .collect(),
            )
        })()
        .unwrap_or_default();

        for seed_hash in candidates {
            let ctx = Arc::clone(self);
            self.subtasks
                .spawn_sync("shielded_init_after_protocol_update", async move {
                    let handle = tokio::runtime::Handle::current();
                    let _ = tokio::task::spawn_blocking(move || {
                        handle.block_on(async {
                            match ctx.initialize_shielded_wallet(seed_hash).await {
                                Ok(_) => {
                                    tracing::info!(
                                        seed = %hex::encode(seed_hash),
                                        "Shielded wallet initialized after protocol version update"
                                    );
                                    ctx.queue_shielded_sync(seed_hash);
                                }
                                Err(e) => tracing::debug!(
                                    seed = %hex::encode(seed_hash),
                                    error = %e,
                                    "Shielded wallet init failed after protocol version update"
                                ),
                            }
                        })
                    })
                    .await;
                });
        }
    }

    /// Queue async SyncNotes -> CheckNullifiers for an already-initialized
    /// shielded wallet. Tracked via `subtasks` so it participates in graceful
    /// shutdown and cancellation.
    ///
    /// Uses `spawn_blocking(block_on(...))` because the async methods on
    /// `Arc<Self>` produce futures that borrow `self`, which the compiler
    /// cannot prove are `'static` (rust-lang/rust#100013). The trampoline
    /// resolves the futures synchronously on a blocking thread, satisfying
    /// the `'static` bound required by `spawn_sync`.
    fn queue_shielded_sync(self: &Arc<Self>, seed_hash: WalletSeedHash) {
        let ctx = Arc::clone(self);
        self.subtasks.spawn_sync("shielded_sync", async move {
            let handle = tokio::runtime::Handle::current();
            let result = tokio::task::spawn_blocking(move || {
                handle.block_on(async {
                    match ctx.sync_shielded_notes(seed_hash).await {
                        Ok(_) => {
                            if let Err(e) = ctx.check_nullifiers_task(seed_hash).await {
                                tracing::debug!(
                                    seed = %hex::encode(seed_hash),
                                    error = %e,
                                    "Shielded nullifier check after init failed"
                                );
                            }
                        }
                        Err(e) => tracing::debug!(
                            seed = %hex::encode(seed_hash),
                            error = %e,
                            "Shielded note sync after init failed"
                        ),
                    }
                })
            })
            .await;
            if let Err(e) = result {
                tracing::debug!(
                    seed = %hex::encode(seed_hash),
                    error = %e,
                    "Shielded sync task panicked"
                );
            }
        });
    }

    /// Queue automatic discovery of identities derived from a wallet.
    /// Checks identity indices 0 through max_identity_index for existing identities on the network.
    pub fn queue_wallet_identity_discovery(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        max_identity_index: u32,
    ) {
        let ctx = Arc::clone(self);
        let wallet_clone = Arc::clone(wallet);
        self.subtasks
            .spawn_sync("wallet_identity_discovery", async move {
                if let Err(error) = ctx
                    .discover_identities_from_wallet(&wallet_clone, max_identity_index)
                    .await
                {
                    tracing::warn!(
                        %error,
                        "Failed to discover identities from wallet"
                    );
                }
            });
    }

    pub async fn bootstrap_loaded_wallets(self: &Arc<Self>) {
        let wallets: Vec<_> = {
            let guard = self.wallets.read().unwrap();
            guard.values().cloned().collect()
        };

        for wallet in wallets.iter() {
            // Cold boot has no passphrase in hand, so this is a no-op for
            // password wallets (they wait for their unlock gesture) and is
            // unnecessary for no-password wallets (the chokepoint's unprotected
            // fast-path covers them). Kept for the promote-before-bootstrap
            // ordering invariant: a seed already in the session cache resolves
            // the JIT bootstrap below without a prompt.
            self.handle_wallet_unlocked(wallet, None);
            self.bootstrap_wallet_addresses_jit(wallet).await;
        }
    }

    /// Update wallet platform address info from SDK-returned AddressInfos.
    /// This uses the proof-verified data from SDK operations rather than fetching.
    pub(crate) fn update_wallet_platform_address_info_from_sdk(
        &self,
        seed_hash: WalletSeedHash,
        address_infos: &dash_sdk::query_types::AddressInfos,
    ) -> Result<(), TaskError> {
        let wallet_arc = {
            let wallets = self.wallets.read()?;
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or(TaskError::WalletNotFound)?
        };

        let mut wallet = wallet_arc.write()?;

        for (platform_addr, maybe_info) in address_infos.iter() {
            if let Some(info) = maybe_info {
                // Convert PlatformAddress to core Address using the network
                let core_addr = platform_addr.to_address_with_network(self.network);

                // Update in-memory wallet state
                wallet.set_platform_address_info(core_addr.clone(), info.balance, info.nonce);

                // Persist to per-wallet k/v
                if let Err(e) = AppContext::set_platform_address_info(
                    self,
                    &seed_hash,
                    &core_addr,
                    info.balance,
                    info.nonce,
                ) {
                    tracing::warn!("Failed to store Platform address info in k/v: {}", e);
                }

                tracing::debug!(
                    "Updated platform address {} balance={} nonce={} from SDK response",
                    core_addr,
                    info.balance,
                    info.nonce
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskResult;
    use crate::app_dir::ensure_env_file;
    use crate::context::AppContext;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;

    /// Build an offline `AppContext` for testnet in an isolated temp dir. No
    /// network I/O happens at construction: the SDK and Core client are built
    /// from bundled `.env` addresses but connect lazily. The `TempDir` must
    /// outlive the context — its drop deletes the data dir.
    fn offline_testnet_context() -> (Arc<AppContext>, SenderAsync<TaskResult>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let (ctx, sender) = offline_testnet_context_at(temp_dir.path());
        (ctx, sender, temp_dir)
    }

    /// Build an offline testnet `AppContext` rooted at an existing `data_dir`.
    /// Splitting this out lets a test build a second, independent context over
    /// the *same* on-disk sidecars to simulate a process restart (cold boot).
    fn offline_testnet_context_at(
        data_dir: &std::path::Path,
    ) -> (Arc<AppContext>, SenderAsync<TaskResult>) {
        let data_dir = data_dir.to_path_buf();
        ensure_env_file(&data_dir);

        let db = Arc::new(
            create_database_at_path(&data_dir.join("data.db")).expect("create test database"),
        );
        let subtasks = Arc::new(TaskManager::new());
        let connection_status = Arc::new(ConnectionStatus::new());
        let egui_ctx = egui::Context::default();
        let app_kv = AppContext::open_app_kv(&data_dir).expect("open app k/v");

        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            subtasks,
            connection_status,
            egui_ctx,
            app_kv,
        )
        .expect("AppContext::new should succeed offline with bundled testnet config");

        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        (ctx, sender)
    }

    /// Before the wallet seam is wired, `start_spv()` must fail fast with the
    /// typed `WalletBackendNotYetWired` rather than silently swallowing the
    /// request. This is the gate the speculative pre-wire callers were tripping.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_spv_errors_when_backend_not_wired() {
        let (ctx, _sender, _tmp) = offline_testnet_context();

        assert!(
            ctx.wallet_backend().is_err(),
            "precondition: backend must be unwired before ensure_wallet_backend"
        );
        let err = ctx
            .start_spv()
            .expect_err("start_spv must fail before the backend is wired");
        assert!(
            matches!(err, TaskError::WalletBackendNotYetWired),
            "expected WalletBackendNotYetWired, got: {err:?}"
        );
    }

    /// After wiring the backend, the synchronous gate is gone: `start_spv()`
    /// returns `Ok` and the backend's start latch flips to started once the
    /// spawned start runs. Mirrors the production "start on wiring completion"
    /// path without faking a sync loop — the upstream run loop is shut down
    /// immediately afterwards so the test leaves no detached network task.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_spv_starts_after_backend_wired() {
        let (ctx, sender, _tmp) = offline_testnet_context();

        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx
            .wallet_backend()
            .expect("backend must be wired after ensure_wallet_backend");
        assert!(
            !backend.is_started(),
            "wiring alone must not start chain sync"
        );

        ctx.start_spv()
            .expect("start_spv must not error synchronously once the backend is wired");

        // The spawned start flips the latch synchronously at its head; poll
        // with a bounded timeout so the test never hangs if the runtime is busy.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while !backend.is_started() {
            if tokio::time::Instant::now() >= deadline {
                panic!("backend.start() did not run within the timeout");
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        backend.shutdown().await;
    }

    /// The async chokepoint wires the backend and starts chain sync in one call,
    /// so a caller need not have wired the backend beforehand. Pins the
    /// "ensure-then-start" sequencing the GUI/MCP/network-switch paths share.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ensure_wallet_backend_and_start_spv_wires_then_starts() {
        let (ctx, sender, _tmp) = offline_testnet_context();

        assert!(
            ctx.wallet_backend().is_err(),
            "precondition: backend unwired before the chokepoint"
        );

        ctx.ensure_wallet_backend_and_start_spv(sender)
            .await
            .expect("chokepoint should wire then start offline");

        let backend = ctx
            .wallet_backend()
            .expect("backend must be wired after the chokepoint");
        assert!(
            backend.is_started(),
            "chokepoint must have started chain sync"
        );

        backend.shutdown().await;
    }

    /// QA-007: a failure at the (fallible) wiring step must surface — the
    /// chokepoint returns `Err` AND flips the SPV indicator to `Error`, so the
    /// user does not silently fall back to `Disconnected` with no feedback.
    ///
    /// Induces the wiring failure offline by planting a regular file where the
    /// per-network SPV storage directory would be created: `WalletBackend::new`
    /// calls `create_dir_all(data_dir/spv/testnet)`, which cannot succeed when a
    /// path component (`spv`) is a file rather than a directory.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn chokepoint_wiring_failure_flips_indicator_to_error() {
        let (ctx, sender, _tmp) = offline_testnet_context();

        // Block the SPV storage dir creation: a file at `data_dir/spv` makes
        // `create_dir_all(.../spv/testnet)` fail deterministically (no reliance
        // on filesystem permissions, which root can bypass in CI).
        std::fs::write(ctx.data_dir().join("spv"), b"not a directory")
            .expect("plant blocking file at the spv path");

        assert_ne!(
            ctx.connection_status.spv_status(),
            SpvStatus::Error,
            "precondition: indicator must not already be in the Error state"
        );

        let err = ctx
            .ensure_wallet_backend_and_start_spv(sender)
            .await
            .expect_err("wiring must fail when the spv path is blocked by a file");
        assert!(
            matches!(err, TaskError::FileSystem { .. }),
            "expected a FileSystem wiring error, got: {err:?}"
        );

        assert_eq!(
            ctx.connection_status.spv_status(),
            SpvStatus::Error,
            "wiring failure must flip the SPV indicator to Error"
        );
    }

    /// SEC-001/SEC-002 regression, adapted to the JIT secret model: a
    /// no-password wallet must remain signable after a cold-boot hydration
    /// without any seed ever being parked in a long-lived cache.
    ///
    /// Under the JIT chokepoint there is no `inner.seeds` cache to fill or
    /// clear; signing decrypts the seed just-in-time from the encrypted vault
    /// envelope. For a no-password wallet (`uses_password = false`) the
    /// chokepoint's unprotected fast-path decrypts with **no passphrase and no
    /// prompt** — so the wallet signs whether or not the session cache holds
    /// it. This test proves that:
    ///   1. a freshly-registered no-password wallet signs in-process; and
    ///   2. after `forget_all_secrets()` wipes the session cache (the exact
    ///      state a real cold-boot leaves: watch-only, nothing remembered) the
    ///      wallet STILL signs — the seed is pulled from the vault on demand.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_password_wallet_resignable_via_unlock_chokepoint() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let seed = [0x24u8; 64];
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("cold-boot".to_string()),
            None, // no password
        )
        .expect("build no-password wallet");
        assert!(wallet.is_open(), "a no-password wallet is open on creation");

        let (seed_hash, wallet_arc) = ctx.register_wallet(wallet, &seed).expect("register wallet");
        let backend = ctx.wallet_backend().expect("backend wired");

        // Live (same-process) state: registration wrote the seed envelope to
        // the vault, so the chokepoint can decrypt the no-password seed.
        backend
            .assert_can_sign(&seed_hash)
            .await
            .expect("freshly-registered no-password wallet must sign in-process");

        // Simulate the seedless cold-boot state: wipe the session cache so
        // nothing is remembered (what hydration leaves behind). The wallet is
        // still `Open` for display, but no plaintext seed is cached anywhere.
        backend.forget_all_secrets();
        assert!(
            wallet_arc.read().unwrap().is_open(),
            "the wallet is still Open after the session cache is dropped"
        );

        // The JIT guarantee: a no-password wallet signs from the vault with no
        // prompt and no cache — the unprotected fast-path covers it.
        backend
            .assert_can_sign(&seed_hash)
            .await
            .expect("no-password wallet must sign after cold-boot via the JIT fast-path");

        backend.shutdown().await;
    }

    /// QA-007: leaving a network must not strand session-cached secrets on the
    /// outgoing context. `finalize_network_switch` funnels through
    /// [`WalletBackend::forget_all_secrets`]; this exercises that exact call
    /// against a populated session cache and asserts it is emptied — the JIT
    /// design's eager "no secrets linger across a network change" guarantee.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn network_switch_path_clears_outgoing_session_cache() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let seed = [0x31u8; 64];
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("switching".to_string()),
            None,
        )
        .expect("build wallet");
        let (seed_hash, _wallet_arc) =
            ctx.register_wallet(wallet, &seed).expect("register wallet");

        let backend = ctx.wallet_backend().expect("backend wired");
        let scope = crate::wallet_backend::SecretScope::HdSeed { seed_hash };

        // Promote the seed into the session cache (what the unlock gesture or a
        // remembered op leaves behind).
        let held = zeroize::Zeroizing::new(seed);
        backend.secret_access().remember_session(
            &scope,
            crate::wallet_backend::SecretPlaintext::HdSeed(&held),
            crate::wallet_backend::RememberPolicy::UntilAppClose,
        );
        assert!(
            backend.secret_access().is_session_cached(&scope),
            "precondition: the seed is session-cached before the switch"
        );

        // The exact call `finalize_network_switch` makes on the outgoing
        // context before leaving it.
        backend.forget_all_secrets();

        assert!(
            !backend.secret_access().is_session_cached(&scope),
            "the outgoing context's session cache must be empty after the switch path runs"
        );

        backend.shutdown().await;
    }
}
