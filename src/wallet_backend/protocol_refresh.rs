//! Best-effort metadata probes owned by one running SPV session.

use super::coordinator_gate::CoordinatorGate;
use super::poison::MutexRecover;
use crate::context::AppContext;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// Upper bound, in health ticks, on the wait before retrying a failed read.
const MAX_RETRY_TICKS: u32 = 32;

/// Coalesced metadata refreshes admitted only during the active SPV session.
#[derive(Default)]
pub(super) struct ProtocolRefresh {
    session: Mutex<Option<Arc<Session>>>,
}

struct Session {
    ready: Arc<CoordinatorGate>,
    wake: Arc<Notify>,
    cancel: CancellationToken,
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.get_mut().as_ref() {
            task.abort();
        }
    }
}

enum ProbeEvent {
    Requested,
    HealthTick,
    Completed(bool),
}

impl ProtocolRefresh {
    /// Install a worker after the previous SPV session has drained.
    pub(super) fn start(&self, context: Weak<AppContext>, ready: Arc<CoordinatorGate>) {
        let health_context = context.clone();
        self.start_with(
            ready,
            move || {
                let context = health_context.clone();
                async move {
                    let Some(ctx) = context.upgrade() else {
                        return false;
                    };
                    let Ok(backend) = ctx.wallet_backend() else {
                        return false;
                    };
                    let spv = backend.inner.pwm.spv();
                    spv.sync_progress()
                        .await
                        .is_some_and(|progress| progress.is_synced())
                        && spv.wait_until_ready(Duration::from_millis(50)).await
                        && !ctx
                            .sdk
                            .load()
                            .address_list()
                            .get_live_addresses()
                            .is_empty()
                }
            },
            move || {
                let context = context.clone();
                async move {
                    if let Some(ctx) = context.upgrade() {
                        let sdk = ctx.sdk.load_full();
                        ctx.refresh_platform_protocol(&sdk).await
                    } else {
                        false
                    }
                }
            },
            Duration::from_secs(1),
        );
    }

    fn start_with<H, Health, F, Refresh>(
        &self,
        ready: Arc<CoordinatorGate>,
        health: H,
        refresh: F,
        period: Duration,
    ) where
        H: Fn() -> Health + Send + 'static,
        Health: Future<Output = bool> + Send + 'static,
        F: Fn() -> Refresh + Send + 'static,
        Refresh: Future<Output = bool> + Send + 'static,
    {
        let mut slot = self.session.lock_recover();
        if slot.is_some() {
            return;
        }
        let wake = Arc::new(Notify::new());
        let cancel = CancellationToken::new();
        let worker_wake = Arc::clone(&wake);
        let worker_cancel = cancel.clone();
        let worker_ready = Arc::clone(&ready);
        let task = tokio::spawn(async move {
            let mut ticks = tokio::time::interval(period);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut current: Option<Pin<Box<Refresh>>> = None;
            let mut pending = false;
            let mut armed = false;
            let mut previously_healthy = false;
            let mut has_synced = false;
            let mut last_probe_succeeded = false;
            let mut first_healthy_waiting = false;
            let mut retry_backoff = 0u32;
            let mut retry_in = 0u32;
            loop {
                let event = tokio::select! {
                    biased;
                    _ = worker_cancel.cancelled() => break,
                    completed = async {
                        match current.as_mut() {
                            Some(probe) => probe.as_mut().await,
                            None => std::future::pending().await,
                        }
                    }, if current.is_some() => ProbeEvent::Completed(completed),
                    _ = worker_wake.notified() => ProbeEvent::Requested,
                    _ = ticks.tick(), if armed => ProbeEvent::HealthTick,
                };
                match event {
                    ProbeEvent::Requested => {
                        armed = true;
                        pending = true;
                    }
                    ProbeEvent::Completed(succeeded) => {
                        current = None;
                        last_probe_succeeded = succeeded;
                        if first_healthy_waiting {
                            pending |= !succeeded && previously_healthy;
                            first_healthy_waiting = false;
                        }
                        // A healthy session has no recovery edge to retry on.
                        if succeeded {
                            retry_backoff = 0;
                            retry_in = 0;
                        } else if previously_healthy {
                            retry_backoff = (retry_backoff * 2).clamp(1, MAX_RETRY_TICKS);
                            retry_in = retry_backoff;
                        }
                    }
                    ProbeEvent::HealthTick => {
                        let healthy = tokio::select! {
                            biased;
                            _ = worker_cancel.cancelled() => break,
                            healthy = health() => healthy,
                        };
                        if healthy && !previously_healthy {
                            if has_synced {
                                pending = true;
                            } else if current.is_some() {
                                first_healthy_waiting = true;
                            } else {
                                pending |= !last_probe_succeeded;
                            }
                        }
                        if !healthy {
                            retry_in = 0;
                        } else if retry_in > 0 && current.is_none() {
                            retry_in -= 1;
                            pending |= retry_in == 0;
                        }
                        previously_healthy = healthy;
                        has_synced |= healthy;
                    }
                }
                if pending && current.is_none() && worker_ready.masternodes_ready() {
                    pending = false;
                    current = Some(Box::pin(refresh()));
                }
            }
        });
        *slot = Some(Arc::new(Session {
            ready,
            wake,
            cancel,
            task: tokio::sync::Mutex::new(Some(task)),
        }));
    }

