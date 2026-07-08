use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::dashpay::ContactStatus;
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

// The interim "protection removed" disclosure notices (HD wallet + imported
// key) and their shared at-rest detail copy were retired with the Tier-2
// adoption: lazy migration now RE-WRAPS protected secrets under the same
// password (it never downgrades them to a password-free at-rest form), so there
// is nothing to disclose.

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

        // Reset the upstream shielded coordinator (quiesces its sync loop and
        // empties the per-network store) and unlink DET's two retired legacy
        // shielded files. The coordinator reset is async, so it runs off-thread
        // as a best-effort subtask; the legacy-file unlinks are synchronous and
        // scoped strictly to THIS network's spv directory.
        if let Ok(backend) = self.wallet_backend() {
            cleanup_legacy_shielded_files(backend.spv_storage_dir())?;

            let ctx = Arc::clone(self);
            self.subtasks
                .spawn_sync("shielded_coordinator_clear", async move {
                    if let Ok(backend) = ctx.wallet_backend()
                        && let Err(error) = backend.clear_shielded().await
                    {
                        tracing::warn!(%error, "Shielded coordinator reset failed during clear");
                    }
                });
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

    /// Stop chain sync IN PLACE, keeping the wired wallet backend so the next
    /// Connect restarts the SAME instance.
    ///
    /// This is the disconnect counterpart to
    /// [`Self::ensure_wallet_backend_and_start_spv`] and the single chokepoint
    /// for "stop SPV". The sequence is:
    ///
    /// 1. Flip the SPV indicator to [`SpvStatus::Stopping`] so the UI shows
    ///    "Disconnecting…" immediately, before the async teardown runs.
    /// 2. Stop the backend IN PLACE ([`WalletBackend::stop_in_place`]): stop the
    ///    upstream chain-sync run loop and quiesce the three coordinators, but
    ///    KEEP the `WalletBackend` (and its `Arc<SqlitePersister>`) wired in the
    ///    AppContext slot, re-arming the one-shot start latch and coordinator
    ///    gate so the same instance can restart. The backend is NOT shut down or
    ///    unwired here.
    /// 3. Flip the indicator to [`SpvStatus::Stopped`] and clear the live peer
    ///    count, sync progress, and last error; re-arm the quorum gate and the
    ///    one-shot identity-sweep flag; then recompute the overall state — which
    ///    lands on `Disconnected` now that SPV is inactive.
    ///
    /// Restart-in-place is deliberate: because the persister DB is never closed
    /// and reopened, the next same-network Connect fast-paths on the populated
    /// slot and restarts on the re-armed latch, so a reconnect cannot hit
    /// `WalletStorageError::AlreadyOpen` — impossible by construction, no release
    /// barrier needed. Full teardown ([`WalletBackend::shutdown`], which quiesces
    /// the coordinators so the persister can drop) never runs on a GUI path: a
    /// GUI network switch keeps the outgoing per-network context cached (only its
    /// secrets are forgotten), and GUI app-close aborts the subtasks and exits.
    /// `shutdown` runs only on the MCP network-switch tool (draining the outgoing
    /// context before the swap) and the headless / MCP-server close — all on a
    /// different persister path than any live one, so none can race a reopen.
    ///
    /// Idempotent: a call with no wired backend still settles the indicator on
    /// `Stopped`/`Disconnected`. The teardown is async (upstream `stop_in_place`
    /// is async), so GUI callers dispatch this via `AppAction::StopSpv` rather
    /// than blocking the frame loop. That dispatch claims the stop synchronously
    /// with
    /// [`ConnectionStatus::begin_spv_stop`](crate::context::connection_status::ConnectionStatus::begin_spv_stop)
    /// (button disables on the click frame, second click deduped); the redundant
    /// `Stopping` flip here keeps direct callers self-contained.
    pub async fn stop_spv(self: &Arc<Self>) {
        self.connection_status.set_spv_status(SpvStatus::Stopping);
        self.connection_status.refresh_state();

        // Restart-in-place disconnect: keep the `WalletBackend` (and its
        // `Arc<SqlitePersister>`) wired in the AppContext slot — do NOT unwire
        // or drop it. `stop_in_place` stops the SPV run loop and
        // quiesces the three coordinators while leaving the backend + persister
        // alive, and re-arms the start latch + coordinator gate so the next
        // same-network Connect restarts on the SAME instance (the reconnect
        // reuses it via `ensure_wallet_backend`'s populated-slot fast path).
        // Because the persister DB is never closed/reopened, the reconnect
        // cannot hit `WalletStorageError::AlreadyOpen` — by construction, so no
        // release barrier is needed. (A network SWITCH is a different path: it
        // uses a per-network context with a different persister and is
        // unaffected by this.)
        //
        // Restart-in-place runtime safety: all three upstream coordinators clear
        // their cancel slot under a `background_generation` guard in the pinned
        // platform rev (`platform_address_sync` gained it in b4506492, matching
        // `identity_sync`/`shielded_sync`), so a rapid reconnect cannot leak an
        // uncancellable / duplicate sync loop (Q3).
        //
        // TODO(dash-spv#824): restart-in-place fully recreates the upstream DashSpvClient
        // in SpvRuntime::run(), opening a reinit window. A block arriving at tip during
        // that window can freeze dash-spv's filter committed_height one block below
        // permanently → is_synced() stuck false → UI stuck on "Syncing…". Upstream bug:
        // dashpay/rust-dashcore#824; DET's reconnect is the trigger. DET-side mitigations:
        // quiesce header/block intake until filter init completes, or add a stall watchdog.
        if let Ok(backend) = self.wallet_backend() {
            backend.stop_in_place().await;
        }

        self.connection_status.set_spv_status(SpvStatus::Stopped);
        self.connection_status.set_spv_connected_peers(0);
        self.connection_status.set_spv_sync_progress(None);
        self.connection_status.set_spv_last_error(None);
        // Re-arm the quorum gate so the next reconnect re-syncs the masternode
        // list on the same backend instance (`stop_in_place` keeps the backend
        // wired). Leaving the flag set would let early proof calls through
        // before quorums exist again, re-triggering the DAPI self-ban storm.
        self.connection_status.set_masternodes_ready(false);
        // Re-arm the automatic identity sweep so it runs once per session.
        self.identity_autodiscovery_fired
            .store(false, std::sync::atomic::Ordering::SeqCst);
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

    /// Whether `wallet` still needs its bootstrap address set derived.
    ///
    /// `true` for a fresh wallet (no known addresses) or one created with only
    /// a Core address (no Platform-payment addresses yet). Idempotent: a
    /// fully-bootstrapped wallet returns `false`.
    fn wallet_needs_bootstrap(guard: &Wallet) -> bool {
        // INTENTIONAL: Bootstrap checks only PlatformPayment address
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

        // An open wallet always enters the seed scope: shielded key binding runs
        // on every cold boot and `ensure_shielded_bound` is idempotent (upstream
        // does an in-memory check and returns immediately when already bound), so
        // there is no cheap pre-check that would let us skip the scope. Address
        // bootstrap and upstream registration are re-checked inside the scope, so
        // entering with nothing to do is harmless. The upstream 60 s
        // ShieldedSyncManager loop picks up any newly bound wallets automatically.
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
                    // Identity G-3 reconcile: register every DET-known wallet-owned
                    // identity into the upstream manager so identity ops (top-up)
                    // can find them. Seed-free, idempotent; runs after the wallet
                    // is upstream-registered. Best-effort — retried next boot/unlock.
                    self.reconcile_managed_identities(&backend, &seed_hash).await;
                    // Phase C-bind: lazily bind Orchard ZIP-32 keys for this wallet.
                    // Best-effort — a failure only defers the first shielded op prompt.
                    // The upstream ShieldedSyncManager 60s loop picks up any newly
                    // bound wallets automatically; no manual sync trigger needed.
                    if let Err(error) =
                        backend.ensure_shielded_bound(&seed_hash, seed).await
                    {
                        tracing::debug!(
                            wallet = %hex::encode(seed_hash),
                            %error,
                            "Shielded bind deferred; will retry on next unlock"
                        );
                    }
                    // Register every established contact's DIP-15 receiving
                    // account so SPV watches the addresses each contact pays us
                    // at. Seed-bearing (the receiving path is hardened) and
                    // reachable only here where the seed is already open, so a
                    // locked wallet is skipped, never prompted. Best-effort;
                    // re-runs every boot/unlock because upstream keeps contact
                    // accounts in runtime state only.
                    self.register_established_contact_accounts(&backend, &seed_hash, seed)
                        .await;
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

    /// Register every DET-known, wallet-owned identity for `seed_hash` into the
    /// upstream `IdentityManager`, so identity ops that look identities up there
    /// (currently: top-up) find them instead of raising `IdentityNotFound`.
    ///
    /// Best-effort, idempotent, and **seed-free** — a per-identity failure is
    /// logged and never aborts the rest. Called inside
    /// [`Self::bootstrap_wallet_addresses_jit`]'s seed scope after upstream
    /// registration (the seam reached from both cold boot and unlock); the seed
    /// is not used, so it is also safe while the wallet is locked.
    async fn reconcile_managed_identities(
        &self,
        backend: &WalletBackend,
        seed_hash: &WalletSeedHash,
    ) {
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        let identities = match self.load_local_qualified_identities_for_wallet(seed_hash) {
            Ok(identities) => identities,
            Err(error) => {
                tracing::warn!(
                    wallet = %hex::encode(seed_hash),
                    %error,
                    "Identity reconcile: sidecar read failed; will retry next boot/unlock"
                );
                return;
            }
        };
        let mut added = 0usize;
        for qi in &identities {
            let Some(index) = qi.wallet_index else {
                continue;
            };
            match backend
                .ensure_identity_managed(seed_hash, &qi.identity, index)
                .await
            {
                Ok(true) => added += 1,
                Ok(false) => {}
                Err(error) => tracing::debug!(
                    identity = %qi.identity.id(),
                    %error,
                    "Identity reconcile deferred; will retry next boot/unlock"
                ),
            }
        }
        if added > 0 {
            tracing::info!(
                wallet = %hex::encode(seed_hash),
                added,
                "Reconciled DET identities into the upstream manager"
            );
        }
    }

    /// Register the DIP-15 receiving accounts of every established contact of
    /// every identity on `seed_hash`, so SPV watches the addresses each contact
    /// pays us at. Best-effort — a failure is logged and retried next unlock.
    ///
    /// Called inside [`Self::bootstrap_wallet_addresses_jit`]'s seed scope, so
    /// the seed is already open and a locked wallet is never reached.
    async fn register_established_contact_accounts(
        &self,
        backend: &WalletBackend,
        seed_hash: &WalletSeedHash,
        seed: &[u8; 64],
    ) {
        let pairs = self.established_contact_pairs(backend, seed_hash).await;
        match backend
            .register_contact_receiving_accounts(seed_hash, seed, &pairs)
            .await
        {
            Ok(0) => {}
            Ok(count) => tracing::info!(
                wallet = %hex::encode(seed_hash),
                count,
                "Registered contact receiving accounts for watching"
            ),
            Err(error) => tracing::debug!(
                wallet = %hex::encode(seed_hash),
                %error,
                "Contact receiving-account registration deferred; will retry next unlock"
            ),
        }
    }

    /// Collect `(owner, contact)` identity-id pairs for every accepted contact
    /// of each local identity whose DashPay wallet is `seed_hash`.
    async fn established_contact_pairs(
        &self,
        backend: &WalletBackend,
        seed_hash: &WalletSeedHash,
    ) -> Vec<(
        dash_sdk::platform::Identifier,
        dash_sdk::platform::Identifier,
    )> {
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        use dash_sdk::platform::Identifier;

        let Ok(identities) = self.load_local_qualified_identities() else {
            return Vec::new();
        };
        let view = backend.dashpay_view();
        let mut pairs = Vec::new();
        for identity in &identities {
            if identity.dashpay_wallet_seed_hash().as_ref() != Some(seed_hash) {
                continue;
            }
            let owner = identity.identity.id();
            for contact in view.contacts(&owner).await {
                if contact.contact_status != ContactStatus::Accepted {
                    continue;
                }
                if let Ok(contact_id) = Identifier::from_bytes(&contact.contact_identity_id) {
                    pairs.push((owner, contact_id));
                }
            }
        }
        pairs
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
            // Tier-2 keep-protection: the seed re-wraps under the same password
            // inside the chokepoint — no downgrade to finalize, `uses_password`
            // stays accurate. The verified-open just promotes it to the cache.
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

        // The background all-wallets sweep skips a wallet that is locked at
        // Platform-ready time, so a just-unlocked wallet is searched here. This
        // is the "searched after unlock" path the all-wallets sweep documents.
        self.queue_unlocked_wallet_identity_discovery(wallet);
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

    /// Bind Orchard ZIP-32 keys for all currently-open wallets that have not
    /// yet been shielded-bound through the upstream coordinator.  Called when
    /// the network protocol version first crosses the shielded threshold —
    /// at that point open wallets are typically already bootstrapped and
    /// registered, so the regular JIT path in
    /// [`Self::bootstrap_wallet_addresses_jit`] would have been the right
    /// vehicle, but it may not have run yet for wallets opened before the
    /// version was known.
    ///
    /// Reuses `bootstrap_wallet_addresses_jit` (which now unconditionally
    /// calls `ensure_shielded_bound`) so the logic is not duplicated.
    /// The upstream 60 s `ShieldedSyncManager` loop picks up any newly bound
    /// wallets automatically — no manual sync trigger needed.
    /// Snapshot the currently-open wallet arcs, dropping the read lock before
    /// returning. A locked protected wallet hydrates `WalletSeed::Closed`, so
    /// `is_open()` excludes it — the single source of truth for "which wallets a
    /// background pass may touch without a passphrase prompt."
    fn open_wallets(self: &Arc<Self>) -> Vec<Arc<RwLock<Wallet>>> {
        self.wallets
            .read()
            .ok()
            .map(|wallets| {
                wallets
                    .values()
                    .filter(|w| w.read().ok().map(|g| g.is_open()).unwrap_or(false))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Count wallets that block the cold-start completion sentinel: an OPEN
    /// wallet not yet registered with the upstream wallet backend, OR any
    /// wallet whose lock cannot be read.
    ///
    /// The migration writes its sentinel only when this is zero. Soundness for
    /// the registered set relies on the copy step rejecting exactly what
    /// hydration drops (see
    /// `migration::finish_unwire::hd_seed_row_is_hydratable`), so every wallet
    /// that reached the vault is hydrated and seen here.
    ///
    /// Counted (sentinel withheld):
    /// - a readable, open, not-yet-registered wallet;
    /// - any wallet whose `RwLock` cannot be read — fail-safe, so a poisoned
    ///   lock can never green-light a premature "completed".
    ///
    /// Excluded (does not block):
    /// - a readable, `Closed` / locked password-protected wallet — it registers
    ///   on its unlock gesture, so requiring it would wedge the sentinel on a
    ///   protected install.
    ///
    /// Counts over the raw `self.wallets` map, NOT the [`Self::open_wallets`]
    /// snapshot — that snapshot already drops a poisoned-lock wallet before the
    /// fail-safe could see it. A poisoned OUTER map lock is recovered via
    /// `into_inner` so a prior panic elsewhere cannot zero the count. When the
    /// backend is not yet wired nothing is registered, so every open (or
    /// unreadable) wallet counts.
    pub(crate) fn unregistered_open_wallet_count(self: &Arc<Self>) -> usize {
        let backend = self.wallet_backend().ok();
        let guard = match self.wallets.read() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard
            .values()
            .filter(|w| match w.read() {
                // Unreadable per-wallet lock: cannot prove it is registered, so
                // fail safe and count it (withholds the sentinel).
                Err(_) => true,
                // Readable Closed / locked-protected: excluded — it registers on
                // its unlock gesture, so requiring it would wedge the sentinel.
                Ok(g) if !g.is_open() => false,
                // Readable and open: unregistered unless the wired backend knows
                // it. With no backend wired nothing is registered, so it counts.
                Ok(g) => backend
                    .as_ref()
                    .map(|b| b.registered_wallet_id(&g.seed_hash()).is_none())
                    .unwrap_or(true),
            })
            .count()
    }

    pub(crate) fn init_missing_shielded_wallets(self: &Arc<Self>) {
        for wallet in self.open_wallets() {
            let ctx = Arc::clone(self);
            self.subtasks
                .spawn_sync("shielded_bind_after_protocol_update", async move {
                    ctx.bootstrap_wallet_addresses_jit(&wallet).await;
                });
        }
    }

    /// Queue automatic, gap-limited identity discovery for every open wallet,
    /// once per SPV session.
    ///
    /// Fired when Platform becomes reachable (masternode list `Synced`). A
    /// single [`AtomicBool`](std::sync::atomic::AtomicBool) latch makes it run at
    /// most once per session — a re-entrant nudge (e.g. a repeated readiness
    /// event) is a no-op until [`stop_spv`](Self::stop_spv) clears the latch on
    /// the next reconnect.
    ///
    /// Locked, password-protected wallets are skipped here: the sweep runs with
    /// `allow_prompt = false`, so it never pops a passphrase modal for a wallet
    /// the user has not unlocked. Such a wallet is picked up later, with the
    /// user's consent, when it is unlocked
    /// (see [`Self::queue_wallet_identity_discovery`]).
    pub fn queue_all_wallets_identity_discovery(self: &Arc<Self>) {
        use std::sync::atomic::Ordering;

        // One-shot per session: skip if already fired.
        if self
            .identity_autodiscovery_fired
            .swap(true, Ordering::SeqCst)
        {
            tracing::debug!("All-wallets identity discovery already ran this session; skipping");
            return;
        }

        // Snapshot only open wallets — a locked protected wallet hydrates closed
        // (`is_open() == false`) and is skipped so the background sweep cannot
        // trigger a passphrase prompt.
        let open_wallets = self.open_wallets();

        if open_wallets.is_empty() {
            tracing::debug!("No open wallets to run automatic identity discovery for");
            return;
        }

        tracing::info!(
            wallet_count = open_wallets.len(),
            "Starting automatic identity discovery for all open wallets"
        );

        for wallet in open_wallets {
            let ctx = Arc::clone(self);
            self.subtasks
                .spawn_sync("all_wallets_identity_discovery", async move {
                    if let Err(error) = ctx
                        .discover_identities_gap_limited(&wallet, 0, false, None)
                        .await
                    {
                        tracing::warn!(
                            %error,
                            "Automatic identity discovery failed for a wallet"
                        );
                    }
                });
        }
    }

    /// Queue gap-limited identity discovery for a single wallet the user just
    /// unlocked, so a wallet that was locked during the all-wallets sweep still
    /// gets discovered this session.
    ///
    /// Independent of the once-per-session `identity_autodiscovery_fired` latch
    /// (that guards the all-wallets sweep, not per-wallet unlock). Gated on
    /// Platform readiness: if the masternode list is not yet `Synced`, this is a
    /// no-op — the wallet is now open, so the upcoming all-wallets sweep covers
    /// it. The user is present for the unlock, so `allow_prompt = true`; no
    /// prompt occurs anyway because the unlock just promoted the seed to the
    /// session cache. Idempotent with the sweep: discovery is
    /// update-preserving-alias, so a double-run is harmless.
    pub fn queue_unlocked_wallet_identity_discovery(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
    ) {
        if !self.connection_status.masternodes_ready() {
            tracing::debug!(
                "Platform not ready yet; deferring unlocked-wallet identity discovery to the all-wallets sweep"
            );
            return;
        }

        let ctx = Arc::clone(self);
        let wallet = Arc::clone(wallet);
        self.subtasks
            .spawn_sync("unlocked_wallet_identity_discovery", async move {
                if let Err(error) = ctx
                    .discover_identities_gap_limited(&wallet, 0, true, None)
                    .await
                {
                    tracing::warn!(
                        %error,
                        "Identity discovery failed for the just-unlocked wallet"
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

                wallet.set_platform_address_info(core_addr.clone(), info.balance, info.nonce);

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

/// Unlink DET's two retired legacy shielded files from `spv_dir` (the active
/// network's spv directory), tolerating a missing file.
///
/// These are the files DET's deleted shielded subsystem owned:
/// `det-shielded.sqlite` (the plaintext note sidecar) and
/// `shielded-commitment-tree.sqlite` (the grovedb commitment tree). The
/// upstream coordinator's store (`platform-wallet-shielded.sqlite`) is a
/// DIFFERENT file and is deliberately NOT touched here — it is reset via the
/// coordinator's own `clear_shielded`. Scoped strictly to `spv_dir` so a clear
/// of one network can never reach another network's files.
fn cleanup_legacy_shielded_files(spv_dir: &Path) -> Result<(), TaskError> {
    const LEGACY_SHIELDED_FILES: [&str; 2] =
        ["det-shielded.sqlite", "shielded-commitment-tree.sqlite"];
    for file in LEGACY_SHIELDED_FILES {
        let path = spv_dir.join(file);
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(TaskError::FileSystem { source: e });
        }
    }
    Ok(())
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
    /// successful start, `stop_spv` stops chain sync IN PLACE — keeping the
    /// backend wired for a restart — and settles the indicator on `Stopped` /
    /// `Disconnected`. Regression guard ensuring the Disconnect button drives
    /// the overall state out of its active value while preserving the backend.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_spv_in_place_keeps_backend_and_disconnects_indicator() {
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

        let backend = ctx
            .wallet_backend()
            .expect("stop_spv must KEEP the backend wired for restart-in-place (NOT unwire it)");
        assert!(
            !backend.is_started(),
            "stop_spv must re-arm the start latch so the next Connect can restart"
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

    /// Restart-in-place reconnect: a same-network Disconnect → Connect keeps the
    /// SAME `WalletBackend` (and its `Arc<SqlitePersister>`) wired, so the
    /// persister DB is never closed/reopened and `AlreadyOpen` is impossible by
    /// construction — no release barrier needed. Drives the real production
    /// path: `stop_spv()` (in-place) then `ensure_wallet_backend_and_start_spv()`.
    ///
    /// Validated offline (passes now): the backend pointer is identical across
    /// disconnect→connect (reuse, not rebuild); `is_started()` is cleared by
    /// `stop_spv` and re-set by the reconnect (latch + gate re-armed); the
    /// reconnect returns `Ok` with no `AlreadyOpen`.
    ///
    /// Upstream Q3 race protection now lives in the pinned platform rev: all
    /// three coordinators (incl. `platform_address_sync` since b4506492) gate
    /// their cancel-slot clear on `background_generation`, so a rapid restart of
    /// the SAME instance cannot leak an uncancellable / duplicate loop. This
    /// offline test asserts the DET-level reuse/restart contract; it does not
    /// itself force the timing race — full live behavior is covered by the
    /// network-gated (`#[ignore]`d) backend-e2e B-reconnect test.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reconnect_restart_in_place_reuses_backend() {
        use crate::context::connection_status::OverallConnectionState;

        let _reopen_guard = backend_reopen_lock().await;

        let (ctx, sender, _tmp) = offline_testnet_context();

        ctx.ensure_wallet_backend_and_start_spv(sender.clone())
            .await
            .expect("initial start should wire then start offline");
        let first = ctx.wallet_backend().expect("backend wired after start");
        assert!(first.is_started(), "initial start must latch the backend");
        let first_ptr = Arc::as_ptr(&first);
        drop(first);

        // Disconnect IN PLACE via the production chokepoint: the backend stays
        // wired (slot not taken), the start latch is re-armed, the indicator
        // settles on Disconnected.
        ctx.stop_spv().await;
        let after_stop = ctx
            .wallet_backend()
            .expect("stop_spv must KEEP the backend wired for restart-in-place");
        assert!(
            !after_stop.is_started(),
            "stop_spv must re-arm the start latch (is_started == false)"
        );
        assert_eq!(
            ctx.connection_status().overall_state(),
            OverallConnectionState::Disconnected,
            "stop_spv must settle the indicator on Disconnected"
        );
        assert!(
            !ctx.connection_status().masternodes_ready(),
            "stop_spv must re-arm the quorum gate (masternodes_ready == false)"
        );
        drop(after_stop);

        // Reconnect: `ensure_wallet_backend` fast-paths on the populated slot
        // (no `WalletBackend::new`, no `SqlitePersister::open`), so the SAME
        // instance restarts — structurally immune to `AlreadyOpen`.
        ctx.ensure_wallet_backend_and_start_spv(sender)
            .await
            .expect("reconnect should restart the SAME backend in place");
        let second = ctx
            .wallet_backend()
            .expect("backend still wired after reconnect");
        assert_eq!(
            first_ptr,
            Arc::as_ptr(&second),
            "restart-in-place must REUSE the same backend, not rebuild it"
        );
        assert!(
            second.is_started(),
            "reconnect must restart chain sync on the reused backend's re-armed latch"
        );

        second.shutdown().await;
    }

    /// Two genuinely-parallel first-open attempts on the SAME never-wired context
    /// must NOT race into a double `WalletBackend::new` / `SqlitePersister::open`.
    /// The upstream persister is single-open-per-path, so a concurrent double-open
    /// errors — `WalletStorageError::AlreadyOpen` against a live persister (the
    /// reported production symptom) or a DB-init race on a fresh file.
    ///
    /// This guards the GUI's `finalize_network_switch` fast path, which spawns a
    /// `wallet-backend-eager-init` subtask on every switch with no re-entrancy
    /// guard: a rapid switch-away-and-back to the same (already-cached) network
    /// fires a second eager-init for the same context before the first finishes
    /// wiring. `ensure_wallet_backend` serializes them behind the per-context
    /// `wallet_backend_build` mutex with a double-checked slot — the first builds
    /// and stores, the second re-checks under the guard, sees the populated slot,
    /// and no-ops. One open, one shared backend, no error. The eager-init entry
    /// `ensure_wallet_backend_and_start_spv` delegates its open to exactly this
    /// function, so guarding the open here covers that path too.
    ///
    /// Deleting the guard (fast-path recheck + build mutex + post-guard recheck)
    /// makes both racers reach `WalletBackend::new` and the second's open fails —
    /// verified: the test then panics on the `must succeed` expectation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_ensure_wallet_backend_does_not_double_open() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        assert!(
            ctx.wallet_backend().is_err(),
            "precondition: backend must be unwired before the concurrent race"
        );

        let ctx_a = Arc::clone(&ctx);
        let ctx_b = Arc::clone(&ctx);
        let sender_a = sender.clone();
        let sender_b = sender.clone();
        let a = tokio::spawn(async move { ctx_a.ensure_wallet_backend(sender_a).await });
        let b = tokio::spawn(async move { ctx_b.ensure_wallet_backend(sender_b).await });
        let (ra, rb) = tokio::join!(a, b);

        ra.expect("first-open task A must not panic")
            .expect("concurrent first-open A must succeed — a double-open would error");
        rb.expect("first-open task B must not panic")
            .expect("concurrent first-open B must succeed — a double-open would error");

        // Exactly one backend was built and both racers converged on it (first
        // writer wins; the second no-ops on the populated slot).
        let backend = ctx
            .wallet_backend()
            .expect("backend must be wired after the concurrent open");

        backend.shutdown().await;
    }

    /// A failure at the (fallible) wiring step must surface — the
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

    /// Cold-boot signability regression, adapted to the JIT secret model: a
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

    /// Leaving a network must not strand session-cached secrets on the
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
                        uses_password: false,
                        password_hint: None,
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

        // A no-password wallet persists the RAW seed via the seam (no legacy
        // envelope), and the xpub rides in the WalletMeta sidecar.
        let raw = WalletSeedView::new(&ctx.secret_store())
            .get_raw(&seed_hash)
            .expect("vault read must not error")
            .expect("the raw seed must be persisted at register time, even unwired");
        assert_eq!(
            &*raw, &seed,
            "persisted raw seed must equal the wallet seed"
        );
        assert!(
            WalletSeedView::new(&ctx.secret_store())
                .legacy_envelope_get(&seed_hash)
                .unwrap()
                .is_none(),
            "no legacy envelope is written for a no-password wallet"
        );
        let meta = WalletMetaView::new(&ctx.app_kv())
            .get(Network::Testnet, &seed_hash)
            .expect("wallet-meta sidecar persisted at register time");
        assert!(!meta.uses_password, "no-password wallet meta flag");
        assert_eq!(
            meta.xpub_encoded,
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

    /// QA-005 — `unregistered_open_wallet_count` must count a wallet whose
    /// `RwLock` is poisoned, so a prior panic can never let a premature
    /// "completed" sentinel through. The pre-QA-005 implementation counted over
    /// the `open_wallets()` snapshot, which drops a poisoned-lock wallet
    /// (`read().ok()...unwrap_or(false)`) before the fail-safe could see it —
    /// that version returns 0 here and fails this test.
    #[tokio::test]
    async fn unregistered_count_fails_safe_on_poisoned_wallet_lock() {
        let (ctx, _sender, _tmp) = offline_testnet_context();

        // One wallet, inserted straight into the map (no backend wired).
        let wallet =
            crate::model::wallet::Wallet::new_from_seed([0x42u8; 64], Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();
        let arc = Arc::new(std::sync::RwLock::new(wallet));
        ctx.wallets
            .write()
            .expect("wallets map lock")
            .insert(seed_hash, Arc::clone(&arc));

        // Poison the wallet's lock by panicking while holding its write guard.
        let poisoner = Arc::clone(&arc);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.write().expect("acquire write lock");
            panic!("intentional poison for the fail-safe test");
        })
        .join();
        assert!(
            arc.read().is_err(),
            "precondition: the wallet lock must be poisoned",
        );

        assert_eq!(
            ctx.unregistered_open_wallet_count(),
            1,
            "a poisoned wallet lock must fail safe (counted), not be silently dropped",
        );
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
    /// encrypted seed-envelope vault entry. Before the fix, `remove_wallet`
    /// only touched SQLite + the in-memory map, leaving the encrypted seed on
    /// disk. (DET's plaintext shielded sidecar was retired in Phase D; Orchard
    /// state now lives in the upstream coordinator, detached on removal.)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_wallet_wipes_seed_envelope() {
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

        // Precondition: the raw seed is present (no-password wallet stores raw).
        assert!(
            WalletSeedView::new(&ctx.secret_store())
                .get_raw(&seed_hash)
                .expect("vault read")
                .is_some(),
            "precondition: the raw seed must exist before removal"
        );

        ctx.remove_wallet(&seed_hash).expect("remove wallet");

        // The seed (the JIT decrypt source) is gone in BOTH forms.
        let store = ctx.secret_store();
        let view = WalletSeedView::new(&store);
        assert!(
            view.get_raw(&seed_hash)
                .expect("raw read after removal")
                .is_none(),
            "the raw seed must be deleted from the vault on removal"
        );
        assert!(
            view.legacy_envelope_get(&seed_hash)
                .expect("legacy read after removal")
                .is_none(),
            "any legacy envelope must also be gone on removal"
        );

        backend.shutdown().await;
    }

    /// Removing a wallet evicts its shielded balance snapshot from
    /// `AppContext::shielded_balances`. The seed hash is deterministic from the
    /// seed, so without eviction a re-import of the same recovery phrase would
    /// surface the removed wallet's stale shielded balance until the next sync
    /// overwrote it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_wallet_evicts_shielded_balance_snapshot() {
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

        let backend = ctx.wallet_backend().expect("backend wired");

        // Seed a snapshot entry as the sync-completed push writer would.
        ctx.shielded_balances
            .lock()
            .expect("lock shielded_balances")
            .insert(seed_hash, 123_456);
        assert_eq!(
            ctx.shielded_balance_credits(&seed_hash),
            123_456,
            "precondition: the snapshot entry must exist before removal"
        );

        ctx.remove_wallet(&seed_hash).expect("remove wallet");

        assert!(
            ctx.shielded_balances
                .lock()
                .expect("lock shielded_balances")
                .get(&seed_hash)
                .is_none(),
            "the shielded balance snapshot must be evicted on removal"
        );

        backend.shutdown().await;
    }

    /// F17/F20 (fresh-install regression): removing a wallet must still wipe
    /// its secret-bearing state on a truly-fresh install where the legacy
    /// `wallet`/`wallet_addresses`/`utxos` tables are gated OUT of the schema.
    ///
    /// The sibling `remove_wallet_wipes_seed_envelope`
    /// builds its context with `create_tables(true)`, which force-creates
    /// those legacy tables and therefore masks this path. Here the real
    /// `Database::initialize` fresh path runs, so the unguarded
    /// `SELECT address FROM wallet_addresses` in `Database::remove_wallet`
    /// errored with `no such table` and propagated through
    /// `AppContext::remove_wallet` BEFORE the secret wipe — leaving the seed
    /// envelope on disk. The existence-guarded
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

        // Precondition: the raw seed exists.
        assert!(
            WalletSeedView::new(&ctx.secret_store())
                .get_raw(&seed_hash)
                .expect("vault read")
                .is_some(),
            "precondition: the raw seed must exist before removal"
        );

        // Pre-fix this returned `Err(no such table: wallet_addresses)` and the
        // wipe below never ran.
        ctx.remove_wallet(&seed_hash)
            .expect("remove_wallet must succeed on a fresh install");

        let store = ctx.secret_store();
        let view = WalletSeedView::new(&store);
        assert!(
            view.get_raw(&seed_hash)
                .expect("raw read after removal")
                .is_none(),
            "the raw seed must be deleted from the vault on a fresh install"
        );
        assert!(
            view.legacy_envelope_get(&seed_hash)
                .expect("legacy read after removal")
                .is_none(),
            "no legacy envelope must survive removal on a fresh install"
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
                .get_raw(&seed_hash)
                .expect("vault read")
                .is_some(),
            "precondition: raw seed must exist before clear"
        );

        ctx.clear_network_database()
            .expect("clear_network_database should succeed");

        // The wallet must not rehydrate: its meta and seed (both forms) are gone.
        assert!(
            WalletMetaView::new(&ctx.app_kv())
                .get(Network::Testnet, &seed_hash)
                .is_none(),
            "wallet-meta sidecar must be empty after clear (no rehydration)"
        );
        let store = ctx.secret_store();
        let view = WalletSeedView::new(&store);
        assert!(
            view.get_raw(&seed_hash)
                .expect("raw read after clear")
                .is_none(),
            "raw seed must be deleted from the vault after clear"
        );
        assert!(
            view.legacy_envelope_get(&seed_hash)
                .expect("legacy read after clear")
                .is_none(),
            "no legacy envelope must survive clear"
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

    /// When the wallet-meta sidecar write fails, `register_wallet`
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
                    // Unprotected wallet: salt/nonce must be empty,
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
                    // Unprotected wallet: salt/nonce must be empty, the
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

    /// Protected cold-start hydration — a *password-protected* wallet migrated
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

        // Tier-2 keep-protection migration post-conditions. The
        // unlock decrypted the legacy AES-GCM envelope and RE-WRAPPED the seed
        // as a Tier-2 object-password envelope (protection KEPT, not downgraded
        // to a raw secret), then dropped the legacy envelope.
        let store = ctx.secret_store();
        let seed_view = WalletSeedView::new(&store);
        // Steady state is Tier-2 protected.
        assert_eq!(
            seed_view.scheme(&seed_hash).expect("scheme"),
            crate::wallet_backend::secret_seam::SecretScheme::Protected,
            "the seed must be re-wrapped to Tier-2, never downgraded to raw"
        );
        // A raw (password-free) read of a protected seed must fail — never strip.
        assert!(
            seed_view.get_raw(&seed_hash).is_err(),
            "a raw read of a Tier-2-protected seed must fail"
        );
        // It reads back only WITH the object password, byte-for-byte.
        let pw = platform_wallet_storage::secrets::SecretString::new(passphrase);
        let protected = seed_view
            .get_protected(&seed_hash, &pw)
            .expect("protected read")
            .expect("the seed must be re-stored as Tier-2 after the migrating unlock");
        assert_eq!(
            &*protected, &seed,
            "Tier-2 seed must equal the true 64-byte seed"
        );
        assert!(
            seed_view
                .legacy_envelope_get(&seed_hash)
                .expect("legacy read")
                .is_none(),
            "the legacy envelope must be deleted after migration"
        );
        // The sidecar password flag STAYS true — protection was kept, so the
        // metadata stays accurate (no downgrade flip).
        let meta = WalletMetaView::new(&ctx.app_kv())
            .get(Network::Testnet, &seed_hash)
            .expect("wallet meta present");
        assert!(
            meta.uses_password,
            "WalletMeta.uses_password must stay true — Tier-2 keeps protection"
        );

        // A SECOND secret resolve still requires the object password (Tier-2 is
        // not prompt-free): a scripted prompt that supplies it resolves the seed.
        use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
        use crate::wallet_backend::{SecretAccess, SecretScope};
        let prompt = std::sync::Arc::new(TestPrompt::new([ScriptedAnswer::once(passphrase)]));
        let sa = SecretAccess::new(ctx.secret_store(), prompt.clone(), Network::Testnet);
        let resolved = sa
            .with_secret(&SecretScope::HdSeed { seed_hash }, |pt| {
                Ok(pt.expose_hd_seed().copied())
            })
            .await
            .expect("second resolve with the password");
        assert_eq!(resolved, Some(seed), "password resolve returns the seed");
        assert_eq!(
            prompt.ask_count(),
            1,
            "the protected seed prompts exactly once"
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

    // ──────────────────────────────────────────────────────────────────────
    // Automatic identity-discovery trigger / latch / re-arm
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn all_wallets_discovery_latch_is_one_shot_until_stop_spv() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");

        assert!(
            !ctx.identity_autodiscovery_fired.load(Ordering::SeqCst),
            "latch starts unfired"
        );

        // First fire latches; a second fire is swallowed (no second sweep).
        ctx.queue_all_wallets_identity_discovery();
        assert!(
            ctx.identity_autodiscovery_fired.load(Ordering::SeqCst),
            "first call must set the one-shot latch"
        );
        ctx.queue_all_wallets_identity_discovery();
        assert!(
            ctx.identity_autodiscovery_fired.load(Ordering::SeqCst),
            "latch stays set; the second call is a no-op"
        );

        // stop_spv re-arms the latch so the next reconnect runs discovery again.
        ctx.stop_spv().await;
        assert!(
            !ctx.identity_autodiscovery_fired.load(Ordering::SeqCst),
            "stop_spv must clear the latch to re-arm discovery on reconnect"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_wallets_snapshot_excludes_locked_wallets() {
        use crate::database::test_helpers::seed_legacy_protected_hd_wallet_row;
        use crate::model::wallet::encryption::encrypt_message;

        let (ctx, sender, _tmp) = offline_testnet_context();

        // A locked, password-protected wallet staged via the legacy migration
        // row: it hydrates `WalletSeed::Closed` and must be excluded.
        let locked_seed = [0x77u8; 64];
        let locked_hash: WalletSeedHash =
            crate::model::wallet::ClosedKeyItem::compute_seed_hash(&locked_seed);
        let epk = legacy_master_epk_bytes(&locked_seed);
        let (encrypted_seed, salt, nonce) =
            encrypt_message(&locked_seed, "a-passphrase-never-fed-back").expect("encrypt seed");
        seed_legacy_protected_hd_wallet_row(
            &ctx.db,
            &locked_hash,
            &encrypted_seed,
            &salt,
            &nonce,
            &epk,
            "locked-wallet",
            None,
            Network::Testnet,
        )
        .expect("insert legacy protected wallet row");

        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");

        // An open, no-password wallet registered alongside it.
        let open_seed = [0x66u8; 64];
        let open_wallet =
            crate::model::wallet::Wallet::new_from_seed(open_seed, Network::Testnet, None, None)
                .expect("build open wallet");
        let open_hash = open_wallet.seed_hash();
        ctx.register_wallet(open_wallet, &open_seed, WalletOrigin::Fresh)
            .expect("register open wallet");

        let snapshot: Vec<WalletSeedHash> = ctx
            .open_wallets()
            .iter()
            .map(|w| w.read().unwrap().seed_hash())
            .collect();

        assert!(
            snapshot.contains(&open_hash),
            "the open wallet must be in the snapshot"
        );
        assert!(
            !snapshot.contains(&locked_hash),
            "the locked protected wallet must be excluded from the snapshot"
        );

        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rediscovery_update_preserves_user_alias_and_wallet_binding() {
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::platform::Identifier;
        use std::collections::BTreeMap;

        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");

        let identity_id = Identifier::from([7u8; 32]);
        let make_qi = |alias: Option<&str>| {
            let identity = Identity::create_basic_identity(identity_id, ctx.platform_version())
                .expect("basic identity");
            QualifiedIdentity {
                identity,
                associated_voter_identity: None,
                associated_operator_identity: None,
                associated_owner_key_id: None,
                identity_type: IdentityType::User,
                alias: alias.map(str::to_string),
                private_keys: KeyStorage {
                    private_keys: BTreeMap::new(),
                },
                dpns_names: vec![],
                associated_wallets: BTreeMap::new(),
                secret_access: None,
                wallet_index: None,
                top_ups: BTreeMap::new(),
                status: IdentityStatus::Active,
                network: Network::Testnet,
            }
        };

        // Initial store with a user alias and a wallet binding.
        let wallet_hash: WalletSeedHash = [0x09u8; 32];
        ctx.insert_local_qualified_identity(&make_qi(Some("my-id")), &Some((wallet_hash, 3)))
            .expect("insert identity with alias");

        // Simulate re-discovery: build a FRESH QI with no alias, carry the
        // existing alias (the carry-over under test), then update in place.
        let mut refreshed = make_qi(None);
        let existing = ctx
            .get_identity_by_id(&identity_id)
            .expect("load existing")
            .expect("identity present");
        refreshed.alias = existing.alias;
        ctx.update_local_qualified_identity(&refreshed)
            .expect("update preserving alias");

        // The alias survives, and the wallet binding is preserved by the update.
        let reloaded = ctx
            .get_identity_by_id(&identity_id)
            .expect("reload identity")
            .expect("identity present after update");
        assert_eq!(
            reloaded.alias.as_deref(),
            Some("my-id"),
            "the user alias must survive a re-discovery update (F-1 regression guard)"
        );
        assert_eq!(
            reloaded.wallet_index,
            Some(3),
            "the wallet binding index must be preserved across the update"
        );

        backend.shutdown().await;
    }

    /// C.7 regression guard: `ensure_identity_funding_accounts` must succeed on
    /// a cold-booted (watch-only) wallet for a fresh `IdentityTopUp{index}`.
    ///
    /// # Background
    ///
    /// DET always reloads wallets **seedless** from the upstream persister.
    /// `WalletBackend::new` → `load_from_persistor_seedless` → upstream
    /// `load_from_persistor()` → `Wallet::new_watch_only(…)`.  The wallet has
    /// the BIP44/BIP32 accounts it was persisted with, but **no root private
    /// key**.
    ///
    /// `WalletAccountCreationOptions::Default` (used by
    /// `register_wallet_from_seed`) creates `IdentityRegistration` by default
    /// and persists it in the account manifest.  `IdentityTopUp{n}` is NOT
    /// created by default — it is added only after a register/top-up, so on
    /// every cold boot the manifest lacks it.
    ///
    /// Before the fix, `provision_identity_funding_account` called
    /// `kw.add_account(account_type, None)`.  On a cold-boot wallet that path
    /// reaches `root_extended_keys.rs:428` and fails:
    ///
    ///   `WalletBackend { source: AssetLockTransaction("Invalid parameter:
    ///    Watch-only wallet has no private key") }`
    ///
    /// After the fix it builds a short-lived signable wallet from the provided
    /// seed bytes, derives the account xpub, and calls
    /// `kw.add_account(account_type, Some(xpub))` — succeeds regardless of
    /// private-key availability.
    ///
    /// # Why deterministic
    ///
    /// The cold-booted wallet unconditionally has no root private key; the
    /// failure path is hit every time regardless of timing or network state.
    ///
    /// # Test structure
    ///
    /// Two-boot scenario to match production:
    ///   1. **Boot 1**: wire backend, write both sidecars (wallet-meta + upstream
    ///      persister) from seed.
    ///   2. **Boot 2 (cold)**: `WalletBackend::new` over a copy of the same
    ///      data dir runs `load_from_persistor_seedless` — the upstream wallet is
    ///      loaded watch-only.  Then `ensure_identity_funding_accounts` for a
    ///      fresh `IdentityTopUp{3}` must return `Ok`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ensure_identity_funding_accounts_succeeds_on_cold_booted_watch_only_wallet() {
        // ── Boot 1: write wallet-meta sidecar + upstream persister from seed ──

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let seed = [0xC7u8; 64];
        let seed_hash = {
            let wallet = crate::model::wallet::Wallet::new_from_seed(
                seed,
                Network::Testnet,
                None,
                None, // no password
            )
            .expect("build wallet");
            let h = wallet.seed_hash();

            let (ctx, sender) = offline_testnet_context_at(temp_dir.path());

            // Register the wallet BEFORE wiring the backend.  register_wallet
            // writes the DET sidecars (seed-envelope vault + wallet-meta), but
            // register_wallet_upstream checks ctx.wallet_backend() and, finding it
            // not yet wired, returns early without spawning the background
            // "wallet_upstream_registration" subtask.  This avoids the concurrency
            // hazard: if the backend were wired first the background subtask would
            // race with the synchronous register_wallet_from_seed call below —
            // both call create_wallet_from_seed_bytes for the same wallet.  The
            // upstream register_wallet inserts into wallet_manager (step A) and into
            // self.wallets (step B) with async work in between; a concurrent caller
            // that arrives between A and B sees WalletAlreadyExists but then
            // get_wallet returns None → WalletNotFound panic.  Under CI load
            // (1000+ concurrent tests) this window is reliably hit.
            ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
                .expect("boot 1: ctx.register_wallet");

            // Wire the backend now so the explicit registration below has the
            // upstream persister available.
            ctx.ensure_wallet_backend(sender)
                .await
                .expect("boot 1: ensure_wallet_backend offline");

            // Write the upstream persister synchronously — no background subtask
            // is in flight (we didn't wire the backend when register_wallet ran),
            // so this call is race-free.
            let backend1 = ctx.wallet_backend().expect("boot 1 backend");
            backend1
                .register_wallet_from_seed(&h, &seed, Some(0))
                .await
                .expect("boot 1: upstream register");
            backend1.shutdown().await;

            h
        };
        // ctx is dropped here, releasing app_kv / secret_store file handles.

        // ── Cold-boot copy: avoid file-lock conflicts with lingering subtasks ──
        //
        // The background registration subtask may still hold an Arc<WalletBackend>
        // (and thus an open SqlitePersister handle on temp_dir).  We copy the
        // on-disk state to a fresh path so Boot 2's SqlitePersister::open does
        // not collide with the old one.  Identical on-disk bytes — the fund-
        // routing gate and the persisted manifest are preserved.
        let cold_dir = tempfile::tempdir().expect("cold tempdir");
        fn copy_dir_rec(src: &std::path::Path, dst: &std::path::Path) {
            std::fs::create_dir_all(dst).expect("mkdir");
            for entry in std::fs::read_dir(src).expect("read_dir") {
                let entry = entry.expect("entry");
                let to = dst.join(entry.file_name());
                if entry.path().is_dir() {
                    copy_dir_rec(&entry.path(), &to);
                } else {
                    std::fs::copy(entry.path(), to).expect("copy file");
                }
            }
        }
        copy_dir_rec(temp_dir.path(), cold_dir.path());

        // ── Boot 2 (cold): load from persister → watch-only upstream wallet ──

        let (ctx2, sender2) = offline_testnet_context_at(cold_dir.path());
        ctx2.ensure_wallet_backend(sender2)
            .await
            .expect("boot 2 (cold): ensure_wallet_backend offline");
        let backend2 = ctx2.wallet_backend().expect("boot 2 backend");

        assert!(
            backend2.is_wallet_registered(&seed_hash),
            "cold boot must load the wallet from the persisted sidecars"
        );

        // `IdentityTopUp{3}` is absent from the account manifest (it is never
        // created by WalletAccountCreationOptions::Default) — so the cold-booted
        // watch-only wallet triggers the provisioning branch.
        //
        // Before the fix: kw.add_account(IdentityTopUp{3}, None)
        //   → "Watch-only wallet has no private key" → Err
        // After the fix: builds a seed wallet, derives the account xpub,
        //   calls kw.add_account(IdentityTopUp{3}, Some(xpub)) → Ok
        let registration_index = 3u32;
        backend2
            .ensure_identity_funding_accounts(&seed_hash, &seed, registration_index)
            .await
            .expect(
                "cold-booted watch-only wallet: IdentityTopUp{3} provisioning must succeed; \
                 if 'Watch-only wallet has no private key' appears the fix has been reverted",
            );

        // Idempotent: both accounts now present — second call is a no-op.
        backend2
            .ensure_identity_funding_accounts(&seed_hash, &seed, registration_index)
            .await
            .expect("second call must be idempotent (both accounts already present)");

        backend2.shutdown().await;
    }

    /// Build a minimal basic identity for manager-reconcile tests — only its
    /// id() and (empty) public_keys() are read by `add_identity`.
    fn basic_test_identity() -> dash_sdk::dpp::identity::Identity {
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;
        use dash_sdk::platform::Identifier;
        Identity::create_basic_identity(Identifier::random(), PlatformVersion::latest())
            .expect("basic identity")
    }

    /// Wrap a basic identity in a minimal wallet-owned `QualifiedIdentity` for
    /// sidecar-reconcile tests.
    fn wallet_owned_qualified_identity(
        wallet_index: Option<u32>,
    ) -> crate::model::qualified_identity::QualifiedIdentity {
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
        QualifiedIdentity {
            identity: basic_test_identity(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: vec![],
            associated_wallets: std::collections::BTreeMap::new(),
            secret_access: None,
            wallet_index,
            top_ups: std::collections::BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    /// `ensure_identity_managed` on a wallet that is not upstream-registered
    /// fails with `WalletNotLoaded` (and the reconcile driver logs-and-skips it
    /// rather than aborting).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ensure_identity_managed_unregistered_wallet_is_wallet_not_loaded() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");

        let unknown_seed: WalletSeedHash = [0x5Au8; 32];
        let identity = basic_test_identity();
        let err = backend
            .ensure_identity_managed(&unknown_seed, &identity, 0)
            .await
            .expect_err("an unregistered wallet must not resolve");
        assert!(
            matches!(err, TaskError::WalletNotLoaded),
            "expected WalletNotLoaded, got: {err:?}"
        );

        backend.shutdown().await;
    }

    /// `ensure_identity_managed` registers a previously-unknown identity (→
    /// `true`), then a second call is a no-op (→ `false`). Runs with no secret
    /// session promoted, proving the reconcile is seed-free / locked-safe.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ensure_identity_managed_registers_then_noops_while_locked() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");

        let seed = [0x2Cu8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();
        backend
            .register_wallet_from_seed(&seed_hash, &seed, None)
            .await
            .expect("register wallet with upstream manager");

        let identity = basic_test_identity();

        // No secret session is open here — the wallet is effectively locked.
        let first = backend
            .ensure_identity_managed(&seed_hash, &identity, 0)
            .await
            .expect("registering a new identity must succeed while locked");
        assert!(first, "first call newly registers the identity");

        let second = backend
            .ensure_identity_managed(&seed_hash, &identity, 0)
            .await
            .expect("second call must be idempotent");
        assert!(!second, "second call is a no-op (already managed)");

        backend.shutdown().await;
    }

    /// `reconcile_managed_identities` registers exactly the wallet-owned
    /// identities (`wallet_index.is_some()` and matching `seed_hash`) and leaves
    /// index-less sidecar entries alone — proven via the idempotent
    /// `ensure_identity_managed` (already-managed → `false`, never-managed →
    /// `true`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reconcile_managed_identities_registers_only_wallet_owned() {
        let (ctx, sender, _tmp) = offline_testnet_context();
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("ensure_wallet_backend should succeed offline");
        let backend = ctx.wallet_backend().expect("backend wired");

        let seed = [0x3Du8; 64];
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(seed, Network::Testnet, None, None)
                .expect("build wallet");
        let seed_hash = wallet.seed_hash();
        backend
            .register_wallet_from_seed(&seed_hash, &seed, None)
            .await
            .expect("register wallet with upstream manager");

        // Two wallet-owned identities (should be reconciled) and one index-less
        // identity (should be skipped by the `wallet_index.is_some()` filter).
        let owned_a = wallet_owned_qualified_identity(Some(0));
        let owned_b = wallet_owned_qualified_identity(Some(1));
        let detached = wallet_owned_qualified_identity(None);
        ctx.insert_local_qualified_identity(&owned_a, &Some((seed_hash, 0)))
            .expect("insert owned_a");
        ctx.insert_local_qualified_identity(&owned_b, &Some((seed_hash, 1)))
            .expect("insert owned_b");
        ctx.insert_local_qualified_identity(&detached, &None)
            .expect("insert detached");

        ctx.reconcile_managed_identities(&backend, &seed_hash).await;

        // The two wallet-owned identities are now managed → ensure is a no-op.
        assert!(
            !backend
                .ensure_identity_managed(&seed_hash, &owned_a.identity, 0)
                .await
                .expect("owned_a"),
            "wallet-owned identity A must already be managed after reconcile"
        );
        assert!(
            !backend
                .ensure_identity_managed(&seed_hash, &owned_b.identity, 1)
                .await
                .expect("owned_b"),
            "wallet-owned identity B must already be managed after reconcile"
        );
        // The index-less identity was skipped → ensure newly registers it.
        assert!(
            backend
                .ensure_identity_managed(&seed_hash, &detached.identity, 0)
                .await
                .expect("detached"),
            "index-less identity must have been skipped by the reconcile filter"
        );

        backend.shutdown().await;
    }
}
