//! Quorum-readiness gate for the Platform/identity sync coordinators.
//!
//! At fresh boot the upstream platform-address and identity coordinators must
//! not run until the SPV masternode list has synced — they issue proof-verifying
//! DAPI calls that fail locally ("no masternode list yet") and get every queried
//! node banned by the SDK (`ban_failed_address: true`), bricking Platform.
//!
//! [`WalletBackend::start`](super::WalletBackend::start) spawns SPV immediately
//! but defers the coordinator starts: it *arms* this gate with a start action,
//! then fires it. The gate runs the action exactly once, and only when both
//! armed and masternodes-ready. The two readiness paths converge here:
//!
//! * already ready at `start()` time → `arm` fires immediately, or
//! * not ready yet → the `EventBridge` calls [`CoordinatorGate::on_masternodes_ready`]
//!   when the masternode list reaches `Synced`, which fires the armed action.
//!
//! The drop+rebuild reconnect path builds a fresh backend (and a fresh gate)
//! each time, so the latch re-arms naturally. The restart-in-place reconnect
//! path reuses the same backend and gate instead; it calls
//! [`CoordinatorGate::reset`] to re-arm the gate for the next `start()`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The one-shot start action: starts the platform-address and identity sync
/// coordinators. Cheap to construct (clones two `Arc` handles), invoked at most
/// once per gate.
type StartAction = Box<dyn Fn() + Send + Sync>;

/// Idempotent latch that starts the Platform sync coordinators exactly once,
/// the first time both an arming action is installed and masternodes are ready.
#[derive(Default)]
pub(super) struct CoordinatorGate {
    /// Whether the SPV masternode list has finished syncing. Set by the
    /// `EventBridge`; mirrors `ConnectionStatus::masternodes_ready`.
    masternodes_ready: AtomicBool,
    /// The start action, installed by `WalletBackend::start`. Held in a
    /// `Mutex<Option<…>>` (not a `OnceLock`) so [`Self::reset`] can clear it
    /// for a restart-in-place reconnect, which re-arms this same gate rather
    /// than building a fresh one.
    action: Mutex<Option<StartAction>>,
    /// Single-winner guard so the action runs exactly once across the two
    /// concurrent fire paths (`arm` and `on_masternodes_ready`).
    fired: AtomicBool,
}

impl std::fmt::Debug for CoordinatorGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoordinatorGate")
            .field("masternodes_ready", &self.masternodes_ready())
            .field("armed", &self.action.lock().is_ok_and(|a| a.is_some()))
            .field("fired", &self.fired.load(Ordering::SeqCst))
            .finish()
    }
}

impl CoordinatorGate {
    /// Whether masternodes have been reported ready to this gate.
    pub(super) fn masternodes_ready(&self) -> bool {
        self.masternodes_ready.load(Ordering::SeqCst)
    }