    /// Coalesce requests, including one follow-up requested during a probe.
    pub(super) fn queue(&self) -> bool {
        let slot = self.session.lock_recover();
        if let Some(session) = slot.as_ref()
            && !session.cancel.is_cancelled()
            && session.ready.masternodes_ready()
        {
            session.wake.notify_one();
            true
        } else {
            false
        }
    }

    /// Close admission and drop every in-flight proof before SPV loses its keys.
    pub(super) async fn stop(&self) {
        let session = self.session.lock_recover().clone();
        if let Some(session) = session {
            session.cancel.cancel();
            let mut task = session.task.lock().await;
            if let Some(handle) = task.as_mut()
                && let Err(error) = handle.await
            {
                tracing::warn!(?error, "Platform metadata probe stopped unexpectedly");
            }
            *task = None;
            let mut slot = self.session.lock_recover();
            if slot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                *slot = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    async fn next<T>(rx: &mut tokio::sync::mpsc::UnboundedReceiver<T>) -> T {
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("worker responds")
            .expect("worker sender")
    }

    fn start_counting(
        probe: &ProtocolRefresh,
        ready: Arc<CoordinatorGate>,
    ) -> tokio::sync::mpsc::UnboundedReceiver<()> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        probe.start_with(
            ready,
            || std::future::ready(false),
            move || {
                tx.send(()).expect("probe receiver");
                std::future::ready(true)
            },
            Duration::from_millis(10),
        );
        rx
    }

    #[tokio::test]
    async fn protocol_refresh_waits_for_keys_and_rearms_only_after_stop() {
        let probe = ProtocolRefresh::default();
        let ready = Arc::new(CoordinatorGate::default());
        for _ in 0..2 {
            let mut rx = start_counting(&probe, Arc::clone(&ready));
            assert!(!probe.queue(), "quorum keys are unavailable");
            ready.on_masternodes_ready();
            assert!(probe.queue());
            assert!(probe.queue());
            next(&mut rx).await;
            tokio::task::yield_now().await;
            assert!(rx.try_recv().is_err(), "requests coalesce");
            probe.stop().await;
            assert!(!probe.queue(), "closed session rejects new requests");
            ready.reset();
        }
    }

    #[tokio::test]
    async fn protocol_refresh_keeps_network_readiness_separate_from_the_gui_status() {
        let gui_status = crate::context::connection_status::ConnectionStatus::new();
        let network_a = ProtocolRefresh::default();
        let network_b = ProtocolRefresh::default();
        let ready_a = Arc::new(CoordinatorGate::default());
        let ready_b = Arc::new(CoordinatorGate::default());
        let mut rx_a = start_counting(&network_a, Arc::clone(&ready_a));
        let mut rx_b = start_counting(&network_b, Arc::clone(&ready_b));
        gui_status.set_masternodes_ready(true);
        ready_a.on_masternodes_ready();
        assert!(network_a.queue());
        next(&mut rx_a).await;
        assert!(
            !network_b.queue(),
            "A's global status cannot admit B's proof"
        );
        gui_status.reset();
        assert!(
            network_a.queue(),
            "GUI network switch does not revoke A's keys"
        );
        next(&mut rx_a).await;
        assert!(rx_b.try_recv().is_err());
        ready_b.on_masternodes_ready();
        assert!(network_b.queue());
        next(&mut rx_b).await;
        network_a.stop().await;
        network_b.stop().await;
    }

    #[tokio::test]
    async fn protocol_refresh_retries_on_recovery_without_restarting_the_session() {
        for (initial_succeeded, rebuild_failed) in [(false, false), (true, false), (true, true)] {
            let probe = ProtocolRefresh::default();
            let ready = Arc::new(CoordinatorGate::default());
            ready.on_masternodes_ready();
            let healthy = Arc::new(AtomicBool::new(false));
            let observed = Arc::clone(&healthy);
            let (checks, mut checks_rx) = tokio::sync::mpsc::unbounded_channel();
            let (calls, mut calls_rx) = tokio::sync::mpsc::unbounded_channel();
            let response = Arc::new(AtomicBool::new(initial_succeeded));
            let next_response = Arc::clone(&response);
            probe.start_with(
                ready,
                move || {
                    let value = observed.load(Ordering::SeqCst);
                    checks.send(value).expect("health receiver");
                    std::future::ready(value)
                },
                move || {
                    calls.send(()).expect("probe receiver");
                    std::future::ready(next_response.load(Ordering::SeqCst))
                },
                Duration::from_millis(10),
            );
            assert!(probe.queue());
            next(&mut calls_rx).await;
            assert!(!next(&mut checks_rx).await);
            if rebuild_failed {
                response.store(false, Ordering::SeqCst);
                assert!(probe.queue());
                next(&mut calls_rx).await;
            }
            response.store(true, Ordering::SeqCst);
            healthy.store(true, Ordering::SeqCst);
            while !next(&mut checks_rx).await {}
            if !initial_succeeded || rebuild_failed {
                next(&mut calls_rx).await;
            }
            tokio::task::yield_now().await;
            assert!(
                calls_rx.try_recv().is_err(),
                "successful startup is not duplicated"
            );
            healthy.store(false, Ordering::SeqCst);
            while next(&mut checks_rx).await {}
            healthy.store(true, Ordering::SeqCst);
            next(&mut calls_rx).await;
            tokio::task::yield_now().await;
            assert!(calls_rx.try_recv().is_err(), "stable Synced does not retry");
            probe.stop().await;
        }
    }

    #[tokio::test]
    async fn protocol_refresh_retries_a_failed_read_while_healthy() {
        let probe = ProtocolRefresh::default();
        let ready = Arc::new(CoordinatorGate::default());
        ready.on_masternodes_ready();
        let (checks, mut checks_rx) = tokio::sync::mpsc::unbounded_channel();
        let (calls, mut calls_rx) = tokio::sync::mpsc::unbounded_channel();
        let responses = Mutex::new(std::collections::VecDeque::from([true, false, true]));
        probe.start_with(
            ready,
            move || {
                checks.send(()).expect("health receiver");
                std::future::ready(true)
            },
            move || {
                calls.send(()).expect("probe receiver");
                std::future::ready(responses.lock_recover().pop_front().unwrap_or(true))
            },
            Duration::from_millis(10),
        );
        assert!(probe.queue());
        next(&mut calls_rx).await;
        next(&mut checks_rx).await;
        assert!(probe.queue());
        next(&mut calls_rx).await;
        next(&mut calls_rx).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            calls_rx.try_recv().is_err(),
            "a successful retry ends the backoff"
        );
        probe.stop().await;
    }

