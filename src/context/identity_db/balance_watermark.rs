//! Height watermark of each stored identity's credit balance.
//!
//! The balance lives in the identity blob; the watermark beside it, in its own
//! [`DetScope::Identity`] slot, records the balance and Platform block height
//! of the newest verified observation applied. Every record write reconciles
//! the balance it carries against that slot through
//! [`reconcile_balance`], under the identity's record lock, so an older
//! observation, or a copy of a balance read earlier, never replaces a newer
//! one. An identity with no slot (stored before heights were tracked, or never
//! observed with a height) takes every balance, as before.

use super::*;
use crate::model::identity_balance::{BalanceProvenance, BalanceWatermark, reconcile_balance};
use dash_sdk::dpp::identity::accessors::IdentitySettersV0;

/// Watermark slot, scoped to [`DetScope::Identity`]: a [`BalanceWatermark`].
/// Cleared with the identity by `purge_identity_scope`.
pub(super) const BALANCE_WATERMARK_KEY: &str = "det:identity_balance_watermark:v1";

/// The balance `id`'s record may store when a write carries `incoming` from
/// `provenance`, storing the watermark that decision yields. The caller holds
/// the identity's record lock.
///
/// The watermark binds only a stored record: without one (`record_exists`
/// false) a slot left by a removal that stopped part-way is dropped, so it
/// cannot hold back the next record stored for that identity.
pub(super) fn reconcile_stored_balance(
    kv: &DetKv,
    id: &[u8; 32],
    record_exists: bool,
    incoming: Credits,
    provenance: BalanceProvenance,
) -> std::result::Result<Credits, TaskError> {
    let scope = DetScope::Identity(id);
    let slot: Option<BalanceWatermark> =
        kv.get(scope, BALANCE_WATERMARK_KEY).map_err(identity_err)?;
    let stored = slot.filter(|_| record_exists);
    let reconciled = reconcile_balance(stored, incoming, provenance);
    match reconciled.watermark {
        Some(watermark) => kv
            .put(scope, BALANCE_WATERMARK_KEY, &watermark)
            .map_err(identity_err)?,
        None if !record_exists && slot.is_some() => kv
            .delete(scope, BALANCE_WATERMARK_KEY)
            .map_err(identity_err)?,
        None => {}
    }
    if reconciled.credits != incoming {
        let identity_id = Identifier::from(*id);
        match provenance {
            // Either a copy read before the newer balance landed, or a backend
            // write that carries a fresh balance without its block height and
            // so must be given one (or `ConfirmedUnordered`) to be stored.
            BalanceProvenance::Unobserved => tracing::warn!(
                identity_id = %identity_id,
                carried = incoming,
                kept = reconciled.credits,
                "Kept the newer stored identity balance over one saved without a block height"
            ),
            _ => tracing::debug!(
                identity_id = %identity_id,
                carried = incoming,
                kept = reconciled.credits,
                ?provenance,
                "Ignored an identity balance older than the stored one"
            ),
        }
    }
    Ok(reconciled.credits)
}

impl AppContext {
    /// Reconcile the balance `qualified_identity` carries from `provenance`
    /// against its stored watermark, and adopt the balance to keep. The
    /// caller holds the identity's record lock and writes the record next.
    ///
    /// An identity with no stored record is left as it is: its first record
    /// takes the balance it carries, as every record did before heights were
    /// tracked.
    pub(crate) fn adopt_balance_observation_locked(
        &self,
        qualified_identity: &mut QualifiedIdentity,
        provenance: BalanceProvenance,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let id = qualified_identity.identity.id().to_buffer();
        if !kv
            .contains(DetScope::Identity(&id), IDENTITY_KEY)
            .map_err(identity_err)?
        {
            return Ok(());
        }
        let credits = reconcile_stored_balance(
            &kv,
            &id,
            true,
            qualified_identity.identity.balance(),
            provenance,
        )?;
        qualified_identity.identity.set_balance(credits);
        Ok(())
    }

    /// [`Self::update_local_qualified_identity`] for a record whose balance
    /// comes from `provenance`. `qualified_identity` adopts the balance
    /// actually kept: the stored one when the carried balance is older.
    pub(crate) fn update_local_qualified_identity_observed(
        &self,
        qualified_identity: &mut QualifiedIdentity,
        provenance: BalanceProvenance,
    ) -> std::result::Result<(), TaskError> {
        let lock = self.identity_record_lock(qualified_identity.identity.id());
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.adopt_balance_observation_locked(qualified_identity, provenance)?;
        self.write_local_qualified_identity_locked(qualified_identity)
    }

