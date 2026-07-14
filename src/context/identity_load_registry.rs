//! In-flight registry for identity loads.
//!
//! `IdentityLoadMode::RejectIfExists` checks the store, then fetches from the
//! network, then inserts — three steps, not one atomic one. Two overlapping
//! loads of the same identity can therefore both pass the check and both insert,
//! the last one clobbering the first node's alias, keys and protection tier. The
//! registry closes that window: a load claims its identity id for the whole
//! check → fetch → insert span, and a second load of the same id is rejected up
//! front, whichever screen, tool or CLI dispatched it.
//!
//! It is also the only truthful answer to "is that load still running?" — task
//! results reach only the visible screen, so a screen that was away while its
//! load finished (or failed) cannot learn the outcome from a callback.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use dash_sdk::platform::Identifier;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;

/// The set of identity ids whose load task is currently running.
pub(crate) type InFlightIdentityLoads = Arc<Mutex<HashSet<Identifier>>>;

/// An exclusive claim on loading one identity, released when dropped.
///
/// Held for the whole check → fetch → insert span of a load task, so no second
/// load of the same identity can overlap it.
#[derive(Debug)]
pub struct IdentityLoadGuard {
    loads: InFlightIdentityLoads,
    identity_id: Identifier,
}

impl Drop for IdentityLoadGuard {
    fn drop(&mut self) {
        self.loads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.identity_id);
    }
}

impl AppContext {
    /// Claim `identity_id` for a load, excluding any concurrent load of the same
    /// identity for as long as the returned guard lives.
    ///
    /// # Errors
    ///
    /// Returns [`TaskError::IdentityLoadInProgress`] when a load of this identity
    /// is already running.
    pub fn begin_identity_load(
        &self,
        identity_id: Identifier,
    ) -> Result<IdentityLoadGuard, TaskError> {
        let claimed = self
            .identity_loads_in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(identity_id);

        if !claimed {
            return Err(TaskError::IdentityLoadInProgress { identity_id });
        }
        Ok(IdentityLoadGuard {
            loads: self.identity_loads_in_flight.clone(),
            identity_id,
        })
    }

    /// Whether a load task for `identity_id` is still running. Cheap enough to
    /// call from the frame loop.
    pub fn identity_load_in_flight(&self, identity_id: &Identifier) -> bool {
        self.identity_loads_in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(identity_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;

    #[test]
    fn a_claim_excludes_a_concurrent_load_of_the_same_identity() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let id = Identifier::from([0x01; 32]);
        let other = Identifier::from([0x02; 32]);

        assert!(!ctx.identity_load_in_flight(&id));

        let guard = ctx.begin_identity_load(id).expect("first claim");
        assert!(ctx.identity_load_in_flight(&id));
        assert!(
            matches!(
                ctx.begin_identity_load(id),
                Err(TaskError::IdentityLoadInProgress { identity_id }) if identity_id == id
            ),
            "a second load of the same identity must be rejected, not raced"
        );

        // A different identity is unrelated — it may load concurrently.
        let other_guard = ctx.begin_identity_load(other).expect("unrelated claim");
        assert!(ctx.identity_load_in_flight(&other));
        drop(other_guard);
        assert!(!ctx.identity_load_in_flight(&other));

        // Releasing the claim (task finished or failed) re-opens the identity.
        drop(guard);
        assert!(!ctx.identity_load_in_flight(&id));
        ctx.begin_identity_load(id)
            .expect("a released identity can be loaded again");
    }
}
