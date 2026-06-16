use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::spv_status::SpvStatus;
use crate::model::wallet::birth_height::{WalletOrigin, registration_birth_height};
use crate::model::wallet::meta::WalletMeta;
use crate::model::wallet::seed_envelope::StoredSeedEnvelope;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::wallet_backend::{DetScope, WalletBackend, WalletMetaView, WalletSeedView};
use dash_sdk::dpp::dashcore::Network;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

/// Number of identity-authentication keys warmed per known identity index
/// during the JIT bootstrap (D4b). Matches the readers' auth-key lookup
/// window so the common identity-load path serves entirely from cache.
const AUTH_PUBKEY_WARM_KEY_COUNT: u32 = 12;

/// The upstream `dash-spv` `DiskStorageManager` chain-cache entries under the
/// per-network SPV directory. Each is a subfolder except `peers.dat`. The
/// wallet/shielded SQLite sidecars in the same directory are deliberately
/// excluded — clearing the chain cache must not touch funds or secrets.
const SPV_CHAIN_STORAGE_ENTRIES: [&str; 7] = [
    "block_headers",
    "filter_headers",
    "filters",
    "blocks",
    "metadata",
    "masternodestate",
    "peers.dat",
];

/// Per-network SPV storage directory: `<data_dir>/spv/<network>/`. Mirrors
/// `WalletBackend::resolve_spv_storage_dir` so the path resolves identically
/// whether or not the wallet backend is wired yet.
fn spv_storage_dir(data_dir: &Path, network: Network) -> PathBuf {
    let segment = match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    };
    data_dir.join("spv").join(segment)
}

/// Remove the upstream chain-sync cache files under `spv_dir`, leaving the
/// wallet (`platform-wallet.sqlite`) and shielded sidecars untouched. The
/// `DiskStorageManager` lock lives at `<spv_dir>.lock` (a sibling of the
/// directory); it is removed too so a stale lock cannot block the next sync.
/// A missing entry is the expected fresh/never-synced state and is tolerated.
fn clear_spv_chain_storage(spv_dir: &Path) -> Result<(), TaskError> {
    for entry in SPV_CHAIN_STORAGE_ENTRIES {
        let path = spv_dir.join(entry);
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(e) = result
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(TaskError::FileSystem { source: e });
        }
    }

    let lock_path = spv_dir.with_extension("lock");
    if let Err(e) = std::fs::remove_file(&lock_path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(TaskError::FileSystem { source: e });
    }

    Ok(())
}

impl AppContext {
    /// Delete the cached chain-sync data (headers, filters, blocks, masternode
    /// state, peers) for this network so the next connection re-syncs from
    /// scratch.
    ///
    /// Only the upstream `dash-spv` `DiskStorageManager` files under the
    /// per-network SPV directory are removed; the wallet state
    /// (`platform-wallet.sqlite`) and the shielded commitment tree are left
    /// intact — clearing the chain cache must never touch funds or secrets. The
    /// "Clear SPV Data" control is enabled only while sync is stopped, so the
    /// `DiskStorageManager` has released its file lock and the deletes do not
    /// race a live writer. A missing directory (never synced) is success.
    pub fn clear_spv_data(&self) -> Result<(), TaskError> {
        let spv_dir = spv_storage_dir(&self.data_dir, self.network);
        clear_spv_chain_storage(&spv_dir)
    }

    pub fn clear_network_database(self: &Arc<Self>) -> Result<(), TaskError> {
        self.db.clear_network_data(self.network)?;

        // F60: permanently delete every wallet's secret-bearing state so the
        // "delete all local data" promise holds — wallets must NOT rehydrate
        // on next launch and encrypted seeds must NOT persist. Clear the
        // persisted state (seed-envelope vault, wallet-meta + single-key
        // sidecars, shielded notes, session cache) BEFORE the in-memory maps
        // below, so a mid-failure crash cannot strand a recoverable seed. The
        // upstream (watch-only) persistor rows have no seed and are removed
        // asynchronously off the main thread. Best-effort when the backend is
        // not wired yet — there is no such state in that case.
        if let Ok(backend) = self.wallet_backend() {
            let upstream_ids = backend.forget_all_wallets_local();
            for wallet_id in upstream_ids {
                let backend = Arc::clone(&backend);
                self.subtasks
                    .spawn_sync("wallet_upstream_removal", async move {
                        if let Err(error) = backend.remove_upstream_wallet(&wallet_id).await {
                            tracing::warn!(%error, "Upstream wallet removal failed during clear");
                        }
                    });
            }
        }

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
        &self,
        address: &str,
        passphrase: &str,
    ) -> Result<(), TaskError> {
        self.wallet_backend()?
            .single_key()
            .verify_passphrase(address, passphrase)
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

    /// Stop chain sync and drop the wired wallet backend so the next Connect
    /// rebuilds it from a clean slate.
    ///
    /// This is the disconnect counterpart to
    /// [`Self::ensure_wallet_backend_and_start_spv`] and the single chokepoint
    /// for "stop SPV". The sequence is:
    ///
    /// 1. Flip the SPV indicator to [`SpvStatus::Stopping`] so the UI shows
    ///    "Disconnecting…" immediately, before the async teardown runs.
    /// 2. Shut the wallet backend down ([`WalletBackend::shutdown`]), stopping
    ///    the upstream chain-sync run loop and the periodic coordinators.
    /// 3. Unwire the backend. Its start latch is one-shot, so the dropped
    ///    instance could never restart sync — the next Connect calls
    ///    [`Self::ensure_wallet_backend_and_start_spv`], which rebuilds a fresh
    ///    backend with a fresh latch.
    /// 4. Flip the indicator to [`SpvStatus::Stopped`] and clear the live peer
    ///    count, sync progress, and last error, then recompute the overall
    ///    state — which lands on `Disconnected` now that SPV is inactive.
    ///
    /// Idempotent: a call with no wired backend still settles the indicator on
    /// `Stopped`/`Disconnected`. The teardown is async (upstream `shutdown` is
    /// async), so GUI callers dispatch this via `AppAction::StopSpv` rather than
    /// blocking the frame loop. That dispatch claims the stop synchronously with
    /// [`ConnectionStatus::begin_spv_stop`](crate::context::connection_status::ConnectionStatus::begin_spv_stop)
    /// (button disables on the click frame, second click deduped); the redundant
    /// `Stopping` flip here keeps direct callers self-contained.
    pub async fn stop_spv(self: &Arc<Self>) {
        self.connection_status.set_spv_status(SpvStatus::Stopping);
        self.connection_status.refresh_state();

        if let Some(backend) = self.take_wallet_backend() {
            backend.shutdown().await;
        }

        self.connection_status.set_spv_status(SpvStatus::Stopped);
        self.connection_status.set_spv_connected_peers(0);
        self.connection_status.set_spv_sync_progress(None);
        self.connection_status.set_spv_last_error(None);
        // Re-arm the quorum gate: the next reconnect builds a fresh backend
        // whose SPV session must re-sync the masternode list. Leaving the flag
        // set would let early proof calls through before quorums exist again,
        // re-triggering the DAPI self-ban storm.
        self.connection_status.set_masternodes_ready(false);
        self.connection_status.refresh_state();
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
    /// registration are still found (PROJ-010).
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
        // are watched and received funds become visible (W1; PROJ-010). The
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
        self.subtasks
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
    /// boot, so it succeeds even before the wallet backend is wired (PROJ-010):
    /// the backend, once built, reuses the very same vault handle.
    fn write_seed_envelope(&self, wallet: &Wallet) -> Result<(), TaskError> {
        let seed_hash = wallet.seed_hash();
        let envelope = StoredSeedEnvelope {
            encrypted_seed: wallet.encrypted_seed_slice().to_vec(),
            salt: wallet.salt().to_vec(),
            nonce: wallet.nonce().to_vec(),
            password_hint: wallet.password_hint().clone(),
            uses_password: wallet.uses_password,
            xpub_encoded: wallet
                .master_bip44_ecdsa_extended_public_key
                .encode()
                .to_vec(),
        };
        WalletSeedView::new(&self.secret_store).set(&seed_hash, &envelope)
    }

    /// Persist a newly-registered wallet's metadata (alias / is_main /
    /// core_wallet_name + master xpub) to the wallet-meta sidecar.
    /// **Fail-closed** (SEC-002): cold-boot hydration enumerates ONLY this
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
        };
        WalletMetaView::new(&self.app_kv).set(self.network, &seed_hash, &meta)
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
    ///
    /// This is also the W2 cold-boot reconciliation point (PROJ-010): inside
    /// the same prompt-free seed scope it registers any wallet present in DET
    /// sidecars but absent from the upstream SPV persistor (migrated installs,
    /// wallets created before the fix, post-reset), so received funds become
    /// visible without a launch-time password prompt. Registration is
    /// independent of address bootstrap: an already-bootstrapped wallet that
    /// was never registered upstream still gets registered here.
    pub async fn bootstrap_wallet_addresses_jit(&self, wallet: &Arc<RwLock<Wallet>>) {
        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        let seed_hash = {
            let Ok(guard) = wallet.read() else {
                return;
            };
            // Gate on the open seed being resolvable prompt-free: an open
            // wallet at cold boot is either unprotected (no-prompt fast-path) or
            // already session-cached via the unlock gesture. A locked protected
            // wallet is skipped to avoid a surprise startup prompt.
            if !guard.is_open() {
                return;
            }
            guard.seed_hash()
        };

        // Enter the seed scope when there is any seed-dependent work to do:
        // address bootstrap OR upstream registration. A fully-bootstrapped,
        // already-registered wallet needs neither and is skipped without
        // touching the vault.
        let needs_bootstrap = wallet
            .read()
            .map(|g| Self::wallet_needs_bootstrap(&g))
            .unwrap_or(false);
        let needs_registration = !backend.is_wallet_registered(&seed_hash);
        if !needs_bootstrap && !needs_registration {
            return;
        }

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
                    // W2 cold-boot reconciliation: register with the upstream
                    // SPV backend if this wallet is not yet known to it, using
                    // the seed already open in this scope. Idempotent and
                    // genesis-floored so pre-existing deposits are found
                    // (PROJ-010). Best-effort — a failure is retried on the
                    // next boot.
                    if let Err(error) = backend.ensure_upstream_registered(&seed_hash, seed).await {
                        tracing::warn!(
                            wallet = %hex::encode(seed_hash),
                            %error,
                            "W2 upstream registration failed; will retry at next cold boot"
                        );
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

        // W2 reconciliation on the unlock gesture (PROJ-010). A
        // password-protected wallet hydrates `Closed` at cold boot, so
        // `bootstrap_wallet_addresses_jit` skips it (no surprise startup prompt)
        // and it is never upstream-registered until the seed becomes available.
        // The unlock just verified the passphrase and promoted the seed into the
        // session cache above, so re-driving the JIT bootstrap now registers the
        // wallet with the upstream SPV backend without a second prompt — the
        // difference between the wallet being usable this session and a
        // `WalletNotLoaded` until the next launch. Idempotent (an
        // already-registered wallet is a no-op) and resolved prompt-free from the
        // session cache. The in-memory wallet is already flipped `Open` by the
        // unlock callsite before this runs, so the JIT `is_open()` gate passes.
        self.drive_unlock_registration(wallet);
    }

    /// Spawn the unlock-triggered JIT bootstrap/registration for a wallet whose
    /// seed was just promoted to the session cache by [`Self::handle_wallet_unlocked`].
    ///
    /// `handle_wallet_unlocked` is synchronous (called from the UI thread) while
    /// [`Self::bootstrap_wallet_addresses_jit`] is async, so the reconciliation
    /// runs on a tracked subtask — mirroring [`Self::register_wallet_upstream`].
    /// Best-effort: the JIT bootstrap logs and swallows its own failures, and a
    /// missing-backend cold-boot path is covered by `bootstrap_loaded_wallets`.
    fn drive_unlock_registration(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>) {
        let ctx = Arc::clone(self);
        let wallet = Arc::clone(wallet);
        self.subtasks
            .spawn_sync("wallet_unlock_registration", async move {
                ctx.bootstrap_wallet_addresses_jit(&wallet).await;
            });
    }

    /// Wipe the session-cached seed when a wallet is locked.
    ///
    /// Unlock promotes the seed into the JIT session cache with
    /// [`RememberPolicy::UntilAppClose`](crate::wallet_backend::RememberPolicy),
    /// and signing resolves cache-first — so without this the wallet would keep
    /// signing prompt-free after a "lock", leaving plaintext seed bytes
    /// resident. Forgetting the seed's scope here restores the locked
    /// guarantee: the next signing op re-prompts (or, for a no-password wallet,
    /// resolves through the chokepoint's unprotected fast-path). Mirrors
    /// [`Self::handle_wallet_unlocked`]'s promotion, in reverse.
    pub fn handle_wallet_locked(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>) {
        let Ok(seed_hash) = wallet.read().map(|guard| guard.seed_hash()) else {
            return;
        };
        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        backend
            .secret_access()
            .forget(&crate::wallet_backend::SecretScope::HdSeed { seed_hash });
        tracing::trace!(
            wallet = %hex::encode(seed_hash),
            "Session-cached seed wiped on wallet lock"
        );
    }

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
        let db = Arc::new(
            create_database_at_path(&data_dir.join("data.db")).expect("create test database"),
        );
        offline_testnet_context_with_db(data_dir, db)
    }

    /// Build an offline testnet `AppContext` whose `data.db` went through the
    /// **real** `Database::initialize` fresh-install path (the path production
    /// uses at `app.rs:322`), which gates the legacy `wallet`/`wallet_addresses`
    /// tables OUT. Use this for fresh-install regression tests; the default
    /// helper force-creates those tables via `create_tables(true)`.
    fn offline_testnet_context_fresh_init(
        data_dir: &std::path::Path,
    ) -> (Arc<AppContext>, SenderAsync<TaskResult>) {
        let db_file = data_dir.join("data.db");
        let db = crate::database::Database::new(&db_file).expect("create fresh test database");
        db.initialize(&db_file)
            .expect("fresh Database::initialize should succeed");
        offline_testnet_context_with_db(data_dir, Arc::new(db))
    }

    fn offline_testnet_context_with_db(
        data_dir: &std::path::Path,
        db: Arc<crate::database::Database>,
    ) -> (Arc<AppContext>, SenderAsync<TaskResult>) {
        let data_dir = data_dir.to_path_buf();
        ensure_env_file(&data_dir);

        let subtasks = Arc::new(TaskManager::new());
        let connection_status = Arc::new(ConnectionStatus::new());
        let egui_ctx = egui::Context::default();
        let app_kv = AppContext::open_app_kv(&data_dir).expect("open app k/v");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("open secret store");

        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            subtasks,
            connection_status,
            egui_ctx,
            app_kv,
            secret_store,
        )
        .expect("AppContext::new should succeed offline with bundled testnet config");

        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        (ctx, sender)
    }

