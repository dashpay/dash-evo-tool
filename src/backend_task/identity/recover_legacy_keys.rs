//! Explicit, per-identity recovery of keys and role links stranded in the
//! preserved legacy `data.db` (issue #889).
//!
//! The cold-start migration skips an identity that is already in the modern
//! store, wholesale, because field absence cannot be told apart from a
//! deliberate removal. An identity that was only *partially* loaded before the
//! upgrade therefore keeps its remaining keys in the legacy file, reachable by
//! nothing: the load form rejects the duplicate ProTxHash, and the per-key
//! screen wants a WIF the user may no longer hold.
//!
//! This is the way in, and it is entirely user-driven. Detection lists; the
//! user decides; the merge writes only what was approved and only what is
//! still missing. Nothing here runs at migration time or launch time, and
//! `data.db` is opened read-only, so recovery is repeatable indefinitely.

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

use super::BackendTaskSuccessResult;
use super::protect_identity_keys::reject_resident_identity_plaintext;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::database::legacy_import::{LegacyIdentityLookup, read_identity_row};
use crate::model::legacy_recovery::{
    RecoveryItem, RecoveryPlan, apply_recovery_plan, compute_recovery_plan,
};
use crate::model::qualified_identity::QualifiedIdentity;

const LOG_TARGET: &str = "backend_task::identity::recover_legacy_keys";

