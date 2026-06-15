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
//! A fresh backend (and a fresh gate) is built on every reconnect, so the latch
//! re-arms naturally — there is no cross-reconnect state to clear here.

use std::sync::OnceLock;
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
    /// The start action, installed once by `WalletBackend::start`.
    action: OnceLock<StartAction>,
    /// Single-winner guard so the action runs exactly once across the two
    /// concurrent fire paths (`arm` and `on_masternodes_ready`).
    fired: AtomicBool,
}

impl std::fmt::Debug for CoordinatorGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoordinatorGate")
            .field("masternodes_ready", &self.masternodes_ready())
            .field("armed", &self.action.get().is_some())
            .field("fired", &self.fired.load(Ordering::SeqCst))
            .finish()
    }
}

impl CoordinatorGate {
    pub(super) fn new() -> Self {
        Self::default()
    }

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
        if self.action.set(action).is_err() {
            tracing::debug!("CoordinatorGate already armed; ignoring second arm");
            return;
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
        if let Some(action) = self.action.get() {
            tracing::info!("Masternode list synced; starting Platform sync coordinators");
            action();
        }
    }

    /// Pure decision: the action may fire when masternodes are ready, an action
    /// is armed, and it has not fired yet. Side-effect-free, unit-testable.
    fn should_fire(&self) -> bool {
        self.masternodes_ready() && self.action.get().is_some() && !self.has_fired()
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
        let gate = CoordinatorGate::new();

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
        let gate = CoordinatorGate::new();

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
        let gate = CoordinatorGate::new();

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
        let gate = CoordinatorGate::new();
        assert!(!gate.should_fire());

        // Ready, not armed → no.
        let gate = CoordinatorGate::new();
        gate.masternodes_ready.store(true, Ordering::SeqCst);
        assert!(!gate.should_fire());

        // Armed, not ready → no.
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = CoordinatorGate::new();
        let _ = gate.action.set(counting_action(&calls));
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
        let gate = Arc::new(CoordinatorGate::new());
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
}
