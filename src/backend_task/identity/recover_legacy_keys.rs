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
            Some(mut legacy) => {
                let plan = compute_recovery_plan(&modern, &legacy);
                // This runs every time the user opens a screen showing the
                // identity, so the stranded plaintext it decoded to count
                // candidates must not be released to the allocator intact.
                let _ = legacy.private_keys.take_plaintext_for_vault();
                plan
            }
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
    /// On a password-protected identity the password is verified up front, and
    /// re-proved against the vault as it stands at write time, before every
    /// merged key is sealed under it — all *before* the record is written, so
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
    /// legacy row, [`TaskError::LegacyRecoveryIdentityChanged`] when the
    /// identity's key protection changed under the flow, and the prompt's own
    /// cancel / unavailable errors. Every one of them leaves the stored record
    /// unchanged.
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

        // Refuse early while the storage migration is running, so a password
        // prompt never opens for work that cannot be written. The write section
        // re-checks under the guard; this is the courtesy check, not the gate.
        self.require_storage_migration_idle()?;

        let modern = self
            .get_local_qualified_identity(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let Some(legacy) = self.legacy_identity_record(identity_id)? else {
            claim.loaded();
            return Ok(nothing_recovered(identity_id));
        };

        let password = self
            .verify_recovery_password(&modern, legacy, &approved)
            .await?;
        let outcome = self.persist_legacy_recovery(identity_id, &approved, password.as_ref())?;
        claim.loaded();
        Ok(outcome)
    }

    /// The identity password this restore needs, verified against an
    /// ALREADY-LOADED `modern` record — `None` when the identity is keyless.
    ///
    /// Dry-runs the merge to answer two questions before anything is written:
    /// is there anything left to restore at all (no prompt for work that would
    /// not happen), and will the record the at-rest encoder sees carry a
    /// protected key? The second question is asked of the *merged* record, not
    /// of `modern`, because that is the record the encoder's downgrade guard
    /// will evaluate.
    ///
    /// Holds no lock across the prompt: it is a modal with no timeout, and the
    /// storage-migration mutex it used to sit inside is the same one identity
    /// removal and the post-migration refresh take.
    async fn verify_recovery_password(
        &self,
        modern: &QualifiedIdentity,
        legacy: QualifiedIdentity,
        approved: &[RecoveryItem],
    ) -> Result<Option<crate::wallet_backend::VerifiedIdentityPassword>, TaskError> {
        let mut preview = apply_recovery_plan(modern, legacy, approved);
        if preview.applied.is_empty() {
            return Ok(None);
        }
        let verify_scope = self.protected_identity_verify_scope(&preview.merged)?;
        // The dry run is discarded; the write section merges again against the
        // record as it stands then. Wipe its plaintext rather than dropping it.
        let _ = preview.merged.private_keys.take_plaintext_for_vault();

        let Some(verify_scope) = verify_scope else {
            return Ok(None);
        };
        // A key still resident as plaintext means an earlier vault migration
        // did not finish. Sealing around it would half-protect the identity and
        // then trip the guard at persist, so refuse here — before any vault
        // write — with its established remedy.
        reject_resident_identity_plaintext(&modern.private_keys)?;
        Ok(Some(
            self.wallet_backend()?
                .secret_access()
                .verify_identity_object_password(&verify_scope)
                .await?,
        ))
    }

    /// Merge and write, in one fully synchronous critical section.
    ///
    /// Everything is read again here, under this identity's record guard: the
    /// password prompt may have been open for minutes, and the record can have
    /// moved on (a refresh, a registered DPNS name) or been deleted outright.
    /// Re-reading and re-merging makes the write a superset of the record as it
    /// *is*, so a deleted identity cannot be resurrected. The guard is what
    /// makes that read-merge-write atomic against every other whole-record
    /// writer — refreshes, key edits, top-ups, transfers, DPNS registration —
    /// none of which take the load claim, and any of which would otherwise
    /// overwrite this merge with the snapshot it read a moment earlier.
    /// The re-merge costs nothing the allowlist-intersection rule did not
    /// already make idempotent.
    fn persist_legacy_recovery(
        &self,
        identity_id: Identifier,
        approved: &[RecoveryItem],
        password: Option<&crate::wallet_backend::VerifiedIdentityPassword>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // The storage migration owns the identity store while it runs; the
        // delete path takes the same two-part guard for the same reason.
        let _migration_guard = self
            .migration_run
            .try_lock()
            .map_err(|_| TaskError::WalletStorageNotReady)?;
        if self.migration_status().state().is_in_progress() {
            return Err(TaskError::WalletStorageNotReady);
        }
        // Lock order: the storage-migration mutex above, then the per-identity
        // record guard — the same order the delete path takes.
        let record_lock = self.identity_record_lock(identity_id);
        let _record_guard = record_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let modern = self
            .get_local_qualified_identity(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let Some(legacy) = self.legacy_identity_record(identity_id)? else {
            return Ok(nothing_recovered(identity_id));
        };
        let mut applied = apply_recovery_plan(&modern, legacy, approved);
        let excluded = std::mem::take(&mut applied.excluded);

        // Nothing left to restore, so no write at all.
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

        // Branch on the same predicate over the same record the at-rest
        // downgrade guard evaluates, so its trigger is false either way: Tier-2
        // seals every merged key before the write, and Tier-1 has no protected
        // key for the guard to protect.
        if let Some(verify_scope) = self.protected_identity_verify_scope(&applied.merged)? {
            let password = password.ok_or(TaskError::LegacyRecoveryIdentityChanged)?;
            // The password was proved against the vault as it stood when the
            // prompt closed, which is not this vault: an identity unprotected
            // and re-protected under a NEW password while the prompt was open
            // still reads as protected here. Sealing under the old password
            // would leave the identity needing two, so it is proved again
            // against the scope that exists now — under the record guard, which
            // keeps a tier change from landing between this check and the seal.
            if !self
                .wallet_backend()?
                .secret_access()
                .identity_object_password_still_opens(&verify_scope, password)?
            {
                return Err(TaskError::LegacyRecoveryIdentityChanged);
            }
            reject_resident_identity_plaintext(&modern.private_keys)?;
            self.seal_merged_plaintext_keys(&mut applied.merged, password)?;
        }

        self.write_local_qualified_identity_locked(&applied.merged)?;

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

    /// Refuse while the storage migration owns the identity store, without
    /// holding its mutex afterwards. The delete path takes the same two-part
    /// check.
    fn require_storage_migration_idle(&self) -> Result<(), TaskError> {
        let _probe = self
            .migration_run
            .try_lock()
            .map_err(|_| TaskError::WalletStorageNotReady)?;
        if self.migration_status().state().is_in_progress() {
            return Err(TaskError::WalletStorageNotReady);
        }
        Ok(())
    }

    /// The identity `identity_id` names in the preserved legacy `data.db`, or
    /// `None` for every ordinary "not here" answer. A row that exists but will
    /// not decode is an error instead: reading it as empty would close the
    /// recovery offer on data that is still on disk.
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
    use crate::context::identity_load_registry::IdentityLoadPhase;
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
    use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
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

    /// A prompt that parks the way a real modal parks on the user: it signals
    /// that it is open, then waits to be released before answering. The only
    /// way to observe what a task holds *while* a password dialog sits there.
    struct BlockingPrompt {
        passphrase: String,
        asked: tokio::sync::Notify,
        released: tokio::sync::Notify,
    }

    impl BlockingPrompt {
        fn new(passphrase: &str) -> Self {
            Self {
                passphrase: passphrase.to_string(),
                asked: tokio::sync::Notify::new(),
                released: tokio::sync::Notify::new(),
            }
        }

        /// Resolve once the prompt has been opened.
        async fn wait_until_asked(&self) {
            self.asked.notified().await;
        }

        /// Let the parked prompt answer.
        fn release(&self) {
            self.released.notify_one();
        }
    }

    #[async_trait::async_trait]
    impl SecretPrompt for BlockingPrompt {
        async fn request(
            &self,
            _request: crate::wallet_backend::secret_prompt::SecretPromptRequest,
        ) -> Result<
            crate::wallet_backend::secret_prompt::SecretPromptReply,
            crate::wallet_backend::secret_prompt::SecretPromptCancelled,
        > {
            self.asked.notify_one();
            self.released.notified().await;
            Ok(
                crate::wallet_backend::secret_prompt::SecretPromptReply::new(
                    SecretString::new(&self.passphrase),
                    crate::wallet_backend::secret_prompt::RememberPolicy::None,
                ),
            )
        }

        fn is_interactive(&self) -> bool {
            true
        }
    }

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
            private_keys.insert_at(
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

    /// B1 — the canonical case, end to end on a keyless (Tier-1) node: a
    /// masternode whose owner and voting keys are still only in the legacy file
    /// gets them back. The keys must land in the vault, the record must show
    /// both roles present, the at-rest blob must carry no plaintext, and the
    /// legacy rows must survive untouched (AC-5).
    ///
    /// The modern record carries the voter-identity link a re-load kept while
    /// rebuilding the key map without a voting key. That link is the witness
    /// that makes the stranded voting key restorable: a record naming no voter
    /// identity has none but the legacy file itself, and the file cannot vouch
    /// for its own contents (`a_voter_key_only_the_legacy_blob_vouches_for_is_excluded`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b1_tier1_masternode_recovers_owner_and_voting_keys() {
        let owner = test_key(1, Purpose::OWNER, 0xA1);
        let voting = test_key(2, Purpose::VOTING, 0xB2);
        let owner_secret = owner.secret;
        let voting_secret = voting.secret;

        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        // The modern record: the chain's keys are known but none of their
        // private halves are held.
        let mut modern = identity_with_keys(0x11, IdentityType::Masternode, &[&owner], vec![]);
        modern.associated_voter_identity = Some(voter_identity(0x99, &[&voting]));
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert the modern record");

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
            2,
            "both stranded keys are candidates; the voter link is already held",
        );

        let (applied, stale) = completion_of(
            ctx.recover_legacy_identity_data(identity_id, plan.approved_items())
                .await
                .expect("recovery must succeed"),
        );
        assert_eq!(applied.len(), 2, "every approved item was restored");
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
    /// Drives `verify_recovery_password`, the preflight the task runs before it
    /// prompts, because no production write path can persist this record shape.
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
            .verify_recovery_password(
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

    /// B13 — an identity that gains password protection mid-flight fails
    /// closed. The dry run on a keyless identity decides no password is needed;
    /// a concurrent `ProtectIdentityKeys` makes the record Tier-2 before the
    /// write section re-reads it, and the write must refuse rather than seal
    /// the merged keys under a password state nothing ever verified.
    ///
    /// Drives `persist_legacy_recovery` directly with the `None` the dry run
    /// produced, the way B5 drives `verify_recovery_password`: a Tier-1 restore
    /// never prompts, so there is no await point between the two for a
    /// concurrent task to land in deterministically. The dry run is exercised
    /// first, so the `None` under test is the one production would carry.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b13_an_identity_protected_during_the_flow_fails_closed() {
        let offline = Offline::new(Some(Arc::new(TestPrompt::never()))).await;
        let ctx = &offline.ctx;

        let owner = test_key(1, Purpose::OWNER, 0x1A);
        let stranded = test_key(2, Purpose::TRANSFER, 0x2B);
        let modern = identity_with_keys(
            0xB1,
            IdentityType::Masternode,
            &[&stranded],
            vec![(M, &owner, owner.clear())],
        );
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert the keyless modern record");
        let legacy = identity_with_keys(
            0xB1,
            IdentityType::Masternode,
            &[],
            vec![(M, &stranded, stranded.clear())],
        );
        offline.stage_legacy(&legacy);

        let approved = vec![RecoveryItem::Key {
            target: M,
            key_id: 2,
        }];
        let stored = ctx
            .get_local_qualified_identity(&identity_id)
            .expect("read the record")
            .expect("record stored");
        assert!(
            ctx.verify_recovery_password(&stored, legacy, &approved)
                .await
                .expect("the dry run must succeed")
                .is_none(),
            "a keyless identity needs no password, which is what makes this guard reachable",
        );

        // The transition the guard exists for, landing before the write.
        ctx.protect_identity_keys(identity_id, Secret::new(PW), None)
            .expect("seal the identity Tier-2");
        let before = ctx
            .stored_identity_blob(&identity_id)
            .expect("read stored blob")
            .expect("record stored");

        let error = ctx
            .persist_legacy_recovery(identity_id, &approved, None)
            .expect_err("the write must refuse a password state nothing verified");
        assert!(
            matches!(error, TaskError::LegacyRecoveryIdentityChanged),
            "expected LegacyRecoveryIdentityChanged, got {error:?}",
        );
        assert_unchanged(&offline, identity_id, &before);

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

    /// The password prompt is a modal with no timeout, so nothing global may be
    /// held while it is open. The storage-migration mutex it used to sit inside
    /// is the same one identity removal and the post-migration DAPI refresh
    /// take: holding it made deleting an *unrelated* identity fail with
    /// "storage is still being updated" for as long as the dialog sat there.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_open_password_prompt_blocks_no_unrelated_work() {
        let prompt = Arc::new(BlockingPrompt::new(PW));
        let offline = Offline::new(Some(prompt.clone())).await;
        let (identity_id, _) = seed_tier2_with_a_stranded_key(&offline).await;

        // An identity the restore never touches.
        let unrelated = identity_with_keys(0xAB, IdentityType::User, &[], vec![]);
        let unrelated_id = unrelated.identity.id();
        offline
            .ctx
            .insert_local_qualified_identity(&unrelated, &None)
            .expect("insert the unrelated identity");

        let ctx = offline.ctx.clone();
        let restore = tokio::spawn(async move {
            ctx.recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 2,
                }],
            )
            .await
        });
        prompt.wait_until_asked().await;

        offline
            .ctx
            .delete_local_qualified_identity(&unrelated_id)
            .expect("removing an unrelated identity must not wait on an open password prompt");
        assert!(
            offline.ctx.migration_run.try_lock().is_ok(),
            "the storage-migration mutex must stay free while a prompt is open",
        );

        prompt.release();
        let (applied, _) = completion_of(
            restore
                .await
                .expect("the restore task must not panic")
                .expect("the restore must complete once the password arrives"),
        );
        assert_eq!(applied.len(), 1, "the answered restore still lands");

        offline.shutdown().await;
    }

    /// Another writer can persist the same identity while the prompt is open —
    /// nothing else takes the load claim. The restore must merge into the record
    /// as it stands at write time, not into the snapshot it read before
    /// prompting, or it would silently revert that update.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_update_landing_during_the_prompt_is_not_reverted() {
        let prompt = Arc::new(BlockingPrompt::new(PW));
        let offline = Offline::new(Some(prompt.clone())).await;
        let (identity_id, _) = seed_tier2_with_a_stranded_key(&offline).await;

        let ctx = offline.ctx.clone();
        let restore = tokio::spawn(async move {
            ctx.recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 2,
                }],
            )
            .await
        });
        prompt.wait_until_asked().await;

        // The shape a refresh or a DPNS registration writes: read, change, save.
        let mut concurrent = offline
            .ctx
            .get_local_qualified_identity(&identity_id)
            .expect("read the record")
            .expect("record stored");
        concurrent.alias = Some("renamed while the prompt was open".to_string());
        offline
            .ctx
            .update_local_qualified_identity(&concurrent)
            .expect("a concurrent writer takes no claim, so this lands");

        prompt.release();
        restore
            .await
            .expect("the restore task must not panic")
            .expect("the restore must complete");

        let stored = offline
            .ctx
            .get_local_qualified_identity(&identity_id)
            .expect("read back")
            .expect("still stored");
        assert_eq!(
            stored.alias.as_deref(),
            Some("renamed while the prompt was open"),
            "the update that landed during the prompt must survive the restore",
        );
        assert!(
            stored.private_keys.has(&(M, 2)),
            "and the restored key must still be there",
        );

        offline.shutdown().await;
    }

    /// A restore that finds nothing staged in the previous version's data did
    /// not fail — it succeeded with nothing to do. The load registry has to say
    /// so, or every ordinary identity leaves a `Failed` record behind.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_run_with_nothing_staged_is_recorded_as_loaded() {
        let offline = Offline::new(None).await;
        let ctx = &offline.ctx;

        let modern = identity_with_keys(0xAC, IdentityType::User, &[], vec![]);
        let identity_id = modern.identity.id();
        ctx.insert_local_qualified_identity(&modern, &None)
            .expect("insert modern record");

        let (applied, _) = completion_of(
            ctx.recover_legacy_identity_data(
                identity_id,
                vec![RecoveryItem::Key {
                    target: M,
                    key_id: 1,
                }],
            )
            .await
            .expect("a run with nothing staged is a success"),
        );
        assert!(applied.is_empty());
        assert_eq!(
            ctx.last_identity_load_phase(&identity_id),
            Some(IdentityLoadPhase::Loaded),
            "a benign no-op must not be recorded as a failed load",
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

    /// B14 — the password the prompt verified proves nothing about the vault the
    /// seal will write to. `VerifiedIdentityPassword` records that one protected
    /// key opened when the prompt closed; an identity unprotected and
    /// re-protected under a DIFFERENT password after that still reads as
    /// protected at write time, so the restore would seal the recovered keys
    /// under the stale password and leave one identity needing two.
    ///
    /// The window is real: the tier migrations take neither the recovery claim
    /// nor the storage-migration mutex. Driven through
    /// `persist_legacy_recovery` with the password the preflight produced, the
    /// way B13 drives it with that preflight's `None` — the two are consecutive
    /// statements with no await between them for a concurrent task to land in
    /// deterministically, and parking the prompt itself proves something else
    /// (a tier change during the prompt is caught by the prompt's own re-ask).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn b14_a_password_verified_against_an_obsolete_vault_never_seals() {
        const NEW_PW: &str = "a-different-identity-password";

        let offline =
            Offline::new(Some(Arc::new(TestPrompt::new([ScriptedAnswer::once(PW)])))).await;
        let ctx = &offline.ctx;
        let (identity_id, _) = seed_tier2_with_a_stranded_key(&offline).await;
        let approved = vec![RecoveryItem::Key {
            target: M,
            key_id: 2,
        }];

        // The preflight, as the task runs it: one prompt, verified against the
        // identity as it stands now.
        let modern = ctx
            .get_local_qualified_identity(&identity_id)
            .expect("read the record")
            .expect("record stored");
        let legacy = ctx
            .legacy_identity_record(identity_id)
            .expect("read the legacy row")
            .expect("legacy row staged");
        let password = ctx
            .verify_recovery_password(&modern, legacy, &approved)
            .await
            .expect("the preflight must succeed");
        assert!(
            password.is_some(),
            "a protected identity must carry a verified password, which is what makes this reachable",
        );

        // The vault moves on: keyless, then sealed again under a password this
        // restore never saw.
        ctx.unprotect_identity_keys(identity_id, Secret::new(PW))
            .expect("opt out takes neither the recovery claim nor the migration mutex");
        ctx.protect_identity_keys(identity_id, Secret::new(NEW_PW), None)
            .expect("re-protect under a new password");

        let error = ctx
            .persist_legacy_recovery(identity_id, &approved, password.as_ref())
            .expect_err("a stale password must never seal the recovered keys");
        assert!(
            matches!(error, TaskError::LegacyRecoveryIdentityChanged),
            "expected LegacyRecoveryIdentityChanged, got {error:?}",
        );

        offline.with_keys(identity_id, |view| {
            assert_eq!(
                view.scheme(&M, 2).expect("scheme"),
                SecretScheme::Absent,
                "the recovered key must not have been sealed at all",
            );
            assert_eq!(
                view.scheme(&M, 1).expect("scheme"),
                SecretScheme::Protected,
                "the identity's existing key is still protected",
            );
            assert!(
                view.get_protected(&M, 1, &SecretString::new(NEW_PW))
                    .expect("get_protected")
                    .is_some(),
                "and it opens under the one password the identity now has",
            );
        });

        offline.shutdown().await;
    }

    /// B15 — the serialization claim behind the recovery write: while this
    /// identity's record guard is held — the guard `persist_legacy_recovery`
    /// takes for its whole read → merge → seal → write span — no other
    /// whole-record writer of the same identity can land.
    ///
    /// Every ordinary writer goes through one of these entry points: a refresh,
    /// a top-up, a transfer, a DPNS registration and a key edit all call
    /// `update_local_qualified_identity`. None of them take the identity-load
    /// claim, so before the guard any of them could write its own snapshot
    /// between the merge's read and its write, and erase the restored keys.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn b15_a_held_record_guard_excludes_every_whole_record_writer() {
        use std::time::Duration;

        let offline = Offline::new(None).await;
        let owner = test_key(1, Purpose::OWNER, 0x5A);
        let stored = identity_with_keys(
            0xE5,
            IdentityType::Masternode,
            &[],
            vec![(M, &owner, owner.clear())],
        );
        let identity_id = stored.identity.id();
        offline
            .ctx
            .insert_local_qualified_identity(&stored, &None)
            .expect("insert the record");

        type Writer = fn(&AppContext, Identifier, &QualifiedIdentity);
        let writers: [(&str, Writer); 5] = [
            ("a key edit or refresh", |ctx, _id, qi| {
                ctx.update_local_qualified_identity(qi).expect("update");
            }),
            ("a re-load", |ctx, _id, qi| {
                ctx.insert_local_qualified_identity(qi, &None)
                    .expect("insert");
            }),
            ("a rename", |ctx, id, _qi| {
                ctx.set_identity_alias(&id, Some("renamed")).expect("alias");
            }),
            ("a protection opt-in", |ctx, id, _qi| {
                ctx.protect_identity_keys(id, Secret::new(PW), None)
                    .expect("protect");
            }),
            ("a removal", |ctx, id, _qi| {
                ctx.delete_local_qualified_identity(&id).expect("delete");
            }),
        ];

        for (what, writer) in writers {
            let lock = offline.ctx.identity_record_lock(identity_id);
            let guard = lock.lock().expect("the record guard");

            let (started, has_started) = std::sync::mpsc::channel();
            let (finished, has_finished) = std::sync::mpsc::channel();
            let ctx = offline.ctx.clone();
            let snapshot = stored.clone();
            let thread = std::thread::spawn(move || {
                started.send(()).expect("report the writer started");
                writer(&ctx, identity_id, &snapshot);
                finished.send(()).expect("report the write landed");
            });
            has_started.recv().expect("the writer thread started");

            assert!(
                has_finished
                    .recv_timeout(Duration::from_millis(250))
                    .is_err(),
                "{what} must not land inside a held record guard",
            );
            drop(guard);
            has_finished
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or_else(|_| panic!("{what} must land once the guard is released"));
            thread.join().expect("the writer thread must not panic");
        }

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