    /// [`Self::edit_local_qualified_identity`] whose `edit` sets a balance
    /// from `provenance`.
    pub(crate) fn edit_local_qualified_identity_observed(
        &self,
        identity_id: &Identifier,
        provenance: BalanceProvenance,
        edit: impl FnOnce(&mut QualifiedIdentity) -> std::result::Result<(), TaskError>,
    ) -> std::result::Result<QualifiedIdentity, TaskError> {
        let lock = self.identity_record_lock(*identity_id);
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut fresh = self
            .get_local_qualified_identity(identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        edit(&mut fresh)?;
        self.adopt_balance_observation_locked(&mut fresh, provenance)?;
        self.write_local_qualified_identity_locked(&fresh)?;
        Ok(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_staging::stage_identity_with_vaulted_keys;
    use super::*;

    async fn staged_with_balance(
        credits: Credits,
    ) -> (
        super::super::test_staging::StagedIdentity,
        QualifiedIdentity,
    ) {
        let staged = stage_identity_with_vaulted_keys([0xAA; 32], [0xBB; 32]).await;
        let mut identity = stored(&staged.ctx, &staged.id);
        identity.identity.set_balance(credits);
        staged
            .ctx
            .update_local_qualified_identity(&identity)
            .expect("store the starting balance");
        (staged, identity)
    }

    fn stored(ctx: &AppContext, id: &Identifier) -> QualifiedIdentity {
        ctx.get_local_qualified_identity(id)
            .expect("read the identity")
            .expect("identity present")
    }

    fn observe(
        ctx: &AppContext,
        identity: &QualifiedIdentity,
        credits: Credits,
        provenance: BalanceProvenance,
    ) -> Credits {
        let mut copy = identity.clone();
        copy.identity.set_balance(credits);
        ctx.update_local_qualified_identity_observed(&mut copy, provenance)
            .expect("store the observation");
        copy.identity.balance()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_older_observation_never_replaces_a_newer_balance() {
        let (staged, identity) = staged_with_balance(100).await;

        observe(
            &staged.ctx,
            &identity,
            500,
            BalanceProvenance::ObservedAt(10),
        );
        let adopted = observe(
            &staged.ctx,
            &identity,
            200,
            BalanceProvenance::ObservedAt(9),
        );

        assert_eq!(stored(&staged.ctx, &staged.id).identity.balance(), 500);
        assert_eq!(adopted, 500, "the caller adopts the balance actually kept");
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_newer_observation_replaces_the_balance() {
        let (staged, identity) = staged_with_balance(100).await;

        observe(
            &staged.ctx,
            &identity,
            500,
            BalanceProvenance::ObservedAt(10),
        );
        observe(
            &staged.ctx,
            &identity,
            300,
            BalanceProvenance::ObservedAt(11),
        );

        assert_eq!(stored(&staged.ctx, &staged.id).identity.balance(), 300);
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }

    /// A screen or task saving a copy it read before the observation keeps
    /// its other edits but cannot roll the balance back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stale_copy_keeps_its_edits_but_not_its_balance() {
        let (staged, stale_copy) = staged_with_balance(100).await;
        observe(
            &staged.ctx,
            &stale_copy,
            500,
            BalanceProvenance::ObservedAt(10),
        );

        let mut edited = stale_copy.clone();
        edited.alias = Some("renamed".to_string());
        staged
            .ctx
            .update_local_qualified_identity(&edited)
            .expect("save the copy");

        let after = stored(&staged.ctx, &staged.id);
        assert_eq!(after.identity.balance(), 500);
        assert_eq!(after.alias.as_deref(), Some("renamed"));
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_identity_never_observed_with_a_height_takes_every_balance() {
        let (staged, identity) = staged_with_balance(100).await;

        let mut copy = identity.clone();
        copy.identity.set_balance(42);
        staged
            .ctx
            .update_local_qualified_identity(&copy)
            .expect("save the copy");

        assert_eq!(stored(&staged.ctx, &staged.id).identity.balance(), 42);
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_confirmed_balance_without_height_is_kept_and_guards_later_copies() {
        let (staged, identity) = staged_with_balance(100).await;
        observe(
            &staged.ctx,
            &identity,
            500,
            BalanceProvenance::ObservedAt(10),
        );

        staged
            .ctx
            .edit_local_qualified_identity_observed(
                &staged.id,
                BalanceProvenance::ConfirmedUnordered,
                |fresh| {
                    fresh.identity.set_balance(800);
                    Ok(())
                },
            )
            .expect("store the confirmed balance");
        let mut stale = identity.clone();
        stale.identity.set_balance(1);
        staged
            .ctx
            .update_local_qualified_identity(&stale)
            .expect("save a stale copy");

        assert_eq!(stored(&staged.ctx, &staged.id).identity.balance(), 800);
        observe(&staged.ctx, &identity, 3, BalanceProvenance::ObservedAt(11));
        assert_eq!(stored(&staged.ctx, &staged.id).identity.balance(), 3);
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }

    /// The watermark goes with the identity: one loaded again after removal
    /// starts over rather than inheriting the old balance.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn removing_an_identity_drops_its_watermark() {
        let (staged, identity) = staged_with_balance(100).await;
        observe(
            &staged.ctx,
            &identity,
            500,
            BalanceProvenance::ObservedAt(10),
        );

        staged
            .ctx
            .delete_local_qualified_identity(&staged.id)
            .expect("remove the identity");
        let mut reloaded = identity.clone();
        reloaded.identity.set_balance(42);
        staged
            .ctx
            .insert_local_qualified_identity(&reloaded, &None)
            .expect("load it again");

        assert_eq!(stored(&staged.ctx, &staged.id).identity.balance(), 42);
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }

    /// A slot left behind without a record (a removal that stopped part-way)
    /// does not bind the next record stored for that identity.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_watermark_without_a_record_does_not_bind_a_new_record() {
        let (staged, identity) = staged_with_balance(100).await;
        observe(
            &staged.ctx,
            &identity,
            500,
            BalanceProvenance::ObservedAt(10),
        );
        let kv = staged.ctx.det_kv().expect("identity kv");
        kv.delete(DetScope::Identity(&staged.id.to_buffer()), IDENTITY_KEY)
            .expect("drop the record only");

        let mut fresh = identity.clone();
        fresh.identity.set_balance(42);
        staged
            .ctx
            .insert_local_qualified_identity(&fresh, &None)
            .expect("store a new record");
        fresh.identity.set_balance(7);
        staged
            .ctx
            .update_local_qualified_identity(&fresh)
            .expect("update it");

        assert_eq!(stored(&staged.ctx, &staged.id).identity.balance(), 7);
        staged.ctx.wallet_backend().unwrap().shutdown().await;
    }
}
