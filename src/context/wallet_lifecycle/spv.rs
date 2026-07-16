//! SPV / chain-storage lifecycle: wiring the wallet backend, starting and
//! stopping chain sync, and clearing per-network SPV and database state.

use super::*;

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

    pub async fn clear_network_database(self: &Arc<Self>) -> Result<(), TaskError> {
        let backend = self
            .wallet_backend()
            .map_err(|_| TaskError::WalletDataClearUnavailable)?;

        // F60: permanently delete every wallet's secret-bearing state so the
        // "delete all local data" promise holds — wallets must NOT rehydrate
        // on next launch and encrypted seeds must NOT persist. Clear the
        // persisted state (seed-envelope vault, wallet-meta + single-key
        // sidecars, shielded notes, session cache) BEFORE the in-memory maps
        // below, so a mid-failure crash cannot strand current state. The
        // pre-update database remains a read-only recovery artifact. The
        // upstream (watch-only) persistor rows have no seed and are removed
        // asynchronously off the main thread.
        let ClearAllOutcome {
            upstream_ids,
            mut failures,
        } = backend.forget_all_wallets_local();
        for wallet_id in upstream_ids {
            let backend = Arc::clone(&backend);
            self.subtasks
                .spawn_sync("wallet_upstream_removal", async move {
                    if let Err(error) = backend.remove_upstream_wallet(&wallet_id).await {
                        tracing::warn!(%error, "Upstream wallet removal failed during clear");
                    }
                });
        }

        // D4d: drain the DashPay k/v sidecar. The Global-scoped overlays
        // (blocked / rejected markers, timestamps, reverse address map)
        // share the `det:dashpay:` prefix and come out in one sweep. The
        // per-contact private memos and address-index cursors now live in
        // each owner's `DetScope::Identity` scope (Wave 2 promotion), which
        // the Global sweep cannot reach — so fan the per-owner clear out
        // over the identity index.
        let kv = backend.kv();
        match kv.list(DetScope::Global, Some("det:dashpay:")) {
            Ok(keys) => {
                for k in keys {
                    if let Err(source) = kv.delete(DetScope::Global, &k) {
                        tracing::warn!(key = %k, error = ?source, "DashPay sidecar delete failed");
                        failures.push(TaskError::DashpaySidecarStorage { source });
                    }
                }
            }
            Err(source) => {
                tracing::warn!(error = ?source, "DashPay sidecar listing failed");
                failures.push(TaskError::DashpaySidecarStorage { source });
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
                        failures.push(e);
                    }
                    // Wipe each identity's vault keys and det:identity:* records too —
                    // Tier-1 keyless identity keys (incl. masternode voting/owner/payout)
                    // are plaintext-recoverable, so a full wipe must remove them as well.
                    if let Err(e) = self.delete_local_qualified_identity(&owner) {
                        tracing::warn!(
                            owner = %owner,
                            "Identity private-key wipe failed during clear: {e:?}"
                        );
                        failures.push(e);
                    }
                }
            }
            Err(e) => {
                // A listing failure skips every per-identity key wipe, so it must
                // surface as an incomplete clear — never a silent success that
                // leaves identity private keys on disk.
                tracing::warn!("Identity index listing for DashPay clear failed: {e:?}");
                failures.push(e);
            }
        }

        // Reset the upstream shielded coordinator (quiesces its sync loop and
        // empties the per-network store) and unlink DET's two retired legacy
        // shielded files. The legacy-file unlinks are synchronous and scoped
        // strictly to THIS network's spv directory.
        cleanup_legacy_shielded_files(backend.spv_storage_dir())?;

        if let Err(error) = backend.clear_shielded().await {
            tracing::warn!(%error, "Shielded coordinator reset failed during clear");
            failures.push(error);
        }

        if let Ok(mut wallets) = self.wallets.write() {
            wallets.clear();
        }

        if let Ok(mut single_key_wallets) = self.single_key_wallets.write() {
            single_key_wallets.clear();
        }

        self.has_wallet.store(false, Ordering::Relaxed);

        // Any secret-bearing delete that failed above means data may survive on
        // disk, so never report a clean wipe. The in-memory maps are still
        // cleared; the typed error tells the user to restart and retry.
        if !failures.is_empty() {
            let failed = failures.len();
            let first_error = Box::new(failures.into_iter().next().expect("failures is non-empty"));
            return Err(TaskError::WalletDataClearIncomplete {
                failed,
                first_error,
            });
        }

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
        // Forward-compat: `start()`'s signature is fallible though the current
        // impl is infallible. The reachable start-time failure today is the
        // wiring step above, surfaced via `mark_spv_error`; this branch keeps
        // the start step covered should `start()` begin to fail.
        if let Err(e) = backend.start().await {
            self.mark_spv_error(&e);
        }
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
        // See this method's doc comment for why the reconnect cannot hit
        // `WalletStorageError::AlreadyOpen`.
        //
        // Restart-in-place runtime safety: all three upstream coordinators clear
        // their cancel slot under a `background_generation` guard, so a rapid
        // reconnect cannot leak an uncancellable / duplicate sync loop.
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
}