    #[tokio::test]
    async fn protocol_refresh_stop_drains_an_inflight_proof() {
        struct DropSignal(tokio::sync::mpsc::UnboundedSender<()>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let probe = ProtocolRefresh::default();
        let ready = Arc::new(CoordinatorGate::default());
        ready.on_masternodes_ready();
        let (started, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let (dropped, mut dropped_rx) = tokio::sync::mpsc::unbounded_channel();
        probe.start_with(
            ready,
            || std::future::ready(false),
            move || {
                let started = started.clone();
                let guard = DropSignal(dropped.clone());
                async move {
                    let _guard = guard;
                    started.send(()).expect("probe receiver");
                    std::future::pending::<bool>().await
                }
            },
            Duration::from_millis(10),
        );
        assert!(probe.queue());
        next(&mut started_rx).await;
        assert!(probe.queue(), "coalesced follow-up is pending");
        probe.stop().await;
        assert!(
            dropped_rx.try_recv().is_ok(),
            "proof dropped before stop returns"
        );
        assert!(!probe.queue());
        assert!(
            started_rx.try_recv().is_err(),
            "pending retry was cancelled"
        );
    }

    #[tokio::test]
    async fn protocol_refresh_disconnect_drains_before_quorum_readiness_is_cleared() {
        struct ObserveKeys(
            Arc<CoordinatorGate>,
            tokio::sync::mpsc::UnboundedSender<bool>,
        );
        impl Drop for ObserveKeys {
            fn drop(&mut self) {
                let _ = self.1.send(self.0.masternodes_ready());
            }
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = ctx.wallet_backend().expect("backend");
        let ready = Arc::clone(&backend.inner.coordinator_gate);
        ready.on_masternodes_ready();
        let (started, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let (dropped, mut dropped_rx) = tokio::sync::mpsc::unbounded_channel();
        backend.inner.protocol_refresh.start_with(
            Arc::clone(&ready),
            || std::future::ready(false),
            move || {
                let started = started.clone();
                let guard = ObserveKeys(Arc::clone(&ready), dropped.clone());
                async move {
                    let _guard = guard;
                    started.send(()).expect("probe receiver");
                    std::future::pending::<bool>().await
                }
            },
            Duration::from_millis(10),
        );
        ctx.queue_platform_protocol_refresh();
        next(&mut started_rx).await;
        ctx.stop_spv().await;
        assert_eq!(
            dropped_rx.try_recv(),
            Ok(true),
            "proof dropped before the gate reset"
        );
        assert!(!backend.quorum_keys_ready());
        assert!(!backend.inner.protocol_refresh.queue());
    }

    #[tokio::test]
    async fn protocol_refresh_retains_recovery_while_a_probe_is_in_flight() {
        let probe = ProtocolRefresh::default();
        let ready = Arc::new(CoordinatorGate::default());
        ready.on_masternodes_ready();
        let healthy = Arc::new(AtomicBool::new(true));
        let observed = Arc::clone(&healthy);
        let (checks, mut checks_rx) = tokio::sync::mpsc::unbounded_channel();
        let (calls, mut calls_rx) = tokio::sync::mpsc::unbounded_channel();
        let (respond, responses) = tokio::sync::mpsc::unbounded_channel();
        let responses = Arc::new(tokio::sync::Mutex::new(responses));
        probe.start_with(
            ready,
            move || {
                let value = observed.load(Ordering::SeqCst);
                checks.send(value).expect("health receiver");
                std::future::ready(value)
            },
            move || {
                let responses = Arc::clone(&responses);
                calls.send(()).expect("probe receiver");
                async move { responses.lock().await.recv().await.expect("probe response") }
            },
            Duration::from_millis(10),
        );
        assert!(probe.queue());
        next(&mut calls_rx).await;
        respond.send(true).expect("probe response");
        assert!(next(&mut checks_rx).await);
        assert!(next(&mut checks_rx).await);
        assert!(
            calls_rx.try_recv().is_err(),
            "ready startup is not duplicated"
        );
        assert!(probe.queue());
        next(&mut calls_rx).await;
        healthy.store(false, Ordering::SeqCst);
        while next(&mut checks_rx).await {}
        healthy.store(true, Ordering::SeqCst);
        while !next(&mut checks_rx).await {}
        respond.send(false).expect("probe response");
        next(&mut calls_rx).await;
        respond.send(true).expect("probe response");
        probe.stop().await;
    }
}