    /// Process-global serialization lock for tests that tear a wallet backend
    /// down and immediately rebuild it over the *same* on-disk path. The
    /// upstream persister enforces a single open per `platform-wallet.sqlite`
    /// (`WalletStorageError::AlreadyOpen`); a bootstrap subtask spawned by
    /// `ensure_wallet_backend` may keep its `Arc<WalletBackend>` — and that
    /// open's advisory lock — alive a beat past `stop_spv`, so under parallel
    /// scheduling the reopen can lose the race. Serializing these reopen tests
    /// removes the scheduler pressure so the lingering subtask drops the old
    /// handle before the reopen. Mirrors `support::data_dir_lock` in the
    /// kittest suite. Held across awaits, hence a `tokio::sync::Mutex`.
    async fn backend_reopen_lock() -> tokio::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await
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

    /// The Disconnect chokepoint must produce a *visible* state change: after a
    /// successful start, `stop_spv` unwires the backend and settles the
    /// indicator on `Stopped` / `Disconnected`. Regression guard ensuring the
    /// Disconnect button drives the overall state out of its active value.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_spv_unwires_backend_and_disconnects_indicator() {
        use crate::context::connection_status::OverallConnectionState;

        let (ctx, sender, _tmp) = offline_testnet_context();

        ctx.ensure_wallet_backend_and_start_spv(sender)
            .await
            .expect("chokepoint should wire then start offline");
        assert!(
            ctx.wallet_backend().is_ok(),
            "precondition: backend wired after start"
        );
        // Simulate a session that reached quorum readiness, so the disconnect
        // has a flag to re-arm.
        ctx.connection_status().set_masternodes_ready(true);

        ctx.stop_spv().await;

