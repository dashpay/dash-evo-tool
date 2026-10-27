//! Serializes contact-info read/modify/write operations across UI surfaces.

use dash_sdk::platform::Identifier;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Owner-wide locking also protects allocation of new contact derivation indexes.
#[derive(Debug, Default)]
pub(crate) struct ContactInfoWrites {
    owners: Mutex<HashMap<Identifier, Weak<AsyncMutex<()>>>>,
}

impl ContactInfoWrites {
    /// Hold through the network read, merge and confirmed document write.
    pub(crate) async fn acquire(&self, owner: Identifier) -> OwnedMutexGuard<()> {
        let gate = {
            let mut owners = self
                .owners
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            owners.retain(|_, gate| gate.strong_count() > 0);
            match owners.get(&owner).and_then(Weak::upgrade) {
                Some(gate) => gate,
                None => {
                    let gate = Arc::new(AsyncMutex::new(()));
                    owners.insert(owner, Arc::downgrade(&gate));
                    gate
                }
            }
        };
        gate.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::Poll;

    #[tokio::test]
    async fn contact_info_writes_serialize_each_owner_and_release_after_cancellation() {
        let writes = ContactInfoWrites::default();
        let owner = Identifier::from([1; 32]);
        let first = writes.acquire(owner).await;
        let mut queued = Box::pin(writes.acquire(owner));
        assert!(matches!(futures::poll!(&mut queued), Poll::Pending));
        let other_owner = writes.acquire(Identifier::from([2; 32])).await;
        drop(queued);
        let mut next = Box::pin(writes.acquire(owner));
        assert!(matches!(futures::poll!(&mut next), Poll::Pending));
        drop(first);
        let next = next.await;
        let mut last = Box::pin(writes.acquire(owner));
        assert!(matches!(futures::poll!(&mut last), Poll::Pending));
        drop(next);
        drop(last.await);
        drop(other_owner);
        drop(writes.acquire(owner).await);
        assert_eq!(
            writes.owners.lock().unwrap().len(),
            1,
            "unused gates are pruned"
        );
    }
}
