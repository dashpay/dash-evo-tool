//! Polling helpers for waiting on async state changes.

use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::WalletSeedHash;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Wait until a wallet has at least `min_balance` total duffs (including unconfirmed),
/// polling every 2s. Triggers SPV reconciliation on each poll.
pub async fn wait_for_balance(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
    min_balance: u64,
    wait_timeout: Duration,
) -> Result<u64, String> {
    let start = std::time::Instant::now();
    timeout(wait_timeout, async {
        let mut poll_count = 0u32;
        loop {
            let balance = Some(app_context.snapshot_balance(&wallet_hash).total);
            poll_count += 1;
            if let Some(b) = balance
                && b >= min_balance
            {
                tracing::trace!(
                    elapsed_ms = start.elapsed().as_millis(),
                    polls = poll_count,
                    balance = b,
                    "wait_for_balance: satisfied"
                );
                return b;
            }
            if poll_count.is_multiple_of(5) {
                tracing::trace!(
                    elapsed_ms = start.elapsed().as_millis(),
                    polls = poll_count,
                    current = ?balance,
                    target = min_balance,
                    "wait_for_balance: polling..."
                );
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "Timed out waiting for total balance >= {} duffs",
            min_balance
        )
    })
}

/// Wait until a wallet has at least `min_balance` **spendable** duffs.
///
/// "Spendable" is `DetWalletBalance::spendable()` — the exact set the upstream
/// `CoinSelector` draws from (confirmed + unconfirmed), excluding the immature
/// and locked duffs that only `total` counts. This is the right gate for "can
/// this wallet fund a transaction now": funds that are IS-locked but not yet
/// flagged as instant-locked locally land in `unconfirmed`, so polling
/// `confirmed` alone would miss them and time out even though coin selection
/// could already spend them. Triggers SPV reconciliation on each poll.
pub async fn wait_for_spendable_balance(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
    min_balance: u64,
    wait_timeout: Duration,
) -> Result<u64, String> {
    let start = std::time::Instant::now();
    timeout(wait_timeout, async {
        let mut poll_count = 0u32;
        loop {
            let balance = Some(app_context.snapshot_balance(&wallet_hash).spendable());
            poll_count += 1;
            if let Some(b) = balance
                && b >= min_balance
            {
                tracing::trace!(
                    elapsed_ms = start.elapsed().as_millis(),
                    polls = poll_count,
                    balance = b,
                    "wait_for_spendable_balance: satisfied"
                );
                return b;
            }
            if poll_count.is_multiple_of(5) {
                tracing::trace!(
                    elapsed_ms = start.elapsed().as_millis(),
                    polls = poll_count,
                    current = ?balance,
                    target = min_balance,
                    "wait_for_spendable_balance: polling..."
                );
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
    .await
    .map_err(|_| {
        // Report spendable and total for diagnostics
        let snap = app_context.snapshot_balance(&wallet_hash);
        let (spendable, total) = (snap.spendable(), snap.total);
        format!(
            "Timed out waiting for spendable balance >= {} duffs \
             (spendable: {}, total: {})",
            min_balance, spendable, total
        )
    })
}

/// Ensure a DET-registered wallet is registered with the upstream wallet
/// backend so its addresses are monitored by the `SpvRuntime`.
///
/// Chain sync is owned by upstream `platform-wallet`. A wallet becomes
/// "tracked" once `WalletBackend::ensure_wallets_registered` has run
/// `create_wallet_from_seed_bytes` for it (idempotent), at which point its
/// derived addresses are watched and balance events flow back through the
/// `EventBridge`. This drives that registration and confirms the upstream
/// backend has a snapshot slot for the wallet.
pub async fn wait_for_wallet_in_spv(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
    wait_timeout: Duration,
) -> Result<(), String> {
    let backend = app_context
        .wallet_backend()
        .map_err(|e| format!("Wallet backend not wired: {e}"))?;

    timeout(wait_timeout, async {
        loop {
            match backend.ensure_wallets_registered(app_context).await {
                Ok(()) => {
                    if backend.is_wallet_registered(&wallet_hash) {
                        return Ok(());
                    }
                }
                // `TaskError::WalletBackend` is the upstream wallet runtime's
                // documented "retry in a moment" signal — transient under the
                // serial suite's burst of registrations. Retry within the
                // existing timeout budget. Any other typed error is terminal.
                Err(TaskError::WalletBackend { .. }) => {}
                Err(e) => return Err(format!("ensure_wallets_registered failed: {e}")),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "Timed out after {:?} waiting for wallet to register with the upstream backend",
            wait_timeout
        )
    })?
}

/// Longest the initial SPV sync may go without advancing its progress token
/// before [`wait_for_spv_sync`] reports it stalled.
pub const SPV_STALL_WINDOW: Duration = Duration::from_secs(180);

/// Hard cap on the whole initial SPV sync wait, however steadily it advances.
/// Equals the former worst case (3 init attempts × 600s), now spent on one
/// runtime instead of restarting sync from genesis on a fresh slot.
pub const SPV_SYNC_CAP: Duration = Duration::from_secs(1800);

/// What [`wait_for_spv_sync`] does after a poll that did not see `Running`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpvWaitVerdict {
    /// Keep waiting on the same runtime.
    Continue,
    /// No progress for the stall window.
    Stalled,
    /// The hard cap elapsed.
    Exhausted,
}

/// Decide whether the initial SPV sync wait continues.
///
/// `since_progress` is the time since the progress token last advanced (or
/// since the wait began, if it never has); `elapsed` is the total wait. The cap
/// wins over the stall window so the wait is bounded even while progressing.
pub fn spv_wait_verdict(
    since_progress: Duration,
    elapsed: Duration,
    stall_window: Duration,
    cap: Duration,
) -> SpvWaitVerdict {
    if elapsed >= cap {
        SpvWaitVerdict::Exhausted
    } else if since_progress >= stall_window {
        SpvWaitVerdict::Stalled
    } else {
        SpvWaitVerdict::Continue
    }
}

/// Whether the monotonic SPV progress token
/// ([`spv_progress_token`](dash_evo_tool::context::connection_status::spv_progress_token))
/// moved forward. `None` (no phase actively syncing) never counts as progress,
/// and neither does a token that went back or stayed put.
pub fn spv_progress_advanced(last: Option<u64>, now: Option<u64>) -> bool {
    matches!((last, now), (_, Some(n)) if last.is_none_or(|l| n > l))
}

/// Why the initial SPV sync wait gave up.
#[derive(Debug, thiserror::Error)]
pub enum SpvSyncWaitError {
    #[error(
        "SPV sync made no progress for {stalled_for:?} (stall window {stall_window:?}, \
         {elapsed:?} into the wait); last progress token {last_token:?}"
    )]
    Stalled {
        stalled_for: Duration,
        stall_window: Duration,
        elapsed: Duration,
        last_token: Option<u64>,
    },
    #[error(
        "SPV sync was still progressing but not complete after {elapsed:?} (cap {cap:?}); \
         last progress token {last_token:?}"
    )]
    Exhausted {
        elapsed: Duration,
        cap: Duration,
        last_token: Option<u64>,
    },
}

/// Wait for SPV to complete initial sync (all managers including masternodes).
///
/// `SpvStatus::Running` is set after `SyncComplete` fires, which means
/// MempoolManager is activated and bloom filter is built.
///
/// Progress-aware and bounded: the wait continues on the SAME runtime (and so
/// the same workdir slot, with its stored headers and filters) while the
/// progress token keeps advancing, fails once it stalls for `stall_window`, and
/// never exceeds `cap`. A slow sync therefore resumes instead of panicking into
/// an init retry, which would land on a fresh slot and restart from genesis.
pub async fn wait_for_spv_sync(
    app_context: &Arc<AppContext>,
    stall_window: Duration,
    cap: Duration,
) -> Result<(), SpvSyncWaitError> {
    use dash_evo_tool::context::connection_status::spv_progress_token;
    use dash_evo_tool::model::spv_status::SpvStatus;

    let status = app_context.connection_status();
    let started = tokio::time::Instant::now();
    let mut last_progress_at = started;
    let mut last_token: Option<u64> = None;
    loop {
        if status.spv_status() == SpvStatus::Running {
            return Ok(());
        }
        let token = status
            .spv_sync_progress()
            .as_ref()
            .and_then(spv_progress_token);
        let now = tokio::time::Instant::now();
        if spv_progress_advanced(last_token, token) {
            last_token = token;
            last_progress_at = now;
        }
        let since_progress = now - last_progress_at;
        let elapsed = now - started;
        match spv_wait_verdict(since_progress, elapsed, stall_window, cap) {
            SpvWaitVerdict::Continue => {}
            SpvWaitVerdict::Stalled => {
                return Err(SpvSyncWaitError::Stalled {
                    stalled_for: since_progress,
                    stall_window,
                    elapsed,
                    last_token,
                });
            }
            SpvWaitVerdict::Exhausted => {
                return Err(SpvSyncWaitError::Exhausted {
                    elapsed,
                    cap,
                    last_token,
                });
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Wait until the upstream `SpvRuntime` has connected to peers and begun
/// syncing.
///
/// Chain sync is upstream-owned; the only signal DET observes is the
/// push-based `ConnectionStatus` fed by the wallet-backend `EventBridge`.
/// `on_progress` / `on_sync_event` move the status to `Syncing` (or
/// `Running`) only once a peer is connected and header sync has started —
/// upstream cannot make sync progress without a peer. The dedicated
/// `PeersUpdated` peer-count atomic is not reliably populated by this
/// upstream revision, so an active sync status (not the raw count) is the
/// authoritative "we have peers" signal.
pub async fn wait_for_spv_peers(
    app_context: &Arc<AppContext>,
    wait_timeout: Duration,
) -> Result<(), String> {
    use dash_evo_tool::model::spv_status::SpvStatus;
    let cs = app_context.connection_status();
    timeout(wait_timeout, async {
        loop {
            // A non-zero peer count is the strongest signal when present,
            // but `Syncing`/`Running` already implies a connected peer.
            if cs.spv_connected_peers() > 0
                || matches!(cs.spv_status(), SpvStatus::Syncing | SpvStatus::Running)
            {
                return Ok(());
            }
            if cs.spv_status() == SpvStatus::Error {
                return Err(format!(
                    "SPV entered Error state while waiting for peers: {}",
                    cs.spv_last_error().unwrap_or_default()
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "Timed out after {:?} waiting for SPV to connect to a peer (status: {})",
            wait_timeout,
            cs.spv_status()
        )
    })?
}

#[cfg(test)]
mod spv_wait_tests {
    use super::*;

    const STALL: Duration = SPV_STALL_WINDOW;
    const CAP: Duration = SPV_SYNC_CAP;
    const fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn keeps_waiting_while_progressing_past_the_old_600s_deadline() {
        assert_eq!(
            spv_wait_verdict(secs(5), secs(900), STALL, CAP),
            SpvWaitVerdict::Continue
        );
    }

    #[test]
    fn stalls_after_the_stall_window_without_progress() {
        assert_eq!(
            spv_wait_verdict(STALL, secs(400), STALL, CAP),
            SpvWaitVerdict::Stalled
        );
        assert_eq!(
            spv_wait_verdict(STALL - secs(1), secs(400), STALL, CAP),
            SpvWaitVerdict::Continue
        );
    }

    #[test]
    fn cap_bounds_the_wait_even_while_progressing() {
        assert_eq!(
            spv_wait_verdict(secs(0), CAP, STALL, CAP),
            SpvWaitVerdict::Exhausted
        );
        // The cap wins when both limits are hit on the same poll.
        assert_eq!(
            spv_wait_verdict(STALL, CAP, STALL, CAP),
            SpvWaitVerdict::Exhausted
        );
    }

    #[test]
    fn stall_window_is_shorter_than_the_cap() {
        const { assert!(SPV_STALL_WINDOW.as_secs() < SPV_SYNC_CAP.as_secs()) };
    }

    #[test]
    fn progress_token_advance_rules() {
        let token = |step: u64, height: u64| Some((step << 32) | height);
        // First token seen counts as progress.
        assert!(spv_progress_advanced(None, token(1, 10)));
        // Height climbing within a phase.
        assert!(spv_progress_advanced(token(1, 10), token(1, 11)));
        // A new phase restarts its height but the token still increases.
        assert!(spv_progress_advanced(token(1, 1_500_000), token(2, 0)));
        // No token yet, unchanged, or regressed: not progress.
        assert!(!spv_progress_advanced(None, None));
        assert!(!spv_progress_advanced(token(1, 10), None));
        assert!(!spv_progress_advanced(token(1, 10), token(1, 10)));
        assert!(!spv_progress_advanced(token(2, 0), token(1, 99)));
    }
}