        assert!(
            ctx.wallet_backend().is_err(),
            "stop_spv must unwire the backend so the next Connect rebuilds it"
        );
        assert!(
            !ctx.connection_status().masternodes_ready(),
            "stop_spv must re-arm the quorum gate so the next reconnect waits for masternode re-sync"
        );
        assert_eq!(
            ctx.connection_status().spv_status(),
            SpvStatus::Stopped,
            "stop_spv must leave the SPV indicator Stopped"
        );
        assert_eq!(
            ctx.connection_status().overall_state(),
            OverallConnectionState::Disconnected,
            "stop_spv must leave the overall state Disconnected"
        );
        assert_eq!(
            ctx.connection_status().spv_connected_peers(),
            0,
            "stop_spv must clear the live peer count"
        );
    }

    /// `stop_spv` is idempotent: calling it with no wired backend must not panic
    /// and must still settle the indicator on `Stopped` / `Disconnected`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_spv_is_idempotent_without_a_wired_backend() {
        use crate::context::connection_status::OverallConnectionState;

        let (ctx, _sender, _tmp) = offline_testnet_context();
        assert!(
            ctx.wallet_backend().is_err(),
            "precondition: backend unwired"
        );

        ctx.stop_spv().await;

        assert_eq!(ctx.connection_status().spv_status(), SpvStatus::Stopped);
        assert_eq!(
            ctx.connection_status().overall_state(),
            OverallConnectionState::Disconnected
        );
    }

    /// Reconnect round trip: start → `stop_spv` → restart must rebuild a *fresh*
    /// backend and restart chain sync, proving the disconnect leaves the system
    /// in a reconnectable state (the one-shot start latch does not strand it).
    ///
    /// Offline scope: this asserts the deterministic rebuild + rewire + restart
    /// — a new backend instance, wired again, with `is_started()` set on the new
    /// instance (its fresh latch fired). The indicator's onward transition to
    /// `Syncing`/`Running` is network-driven (pushed by the `EventBridge` from
    /// live SPV events) and so is exercised by the backend-e2e suite, not here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reconnect_after_stop_rebuilds_fresh_backend_and_restarts() {
        use crate::context::connection_status::OverallConnectionState;

        // Serialize this reopen-same-path test against any sibling that races on
        // the upstream single-open advisory lock; see `backend_reopen_lock`.
        let _reopen_guard = backend_reopen_lock().await;

        let (ctx, sender, _tmp) = offline_testnet_context();

        ctx.ensure_wallet_backend_and_start_spv(sender.clone())
            .await
            .expect("initial start should wire then start offline");
        let first = ctx.wallet_backend().expect("backend wired after start");
        assert!(first.is_started(), "initial start must latch the backend");
        // Capture the old backend's identity (raw pointer) and a weak handle,
        // then release the strong ref before reconnecting. The upstream
        // persister enforces a single open per path
        // (WalletStorageError::AlreadyOpen); a lingering strong ref — `first`
        // here, or a clone held by the upstream run-loop subtask — keeps the old
        // handle's advisory lock alive, so the reconnect's open of the same path
        // would be refused. The fresh-backend identity check below uses the raw
        // pointer; the weak handle lets us prove the old backend is fully torn
        // down before reopening.
        let first_ptr = Arc::as_ptr(&first);
        let first_weak = Arc::downgrade(&first);
        drop(first);

        ctx.stop_spv().await;
        assert!(
            ctx.wallet_backend().is_err(),
            "precondition: stop_spv unwired the backend"
        );
        assert_eq!(
            ctx.connection_status().overall_state(),
            OverallConnectionState::Disconnected,
            "precondition: disconnected before reconnect"
        );

        // Deterministically wait for the last strong ref to drop: `stop_spv`
        // awaits the backend's own shutdown, but a background subtask (the
        // upstream run loop) may briefly outlive that await still holding a
        // backend clone, and with it the SQLite advisory lock. Block the reopen
        // until that clone is gone so the reconnect never races into AlreadyOpen.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while first_weak.strong_count() > 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "old backend was not torn down within the timeout; a subtask still holds it"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        ctx.ensure_wallet_backend_and_start_spv(sender)
            .await
            .expect("reconnect should wire then start a fresh backend offline");

        let second = ctx
            .wallet_backend()
            .expect("backend must be wired again after reconnect");
        assert!(
            first_ptr != Arc::as_ptr(&second),
            "reconnect must rebuild a fresh backend, not revive the dropped one"
        );
        assert!(
            second.is_started(),
            "reconnect must restart chain sync on the fresh backend's latch"
        );

        second.shutdown().await;
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

        let (seed_hash, wallet_arc) = ctx
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");
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
        let (seed_hash, _wallet_arc) = ctx
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");

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

    /// PROJ-010 (W1 idempotency): registering the same wallet twice with the
    /// upstream backend is a no-op the second time — the wallet is watched once,
    /// never double-watched. The pre-fix bug was the *opposite* (a never-watched
    /// wallet); this pins that the new writer is also safe to call repeatedly,
    /// as both W1 (create/import) and W2 (cold-boot) may fire for one wallet in
    /// a single session.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn register_wallet_from_seed_is_idempotent() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");

        let seed = [0x5Au8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();

        assert!(
            !backend.is_wallet_registered(&seed_hash),
            "precondition: wallet must not be registered before the first call"
        );

        backend
            .register_wallet_from_seed(&seed_hash, &seed, Some(0))
            .await
            .expect("first registration must succeed");
        assert!(
            backend.is_wallet_registered(&seed_hash),
            "the wallet must be registered after the first call"
        );
        assert_eq!(
            backend.wallet_count().await,
            1,
            "exactly one wallet is watched after the first registration"
        );

        // Second call: idempotent no-op, no double-watch.
        backend
            .register_wallet_from_seed(&seed_hash, &seed, Some(0))
            .await
            .expect("second registration must be a no-op, not an error");
        assert_eq!(
            backend.wallet_count().await,
            1,
            "a repeat registration must not double-watch the wallet"
        );

        backend.shutdown().await;
    }

    /// Regression guard for issue #7 (now PASSES — was the bug reproducer).
    ///
    /// Before the upstream fix (platform PR #3828), `WalletAccountCreationOptions::Default`
    /// created BOTH a BIP32 account-0 (`m/0'`, depth-1) and a BIP44 account-0
    /// (`m/44'/coin'/0'`, depth-3), but the persistor collapsed both
    /// `StandardAccountType` variants to the single `account_type` label
    /// `"standard"`. They shared the `account_registrations` primary key
    /// `(wallet_id, account_type, account_index)`, so the BIP32 row overwrote the
    /// BIP44 row via `ON CONFLICT DO UPDATE`. The seedless cold-boot reload then
    /// read back the depth-1 xpub, it matched no DET sidecar bridge entry, and the
    /// fund-routing gate rejected every wallet -> systematic WalletNotLoaded.
    ///
    /// The fix distinguishes the two standard accounts in the persistor key:
    /// the label is now `"standard_bip44"` vs `"standard_bip32"`, so both rows
    /// coexist and the BIP44 depth-3 xpub survives alongside the BIP32 one.
    /// This guard asserts the post-fix invariant: a current-binary wallet
    /// survives create -> persist -> real `load_from_persistor_seedless` -> gate,
    /// BOTH standard rows persist, and the stored BIP44 xpub matches the bridge.
    ///
    /// It inspects the persistor `account_registrations` directly (a read-only
    /// rusqlite connection) rather than reopening an AppContext, because the
    /// offline harness can't release the shared `app_kv` advisory lock to reopen.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn issue7_fresh_persistor_bip44_xpub_matches_det_bridge() {
        let _serialize = backend_reopen_lock().await;
        let temp_dir = tempfile::tempdir().expect("tempdir");

        let seed = [0x71u8; 64];
        let (seed_hash, meta_xpub) = {
            // ---- First boot: create + register through the full W1 path ----
            let (ctx, sender) = offline_testnet_context_at(temp_dir.path());
            ctx.ensure_wallet_backend(sender)
                .await
                .expect("ensure_wallet_backend (first boot)");
            let backend = ctx.wallet_backend().expect("backend wired (first boot)");

            let wallet =
                crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                    .expect("build wallet");
            let seed_hash = wallet.seed_hash();
            let det_master_bip44 = wallet.master_bip44_ecdsa_extended_public_key;

            // Write the wallet-meta sidecar (the seedless bridge key) DIRECTLY —
            // avoid `register_wallet`, which spawns an upstream-registration
            // subtask that keeps an `Arc<WalletBackend>` (and the shared app_kv
            // handle) alive and blocks the cold-boot reopen below.
            backend
                .wallet_meta()
                .set(
                    Network::Testnet,
                    &seed_hash,
                    &crate::model::wallet::meta::WalletMeta {
                        alias: String::new(),
                        is_main: false,
                        core_wallet_name: None,
                        xpub_encoded: det_master_bip44.encode().to_vec(),
                    },
                )
                .expect("write wallet-meta sidecar");

            // W1 upstream registration via the REAL create_wallet_from_seed_bytes
            // writer (awaited, no spawn). Confirms the FRESH in-memory create
            // resolves through the gate.
            backend
                .register_wallet_from_seed(&seed_hash, &seed, Some(0))
                .await
                .expect("W1 upstream registration must succeed on first boot");
            let registered_first_boot = backend.is_wallet_registered(&seed_hash);
            eprintln!(
                "ISSUE7 first-boot: registered={} (fresh in-memory create through the gate)",
                registered_first_boot
            );
            assert!(
                registered_first_boot,
                "precondition: a fresh in-memory create must resolve through the gate"
            );
            let meta_xpub = det_master_bip44.encode().to_vec();

            backend.shutdown().await;
            // Drain ctx1's subtasks + drop everything so the persistor + app_kv
            // advisory locks release before the cold-boot reopen.
            let _ = ctx.subtasks.shutdown_async().await;
            drop(backend);
            drop(ctx);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            (seed_hash, meta_xpub)
        };

        // ---- COLD BOOT: real load_from_persistor_seedless over a COPY of the
        // on-disk state. This is the decisive cycle: a CURRENT-binary-written
        // wallet, reloaded through the actual upstream seedless path, run through
        // the SAME fund-routing gate. Does it survive create -> persist ->
        // load_from_persistor_seedless -> gate?
        //
        // The first context's app_kv/persistor advisory locks can linger
        // in-process (a lingering upstream subtask holds an `Arc<WalletBackend>`),
        // so instead of reopening the SAME dir we COPY the whole data dir to a
        // fresh path and cold-boot over the copy — fresh file handles, no lock
        // conflict, identical on-disk bytes (persistor + sidecar + vault). This
        // drives the genuine `load_from_persistor_seedless` (run inside
        // `WalletBackend::new`), not just the blob-decode equivalent.
        fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
            std::fs::create_dir_all(dst).expect("mkdir dst");
            for entry in std::fs::read_dir(src).expect("read_dir") {
                let entry = entry.expect("dir entry");
                let from = entry.path();
                let to = dst.join(entry.file_name());
                if from.is_dir() {
                    copy_dir_recursive(&from, &to);
                } else {
                    std::fs::copy(&from, &to).expect("copy file");
                }
            }
        }
        let cold_dir = tempfile::tempdir().expect("cold tempdir");
        copy_dir_recursive(temp_dir.path(), cold_dir.path());

        let cold_boot_registered = {
            let data_dir = cold_dir.path().to_path_buf();
            let app_kv = AppContext::open_app_kv(&data_dir).expect("cold-boot open app k/v");
            let secret_store =
                AppContext::open_secret_store(&data_dir).expect("cold-boot open secret store");
            let db = Arc::new(
                create_database_at_path(&data_dir.join("data.db")).expect("reopen test database"),
            );
            let ctx2 = AppContext::new(
                data_dir,
                Network::Testnet,
                db,
                Arc::new(TaskManager::new()),
                Arc::new(ConnectionStatus::new()),
                egui::Context::default(),
                app_kv,
                secret_store,
            )
            .expect("cold-boot AppContext::new");
            let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
            let sender2 = SenderAsync::new(tx, ctx2.egui_ctx().clone());
            // ensure_wallet_backend -> WalletBackend::new runs the real
            // load_from_persistor_seedless pass (builds the bridge from the
            // sidecar, loads the persistor, resolves via the fund-routing gate).
            ctx2.ensure_wallet_backend(sender2)
                .await
                .expect("ensure_wallet_backend (cold boot)");
            let backend2 = ctx2.wallet_backend().expect("backend wired (cold boot)");
            let registered = backend2.is_wallet_registered(&seed_hash);
            let watched = backend2.wallet_count().await;
            eprintln!(
                "ISSUE7 COLD-BOOT (real load_from_persistor_seedless): registered={registered} watched_count={watched}"
            );
            backend2.shutdown().await;
            let _ = ctx2.subtasks.shutdown_async().await;
            registered
        };
        let _ = seed_hash;

        // Inspect the persistor on disk directly (a fresh read-only rusqlite
        // connection; SQLite allows concurrent readers, so the lingering app_kv
        // handle on the *other* file does not block this). This shows exactly
        // what the seedless reload would read back for the BIP44 account-0 row —
        // the gate's "loaded" side — without needing a second AppContext.
        let persistor_path = temp_dir
            .path()
            .join("spv")
            .join("testnet")
            .join("platform-wallet.sqlite");
        eprintln!("ISSUE7 persistor exists={}", persistor_path.exists());
        let conn = rusqlite::Connection::open_with_flags(
            &persistor_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("open persistor read-only");
        let rows: Vec<(String, i64, Vec<u8>)> = conn
            .prepare(
                "SELECT account_type, account_index, account_xpub_bytes FROM account_registrations",
            )
            .expect("prepare")
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect();
        eprintln!("ISSUE7 account_registrations rows={}", rows.len());
        for (at, idx, blob) in &rows {
            eprintln!(
                "ISSUE7   row account_type={at:?} index={idx} blob_len={}",
                blob.len()
            );
        }
        eprintln!("ISSUE7 bridge meta_xpub_len={}", meta_xpub.len());

        // The seedless reload needs a BIP44 account-0 ("standard_bip44", 0) row
        // to rebuild the watch-only account the gate reads. If it's absent or
        // under a different key, the gate rejects every wallet on a fresh DB.
        // The label is "standard_bip44" (not the pre-fix "standard"): the fix
        // distinguishes the two StandardAccountType variants so the BIP44 row no
        // longer shares a primary key with — and is no longer overwritten by —
        // the BIP32 account-0 row.
        let bip44_0_blob = rows
            .iter()
            .find(|(at, idx, _)| at == "standard_bip44" && *idx == 0)
            .map(|(_, _, blob)| blob.clone());
        assert!(
            bip44_0_blob.is_some(),
            "ISSUE7: persistor has no BIP44 account-0 (standard_bip44,0) row after W1. rows={rows:?}"
        );

        // Coexistence guarantee (the heart of the fix): the BIP32 account-0 row
        // must ALSO survive — the collision used to drop one of the two. People
        // hold funds on the BIP32 m/0' account, so it must never be clobbered.
        let bip32_0_present = rows
            .iter()
            .any(|(at, idx, _)| at == "standard_bip32" && *idx == 0);
        assert!(
            bip32_0_present,
            "ISSUE7: persistor lost the BIP32 account-0 (standard_bip32,0) row — the collision fix must keep BOTH standard accounts. rows={rows:?}"
        );

        // THE GATE CHECK: decode the stored BIP44 account-0 row exactly as the
        // seedless reload does and compare its account_xpub.encode() to the
        // bridge's meta xpub. If these differ, the fund-routing gate rejects the
        // wallet on a fresh cold boot — the systematic WalletNotLoaded.
        {
            use platform_wallet::changeset::AccountRegistrationEntry;
            let blob = bip44_0_blob.unwrap();
            let cfg = bincode::config::standard();
            let (entry, _): (AccountRegistrationEntry, usize) =
                bincode::serde::decode_from_slice(&blob, cfg).expect("decode stored entry");
            let stored = entry.account_xpub;
            let stored_xpub_encoded = stored.encode().to_vec();
            eprintln!(
                "ISSUE7 GATE: stored_xpub_len={} bridge_xpub_len={} EQ={}",
                stored_xpub_encoded.len(),
                meta_xpub.len(),
                stored_xpub_encoded == meta_xpub
            );
            // FIELD-LEVEL DIFF (the task's exact ask): decode the bridge xpub
            // too and compare every BIP32 field, to localize the divergence.
            let bridge = dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey::decode(&meta_xpub)
                .expect("decode bridge xpub");
            eprintln!(
                "ISSUE7 FIELDS stored: net={:?} depth={} parent_fp={:?} child={:?}",
                stored.network, stored.depth, stored.parent_fingerprint, stored.child_number
            );
            eprintln!(
                "ISSUE7 FIELDS bridge: net={:?} depth={} parent_fp={:?} child={:?}",
                bridge.network, bridge.depth, bridge.parent_fingerprint, bridge.child_number
            );
            eprintln!(
                "ISSUE7 FIELDS pubkey_eq={} chaincode_eq={} depth_eq={} parentfp_eq={} child_eq={} net_eq={}",
                stored.public_key == bridge.public_key,
                stored.chain_code == bridge.chain_code,
                stored.depth == bridge.depth,
                stored.parent_fingerprint == bridge.parent_fingerprint,
                stored.child_number == bridge.child_number,
                stored.network == bridge.network,
            );
            eprintln!(
                "ISSUE7 blob-decode: stored==bridge={} (true confirms the fix)",
                stored_xpub_encoded == meta_xpub
            );

            // THE GATE INVARIANT, as a hard assertion: the persisted BIP44
            // account-0 xpub must equal DET's sidecar bridge xpub. Equality is
            // exactly what the fund-routing gate checks on a seedless cold boot;
            // before the fix the stored row was the depth-1 BIP32 xpub and this
            // differed, rejecting every wallet.
            assert_eq!(
                stored_xpub_encoded, meta_xpub,
                "ISSUE7: stored BIP44 account-0 xpub must match the DET bridge xpub — the fund-routing gate rejects the wallet otherwise"
            );
        }

        // DECISIVE PRIMARY ASSERTION: a CURRENT-binary-written wallet must
        // survive create -> persist -> real load_from_persistor_seedless -> gate.
        // It does not (the persistor stored the depth-1 BIP32 row), so this
        // reproduces the user's systematic WalletNotLoaded on a fresh DB.
        assert!(
            cold_boot_registered,
            "ISSUE7 REPRODUCED (real load_from_persistor_seedless): a current-binary wallet is NOT resolved after cold-boot seedless reload — systematic WalletNotLoaded"
        );
    }

    /// `WalletTask::ListTrackedAssetLocks` reads tracked locks off the UI thread
    /// through the App Task System. This drives the production dispatch path
    /// (`run_backend_task`) for a registered wallet and asserts it returns the
    /// typed `TrackedAssetLocks` result — the route the egui frame loop now uses
    /// instead of the deleted in-runtime blocking read. A freshly-registered
    /// wallet has no locks, so an empty list is the expected, panic-free result.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_tracked_asset_locks_task_returns_typed_result() {
        use crate::backend_task::BackendTask;
        use crate::backend_task::BackendTaskSuccessResult;
        use crate::backend_task::wallet::WalletTask;

        let (ctx, sender, _tmp) = offline_testnet_context();

        let seed = [0x9Eu8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();
        ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");

        // `run_backend_task` wires the backend on first wallet task and
        // registers the wallet with the upstream manager.
        let result = ctx
            .run_backend_task(
                BackendTask::WalletTask(WalletTask::ListTrackedAssetLocks { seed_hash }),
                sender,
            )
            .await
            .expect("listing tracked asset locks must succeed");

        match result {
            BackendTaskSuccessResult::TrackedAssetLocks {
                seed_hash: got_hash,
                locks,
            } => {
                assert_eq!(
                    got_hash, seed_hash,
                    "result must carry the requested wallet"
                );
                assert!(
                    locks.is_empty(),
                    "a freshly-registered wallet has no tracked asset locks"
                );
            }
            other => panic!("expected TrackedAssetLocks, got: {other:?}"),
        }

        ctx.wallet_backend()
            .expect("backend wired")
            .shutdown()
            .await;
    }

    /// PROJ-010 W2 reconciliation (idempotency across the two writers): once a
    /// wallet is registered, the W2 `ensure_upstream_registered` path is a
    /// no-op — it never re-registers or double-watches. This is the cold-boot
    /// bridge's safety property: an already-watched wallet is left untouched
    /// while a missing one is filled exactly once.
    ///
    /// The full cross-process cold-boot reload (a fresh `AppContext` over the
    /// same persistor re-watching the wallet) and the live below-tip funding
    /// repro both require process isolation — DET's `SpvProvider` holds a
    /// strong `Arc<AppContext>`, so a second in-process context cannot open the
    /// same secret-store vault. Those assertions live in the `#[ignore]`
    /// backend-e2e lane (`tests/backend-e2e/wallet_reregistration.rs`), which
    /// runs each context in its own workdir slot.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ensure_upstream_registered_is_noop_when_already_registered() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");

        let seed = [0x6Bu8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();

        // W1 registers it once.
        backend
            .register_wallet_from_seed(&seed_hash, &seed, None)
            .await
            .expect("initial registration must succeed");
        assert_eq!(backend.wallet_count().await, 1);

        // W2 over the same, already-registered wallet is a no-op.
        backend
            .ensure_upstream_registered(&seed_hash, &seed)
            .await
            .expect("W2 must be a no-op, not an error, for a registered wallet");
        assert_eq!(
            backend.wallet_count().await,
            1,
            "W2 must not double-watch an already-registered wallet"
        );

        backend.shutdown().await;
    }

    /// PROJ-010 (root-cause regression): `register_wallet` persists the
    /// seed-envelope sidecar **before** the wallet backend is wired.
    ///
    /// This is the exact ordering the backend-e2e harness uses — register the
    /// framework wallet first, wire the backend second. The pre-fix bug was that
    /// `write_wallet_sidecars` required `self.wallet_backend()`, so the envelope
    /// was never written and the W2 cold-boot bridge could not find a seed to
    /// register from. With the vault handle owned by `AppContext`, the write
    /// succeeds regardless of wiring order. Reading the envelope back through the
    /// shared handle is the assertion that would have failed before the fix.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn register_wallet_persists_seed_envelope_before_backend_wired() {
        let (ctx, _sender, _tmp) = offline_testnet_context();

        assert!(
            ctx.wallet_backend().is_err(),
            "precondition: the backend must be unwired so we exercise the pre-wire path"
        );

        let seed = [0x7Cu8; 64];
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("pre-wire".to_string()),
            None,
        )
        .expect("build no-password wallet");
        let (seed_hash, _wallet_arc) = ctx
            .register_wallet(wallet, &seed, WalletOrigin::Imported)
            .expect("register wallet before the backend is wired");

        let envelope = WalletSeedView::new(&ctx.secret_store())
            .get(&seed_hash)
            .expect("vault read must not error")
            .expect("the seed envelope must be persisted at register time, even unwired");
        assert!(
            !envelope.uses_password,
            "the persisted envelope must carry the no-password flag for the W2 fast-path"
        );
        assert_eq!(
            envelope.xpub_encoded,
            ctx.wallets
                .read()
                .unwrap()
                .get(&seed_hash)
                .unwrap()
                .read()
                .unwrap()
                .master_bip44_ecdsa_extended_public_key
                .encode()
                .to_vec(),
            "the persisted xpub must match the registered wallet's BIP44 account xpub"
        );
    }

    /// PROJ-010 (end-to-end on the harness ordering): a wallet registered
    /// **before** the backend is wired is registered with the upstream SPV
    /// manager once the backend comes up — the W2 cold-boot bridge fires from
    /// the seed envelope persisted at register time.
    ///
    /// This is the in-process half of the live repro: it proves the chain from
    /// the persisted envelope through `bootstrap_loaded_wallets` →
    /// `bootstrap_wallet_addresses_jit` → `ensure_upstream_registered` without a
    /// launch-time prompt (the wallet is unprotected, so the chokepoint's
    /// no-passphrase fast-path resolves the seed). The funded below-tip balance
    /// assertion needs a live testnet and lives in the `#[ignore]` backend-e2e
    /// lane.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wallet_registered_before_wiring_is_upstream_registered_on_cold_boot() {
        let (ctx, sender, _tmp) = offline_testnet_context();

        let seed = [0x8Du8; 64];
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("cold-boot-bridge".to_string()),
            None,
        )
        .expect("build no-password wallet");
        let (seed_hash, _wallet_arc) = ctx
            .register_wallet(wallet, &seed, WalletOrigin::Imported)
            .expect("register wallet before wiring");

        // Wiring runs hydration + the cold-boot bootstrap, which drives the W2
        // bridge from the now-persisted seed envelope.
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");

        assert!(
            backend.is_wallet_registered(&seed_hash),
            "the wallet must be upstream-registered by the W2 bridge after wiring"
        );
        assert_eq!(
            backend.wallet_count().await,
            1,
            "exactly one wallet must be watched after the cold-boot bridge runs"
        );

        backend.shutdown().await;
    }

    /// PROJ-010 (fresh-install regression): on a truly-fresh install the real
    /// `Database::initialize` path gates the legacy `wallet`/`wallet_addresses`
    /// tables OUT, so `register_wallet` must not depend on them. The pre-fix
    /// `store_wallet_with_addresses` ran an unguarded `INSERT INTO wallet` that
    /// failed with `no such table: wallet`, so `register_wallet` returned `Err`
    /// before any in-memory registration — fresh installs could never create or
    /// import a wallet. This drives the exact production path and asserts success
    /// plus in-memory registration.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn register_wallet_succeeds_on_fresh_install_without_legacy_tables() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let (ctx, _sender) = offline_testnet_context_fresh_init(temp_dir.path());

        // Precondition: the fresh-install schema must NOT carry the legacy
        // wallet table — this is the state that exposed the bug. Querying it
        // surfaces sqlite's "no such table: wallet" error.
        let probe = ctx.db.get_wallets(&Network::Testnet);
        assert!(
            probe.is_err(),
            "precondition: fresh install must not create the legacy `wallet` table"
        );

        let seed = [0x9Eu8; 64];
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("fresh-install".to_string()),
            None,
        )
        .expect("build no-password wallet");
        let seed_hash = wallet.seed_hash();

        let (returned_hash, _wallet_arc) = ctx
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register_wallet must succeed on a fresh install");
        assert_eq!(returned_hash, seed_hash);

        assert!(
            ctx.wallets.read().unwrap().contains_key(&seed_hash),
            "the wallet must be registered in-memory after register_wallet"
        );
        assert!(
            ctx.has_wallet.load(Ordering::Relaxed),
            "the has_wallet flag must flip true after a successful registration"
        );
    }

    /// F17/F20 — removing a wallet wipes its secret-bearing state: the
    /// seed-envelope vault entry, the plaintext shielded notes, the shielded
    /// balance, and the nullifier cursor. Before the fix, `remove_wallet` only
    /// touched SQLite + the in-memory map, leaving the encrypted seed and the
    /// plaintext Orchard notes (plus the nullifier cursor) on disk.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_wallet_wipes_seed_envelope_and_shielded_state() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let seed = [0xA1u8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();
        ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");

        let backend = ctx.wallet_backend().expect("backend wired");

        // Seed the shielded sidecar: one note + a nullifier cursor.
        let cmx = [0x01u8; 32];
        let nullifier = [0x02u8; 32];
        backend
            .shielded()
            .insert_shielded_note(
                &seed_hash,
                &crate::wallet_backend::InsertShieldedNote {
                    note_data: &[0u8; 8],
                    position: 0,
                    cmx: &cmx,
                    nullifier: &nullifier,
                    block_height: 100,
                    value: 50,
                    network: "testnet",
                },
            )
            .expect("insert shielded note");
        backend
            .shielded()
            .set_nullifier_sync_info(&seed_hash, "testnet", 100, 200)
            .expect("set nullifier cursor");

        // Preconditions: the seed envelope and shielded state are present.
        assert!(
            WalletSeedView::new(&ctx.secret_store())
                .get(&seed_hash)
                .expect("vault read")
                .is_some(),
            "precondition: the seed envelope must exist before removal"
        );
        assert_eq!(
            backend
                .shielded()
                .get_shielded_balance(&seed_hash, "testnet")
                .unwrap(),
            50
        );

        ctx.remove_wallet(&seed_hash).expect("remove wallet");

        // The encrypted seed envelope (the JIT decrypt source) is gone.
        assert!(
            WalletSeedView::new(&ctx.secret_store())
                .get(&seed_hash)
                .expect("vault read after removal")
                .is_none(),
            "the seed envelope must be deleted from the vault on removal"
        );
        // Plaintext shielded notes, balance, and the nullifier cursor are gone.
        assert!(
            backend
                .shielded()
                .get_unspent_shielded_notes(&seed_hash, "testnet")
                .unwrap()
                .is_empty(),
            "shielded notes must be deleted on removal"
        );
        assert_eq!(
            backend
                .shielded()
                .get_shielded_balance(&seed_hash, "testnet")
                .unwrap(),
            0,
            "shielded balance must be zero after removal"
        );
        assert_eq!(
            backend
                .shielded()
                .get_nullifier_sync_info(&seed_hash, "testnet")
                .unwrap(),
            (0, 0),
            "the nullifier cursor must reset to zero after removal"
        );

        backend.shutdown().await;
    }

    /// F17/F20 (fresh-install regression): removing a wallet must still wipe
    /// its secret-bearing state on a truly-fresh install where the legacy
    /// `wallet`/`wallet_addresses`/`utxos` tables are gated OUT of the schema.
    ///
    /// The sibling `remove_wallet_wipes_seed_envelope_and_shielded_state`
    /// builds its context with `create_tables(true)`, which force-creates
    /// those legacy tables and therefore masks this path. Here the real
    /// `Database::initialize` fresh path runs, so the unguarded
    /// `SELECT address FROM wallet_addresses` in `Database::remove_wallet`
    /// errored with `no such table` and propagated through
    /// `AppContext::remove_wallet` BEFORE the secret wipe — leaving the seed
    /// envelope and plaintext shielded notes on disk. The existence-guarded
    /// statements now no-op cleanly so the caller reaches the wipe.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_wallet_wipes_secrets_on_fresh_install_without_legacy_tables() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let (ctx, sender) = offline_testnet_context_fresh_init(temp_dir.path());

        // Precondition: the fresh-install schema must NOT carry the legacy
        // `wallet_addresses` table — querying it surfaces sqlite's
        // "no such table: wallet" error from `get_wallets`. This is the state
        // under which the unguarded `remove_wallet` aborted before the wipe.
        assert!(
            ctx.db.get_wallets(&Network::Testnet).is_err(),
            "precondition: fresh install must not create the legacy wallet tables"
        );

        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let seed = [0xF6u8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();
        ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");

        let backend = ctx.wallet_backend().expect("backend wired");

        // Seed the shielded sidecar so the wipe has plaintext to remove.
        backend
            .shielded()
            .insert_shielded_note(
                &seed_hash,
                &crate::wallet_backend::InsertShieldedNote {
                    note_data: &[0u8; 8],
                    position: 0,
                    cmx: &[0x01u8; 32],
                    nullifier: &[0x02u8; 32],
                    block_height: 100,
                    value: 50,
                    network: "testnet",
                },
            )
            .expect("insert shielded note");

        // Preconditions: the seed envelope and a shielded note exist.
        assert!(
            WalletSeedView::new(&ctx.secret_store())
                .get(&seed_hash)
                .expect("vault read")
                .is_some(),
            "precondition: the seed envelope must exist before removal"
        );
        assert_eq!(
            backend
                .shielded()
                .get_shielded_balance(&seed_hash, "testnet")
                .unwrap(),
            50,
            "precondition: the shielded note must exist before removal"
        );

        // Pre-fix this returned `Err(no such table: wallet_addresses)` and the
        // wipe below never ran.
        ctx.remove_wallet(&seed_hash)
            .expect("remove_wallet must succeed on a fresh install");

        assert!(
            WalletSeedView::new(&ctx.secret_store())
                .get(&seed_hash)
                .expect("vault read after removal")
                .is_none(),
            "the seed envelope must be deleted from the vault on a fresh install"
        );
        assert_eq!(
            backend
                .shielded()
                .get_shielded_balance(&seed_hash, "testnet")
                .unwrap(),
            0,
            "shielded balance must be zero after removal on a fresh install"
        );

        backend.shutdown().await;
    }

    /// F60 — "delete all local data" must leave no wallet recoverable: the
    /// wallet-meta sidecar (which the cold-boot picker reads) and the
    /// seed-envelope vault (which holds the encrypted seed) must both be
    /// empty. Before the fix, `clear_network_database` cleared only legacy
    /// data.db + the in-memory maps, so wallets rehydrated on next launch and
    /// encrypted seeds persisted.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn clear_network_database_wipes_wallet_meta_and_seed_envelope() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let seed = [0xB2u8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();
        ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");

        // Preconditions: both the meta sidecar and the seed envelope exist.
        assert!(
            WalletMetaView::new(&ctx.app_kv())
                .get(Network::Testnet, &seed_hash)
                .is_some(),
            "precondition: wallet-meta sidecar must exist before clear"
        );
        assert!(
            WalletSeedView::new(&ctx.secret_store())
                .get(&seed_hash)
                .expect("vault read")
                .is_some(),
            "precondition: seed envelope must exist before clear"
        );

        ctx.clear_network_database()
            .expect("clear_network_database should succeed");

        // The wallet must not rehydrate: its meta and encrypted seed are gone.
        assert!(
            WalletMetaView::new(&ctx.app_kv())
                .get(Network::Testnet, &seed_hash)
                .is_none(),
            "wallet-meta sidecar must be empty after clear (no rehydration)"
        );
        assert!(
            WalletSeedView::new(&ctx.secret_store())
                .get(&seed_hash)
                .expect("vault read after clear")
                .is_none(),
            "seed envelope must be deleted from the vault after clear"
        );
        assert!(
            ctx.wallets.read().unwrap().is_empty(),
            "the in-memory wallet map must be empty after clear"
        );

        ctx.wallet_backend()
            .expect("backend wired")
            .shutdown()
            .await;
    }

    /// F131 — locking a wallet must wipe the session-cached seed. Before the
    /// fix `handle_wallet_locked` was an empty no-op, so after an
    /// `UntilAppClose` unlock the plaintext seed stayed resident and the wallet
    /// kept signing with no prompt despite being "locked".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lock_wipes_session_cached_seed() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let seed = [0xC3u8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();
        let (_seed_hash, wallet_arc) = ctx
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");

        let backend = ctx.wallet_backend().expect("backend wired");
        let scope = crate::wallet_backend::SecretScope::HdSeed { seed_hash };

        // Promote the seed into the session cache (what an UntilAppClose unlock
        // leaves behind).
        let held = zeroize::Zeroizing::new(seed);
        backend.secret_access().remember_session(
            &scope,
            crate::wallet_backend::SecretPlaintext::HdSeed(&held),
            crate::wallet_backend::RememberPolicy::UntilAppClose,
        );
        assert!(
            backend.secret_access().is_session_cached(&scope),
            "precondition: the seed is session-cached before the lock"
        );

        ctx.handle_wallet_locked(&wallet_arc);

        assert!(
            !backend.secret_access().is_session_cached(&scope),
            "locking must wipe the session-cached seed"
        );

        backend.shutdown().await;
    }

    /// F62 — when the seed-envelope vault write fails, `register_wallet` must
    /// FAIL CLOSED: return `Err` and NOT keep the wallet. The envelope is the
    /// encrypted seed the W2 cold-boot bridge re-registers from, so silently
    /// keeping an in-session wallet whose seed was never saved would lose the
    /// wallet and its funds at the next launch. Before the fix the envelope
    /// write was best-effort (warn + Ok), so the wallet was kept regardless.
    ///
    /// Induces the write failure permission-free by replacing the vault file
    /// with a directory: the store's atomic `persist` rename onto a directory
    /// path fails deterministically (root cannot bypass this).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn register_wallet_fails_closed_when_seed_envelope_write_fails() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let (ctx, _sender) = offline_testnet_context_at(temp_dir.path());

        // Replace the resident vault file with a directory so the next vault
        // write (the atomic persist rename) fails.
        let vault_path = temp_dir.path().join("secrets").join("det-secrets.pwsvault");
        std::fs::remove_file(&vault_path).expect("remove vault file");
        std::fs::create_dir(&vault_path).expect("plant directory at vault path");

        let seed = [0xD4u8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();

        let result = ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh);
        assert!(
            result.is_err(),
            "register_wallet must fail closed when the seed envelope cannot be saved"
        );
        assert!(
            !ctx.wallets.read().unwrap().contains_key(&seed_hash),
            "a wallet whose seed was not saved must not be kept in memory"
        );
        assert!(
            !ctx.has_wallet.load(Ordering::Relaxed),
            "has_wallet must not flip true when registration fails closed"
        );
    }

    /// SEC-002 — when the wallet-meta sidecar write fails, `register_wallet`
    /// must FAIL CLOSED: return `Err` and NOT keep the wallet. Cold-boot
    /// hydration (`hydrate_wallets_for_network`) enumerates ONLY the meta
    /// sidecar — `ctx.wallets` is rebuilt solely from `WalletMetaView::list`.
    /// A wallet whose seed envelope was saved but whose meta row is missing is
    /// never hydrated, so its funds become unreachable with no self-heal (there
    /// is no upstream→meta reconstruction path). Both sidecars are required, so
    /// the meta write must be fail-closed just like the seed-envelope write.
    ///
    /// Induces the meta-write failure permission-free by dropping the
    /// `meta_global` table from `det-app.sqlite` (which backs `app_kv`) through
    /// a second connection: the next `WalletMetaView::set` upsert errors with
    /// "no such table", deterministically, with no filesystem race.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn register_wallet_fails_closed_when_wallet_meta_write_fails() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let (ctx, _sender) = offline_testnet_context_at(temp_dir.path());

        // Drop the table the wallet-meta sidecar upserts into, so the next
        // `WalletMetaView::set` fails. The persister holds its own connection;
        // a second connection to the same file is enough to drop the shared
        // schema object.
        {
            let meta_db = temp_dir.path().join("det-app.sqlite");
            let conn =
                rusqlite::Connection::open(&meta_db).expect("open det-app.sqlite second handle");
            conn.execute("DROP TABLE meta_global", [])
                .expect("drop meta_global to force the wallet-meta write to fail");
        }

        let seed = [0x17u8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();

        let result = ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh);
        assert!(
            result.is_err(),
            "register_wallet must fail closed when the wallet-meta sidecar cannot be saved"
        );
        assert!(
            !ctx.wallets.read().unwrap().contains_key(&seed_hash),
            "a wallet with no meta row must not be kept in memory (it would never hydrate)"
        );
        assert!(
            !ctx.has_wallet.load(Ordering::Relaxed),
            "has_wallet must not flip true when registration fails closed"
        );
    }

    /// Build a valid BIP44 account-0 master xpub for a legacy wallet row.
    fn legacy_master_epk_bytes(seed: &[u8; 64]) -> Vec<u8> {
        use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
        use dash_sdk::dpp::key_wallet::bip32::{
            ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey,
        };
        let secp = Secp256k1::new();
        let master = ExtendedPrivKey::new_master(Network::Testnet, seed).expect("master key");
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
        ]);
        let account = master.derive_priv(&secp, &path).expect("derive account");
        ExtendedPubKey::from_priv(&secp, &account).encode().to_vec()
    }

    /// F140 — a wallet migrated from legacy `data.db` must be visible right
    /// after the migration completes, NOT only after a second restart. The bug:
    /// `WalletBackend::new` runs `hydrate_context_wallets` against the still-
    /// empty sidecars at first boot; migration then populates the sidecars but
    /// never re-hydrates `ctx.wallets`, so the in-memory map stays empty until
    /// the next launch reads the now-populated sidecars. The fix re-hydrates at
    /// the end of a successful migration.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn migrated_wallet_is_visible_without_second_restart() {
        let (ctx, sender, _tmp) = offline_testnet_context();

        // Seed a legacy `wallet` row with a valid xpub so the migration's
        // seed + meta passes produce a hydratable wallet.
        let seed = [0xE5u8; 64];
        let seed_hash: WalletSeedHash =
            crate::model::wallet::ClosedKeyItem::compute_seed_hash(&seed);
        let epk = legacy_master_epk_bytes(&seed);
        ctx.db
            .execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, alias, is_main,
                    uses_password, password_hint, network, core_wallet_name
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 0, NULL, 'testnet', NULL)",
                rusqlite::params![
                    seed_hash.as_slice(),
                    // Unprotected wallet: salt/nonce must be empty (SEC-007),
                    // the encrypted_seed slot carries the verbatim 64-byte seed.
                    seed.to_vec(),
                    Vec::<u8>::new(),
                    Vec::<u8>::new(),
                    epk,
                    "migrated-wallet",
                ],
            )
            .expect("insert legacy wallet row");

        // Wire the backend: hydration runs now, against the EMPTY sidecars
        // (migration has not run yet), so ctx.wallets is empty.
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        assert!(
            !ctx.wallets.read().unwrap().contains_key(&seed_hash),
            "precondition: the migrated wallet is not yet hydrated (sidecars empty at wiring)"
        );

        // Run the migration. It populates the sidecars AND now re-hydrates.
        crate::backend_task::migration::finish_unwire::run(&ctx)
            .await
            .expect("migration should succeed");

        // The migrated wallet must be visible WITHOUT a second backend build.
        assert!(
            ctx.wallets.read().unwrap().contains_key(&seed_hash),
            "the migrated wallet must be in ctx.wallets right after migration (no second restart)"
        );
        assert!(
            ctx.has_wallet.load(Ordering::Relaxed),
            "has_wallet must be true after a migrated wallet is hydrated"
        );

        ctx.wallet_backend()
            .expect("backend wired")
            .shutdown()
            .await;
    }

    /// F140 (resolve half) — a wallet migrated from legacy `data.db` at cold
    /// start must be RESOLVABLE through the wallet backend right after the
    /// migration completes, NOT only after a second restart. The bug: the
    /// post-migration re-hydration (`hydrate_context_wallets`) refills
    /// `ctx.wallets` (so the wallet shows in the picker and addresses resolve),
    /// but it never re-runs the W2 cold-boot reconciliation
    /// (`bootstrap_loaded_wallets` → `ensure_upstream_registered`). So the
    /// upstream `id_map` stays empty and every seed-keyed operation
    /// (`resolve_wallet`) returns `WalletNotLoaded` until the next launch —
    /// exactly the "wallet still loading" banner that repeats forever in the
    /// field report. The companion F140 test above only proves `ctx.wallets`
    /// visibility; this one proves upstream registration, which is what
    /// `resolve_wallet` keys off.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn migrated_wallet_is_upstream_registered_without_second_restart() {
        let (ctx, sender, _tmp) = offline_testnet_context();

        // Seed a legacy unprotected `wallet` row whose verbatim seed and
        // published xpub agree, so the migration's seed + meta passes produce a
        // wallet the W2 fund-routing gate will accept.
        let seed = [0xD7u8; 64];
        let seed_hash: WalletSeedHash =
            crate::model::wallet::ClosedKeyItem::compute_seed_hash(&seed);
        let epk = legacy_master_epk_bytes(&seed);
        ctx.db
            .execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, alias, is_main,
                    uses_password, password_hint, network, core_wallet_name
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 0, NULL, 'testnet', NULL)",
                rusqlite::params![
                    seed_hash.as_slice(),
                    // Unprotected wallet: salt/nonce must be empty (SEC-007), the
                    // encrypted_seed slot carries the verbatim 64-byte seed.
                    seed.to_vec(),
                    Vec::<u8>::new(),
                    Vec::<u8>::new(),
                    epk,
                    "migrated-wallet",
                ],
            )
            .expect("insert legacy wallet row");

        // Wire the backend: hydration + the cold-boot bootstrap run NOW, against
        // the EMPTY sidecars (the migration has not run yet), so the upstream
        // persistor is empty and nothing is registered.
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");
        assert!(
            !backend.is_wallet_registered(&seed_hash),
            "precondition: the migrated wallet is not yet upstream-registered (sidecars empty at wiring)"
        );

        // Run the cold-start migration. It populates the sidecars, re-hydrates
        // `ctx.wallets`, AND must re-run the W2 cold-boot reconciliation so the
        // just-migrated wallet is registered upstream.
        crate::backend_task::migration::finish_unwire::run(&ctx)
            .await
            .expect("migration should succeed");

        // The migrated wallet must be RESOLVABLE WITHOUT a second backend build:
        // `is_wallet_registered` reads the same `id_map` that `resolve_wallet`
        // consults, so this is a deterministic proxy for "`resolve_wallet`
        // succeeds".
        assert!(
            backend.is_wallet_registered(&seed_hash),
            "the migrated wallet must be upstream-registered right after migration (no second restart)"
        );

        backend.shutdown().await;
    }

    /// F140 (protected half — QA-001) — a *password-protected* wallet migrated
    /// from legacy `data.db` at cold start must hydrate into `ctx.wallets` but
    /// must NOT be upstream-registered until the user unlocks it. The cold-start
    /// migration re-runs the W2 cold-boot bridge
    /// (`bootstrap_loaded_wallets` → `bootstrap_wallet_addresses_jit`), but that
    /// bridge gates on `Wallet::is_open()`: a protected wallet hydrates as
    /// `WalletSeed::Closed`, so `is_open()` is `false` and the bridge returns
    /// early — before any `with_secret_session` (no passphrase prompt) and
    /// before `ensure_upstream_registered` (no registration). The companion
    /// unprotected test above proves eager registration of unprotected wallets;
    /// this one locks in the deferral for protected wallets so it can't
    /// silently regress into a surprise startup prompt or a `WalletLocked`
    /// failure mid-migration.
    ///
    /// It would FAIL if someone dropped the `is_open()` gate and made the
    /// bridge enter the seed scope for a locked protected wallet: the chokepoint
    /// would request a passphrase prompt during migration (the recording prompt
    /// double below would see a non-zero call count), which is exactly the
    /// surprise startup prompt the deferral exists to prevent.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn migrated_protected_wallet_registration_is_deferred_until_unlock() {
        use crate::database::test_helpers::seed_legacy_protected_hd_wallet_row;
        use crate::model::wallet::encryption::encrypt_message;
        use crate::wallet_backend::{
            SecretPrompt, SecretPromptCancelled, SecretPromptReply, SecretPromptRequest,
        };
        use std::sync::atomic::AtomicUsize;

        /// A `SecretPrompt` double that records how many times the chokepoint
        /// asked the host to unlock a wallet, then declines like a headless
        /// host. A still-locked protected wallet must NOT trigger any request
        /// during cold-start migration — the count must stay zero.
        #[derive(Default)]
        struct RecordingPrompt {
            requests: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl SecretPrompt for RecordingPrompt {
            async fn request(
                &self,
                _request: SecretPromptRequest,
            ) -> Result<SecretPromptReply, SecretPromptCancelled> {
                self.requests.fetch_add(1, Ordering::Relaxed);
                Err(SecretPromptCancelled)
            }
            fn is_interactive(&self) -> bool {
                // Interactive on purpose: a non-interactive host would let the
                // chokepoint short-circuit before requesting. We want any
                // attempt to reach `request` so a dropped gate is observable.
                true
            }
        }

        let (ctx, sender, _tmp) = offline_testnet_context();

        // Install the recording prompt BEFORE the backend is built — that is
        // when the chokepoint reads the host (see `install_secret_prompt`).
        let prompt = Arc::new(RecordingPrompt::default());
        ctx.install_secret_prompt(prompt.clone() as Arc<dyn SecretPrompt>);

        // Stage a legacy PROTECTED `wallet` row: the seed is AES-GCM-encrypted
        // under a passphrase the test never feeds back in, so the wallet stays
        // locked across the whole migration. The published BIP44 xpub agrees
        // with the seed so the W2 fund-routing gate would accept it *if* the
        // gate were reached — it must not be.
        let seed = [0x42u8; 64];
        let passphrase = "correct-horse-battery-staple";
        let seed_hash: WalletSeedHash =
            crate::model::wallet::ClosedKeyItem::compute_seed_hash(&seed);
        let epk = legacy_master_epk_bytes(&seed);
        let (encrypted_seed, salt, nonce) =
            encrypt_message(&seed, passphrase).expect("encrypt legacy seed");
        seed_legacy_protected_hd_wallet_row(
            &ctx.db,
            &seed_hash,
            &encrypted_seed,
            &salt,
            &nonce,
            &epk,
            "protected-wallet",
            Some("the usual passphrase"),
            Network::Testnet,
        )
        .expect("insert legacy protected wallet row");

        // Wire the backend: hydration + the cold-boot bootstrap run now against
        // the EMPTY sidecars (migration has not run), so nothing is registered.
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");

        // (a) The cold-start migration must complete with NO error and NO panic.
        // A passphrase prompt is impossible here (offline, headless) — if the
        // deferral broke and the bridge entered the seed scope, the locked
        // envelope would surface `WalletLocked` inside `bootstrap_*`. That path
        // is best-effort/logged (it does not fail the migration), so the strong
        // assertion is the deferred-registration check in (b).
        crate::backend_task::migration::finish_unwire::run(&ctx)
            .await
            .expect("migration must succeed for a protected wallet (no error, no prompt)");

        // (b) The protected wallet is hydrated into `ctx.wallets` (visible in
        // the picker, name preserved) but stays LOCKED — `is_open()` is false.
        let wallet_arc = ctx
            .wallets
            .read()
            .unwrap()
            .get(&seed_hash)
            .cloned()
            .expect("protected wallet must be hydrated into ctx.wallets after migration");
        assert!(
            !wallet_arc.read().unwrap().is_open(),
            "a migrated protected wallet must hydrate locked (WalletSeed::Closed)"
        );
        assert!(
            wallet_arc.read().unwrap().uses_password,
            "the hydrated wallet must carry the password flag"
        );

        // (b cont.) Registration is DEFERRED: the wallet is present in
        // `ctx.wallets` but NOT yet in the upstream `id_map` that
        // `resolve_wallet` keys off. This is the regression trap — eager
        // registration would flip this `true`.
        assert!(
            !backend.is_wallet_registered(&seed_hash),
            "a still-locked protected wallet must NOT be upstream-registered by the migration (deferred to unlock)"
        );

        // (c) The migration itself must not register any wallet at all: with a
        // single locked protected wallet, the watched-wallet set stays empty.
        assert_eq!(
            backend.wallet_count().await,
            0,
            "the migration must register no wallets while the only wallet is locked"
        );

        // (a, strong form) The deferral is prompt-free: the cold-boot bridge
        // must never have asked the host to unlock the wallet. This is the
        // regression trap — dropping the `is_open()` gate would make the bridge
        // enter the seed scope and request a prompt, flipping this above zero.
        assert_eq!(
            prompt.requests.load(Ordering::Relaxed),
            0,
            "the migration must never prompt for a passphrase while a protected wallet is locked"
        );

        backend.shutdown().await;
    }

    /// PROJ-010 (protected-unlock reconciliation — the delete-DB + re-import
    /// acceptance flow): a password-protected wallet that hydrates LOCKED at cold
    /// boot, and is therefore deferred by the W2 bridge (proven by
    /// [`migrated_protected_wallet_registration_is_deferred_until_unlock`]), MUST
    /// become upstream-registered on the unlock gesture — without a second app
    /// restart.
    ///
    /// The gap this guards: before the fix, the unlock path
    /// ([`AppContext::handle_wallet_unlocked`]) only promoted the just-verified
    /// seed into the session cache; it never re-drove
    /// [`AppContext::bootstrap_wallet_addresses_jit`], so the wallet stayed out
    /// of the upstream `id_map` that `resolve_wallet` keys off and every
    /// seed-keyed operation kept failing with `WalletNotLoaded` for the rest of
    /// the session. The fix re-drives the JIT bootstrap from
    /// `handle_wallet_unlocked` once the seed is in the session cache; this test
    /// asserts the post-unlock registration that fix enables.
    ///
    /// Staging mirrors the deferral test: a legacy PROTECTED `wallet` row is
    /// migrated so the wallet hydrates `Closed` (locked) with EMPTY persistor and
    /// is NOT registered. Then the wallet is opened with the real passphrase and
    /// `handle_wallet_unlocked` is invoked exactly as the unlock popup does
    /// (`src/ui/components/wallet_unlock_popup.rs`), passing the passphrase so the
    /// seed resolves prompt-free from the session cache.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_wallet_registers_upstream_on_unlock_without_restart() {
        use crate::database::test_helpers::seed_legacy_protected_hd_wallet_row;
        use crate::model::wallet::encryption::encrypt_message;

        let (ctx, sender, _tmp) = offline_testnet_context();

        // Stage a legacy PROTECTED `wallet` row whose published BIP44 xpub agrees
        // with the seed, so the W2 fund-routing gate accepts it once reached. The
        // passphrase is the one the test feeds back in at unlock time.
        let seed = [0x42u8; 64];
        let passphrase = "correct-horse-battery-staple";
        let seed_hash: WalletSeedHash =
            crate::model::wallet::ClosedKeyItem::compute_seed_hash(&seed);
        let epk = legacy_master_epk_bytes(&seed);
        let (encrypted_seed, salt, nonce) =
            encrypt_message(&seed, passphrase).expect("encrypt legacy seed");
        seed_legacy_protected_hd_wallet_row(
            &ctx.db,
            &seed_hash,
            &encrypted_seed,
            &salt,
            &nonce,
            &epk,
            "protected-wallet",
            Some("the usual passphrase"),
            Network::Testnet,
        )
        .expect("insert legacy protected wallet row");

        // Wire the backend, then run the cold-start migration. This reproduces
        // the boot state of the acceptance flow: the protected wallet hydrates
        // into `ctx.wallets` but stays LOCKED, and the W2 bridge defers it.
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");
        crate::backend_task::migration::finish_unwire::run(&ctx)
            .await
            .expect("migration must succeed for a protected wallet");

        let wallet_arc = ctx
            .wallets
            .read()
            .unwrap()
            .get(&seed_hash)
            .cloned()
            .expect("protected wallet must be hydrated into ctx.wallets after migration");

        // Precondition: the locked protected wallet is NOT yet registered — the
        // exact `WalletNotLoaded`-producing state the unlock must clear.
        assert!(
            !wallet_arc.read().unwrap().is_open(),
            "precondition: the protected wallet hydrates locked"
        );
        assert!(
            !backend.is_wallet_registered(&seed_hash),
            "precondition: a still-locked protected wallet is not upstream-registered"
        );

        // The unlock gesture, exactly as the unlock popup performs it: open the
        // in-memory wallet by verifying the passphrase, then notify the context
        // with that passphrase so the seed is promoted to the session cache and
        // (with the fix) the JIT bootstrap is re-driven.
        wallet_arc
            .write()
            .unwrap()
            .wallet_seed
            .open(passphrase)
            .expect("correct passphrase opens the wallet");
        ctx.handle_wallet_unlocked(&wallet_arc, Some(passphrase));

        // `handle_wallet_unlocked` spawns the registration on a tracked subtask,
        // so poll the `id_map` (what `resolve_wallet` consults) with a bounded
        // deadline rather than racing it. The deadline is generous because the
        // unlock reconciliation uses the genesis-floored `Imported` birth height
        // (`ensure_upstream_registered`), and the upstream
        // `create_wallet_from_seed_bytes` scan-window setup over the empty
        // offline persistor takes several seconds with no chain to read.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        while !backend.is_wallet_registered(&seed_hash) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the protected wallet must be upstream-registered after unlock (no second restart)"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // The wallet is now watched exactly once — the unlock reconciliation does
        // not double-watch.
        assert_eq!(
            backend.wallet_count().await,
            1,
            "exactly one wallet must be watched after the unlock reconciliation"
        );

        backend.shutdown().await;
    }

    /// F61 — clearing the SPV chain cache removes every `dash-spv` storage
    /// folder/file (and the storage lock) under the per-network directory while
    /// leaving the wallet (`platform-wallet.sqlite`) and shielded sidecars
    /// intact. The pre-fix `clear_spv_data` was a no-op that still reported
    /// success.
    #[test]
    fn clear_spv_chain_storage_removes_chain_cache_but_keeps_wallet_sidecars() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let spv_dir = spv_storage_dir(tmp.path(), Network::Testnet);
        std::fs::create_dir_all(&spv_dir).expect("create spv dir");

        // Plant one file inside each chain-storage folder, plus the loose
        // peers.dat and the sibling storage lock.
        for entry in [
            "block_headers",
            "filter_headers",
            "filters",
            "blocks",
            "metadata",
            "masternodestate",
        ] {
            let folder = spv_dir.join(entry);
            std::fs::create_dir_all(&folder).expect("create chain folder");
            std::fs::write(folder.join("segment.dat"), b"x").expect("write chain segment");
        }
        std::fs::write(spv_dir.join("peers.dat"), b"peers").expect("write peers");
        std::fs::write(spv_dir.with_extension("lock"), b"lock").expect("write lock");

        // Plant the wallet + shielded sidecars that must survive the clear.
        let wallet_sqlite = spv_dir.join("platform-wallet.sqlite");
        let shielded_tree = spv_dir.join("shielded-commitment-tree.sqlite");
        std::fs::write(&wallet_sqlite, b"wallet").expect("write wallet sqlite");
        std::fs::write(&shielded_tree, b"tree").expect("write shielded tree");

        clear_spv_chain_storage(&spv_dir).expect("clear must succeed");

        for entry in SPV_CHAIN_STORAGE_ENTRIES {
            assert!(
                !spv_dir.join(entry).exists(),
                "chain-storage entry {entry} must be deleted"
            );
        }
        assert!(
            !spv_dir.with_extension("lock").exists(),
            "the storage lock must be deleted"
        );
        assert!(
            wallet_sqlite.exists(),
            "platform-wallet.sqlite must survive an SPV-cache clear"
        );
        assert!(
            shielded_tree.exists(),
            "the shielded commitment tree must survive an SPV-cache clear"
        );
    }

    /// F61 — a never-synced network has no SPV directory at all; clearing it is
    /// a success, not an error.
    #[test]
    fn clear_spv_chain_storage_is_ok_when_directory_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let spv_dir = spv_storage_dir(tmp.path(), Network::Testnet);
        assert!(
            !spv_dir.exists(),
            "precondition: no spv dir on a fresh install"
        );
        clear_spv_chain_storage(&spv_dir).expect("clearing an absent cache must succeed");
    }

    /// Seed a legacy password-protected `single_key_wallet` row into the
    /// context's `data.db`, encrypted under `password`. Returns the
    /// derived address. The default test DB created `single_key_wallet`
    /// via `create_tables(true)`, so we only INSERT.
    fn seed_legacy_protected_single_key(
        ctx: &Arc<AppContext>,
        raw_key: &[u8; 32],
        password: &str,
        alias: Option<&str>,
    ) -> String {
        use crate::model::wallet::single_key::ClosedSingleKey;
        use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
        use dash_sdk::dpp::dashcore::{Address, PrivateKey, PublicKey};

        let path = ctx.db.db_file_path().expect("data.db path");
        let conn = rusqlite::Connection::open(&path).expect("open data.db");

        let (ciphertext, salt, nonce) =
            ClosedSingleKey::encrypt_private_key(raw_key, password).expect("encrypt");
        let priv_key = PrivateKey::from_byte_array(raw_key, Network::Testnet).expect("priv");
        let secp = Secp256k1::new();
        let pub_key = PublicKey {
            compressed: priv_key.compressed,
            inner: priv_key.inner.public_key(&secp),
        };
        let address = Address::p2pkh(&pub_key, Network::Testnet).to_string();
        let key_hash = ClosedSingleKey::compute_key_hash(raw_key);
        conn.execute(
            "INSERT INTO single_key_wallet
                (key_hash, encrypted_private_key, salt, nonce, public_key,
                 address, alias, uses_password, network)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)",
            rusqlite::params![
                key_hash.as_slice(),
                ciphertext,
                salt,
                nonce,
                pub_key.inner.serialize().to_vec(),
                address,
                alias,
                Network::Testnet.to_string(),
            ],
        )
        .expect("insert legacy protected row");
        address
    }

    /// T-SK-03 end-to-end — a legacy password-protected single-key row is
    /// restored with the correct old password: the key lands in the modern
    /// vault, becomes listable, and drops off the pending list. A wrong
    /// password leaves the legacy row intact and surfaces the generic
    /// failure (no oracle, no corruption).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn restore_protected_single_key_round_trip_and_wrong_password() {
        use crate::backend_task::migration::single_key_restore::{
            list_pending_protected_restores, restore_protected_single_key,
        };
        use crate::wallet_backend::single_key::ImportPassphrase;

        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let mut raw = [0u8; 32];
        raw[31] = 0x2A;
        let address =
            seed_legacy_protected_single_key(&ctx, &raw, "old-legacy-password", Some("savings"));

        // The protected row shows up as pending (still encrypted under the
        // old password; not in the modern vault yet).
        let pending = list_pending_protected_restores(&ctx).expect("list pending");
        assert_eq!(pending.len(), 1, "exactly one protected row awaits restore");
        assert_eq!(pending[0].address, address);

        // Wrong password: generic failure, nothing restored, row intact.
        let err = restore_protected_single_key(
            &ctx,
            &address,
            "WRONG-password",
            ImportPassphrase::default(),
        )
        .expect_err("wrong password must fail");
        assert!(
            matches!(err, TaskError::SingleKeyPassphraseIncorrect),
            "wrong password must surface the generic incorrect error, got {err:?}"
        );
        let still_pending = list_pending_protected_restores(&ctx).expect("re-list pending");
        assert_eq!(
            still_pending.len(),
            1,
            "a failed restore must leave the protected row pending and uncorrupted"
        );

        // Correct password: the key is restored into the modern vault under
        // a fresh passphrase and becomes listable at the same address (S5).
        let restored_addr = restore_protected_single_key(
            &ctx,
            &address,
            "old-legacy-password",
            ImportPassphrase {
                passphrase: Some(zeroize::Zeroizing::new("a-fresh-strong-passphrase".into())),
                hint: Some("the new one".into()),
            },
        )
        .expect("correct password must restore the key");
        assert_eq!(restored_addr, address, "restored address must be stable");

        // It is now in the modern single-key index and no longer pending.
        let backend = ctx.wallet_backend().expect("backend wired");
        let listed = backend.single_key().list();
        assert!(
            listed
                .iter()
                .any(|k| k.address == address && k.has_passphrase),
            "restored key must be listable and passphrase-protected"
        );
        let after = list_pending_protected_restores(&ctx).expect("final pending");
        assert!(
            after.is_empty(),
            "the restored key must drop off the pending list"
        );
    }

    /// A protected key restored WITHOUT choosing a new passphrase
    /// (`has_passphrase == false`) is still fully recovered, so the
    /// data-loss gate must recognize it as restored and permit the future
    /// T7 drop. Before the fix the gate keyed on `has_passphrase` and
    /// would have blocked the drop forever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gate_recognizes_restore_without_new_passphrase() {
        use crate::backend_task::migration::finish_unwire::{
            drop_legacy_single_key_table_when_safe, ensure_legacy_single_key_table_droppable,
        };
        use crate::backend_task::migration::single_key_restore::restore_protected_single_key;
        use crate::wallet_backend::single_key::ImportPassphrase;

        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let mut raw = [0u8; 32];
        raw[31] = 0x5B;
        let address =
            seed_legacy_protected_single_key(&ctx, &raw, "old-legacy-password", Some("plain"));

        // While the protected row is un-restored, the gate must block.
        let blocked = ensure_legacy_single_key_table_droppable(&ctx)
            .expect_err("gate must block while a protected row is un-restored");
        assert!(
            matches!(blocked, TaskError::MigrationFailed { .. }),
            "blocked drop must wrap the migration error, got {blocked:?}"
        );

        // Restore WITHOUT a new passphrase → has_passphrase == false.
        restore_protected_single_key(
            &ctx,
            &address,
            "old-legacy-password",
            ImportPassphrase::default(),
        )
        .expect("restore without a new passphrase must succeed");
        let backend = ctx.wallet_backend().expect("backend wired");
        assert!(
            backend
                .single_key()
                .list()
                .iter()
                .any(|k| k.address == address && !k.has_passphrase),
            "the key must be restored unprotected (has_passphrase == false)"
        );

        // The gate must now recognize the address as restored and permit
        // the drop — keyed on presence, not the passphrase flag.
        ensure_legacy_single_key_table_droppable(&ctx)
            .expect("gate must recognize an unprotected restore as restored");
        drop_legacy_single_key_table_when_safe(&ctx)
            .expect("the sanctioned drop must succeed once every key is restored");
    }

    /// Build a deterministic compressed testnet WIF from `raw` so the
    /// single-key import tests stay offline and reproducible.
    fn testnet_wif_from_raw(raw: &[u8; 32]) -> String {
        use dash_sdk::dpp::dashcore::PrivateKey;
        PrivateKey::from_byte_array(raw, Network::Testnet)
            .expect("valid private key bytes")
            .to_wif()
    }

    /// Importing a **passphrase-protected** single key must NOT retain the
    /// decrypted private key in the long-lived `single_key_wallets` session
    /// map. The in-memory mirror must come back closed — exactly the shape
    /// cold boot reconstructs — so the per-key passphrase is not silently
    /// defeated by a plaintext copy lingering for the whole session.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_single_key_import_does_not_retain_plaintext_in_session_map() {
        use crate::wallet_backend::single_key::ImportPassphrase;

        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let mut raw = [0u8; 32];
        raw[31] = 0x77;
        let wif = testnet_wif_from_raw(&raw);

        let passphrase = ImportPassphrase {
            passphrase: Some(zeroize::Zeroizing::new("a-strong-passphrase".into())),
            hint: Some("the test one".into()),
        };
        let (imported, wallet_arc) = ctx
            .import_single_key_wif(&wif, Some("protected".into()), passphrase)
            .expect("protected import must succeed");
        assert!(
            imported.has_passphrase,
            "the imported metadata must record the per-key passphrase"
        );

        // The in-memory mirror must be closed: no `is_open`, no plaintext key
        // obtainable, and the underlying data must be the encrypted variant.
        let guard = wallet_arc.read().expect("read mirror");
        assert!(
            !guard.is_open(),
            "a protected single key must be mirrored closed, not open with plaintext"
        );
        assert!(
            guard.private_key(Network::Testnet).is_none(),
            "no plaintext private key may be retrievable from the session-map mirror"
        );
        assert!(
            matches!(
                guard.private_key_data,
                crate::model::wallet::single_key::SingleKeyData::Closed(_)
            ),
            "the mirrored key data must be the Closed (encrypted) variant"
        );
        assert!(
            guard.uses_password,
            "the mirror must advertise that it needs a password"
        );

        // The same closed entry must be the one tracked in the session map.
        let key_hash = guard.key_hash();
        drop(guard);
        let map = ctx.single_key_wallets.read().expect("read map");
        let in_map = map.get(&key_hash).expect("imported key present in map");
        assert!(
            !in_map.read().expect("read map entry").is_open(),
            "the session-map entry for a protected key must stay closed"
        );
    }

    /// Companion to the protected-key test: an **unprotected** single key
    /// has no passphrase by definition, so plaintext in the session map is
    /// inherent and the mirror is expected to be open. This guards against
    /// over-correcting and breaking the no-passphrase fast path.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unprotected_single_key_import_mirrors_open() {
        use crate::wallet_backend::single_key::ImportPassphrase;

        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let mut raw = [0u8; 32];
        raw[31] = 0x55;
        let wif = testnet_wif_from_raw(&raw);

        let (imported, wallet_arc) = ctx
            .import_single_key_wif(&wif, Some("plain".into()), ImportPassphrase::default())
            .expect("unprotected import must succeed");
        assert!(
            !imported.has_passphrase,
            "an unprotected import must record no per-key passphrase"
        );

        let guard = wallet_arc.read().expect("read mirror");
        assert!(
            guard.is_open(),
            "an unprotected single key is mirrored open (plaintext is inherent)"
        );
        assert!(
            guard.private_key(Network::Testnet).is_some(),
            "an unprotected mirror exposes its private key for signing"
        );
        assert!(
            !guard.uses_password,
            "an unprotected mirror must not advertise a password requirement"
        );
    }

    /// The "Unlock" gesture for a protected single key must confirm the
    /// passphrase against the vault WITHOUT re-parking the decrypted private
    /// key in the long-lived `single_key_wallets` map. The map entry must stay
    /// closed both before and after a successful unlock; a wrong passphrase
    /// surfaces the generic incorrect error.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_single_key_unlock_verifies_without_reparking_plaintext() {
        use crate::wallet_backend::single_key::ImportPassphrase;

        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        let mut raw = [0u8; 32];
        raw[31] = 0x91;
        let wif = testnet_wif_from_raw(&raw);
        let pass = "a-strong-passphrase";

        let passphrase = ImportPassphrase {
            passphrase: Some(zeroize::Zeroizing::new(pass.into())),
            hint: None,
        };
        let (_imported, wallet_arc) = ctx
            .import_single_key_wif(&wif, Some("protected".into()), passphrase)
            .expect("protected import must succeed");
        let address = wallet_arc.read().expect("read mirror").address.to_string();

        // Closed before the unlock gesture.
        assert!(
            !wallet_arc.read().expect("read mirror").is_open(),
            "a protected key must be closed before unlock"
        );

        // A wrong passphrase surfaces the generic incorrect error and leaves
        // the entry closed.
        let wrong = ctx
            .verify_single_key_passphrase(&address, "not-the-passphrase")
            .expect_err("a wrong passphrase must fail");
        assert!(
            matches!(wrong, TaskError::SingleKeyPassphraseIncorrect),
            "wrong passphrase must surface the generic incorrect error, got {wrong:?}"
        );
        assert!(
            !wallet_arc.read().expect("read mirror").is_open(),
            "a failed unlock must leave the key closed"
        );

        // The correct passphrase verifies successfully — and the key STILL
        // stays closed: no plaintext is re-parked in the session map.
        ctx.verify_single_key_passphrase(&address, pass)
            .expect("the correct passphrase must verify");
        let guard = wallet_arc.read().expect("read mirror");
        assert!(
            !guard.is_open(),
            "a successful unlock must NOT open the map entry (no plaintext re-parked)"
        );
        assert!(
            guard.private_key(Network::Testnet).is_none(),
            "no plaintext private key may be retrievable after unlock"
        );
        assert!(
            matches!(
                guard.private_key_data,
                crate::model::wallet::single_key::SingleKeyData::Closed(_)
            ),
            "the map entry must remain the Closed (encrypted) variant after unlock"
        );
    }
}