impl AppContext {
    /// List what the preserved legacy database could restore for `identity_id`.
    ///
    /// Read-only and offline — the modern record already holds the newer
    /// on-chain identity, so nothing is fetched. Safe to dispatch on every
    /// screen arrival: an identity with nothing stranded yields an empty plan
    /// and the recovery affordance stays hidden.
    ///
    /// # Errors
    ///
    /// [`TaskError::IdentityNotFoundLocally`] when the identity is not stored —
    /// recovery restores into an existing record and never resurrects a deleted
    /// one. [`TaskError::LegacyIdentityUnreadable`] when the legacy row exists
    /// but will not decode.
    pub(super) fn check_legacy_recovery(
        &self,
        identity_id: Identifier,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let modern = self
            .get_local_qualified_identity(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;

        let plan = match self.legacy_identity_record(identity_id)? {
            Some(legacy) => compute_recovery_plan(&modern, &legacy),
            None => RecoveryPlan::default(),
        };

        tracing::debug!(
            target = LOG_TARGET,
            identity = %identity_id,
            candidates = plan.items.len(),
            excluded = plan.excluded.len(),
            "Checked the previous version's saved copy of an identity for restorable keys",
        );
        Ok(BackendTaskSuccessResult::LegacyRecoveryCandidates { identity_id, plan })
    }

    /// Restore the `approved` items into `identity_id`'s stored record.
    ///
    /// Exactly one record write, so the merge either lands whole or leaves the
    /// stored record untouched. Candidacy is recomputed here rather than
    /// trusted from the caller, and only `recomputed candidates ∩ approved` is
    /// written: an item that stopped being missing since the preview is
    /// reported as skipped, and an item the user did not approve is never
    /// restored even if it is missing.
    ///
    /// On a password-protected identity the password is verified up front and
    /// every merged key is sealed under it *before* the record is written, so
    /// the at-rest encoder never sees plaintext on a protected identity and its
    /// downgrade guard cannot trip.
    ///
    /// # Errors
    ///
    /// [`TaskError::LegacyRecoveryNothingApproved`] for an empty allowlist,
    /// [`TaskError::IdentityLoadInProgress`] while another load or recovery
    /// holds this identity, [`TaskError::WalletStorageNotReady`] while the
    /// storage migration runs, [`TaskError::IdentityNotFoundLocally`] for a
    /// deleted identity, [`TaskError::LegacyIdentityUnreadable`] for a corrupt
    /// legacy row, and the prompt's own cancel / unavailable errors. Every one
    /// of them leaves the stored record unchanged.
    pub(super) async fn recover_legacy_identity_data(
        &self,
        identity_id: Identifier,
        approved: Vec<RecoveryItem>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Nothing is restored without an explicit per-item decision, so an
        // empty list is refused rather than read as "restore everything".
        if approved.is_empty() {
            return Err(TaskError::LegacyRecoveryNothingApproved);
        }

        // Claim the identity for the whole read → compute → seal → write span,
        // so a concurrent load, merge-load or second recovery of it cannot
        // interleave writes with this one.
        let claim = self.begin_identity_load(identity_id, None)?;

        // The storage migration owns the identity store while it runs; the
        // delete path takes the same two-part guard for the same reason.
        let _migration_guard = self
            .migration_run
            .try_lock()
            .map_err(|_| TaskError::WalletStorageNotReady)?;
        if self.migration_status().state().is_in_progress() {
            return Err(TaskError::WalletStorageNotReady);
        }

        let modern = self
            .get_local_qualified_identity(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let Some(legacy) = self.legacy_identity_record(identity_id)? else {
            return Ok(nothing_recovered(identity_id));
        };

        let outcome = self
            .recover_into_loaded_identity(&modern, legacy, &approved)
            .await?;
        claim.loaded();
        Ok(outcome)
    }

    /// Merge into an ALREADY-LOADED `modern` record and persist the result.
    ///
    /// Split from [`Self::recover_legacy_identity_data`] — which supplies the
    /// claim, the migration guard, and the two record reads — so the
    /// resident-plaintext preflight, the password branch, the seal and the
    /// single write are exercised on a real `modern` exactly as the task runs
    /// them. That makes "the guard is wired into this path" testable on a
    /// record shape no production write path can produce.
    async fn recover_into_loaded_identity(
        &self,
        modern: &QualifiedIdentity,
        legacy: QualifiedIdentity,
        approved: &[RecoveryItem],
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let identity_id = modern.identity.id();
        let mut applied = apply_recovery_plan(modern, legacy, approved);
        let excluded = std::mem::take(&mut applied.excluded);

        // Nothing left to restore: no write, and — on a protected identity —
        // no password prompt for work that would not happen.
        if applied.applied.is_empty() {
            tracing::debug!(
                target = LOG_TARGET,
                identity = %identity_id,
                skipped_stale = applied.skipped_stale.len(),
                "Legacy recovery found every approved item already in place",
            );
            return Ok(BackendTaskSuccessResult::LegacyRecoveryCompleted {
                identity_id,
                applied: applied.applied,
                skipped_stale: applied.skipped_stale,
                excluded,
            });
        }

        // Branch on the same predicate the at-rest downgrade guard evaluates,
        // so the guard's trigger is false on both sides of this match: Tier-2
        // seals every merged key before the write, and Tier-1 has no protected
        // key for the guard to protect.
        let password = match self.protected_identity_verify_scope(modern)? {
            Some(verify_scope) => {
                // A key still resident as plaintext means an earlier vault
                // migration did not finish. Sealing around it would half-protect
                // the identity and then trip the guard at persist, so refuse
                // here — before any vault write — with its established remedy.
                reject_resident_identity_plaintext(&modern.private_keys)?;
                Some(
                    self.wallet_backend()?
                        .secret_access()
                        .verify_identity_object_password(&verify_scope)
                        .await?,
                )
            }
            None => None,
        };

        if let Some(password) = &password {
            self.seal_merged_plaintext_keys(&mut applied.merged, password)?;
        }

        self.update_local_qualified_identity(&applied.merged)?;

        tracing::info!(
            target = LOG_TARGET,
            identity = %identity_id,
            restored = applied.applied.len(),
            skipped_stale = applied.skipped_stale.len(),
            "Restored identity keys from the previous version's saved copy",
        );
        Ok(BackendTaskSuccessResult::LegacyRecoveryCompleted {
            identity_id,
            applied: applied.applied,
            skipped_stale: applied.skipped_stale,
            excluded,
        })
    }

    /// The identity `identity_id` names in the preserved legacy `data.db`, or
    /// `None` when there is nothing there to recover from.
    ///
    /// `None` covers every ordinary "not here" answer: an install with no
    /// legacy file at all, a missing table, a missing row, an
    /// observed-identity cache row, a NULL blob. A row that exists but will not
    /// decode is an error instead — reading it as an empty record would look
    /// like the previous version held nothing and would close the recovery
    /// affordance on data that is still on disk.
    fn legacy_identity_record(
        &self,
        identity_id: Identifier,
    ) -> Result<Option<QualifiedIdentity>, TaskError> {
        let Some(path) = self.db.db_file_path().filter(|path| path.exists()) else {
            return Ok(None);
        };
        let conn = crate::database::open_legacy_connection_read_only(&path).map_err(|error| {
            tracing::warn!(
                target = LOG_TARGET,
                identity = %identity_id,
                error = ?error,
                "Could not open the previous version's database to look for stranded keys",
            );
            TaskError::LegacyIdentityUnreadable { identity_id }
        })?;

        match read_identity_row(&conn, self.network, &identity_id.to_buffer()) {
            Ok(LegacyIdentityLookup::Found(row)) => Ok(Some(row.qi)),
            Ok(LegacyIdentityLookup::Absent) => Ok(None),
            Ok(LegacyIdentityLookup::Unreadable) => {
                Err(TaskError::LegacyIdentityUnreadable { identity_id })
            }
            Err(error) => {
                tracing::warn!(
                    target = LOG_TARGET,
                    identity = %identity_id,
                    error = ?error,
                    "Could not read the previous version's saved copy of an identity",
                );
                Err(TaskError::LegacyIdentityUnreadable { identity_id })
            }
        }
    }
}

/// The outcome when the legacy file holds nothing for this identity.
fn nothing_recovered(identity_id: Identifier) -> BackendTaskSuccessResult {
    BackendTaskSuccessResult::LegacyRecoveryCompleted {
        identity_id,
        applied: Vec::new(),
        skipped_stale: Vec::new(),
        excluded: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::app::TaskResult;
    use crate::app_dir::ensure_env_file;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::{LegacyIdentityFixture, create_database_at_path};
    use crate::model::legacy_recovery::ExclusionReason;
    use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType, PrivateKeyTarget};
    use crate::model::secret::Secret;
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use crate::wallet_backend::IdentityKeyView;
    use crate::wallet_backend::leak_test_support::assert_no_leak_bytes;
    use crate::wallet_backend::secret_prompt::SecretPrompt;
    use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
    use crate::wallet_backend::secret_seam::SecretScheme;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{Identity, KeyID, KeyType, Purpose, SecurityLevel};
    use dash_sdk::dpp::platform_value::BinaryData;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::IdentityPublicKey;
    use platform_wallet_storage::secrets::SecretString;

    const M: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
    const V: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;
    const PW: &str = "one-identity-password";

    /// An offline, wired `AppContext` on a throwaway data dir whose file-backed
    /// `data.db` carries the legacy `identity` table, ready for staged rows.
    /// The fresh-install schema ladder creates that table, so a legacy row is
    /// staged into it directly.
    struct Offline {
        ctx: Arc<AppContext>,
        _results: tokio::sync::mpsc::Receiver<TaskResult>,
        _data_dir: tempfile::TempDir,
    }

    impl Offline {
        /// Build the context, installing `prompt` before the backend is wired
        /// so the chokepoint picks it up. `None` leaves the default
        /// `NullSecretPrompt`, which is the headless posture.
        async fn new(prompt: Option<Arc<dyn SecretPrompt>>) -> Self {
            let data_dir_guard = tempfile::tempdir().expect("tempdir");
            let data_dir = data_dir_guard.path().to_path_buf();
            ensure_env_file(&data_dir);
            let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
            let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
            let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
            let ctx = AppContext::new(
                data_dir,
                Network::Testnet,
                db,
                Arc::new(TaskManager::new()),
                Arc::new(ConnectionStatus::new()),
                egui::Context::default(),
                app_kv,
                secret_store,
                crate::model::user_role::UserRoleCell::default(),
            )
            .expect("offline testnet AppContext::new");
            if let Some(prompt) = prompt {
                ctx.install_secret_prompt(prompt);
            }
            let (tx, rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
            let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
            ctx.ensure_wallet_backend(sender)
                .await
                .expect("wire wallet backend offline");
            Self {
                ctx,
                _results: rx,
                _data_dir: data_dir_guard,
            }
        }

        async fn shutdown(&self) {
            if let Ok(backend) = self.ctx.wallet_backend() {
                backend.shutdown().await;
            }
        }

        /// Run `inspect` against the vault view over `identity_id`'s keys. A
        /// closure because the view borrows the backend handle, which cannot
        /// outlive the call that produced it.
        fn with_keys<R>(
            &self,
            identity_id: Identifier,
            inspect: impl FnOnce(&IdentityKeyView<'_>) -> R,
        ) -> R {
            let backend = self.ctx.wallet_backend().expect("backend wired");
            inspect(&IdentityKeyView::new(
                backend.secret_store(),
                identity_id.to_buffer(),
            ))
        }

        /// Write `legacy` into the legacy `identity` table, as v0.9.3 stored it.
        fn stage_legacy(&self, legacy: &QualifiedIdentity) {
            self.stage_legacy_blob(legacy.identity.id(), Some(legacy.to_bytes()));
        }

        /// Stage a raw legacy row, for the shapes a decodable identity cannot
        /// express (a NULL blob, a corrupt blob).
        fn stage_legacy_blob(&self, identity_id: Identifier, blob: Option<Vec<u8>>) {
            LegacyIdentityFixture::new(identity_id.to_buffer(), blob, "testnet")
                .insert(&self.ctx.db.locked_conn())
                .expect("stage legacy identity row");
        }
    }

    /// A fixture key pair: a private half plus the public half genuinely
    /// derived from it. The plan only offers a key that still corresponds to
    /// the identity, so these have to be real pairs.
    #[derive(Clone)]
    struct TestKey {
        secret: [u8; 32],
        public: IdentityPublicKey,
    }

    impl TestKey {
        fn clear(&self) -> PrivateKeyData {
            PrivateKeyData::Clear(self.secret)
        }

        fn id(&self) -> KeyID {
            self.public.id()
        }
    }

    /// A key pair for `purpose`, keyed by `secret_byte`.
    fn test_key(id: KeyID, purpose: Purpose, secret_byte: u8) -> TestKey {
        let key_type = KeyType::ECDSA_HASH160;
        let secret = [secret_byte; 32];
        let data = key_type
            .public_key_data_from_private_key_data(&secret, Network::Testnet)
            .expect("derive the public half");
        TestKey {
            secret,
            public: IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id,
                purpose,
                security_level: SecurityLevel::HIGH,
                contract_bounds: None,
                key_type,
                read_only: false,
                data: BinaryData::new(data),
                disabled_at: None,
            }),
        }
    }

    /// Register `keys` on `identity` as public keys the chain holds.
    fn publish_on(identity: &mut Identity, keys: &[&TestKey]) {
        let mut public_keys = identity.public_keys().clone();
        for key in keys {
            public_keys.insert(key.id(), key.public.clone());
        }
        identity.set_public_keys(public_keys);
    }

    /// The chain's view of a node: `identity_type`, publishing `published` and
    /// holding the private halves listed in `held`. A record that holds a key
    /// publishes it too, which is what any real load produces.
    fn identity_with_keys(
        id: u8,
        identity_type: IdentityType,
        published: &[&TestKey],
        held: Vec<(PrivateKeyTarget, &TestKey, PrivateKeyData)>,
    ) -> QualifiedIdentity {
        let mut private_keys = KeyStorage::default();
        let mut identity =
            Identity::create_basic_identity(Identifier::from([id; 32]), PlatformVersion::latest())
                .expect("basic identity");
        publish_on(&mut identity, published);
        for (target, key, data) in held {
            publish_on(&mut identity, &[key]);
            private_keys.private_keys.insert(
                (target, key.id()),
                (QualifiedIdentityPublicKey::from(key.public.clone()), data),
            );
        }
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type,
            alias: None,
            private_keys,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    /// A separate voter identity publishing `keys`, in the shape
    /// `associated_voter_identity` carries.
    fn voter_identity(id: u8, keys: &[&TestKey]) -> (Identity, IdentityPublicKey) {
        let mut identity =
            Identity::create_basic_identity(Identifier::from([id; 32]), PlatformVersion::latest())
                .expect("voter identity");
        publish_on(&mut identity, keys);
        let paired = keys
            .first()
            .map(|key| key.public.clone())
            .unwrap_or_else(|| test_key(0, Purpose::VOTING, 0xF0).public);
        (identity, paired)
    }

    /// The plan a `CheckLegacyRecovery` result carries.
    fn plan_of(result: BackendTaskSuccessResult) -> RecoveryPlan {
        match result {
            BackendTaskSuccessResult::LegacyRecoveryCandidates { plan, .. } => plan,
            other => panic!("expected LegacyRecoveryCandidates, got {other:?}"),
        }
    }

    /// The `(applied, skipped_stale)` item lists a completion result carries.
    fn completion_of(result: BackendTaskSuccessResult) -> (Vec<RecoveryItem>, Vec<RecoveryItem>) {
        match result {
            BackendTaskSuccessResult::LegacyRecoveryCompleted {
                applied,
                skipped_stale,
                ..
            } => (
                applied.into_iter().map(|d| d.item).collect(),
                skipped_stale.into_iter().map(|d| d.item).collect(),
            ),
            other => panic!("expected LegacyRecoveryCompleted, got {other:?}"),
        }
    }

    /// B1 — the canonical case, end to end on a keyless (Tier-1) node: a bare
    /// masternode whose owner and voting keys are still only in the legacy file
    /// gets them back. The keys must land in the vault, the record must show
    /// both roles present, the at-rest blob must carry no plaintext, and the
    /// legacy rows must survive untouched (AC-5).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b1_tier1_bare_masternode_recovers_owner_and_voting_keys() {
        let owner = test_key(1, Purpose::OWNER, 0xA1);
        let voting = test_key(2, Purpose::VOTING, 0xB2);
        let owner_secret = owner.secret;
        let voting_secret = voting.secret;

        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        // The modern record: loaded from its ProTxHash alone, so the chain's
        // keys are known but none of their private halves are held.
        let modern = identity_with_keys(0x11, IdentityType::Masternode, &[&owner], vec![]);
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert the bare modern record");

        // The legacy record: the same identity, with the keys still attached.
        let mut legacy = identity_with_keys(
            0x11,
            IdentityType::Masternode,
            &[],
            vec![(M, &owner, owner.clear()), (V, &voting, voting.clear())],
        );
        legacy.associated_voter_identity = Some(voter_identity(0x99, &[&voting]));
        offline.stage_legacy(&legacy);

        let plan = plan_of(
            ctx.check_legacy_recovery(identity_id)
                .expect("check must succeed"),
        );
        assert_eq!(
            plan.items.len(),
            3,
            "both keys and the voter link are candidates",
        );
        assert_eq!(
            plan.preview_items().len(),
            2,
            "the voter link is previewed as part of its voting key",
        );

        let (applied, stale) = completion_of(
            ctx.recover_legacy_identity_data(identity_id, plan.approved_items())
                .await
                .expect("recovery must succeed"),
        );
        assert_eq!(applied.len(), 3, "every approved item was restored");
        assert!(stale.is_empty());

        let restored = ctx
            .get_local_qualified_identity(&identity_id)
            .expect("read back")
            .expect("still stored");
        let presence = restored.masternode_key_presence();
        assert!(
            presence.owner && presence.voting,
            "the node must now show its owner and voting roles, got {presence:?}",
        );

        // Every restored key lives in the vault, and the at-rest blob carries
        // placeholders rather than key bytes.
        offline.with_keys(identity_id, |view| {
            assert_eq!(
                *view.get(&M, 1).expect("owner key").expect("stored"),
                owner_secret
            );
            assert_eq!(
                *view.get(&V, 2).expect("voting key").expect("stored"),
                voting_secret
            );
        });
        let blob = ctx
            .stored_identity_blob(&identity_id)
            .expect("read stored blob")
            .expect("record stored");
        let rendered = format!("{blob:?}");
        assert_no_leak_bytes(&rendered, &owner_secret, "recovered identity blob (owner)");
        assert_no_leak_bytes(
            &rendered,
            &voting_secret,
            "recovered identity blob (voting)",
        );
        for (target, key_id) in [(M, 1), (V, 2)] {
            assert!(
                restored.private_keys.is_in_vault(&(target.clone(), key_id)),
                "({target:?}, {key_id}) must be a vault placeholder at rest",
            );
        }

        // The legacy file is a read-only recovery artifact: its rows are still
        // there, so recovery can be repeated indefinitely.
        assert!(
            ctx.legacy_identity_record(identity_id)
                .expect("legacy row still readable")
                .is_some(),
            "recovery must never delete the legacy source row",
        );

        offline.shutdown().await;
    }

    /// B2 — on a password-protected (Tier-2) identity the restored key is
    /// sealed under the SAME identity password as the keys already there, and
    /// opens under it. The one-password invariant survives recovery.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b2_tier2_recovery_seals_the_restored_key_under_the_identity_password() {
        let offline =
            Offline::new(Some(Arc::new(TestPrompt::new([ScriptedAnswer::once(PW)])))).await;
        let ctx = &offline.ctx;

        let owner = test_key(1, Purpose::OWNER, 0xC1);
        let payout = test_key(2, Purpose::TRANSFER, 0xD2);
        let modern = identity_with_keys(
            0x22,
            IdentityType::Masternode,
            &[&payout],
            vec![(M, &owner, owner.clear())],
        );
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");
        ctx.protect_identity_keys(identity_id, Secret::new(PW), None)
            .expect("seal the identity Tier-2");

        let legacy = identity_with_keys(
            0x22,
            IdentityType::Masternode,
            &[],
            vec![(M, &owner, owner.clear()), (M, &payout, payout.clear())],
        );
        offline.stage_legacy(&legacy);

        let (applied, _) = completion_of(
            ctx.recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 2,
                }],
            )
            .await
            .expect("recovery must succeed under the verified password"),
        );
        assert_eq!(applied.len(), 1);

        offline.with_keys(identity_id, |view| {
            assert_eq!(
                view.scheme(&M, 2).expect("scheme"),
                SecretScheme::Protected,
                "the restored key must be sealed Tier-2, never keyless",
            );
            assert_eq!(
                *view
                    .get_protected(&M, 2, &SecretString::new(PW))
                    .expect("get_protected")
                    .expect("sealed key present"),
                payout.secret,
                "the restored key opens under the identity's existing password",
            );
        });

        offline.shutdown().await;
    }

    /// B3 — a Tier-2 recovery never proceeds on an unverified password. With no
    /// interactive prompt it fails closed as unavailable; with a wrong password
    /// the prompt re-asks rather than giving up or proceeding. Either way the
    /// stored record is byte-identical and nothing reached the vault.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b3_tier2_recovery_fails_closed_on_an_unverified_password() {
        // Headless: the default NullSecretPrompt cannot ask.
        let headless = Offline::new(None).await;
        let (identity_id, before) = seed_tier2_with_a_stranded_key(&headless).await;
        let error = headless
            .ctx
            .recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 2,
                }],
            )
            .await
            .expect_err("a headless Tier-2 recovery must fail closed");
        assert!(
            matches!(error, TaskError::SecretPromptUnavailable),
            "expected SecretPromptUnavailable, got {error:?}",
        );
        assert_unchanged(&headless, identity_id, &before);
        headless.shutdown().await;

        // Wrong password: the prompt re-asks; dismissing the retry ends the run.
        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::once("not-the-password"),
            ScriptedAnswer::Cancel,
        ]));
        let interactive = Offline::new(Some(prompt.clone())).await;
        let (identity_id, before) = seed_tier2_with_a_stranded_key(&interactive).await;
        let error = interactive
            .ctx
            .recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 2,
                }],
            )
            .await
            .expect_err("a wrong password must never let the recovery proceed");
        assert!(
            matches!(error, TaskError::SecretPromptCancelled),
            "expected SecretPromptCancelled, got {error:?}",
        );
        assert_eq!(
            prompt.ask_count(),
            2,
            "a wrong password must re-ask rather than fail on the first attempt",
        );
        assert_unchanged(&interactive, identity_id, &before);
        interactive.shutdown().await;
    }

    /// B4 — dismissing the password prompt aborts the recovery with the typed
    /// cancel error and writes nothing, because the prompt runs before any
    /// mutation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b4_tier2_prompt_cancel_aborts_with_zero_writes() {
        let offline = Offline::new(Some(Arc::new(TestPrompt::new([ScriptedAnswer::Cancel])))).await;
        let (identity_id, before) = seed_tier2_with_a_stranded_key(&offline).await;

        let error = offline
            .ctx
            .recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 2,
                }],
            )
            .await
            .expect_err("a dismissed prompt must abort the recovery");
        assert!(
            matches!(error, TaskError::SecretPromptCancelled),
            "expected SecretPromptCancelled, got {error:?}",
        );
        assert_unchanged(&offline, identity_id, &before);

        offline.shutdown().await;
    }

    /// B5 — a protected identity still carrying a resident-plaintext key means
    /// an earlier vault migration never finished. Recovery must refuse before
    /// touching the vault, with the error whose remedy actually fixes that
    /// state — not seal around it and trip the downgrade guard at persist.
    ///
    /// Drives `recover_into_loaded_identity`, the post-read core the task runs,
    /// because no production write path can persist this record shape.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b5_resident_plaintext_on_a_protected_identity_fails_before_any_vault_write() {
        let offline = Offline::new(Some(Arc::new(TestPrompt::never()))).await;
        let ctx = &offline.ctx;

        let owner = test_key(1, Purpose::OWNER, 0xE1);
        let stranded = test_key(2, Purpose::TRANSFER, 0xE2);
        let resident = test_key(3, Purpose::AUTHENTICATION, 0xE3);
        let stored = identity_with_keys(
            0x55,
            IdentityType::Masternode,
            &[],
            vec![(M, &owner, owner.clear())],
        );
        let identity_id = stored.identity.id();
        ctx.insert_local_qualified_identity(&stored, &None)
            .expect("insert modern record");
        ctx.protect_identity_keys(identity_id, Secret::new(PW), None)
            .expect("seal the identity Tier-2");
        let before = ctx
            .stored_identity_blob(&identity_id)
            .expect("read stored blob")
            .expect("record stored");

        // The state an interrupted load-path vault migration leaves: a Tier-2
        // key alongside one still resident as plaintext.
        let modern = identity_with_keys(
            0x55,
            IdentityType::Masternode,
            &[&stranded],
            vec![
                (M, &owner, PrivateKeyData::InVault),
                (M, &resident, resident.clear()),
            ],
        );
        let legacy = identity_with_keys(
            0x55,
            IdentityType::Masternode,
            &[],
            vec![(M, &stranded, stranded.clear())],
        );

        let error = ctx
            .recover_into_loaded_identity(
                &modern,
                legacy,
                &[RecoveryItem::Key {
                    target: M,
                    key_id: 2,
                }],
            )
            .await
            .expect_err("resident plaintext on a protected identity must fail closed");
        assert!(
            matches!(error, TaskError::IdentityKeyProtectionIncomplete),
            "expected IdentityKeyProtectionIncomplete, got {error:?}",
        );
        assert_eq!(
            offline.with_keys(identity_id, |view| view.scheme(&M, 2).expect("scheme")),
            SecretScheme::Absent,
            "the refusal must land before any vault write",
        );
        assert_eq!(
            ctx.stored_identity_blob(&identity_id)
                .expect("read stored blob"),
            Some(before),
            "the stored record must be untouched",
        );

        offline.shutdown().await;
    }

    /// B6 — recovery is idempotent by recomputation: the second run finds every
    /// approved item already in place, reports it as stale, and leaves the
    /// stored record byte-identical.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b6_a_second_run_recovers_nothing_and_changes_nothing() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let owner = test_key(1, Purpose::OWNER, 0xF1);
        let modern = identity_with_keys(0x66, IdentityType::Masternode, &[&owner], vec![]);
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");
        let legacy = identity_with_keys(
            0x66,
            IdentityType::Masternode,
            &[],
            vec![(M, &owner, owner.clear())],
        );
        offline.stage_legacy(&legacy);

        let approved = vec![RecoveryItem::Key {
            target: M,
            key_id: 1,
        }];
        let (first, _) = completion_of(
            ctx.recover_legacy_identity_data(identity_id, approved.clone())
                .await
                .expect("first run"),
        );
        assert_eq!(first.len(), 1);
        let after_first = ctx
            .stored_identity_blob(&identity_id)
            .expect("read stored blob");

        let (second, stale) = completion_of(
            ctx.recover_legacy_identity_data(identity_id, approved)
                .await
                .expect("second run must succeed, not error"),
        );
        assert!(second.is_empty(), "the second run restores nothing");
        assert_eq!(
            stale,
            vec![RecoveryItem::Key {
                target: M,
                key_id: 1
            }],
            "the already-restored key is reported as a stale approval",
        );
        assert_eq!(
            ctx.stored_identity_blob(&identity_id)
                .expect("read stored blob"),
            after_first,
            "a no-op run must not rewrite the stored record",
        );

        // And the check now offers nothing, so the affordance self-extinguishes.
        assert!(
            plan_of(
                ctx.check_legacy_recovery(identity_id)
                    .expect("check must succeed")
            )
            .is_empty(),
            "with nothing left stranded the plan must be empty",
        );

        offline.shutdown().await;
    }

    /// B7 — a deleted identity is not eligible. Recovery restores into an
    /// existing record; it must never resurrect one the user removed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b7_a_deleted_identity_is_not_recreated() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let owner = test_key(1, Purpose::OWNER, 0x71);
        let modern = identity_with_keys(0x77, IdentityType::Masternode, &[&owner], vec![]);
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");
        let legacy = identity_with_keys(
            0x77,
            IdentityType::Masternode,
            &[],
            vec![(M, &owner, owner.clear())],
        );
        offline.stage_legacy(&legacy);
        ctx.delete_local_qualified_identity(&identity_id)
            .expect("delete the identity");

        let error = ctx
            .recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 1,
                }],
            )
            .await
            .expect_err("a deleted identity must not be recoverable");
        assert!(
            matches!(error, TaskError::IdentityNotFoundLocally),
            "expected IdentityNotFoundLocally, got {error:?}",
        );
        assert!(
            ctx.get_local_qualified_identity(&identity_id)
                .expect("read back")
                .is_none(),
            "the deleted identity must stay deleted",
        );

        offline.shutdown().await;
    }

    /// B8 — a legacy row that will not decode is corruption, surfaced as a
    /// typed error. Reading it as an empty record would silently report
    /// "nothing to recover" for data that is still on disk.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b8_an_undecodable_legacy_row_is_a_typed_error_with_zero_writes() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let modern = identity_with_keys(0x88, IdentityType::Masternode, &[], vec![]);
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");
        offline.stage_legacy_blob(identity_id, Some(vec![0xFF; 8]));
        let before = ctx
            .stored_identity_blob(&identity_id)
            .expect("read stored blob");

        for error in [
            ctx.check_legacy_recovery(identity_id)
                .expect_err("check must report the corrupt row"),
            ctx.recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 1,
                }],
            )
            .await
            .expect_err("recovery must report the corrupt row"),
        ] {
            assert!(
                matches!(error, TaskError::LegacyIdentityUnreadable { identity_id: got } if got == identity_id),
                "expected LegacyIdentityUnreadable for this identity, got {error:?}",
            );
        }
        assert_eq!(
            ctx.stored_identity_blob(&identity_id)
                .expect("read stored blob"),
            before,
            "a corrupt legacy row must leave the stored record untouched",
        );

        offline.shutdown().await;
    }

    /// B9 — the ordinary "nothing here" shapes are not failures: no legacy row
    /// at all, an observed-identity cache row, and a NULL blob each report an
    /// empty plan so the affordance simply stays hidden.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b9_absent_cache_and_null_legacy_rows_report_nothing_to_recover() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let no_row = identity_with_keys(0x91, IdentityType::Masternode, &[], vec![]);
        let cache_row = identity_with_keys(0x92, IdentityType::Masternode, &[], vec![]);
        let null_blob = identity_with_keys(0x93, IdentityType::Masternode, &[], vec![]);
        for identity in [&no_row, &cache_row, &null_blob] {
            ctx.insert_local_qualified_identity(identity, &None)
                .expect("insert modern record");
        }
        LegacyIdentityFixture::new(
            cache_row.identity.id().to_buffer(),
            Some(cache_row.to_bytes()),
            "testnet",
        )
        .with_is_local(false)
        .insert(&ctx.db.locked_conn())
        .expect("stage the observed-identity cache row");
        offline.stage_legacy_blob(null_blob.identity.id(), None);

        for identity in [&no_row, &cache_row, &null_blob] {
            let identity_id = identity.identity.id();
            let plan = plan_of(
                ctx.check_legacy_recovery(identity_id)
                    .expect("check must succeed, not error"),
            );
            assert!(
                plan.is_empty(),
                "{identity_id} must report nothing to recover",
            );
        }

        offline.shutdown().await;
    }

    /// B10 — recovery takes the same per-identity claim the load paths take, so
    /// a load already running on this identity excludes it rather than racing
    /// its writes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b10_a_concurrent_load_claim_excludes_recovery() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let modern = identity_with_keys(0xA0, IdentityType::Masternode, &[], vec![]);
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");

        let _held = ctx
            .begin_identity_load(identity_id, None)
            .expect("hold the load claim");
        let error = ctx
            .recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 1,
                }],
            )
            .await
            .expect_err("recovery must not run while a load holds the identity");
        assert!(
            matches!(error, TaskError::IdentityLoadInProgress { identity_id: got } if got == identity_id),
            "expected IdentityLoadInProgress for this identity, got {error:?}",
        );

        offline.shutdown().await;
    }

    /// B11 — the flow is not masternode-specific: a `User` identity missing one
    /// of its main-identity keys recovers it the same way.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b11_a_user_identity_recovers_a_missing_main_identity_key() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let held = test_key(1, Purpose::AUTHENTICATION, 0x01);
        let stranded = test_key(2, Purpose::TRANSFER, 0x02);
        let modern = identity_with_keys(
            0xB0,
            IdentityType::User,
            &[&stranded],
            vec![(M, &held, held.clear())],
        );
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");
        let legacy = identity_with_keys(
            0xB0,
            IdentityType::User,
            &[],
            vec![(M, &held, held.clear()), (M, &stranded, stranded.clear())],
        );
        offline.stage_legacy(&legacy);

        let plan = plan_of(
            ctx.check_legacy_recovery(identity_id)
                .expect("check must succeed"),
        );
        assert_eq!(
            plan.items.len(),
            1,
            "only the key the modern record lacks is a candidate",
        );

        let (applied, _) = completion_of(
            ctx.recover_legacy_identity_data(identity_id, plan.approved_items())
                .await
                .expect("recovery must succeed"),
        );
        assert_eq!(applied.len(), 1);
        assert_eq!(
            offline.with_keys(identity_id, |view| *view
                .get(&M, 2)
                .expect("restored key")
                .expect("stored")),
            stranded.secret,
        );

        offline.shutdown().await;
    }

    /// B12 — the wallet link lives beside the blob, not inside it, and the
    /// update writer preserves it from the existing record. A legacy row with
    /// no wallet of its own must not null it out.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b12_recovery_preserves_the_modern_wallet_link() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let seed_hash = [0x77; 32];
        let stranded = test_key(1, Purpose::AUTHENTICATION, 0x0C);
        let modern = identity_with_keys(0xC0, IdentityType::User, &[&stranded], vec![]);
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &Some((seed_hash, 4)))
            .expect("insert modern record with a wallet link");
        let legacy = identity_with_keys(
            0xC0,
            IdentityType::User,
            &[],
            vec![(M, &stranded, stranded.clear())],
        );
        offline.stage_legacy(&legacy);

        ctx.recover_legacy_identity_data(
            identity_id,
            vec![RecoveryItem::Key {
                target: M,
                key_id: 1,
            }],
        )
        .await
        .expect("recovery must succeed");

        assert_eq!(
            ctx.stored_identity_wallet_link(&identity_id)
                .expect("read the wallet link"),
            Some((seed_hash, 4)),
            "the wallet link must survive the recovery write verbatim",
        );

        offline.shutdown().await;
    }

    /// A legacy key excluded as unreadable is reported to the user with its
    /// reason rather than silently dropped, and never restored.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_legacy_encrypted_key_is_reported_as_unrecoverable() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let modern = identity_with_keys(0xD0, IdentityType::Masternode, &[], vec![]);
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");
        let unreadable = test_key(1, Purpose::OWNER, 0x33);
        let legacy = identity_with_keys(
            0xD0,
            IdentityType::Masternode,
            &[],
            vec![(M, &unreadable, PrivateKeyData::Encrypted(vec![0x33; 48]))],
        );
        offline.stage_legacy(&legacy);

        let plan = plan_of(
            ctx.check_legacy_recovery(identity_id)
                .expect("check must succeed"),
        );
        assert!(plan.items.is_empty(), "an unreadable key is not restorable");
        assert_eq!(plan.excluded.len(), 1);
        assert_eq!(plan.excluded[0].1, ExclusionReason::LegacyEncryptedFormat);

        offline.shutdown().await;
    }

    /// A saved key the node rotated away from cannot sign for it any more.
    /// Restoring it would report the payout role as held and retire the remedy
    /// the operator actually needs, so the check reports it as unrestorable and
    /// an approval naming it changes nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_key_the_node_rotated_away_from_is_not_restorable() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let current = test_key(2, Purpose::TRANSFER, 0xD5);
        let rotated_out = test_key(1, Purpose::TRANSFER, 0xD6);
        // The chain knows only the current payout key.
        let modern = identity_with_keys(0xF0, IdentityType::Masternode, &[&current], vec![]);
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");
        let legacy = identity_with_keys(
            0xF0,
            IdentityType::Masternode,
            &[],
            vec![(M, &rotated_out, rotated_out.clear())],
        );
        offline.stage_legacy(&legacy);

        let plan = plan_of(
            ctx.check_legacy_recovery(identity_id)
                .expect("check must succeed"),
        );
        assert!(
            plan.items.is_empty(),
            "a rotated-away key is not restorable"
        );
        assert_eq!(
            plan.excluded
                .iter()
                .map(|(_, reason)| *reason)
                .collect::<Vec<_>>(),
            vec![ExclusionReason::KeyNoLongerOnIdentity],
        );

        let (applied, stale) = completion_of(
            ctx.recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 1,
                }],
            )
            .await
            .expect("an approval naming an unrestorable key is reported, not an error"),
        );
        assert!(applied.is_empty(), "nothing was restorable");
        assert!(
            stale.is_empty(),
            "an unrestorable key is excluded, never reported as already in place",
        );
        assert!(
            !ctx.get_local_qualified_identity(&identity_id)
                .expect("read back")
                .expect("still stored")
                .masternode_key_presence()
                .payout,
            "the node must still report its payout role as missing",
        );

        offline.shutdown().await;
    }

    /// An empty approval list is refused rather than widened into "restore
    /// everything" — nothing is restored without an explicit per-item decision.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_empty_approval_list_is_refused() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let modern = identity_with_keys(0xE0, IdentityType::Masternode, &[], vec![]);
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");

        let error = ctx
            .recover_legacy_identity_data(identity_id, vec![])
            .await
            .expect_err("an empty allowlist must be refused");
        assert!(
            matches!(error, TaskError::LegacyRecoveryNothingApproved),
            "expected LegacyRecoveryNothingApproved, got {error:?}",
        );

        offline.shutdown().await;
    }

    /// Seed a Tier-2 identity that still has one key stranded in the legacy
    /// file, returning its id and the at-rest blob a failed run must not
    /// change. Shared by the three password-failure cases.
    async fn seed_tier2_with_a_stranded_key(offline: &Offline) -> (Identifier, Vec<u8>) {
        let ctx = &offline.ctx;
        let owner = test_key(1, Purpose::OWNER, 0xC1);
        let payout = test_key(2, Purpose::TRANSFER, 0xD2);
        let modern = identity_with_keys(
            0x33,
            IdentityType::Masternode,
            &[&payout],
            vec![(M, &owner, owner.clear())],
        );
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");
        ctx.protect_identity_keys(identity_id, Secret::new(PW), None)
            .expect("seal the identity Tier-2");

        let legacy = identity_with_keys(
            0x33,
            IdentityType::Masternode,
            &[],
            vec![(M, &owner, owner.clear()), (M, &payout, payout.clear())],
        );
        offline.stage_legacy(&legacy);

        let before = ctx
            .stored_identity_blob(&identity_id)
            .expect("read stored blob")
            .expect("record stored");
        (identity_id, before)
    }

    /// Assert a failed recovery changed neither the stored record nor the vault.
    fn assert_unchanged(offline: &Offline, identity_id: Identifier, before: &[u8]) {
        assert_eq!(
            offline
                .ctx
                .stored_identity_blob(&identity_id)
                .expect("read stored blob")
                .as_deref(),
            Some(before),
            "a failed recovery must leave the stored record byte-identical",
        );
        assert_eq!(
            offline.with_keys(identity_id, |view| view.scheme(&M, 2).expect("scheme")),
            SecretScheme::Absent,
            "a failed recovery must write nothing to the vault",
        );
    }
}