    /// Whether the start action has fired.
    pub(super) fn has_fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }

    /// Install the coordinator start action and try to fire immediately.
    ///
    /// Called once by `WalletBackend::start`. If masternodes are already ready
    /// the action runs now (case (a)); otherwise it waits for
    /// [`Self::on_masternodes_ready`] (case (b)). A second arm is ignored — the
    /// action slot is write-once.
    pub(super) fn arm(&self, action: StartAction) {
        {
            let mut slot = self
                .action
                .lock()
                .expect("coordinator gate action mutex poisoned");
            if slot.is_some() {
                tracing::debug!("CoordinatorGate already armed; ignoring second arm");
                return;
            }
            *slot = Some(action);
        }
        self.try_fire();
    }

    /// Record that the masternode list reached `Synced` and try to fire.
    ///
    /// Called by the `EventBridge` from the sync-progress hot path; cheap and
    /// idempotent. Fires the armed action the first time both conditions hold.
    pub(super) fn on_masternodes_ready(&self) {
        self.masternodes_ready.store(true, Ordering::SeqCst);
        self.try_fire();
    }

    /// Run the start action iff armed, ready, and not already fired — claiming
    /// the single-winner `fired` flag so the action runs exactly once even when
    /// `arm` and `on_masternodes_ready` race.
    fn try_fire(&self) {
        if !self.should_fire() {
            return;
        }
        // Claim the fire exactly once. The `should_fire` pre-check is a cheap
        // early-out; this swap is the authoritative single-winner guard.
        if self.fired.swap(true, Ordering::SeqCst) {
            return;
        }
        let slot = self
            .action
            .lock()
            .expect("coordinator gate action mutex poisoned");
        if let Some(action) = slot.as_ref() {
            tracing::info!("Masternode list synced; starting Platform sync coordinators");
            action();
        }
    }

    /// Pure decision: the action may fire when masternodes are ready, an action
    /// is armed, and it has not fired yet. Side-effect-free, unit-testable.
    fn should_fire(&self) -> bool {
        self.masternodes_ready()
            && self.action.lock().is_ok_and(|a| a.is_some())
            && !self.has_fired()
    }

    /// Clear the gate so a restart-in-place reconnect can re-arm it.
    ///
    /// Drops the installed action, clears the single-winner `fired` flag, and
    /// resets `masternodes_ready` to `false`. The last is mandatory: a restart
    /// rebuilds the SPV session, which re-syncs the masternode list from
    /// scratch, so the coordinators must wait for a fresh `Synced` signal
    /// before firing — starting them against a not-yet-synced masternode list
    /// fires proof-verifying DAPI calls that get every queried node banned.
    pub(super) fn reset(&self) {
        *self
            .action
            .lock()
            .expect("coordinator gate action mutex poisoned") = None;
        self.fired.store(false, Ordering::SeqCst);
        self.masternodes_ready.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    /// A start action that counts how many times it ran.
    fn counting_action(counter: &Arc<AtomicUsize>) -> StartAction {
        let counter = Arc::clone(counter);
        Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
    }

    #[test]
    fn arm_before_ready_defers_then_fires_on_ready() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = CoordinatorGate::default();

        gate.arm(counting_action(&calls));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "arming before masternodes are ready must NOT start coordinators"
        );
        assert!(!gate.has_fired());

        gate.on_masternodes_ready();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the ready signal must start coordinators exactly once"
        );
        assert!(gate.has_fired());
    }

    #[test]
    fn ready_before_arm_fires_immediately_on_arm() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = CoordinatorGate::default();

        gate.on_masternodes_ready();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "ready with nothing armed yet starts nothing"
        );

        gate.arm(counting_action(&calls));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "arming when already ready must start coordinators now"
        );
        assert!(gate.has_fired());
    }

    #[test]
    fn fires_exactly_once_across_repeated_signals() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = CoordinatorGate::default();

        gate.arm(counting_action(&calls));
        gate.on_masternodes_ready();
        // Repeated ready signals (upstream re-emits MasternodeStateUpdated) and
        // a redundant arm must not start a second set of coordinators.
        gate.on_masternodes_ready();
        gate.on_masternodes_ready();
        gate.arm(counting_action(&calls));

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "coordinators must start EXACTLY once regardless of repeated signals"
        );
    }

    #[test]
    fn should_fire_decision_table() {
        // Not ready, not armed → no.
        let gate = CoordinatorGate::default();
        assert!(!gate.should_fire());

        // Ready, not armed → no.
        let gate = CoordinatorGate::default();
        gate.masternodes_ready.store(true, Ordering::SeqCst);
        assert!(!gate.should_fire());

        // Armed, not ready → no.
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = CoordinatorGate::default();
        *gate.action.lock().unwrap() = Some(counting_action(&calls));
        assert!(!gate.should_fire());

        // Armed and ready, not fired → yes.
        gate.masternodes_ready.store(true, Ordering::SeqCst);
        assert!(gate.should_fire());

        // Already fired → no.
        gate.fired.store(true, Ordering::SeqCst);
        assert!(!gate.should_fire());
    }

    #[test]
    fn single_winner_under_concurrent_fire() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(CoordinatorGate::default());
        gate.arm(counting_action(&calls));

        let handles: Vec<_> = (0..16)
            .map(|_| {
                let gate = Arc::clone(&gate);
                std::thread::spawn(move || gate.on_masternodes_ready())
            })
            .collect();
        for h in handles {
            h.join().expect("thread panicked");
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "concurrent ready signals must still fire the action exactly once"
        );
    }

    /// Restart-in-place: [`CoordinatorGate::reset`] must re-arm the gate so the
    /// SAME instance fires the coordinators again on a reconnect — clearing the
    /// installed action, the `fired` latch, and `masternodes_ready`.
    #[test]
    fn reset_re_arms_gate_for_restart_in_place() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = CoordinatorGate::default();

        // First arm + ready → fires once.
        gate.arm(counting_action(&calls));
        gate.on_masternodes_ready();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(gate.has_fired());
        assert!(gate.masternodes_ready());

        // Reset clears action, the fired latch, and masternodes_ready.
        gate.reset();
        assert!(!gate.has_fired(), "reset must clear the fired latch");
        assert!(
            !gate.masternodes_ready(),
            "reset must clear masternodes_ready so coordinators re-wait for a fresh sync"
        );

        // A ready signal after reset with nothing re-armed must not fire.
        gate.on_masternodes_ready();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "ready with no action armed after reset must not fire"
        );

        // Re-arm: masternodes are ready again, so the action fires a SECOND
        // time — the gate is reusable across a restart-in-place reconnect.
        gate.arm(counting_action(&calls));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "re-arming a reset gate must fire the coordinators again"
        );
    }

    /// Regression guard for the WEAK capture at `WalletBackend::start`.
    ///
    /// The gate is reachable from the `EventBridge`, which the long-lived SPV
    /// run loop holds, so the arming closure must capture WEAK coordinator
    /// handles — a strong capture would pin the coordinators (and through them
    /// the persister's advisory lock) past backend teardown and break the next
    /// reconnect with `WalletStorageError::AlreadyOpen`.
    ///
    /// This mirrors that pattern with a stand-in coordinator `Arc<AtomicUsize>`:
    /// downgrade it, capture only the `Weak` in the action, drop the strong ref
    /// (simulating teardown), then fire. It goes RED if the closure captured a
    /// strong `Arc` instead: the dropped-strong-ref `strong_count` would stay
    /// `> 0` and `upgrade()` would return `Some` (started == 1), not `None`.
    #[test]
    fn weak_capture_does_not_pin_coordinator_past_teardown() {
        let coordinator = Arc::new(AtomicUsize::new(0));
        let weak_to_coordinator = Arc::downgrade(&coordinator);

        // The action upgrades the weak handle and "starts" the coordinator,
        // recording whether the upgrade observed a live coordinator.
        let started = Arc::new(AtomicUsize::new(0));
        let observed_none = Arc::new(AtomicBool::new(false));
        let started_for_action = Arc::clone(&started);
        let observed_none_for_action = Arc::clone(&observed_none);
        let captured = Arc::downgrade(&coordinator);

        let gate = CoordinatorGate::default();
        gate.arm(Box::new(move || match captured.upgrade() {
            Some(c) => {
                c.fetch_add(1, Ordering::SeqCst);
                started_for_action.fetch_add(1, Ordering::SeqCst);
            }
            None => observed_none_for_action.store(true, Ordering::SeqCst),
        }));

        // Simulate backend teardown: the only strong ref to the coordinator is
        // dropped. A strong capture in the armed closure would keep it alive.
        drop(coordinator);
        assert_eq!(
            weak_to_coordinator.strong_count(),
            0,
            "the gate's armed closure must NOT keep a strong coordinator ref alive"
        );

        // Firing after teardown must not panic and must no-op gracefully.
        gate.on_masternodes_ready();

        assert!(gate.has_fired(), "the gate still latches as fired");
        assert!(
            observed_none.load(Ordering::SeqCst),
            "the action must observe upgrade()==None after teardown"
        );
        assert_eq!(
            started.load(Ordering::SeqCst),
            0,
            "no coordinator may be started once it has been torn down"
        );
    }
}
