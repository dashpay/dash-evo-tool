use super::AppContext;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Identity blob slot, scoped to [`DetScope::Identity`]. One entry per
/// identity; the identity id is carried by the scope, so the key is a
/// fixed slot inside the upstream `meta_identity` table.
const IDENTITY_KEY: &str = "det:identity:v1";

/// Versioned key for the user's custom identity ordering. Holds a single
/// `Vec<[u8; 32]>` of identity IDs in display order. Lives in
/// [`DetScope::Global`] because it spans every identity. Bumping the
/// version suffix is a deliberate breaking change.
const IDENTITY_ORDER_KEY: &str = "det:identity_order:v1";

/// Global enumeration index: the complete set of stored identity ids.
/// [`DetScope::Identity`] has no cross-identity listing, so this Global
/// slot is the authoritative roster the load paths iterate. Maintained
/// on every identity insert and delete. Distinct from
/// [`IDENTITY_ORDER_KEY`], which is a user-ordering view that may lag the
/// full set.
const IDENTITY_INDEX_KEY: &str = "det:identity_index:v1";

/// Scheduled-vote slot key, scoped to [`DetScope::Identity`] of the
/// voter. The full key is `det:scheduled_vote:<contested_name>` — the
/// voter id is carried by the scope.
const SCHEDULED_VOTE_KEY_PREFIX: &str = "det:scheduled_vote:";

/// Global enumeration index: the complete set of voter ids that have at
/// least one scheduled vote. Scheduled votes are scoped to
/// [`DetScope::Identity`] of the voter, which has no cross-voter listing,
/// so this Global slot drives the network-wide enumeration and clear
/// paths. Maintained on insert and pruned when a voter's last scheduled
/// vote is removed.
const SCHEDULED_VOTE_VOTERS_KEY: &str = "det:scheduled_vote_voters:v1";

/// Top-up history slot, scoped to [`DetScope::Identity`]. One entry per
/// identity; the identity id is carried by the scope.
const TOP_UPS_KEY: &str = "det:top_ups:v1";

fn scheduled_vote_key(contested_name: &str) -> String {
    format!("{SCHEDULED_VOTE_KEY_PREFIX}{contested_name}")
}

/// Map a k/v adapter failure to the identity-blob storage error.
fn identity_err(source: KvAdapterError) -> TaskError {
    TaskError::IdentityStorage { source }
}

/// Map a k/v adapter failure to the scheduled-vote storage error.
fn scheduled_vote_err(source: KvAdapterError) -> TaskError {
    TaskError::ScheduledVoteStorage { source }
}

/// Map a k/v adapter failure to the top-up-history storage error.
fn top_up_err(source: KvAdapterError) -> TaskError {
    TaskError::TopUpHistoryStorage { source }
}

fn keep_first_unload_cleanup_error(
    cleanup_error: &mut Option<TaskError>,
    identity_id: Identifier,
    result: std::result::Result<(), TaskError>,
) {
    if let Err(source) = result {
        if cleanup_error.is_none() {
            *cleanup_error = Some(TaskError::IdentityUnloadCleanupFailed {
                identity_id,
                source: Box::new(source),
            });
        } else {
            tracing::warn!(
                identity_id = %identity_id,
                error = ?source,
                "Additional identity unload cleanup step failed"
            );
        }
    }
}

/// Merge `top_ups` into the stored history of `identity_id` (read-merge-write).
///
/// Callers hold a partial view of the history — the top-up flow carries the
/// entries it hydrated plus the one it just confirmed, and the legacy-data
/// import carries only what `data.db` held — so a replacing write would drop
/// every entry the caller never saw. On a colliding index the caller's value
/// wins: it is the fresher of the two. Removal is not expressible here; the
/// purge path deletes the whole key instead.
fn save_top_ups_in(
    kv: &DetKv,
    identity_id: &[u8; 32],
    top_ups: &std::collections::BTreeMap<u32, u64>,
) -> std::result::Result<(), TaskError> {
    let scope = DetScope::Identity(identity_id);
    let mut merged = kv
        .get::<std::collections::BTreeMap<u32, u64>>(scope, TOP_UPS_KEY)
        .map_err(top_up_err)?
        .unwrap_or_default();
    merged.extend(top_ups.iter().map(|(index, amount)| (*index, *amount)));
    kv.put(scope, TOP_UPS_KEY, &merged).map_err(top_up_err)
}

/// Validate a raw voter id and return it as the `[u8; 32]` the
/// [`DetScope::Identity`] scope borrows. Surfaces a typed error rather
/// than panicking on a wrong-length slice.
fn voter_buffer(identity_id: &[u8]) -> std::result::Result<[u8; 32], TaskError> {
    Identifier::from_bytes(identity_id)
        .map(|id| id.to_buffer())
        .map_err(|source| TaskError::InvalidVoterIdentifier { source })
}

/// Decode a stored bincode'd [`QualifiedIdentity`] blob, attaching the
/// active network. The encoder skips `status` / `network` (rehydrated by
/// callers) and `associated_wallets` / `top_ups` (filled by callers from
/// the wallet map and the top-up k/v entry).
fn decode_stored_identity(
    bytes: &[u8],
    network: Network,
) -> std::result::Result<QualifiedIdentity, TaskError> {
    let mut qi = QualifiedIdentity::from_bytes(bytes).map_err(|detail| {
        // `QualifiedIdentity::from_bytes` only fails when bincode decode
        // fails. Re-derive a typed `DecodeError` so the wrapper carries a
        // structured cause instead of a stringified one.
        TaskError::IdentityEncoding {
            source: bincode::error::DecodeError::OtherString(detail),
        }
    })?;
    qi.network = network;
    Ok(qi)
}

/// Persisted shape of a local identity entry. The [`QualifiedIdentity`]
/// itself is stored as its own bincode encoding (`to_bytes()`), with the
/// network-scoped metadata that the pre-C7 SQLite schema kept in dedicated
/// columns alongside. Fields that the encoder skips (status, network) are
/// rehydrated from the wrapper at read time.
#[derive(Serialize, Deserialize)]
struct StoredQualifiedIdentity {
    /// `QualifiedIdentity::to_bytes()` — bincode of the inner struct.
    /// Carries everything `Encode` writes: identity, alias, private keys,
    /// DPNS names, voter/operator associations.
    qi_bytes: Vec<u8>,
    /// Identity status (created/active/etc.). Held outside `qi_bytes`
    /// because the bincode shape deliberately omits status — matches the
    /// pre-C7 column-vs-blob split.
    status: u8,
    /// Identity type label (`User` / `Masternode` / `Evonode`). Stored
    /// alongside the blob so filter queries (voting, user-only) avoid a
    /// full decode pass.
    identity_type: String,
    /// Wallet seed-hash this identity was loaded from, if any. Mirrors
    /// the nullable `wallet` column the SQLite schema carried.
    wallet_hash: Option<[u8; 32]>,
    /// Account index within `wallet_hash`. `Some` iff `wallet_hash` is
    /// also `Some` (mirrors the pre-C7 `CHECK` constraint).
    wallet_index: Option<u32>,
}

impl std::fmt::Debug for StoredQualifiedIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `qi_bytes` is the bincode of a `QualifiedIdentity`, which carries
        // raw private keys — never print it. Emit only the length and the
        // non-secret metadata.
        f.debug_struct("StoredQualifiedIdentity")
            .field("qi_bytes", &"[redacted]")
            .field("qi_bytes_len", &self.qi_bytes.len())
            .field("status", &self.status)
            .field("identity_type", &self.identity_type)
            .field("wallet_hash", &self.wallet_hash)
            .field("wallet_index", &self.wallet_index)
            .finish()
    }
}

/// Persisted shape of a scheduled DPNS vote. Mirrors
/// [`ScheduledDPNSVote`] but with a serde-friendly representation of
/// the SDK's [`ResourceVoteChoice`] (which only derives bincode under
/// this feature set).
#[derive(Debug, Serialize, Deserialize)]
struct StoredScheduledVote {
    voter_id: [u8; 32],
    contested_name: String,
    choice: StoredVoteChoice,
    unix_timestamp: u64,
    executed_successfully: bool,
}

#[derive(Debug, Serialize, Deserialize)]
enum StoredVoteChoice {
    TowardsIdentity([u8; 32]),
    Abstain,
    Lock,
}

impl From<&ScheduledDPNSVote> for StoredScheduledVote {
    fn from(v: &ScheduledDPNSVote) -> Self {
        Self {
            voter_id: v.voter_id.to_buffer(),
            contested_name: v.contested_name.clone(),
            choice: match v.choice {
                ResourceVoteChoice::TowardsIdentity(id) => {
                    StoredVoteChoice::TowardsIdentity(id.to_buffer())
                }
                ResourceVoteChoice::Abstain => StoredVoteChoice::Abstain,
                ResourceVoteChoice::Lock => StoredVoteChoice::Lock,
            },
            unix_timestamp: v.unix_timestamp,
            executed_successfully: v.executed_successfully,
        }
    }
}

impl From<StoredScheduledVote> for ScheduledDPNSVote {
    fn from(v: StoredScheduledVote) -> Self {
        ScheduledDPNSVote {
            voter_id: Identifier::from(v.voter_id),
            contested_name: v.contested_name,
            choice: match v.choice {
                StoredVoteChoice::TowardsIdentity(id) => {
                    ResourceVoteChoice::TowardsIdentity(Identifier::from(id))
                }
                StoredVoteChoice::Abstain => ResourceVoteChoice::Abstain,
                StoredVoteChoice::Lock => ResourceVoteChoice::Lock,
            },
            unix_timestamp: v.unix_timestamp,
            executed_successfully: v.executed_successfully,
        }
    }
}

/// Read the Global identity-id enumeration index. Returns an empty
/// vector when the index has never been written.
fn load_identity_index(kv: &DetKv) -> std::result::Result<Vec<[u8; 32]>, TaskError> {
    Ok(kv
        .get::<Vec<[u8; 32]>>(DetScope::Global, IDENTITY_INDEX_KEY)
        .map_err(identity_err)?
        .unwrap_or_default())
}

/// Add `identity_id` to the Global enumeration index if absent. No-op
/// when the id is already tracked, so repeated inserts stay idempotent.
fn index_add_identity(kv: &DetKv, identity_id: &[u8; 32]) -> std::result::Result<(), TaskError> {
    let mut index = load_identity_index(kv)?;
    if index.contains(identity_id) {
        return Ok(());
    }
    index.push(*identity_id);
    kv.put(DetScope::Global, IDENTITY_INDEX_KEY, &index)
        .map_err(identity_err)
}

/// Remove `identity_id` from the Global enumeration index. No-op when
/// the id is not present.
fn index_remove_identity(kv: &DetKv, identity_id: &[u8; 32]) -> std::result::Result<(), TaskError> {
    let mut index = load_identity_index(kv)?;
    let before = index.len();
    index.retain(|id| id != identity_id);
    if index.len() == before {
        return Ok(());
    }
    kv.put(DetScope::Global, IDENTITY_INDEX_KEY, &index)
        .map_err(identity_err)
}

/// Delete every Identity-scoped child of `id` (blob, top-up history, all
/// scheduled votes) and prune the scheduled-vote voter index. Does not
/// touch the Global identity index — callers decide whether to drop the
/// index entry (single delete) or rewrite it wholesale (devnet sweep).
/// Outcome of [`migrate_keystore_to_vault`], so callers/tests can assert what
/// happened without re-inspecting the blob.
#[derive(Debug, PartialEq, Eq)]
enum KeystoreMigration {
    /// No plaintext keys to migrate — `qi` was untouched.
    Nothing,
    /// The vault write failed; `qi` was restored to its resident plaintext and
    /// the blob was NOT persisted (next load retries — no key loss).
    VaultWriteFailed,
    /// `n` keys moved to the vault and `qi` rewritten to `InVault` placeholders.
    Migrated(usize),
    /// The identity is password-protected, so a resident plaintext key
    /// was NOT migrated to a keyless vault entry. `qi` keeps its resident key (it
    /// still signs this session) and nothing is persisted; the add-key path seals
    /// new keys Tier-2 explicitly.
    ProtectedSkipped,
}

/// Find an existing password-protected (Tier-2) key of this identity, as a
/// [`SecretScope`](crate::wallet_backend::secret_prompt::SecretScope) suitable
/// for verifying the identity's password when sealing a newly-added key.
/// `None` when the identity has no protected key — i.e. the identity
/// is keyless and the default path applies.
fn find_protected_identity_key_scope(
    secret_store: &Arc<platform_wallet_storage::secrets::SecretStore>,
    id: &[u8; 32],
    qi: &QualifiedIdentity,
) -> Option<crate::wallet_backend::secret_prompt::SecretScope> {
    use crate::wallet_backend::secret_prompt::SecretScope;
    use crate::wallet_backend::secret_seam::SecretScheme;
    let view = crate::wallet_backend::IdentityKeyView::new(secret_store, *id);
    qi.private_keys
        .keys_set()
        .into_iter()
        .find_map(|(target, key_id)| match view.scheme(&target, key_id) {
            Ok(SecretScheme::Protected) => Some(SecretScope::IdentityKey {
                identity_id: *id,
                target,
                key_id,
            }),
            _ => None,
        })
}

/// EAGER identity-key migration core (vault-first, crash-safe). Moves any
/// plaintext `Clear`/`AlwaysClear` keys in `qi` into the vault as raw bytes,
/// then asks `persist` to rewrite the blob with `InVault` placeholders.
///
/// Ordering is the funds-safety contract: vault `store_all` happens FIRST. On a
/// vault-write failure `qi` is restored to its pre-migration resident plaintext
/// (so this session can still sign) and `persist` is NOT called — the legacy
/// blob stays for the next retry, and no key is lost on a mid-write fault. A
/// `persist` failure after a successful vault write is recoverable: the legacy
/// blob plus the now-redundant raw vault entries are re-detected next load and
/// the migration re-runs idempotently.
///
/// Factored out of [`AppContext`] so it is unit-testable with a bare
/// `SecretStore` and a controllable `persist` closure.
fn migrate_keystore_to_vault(
    secret_store: &Arc<platform_wallet_storage::secrets::SecretStore>,
    id: &[u8; 32],
    qi: &mut QualifiedIdentity,
    persist: impl FnOnce(&QualifiedIdentity) -> std::result::Result<(), TaskError>,
) -> KeystoreMigration {
    // Probe before cloning: the steady-state (already all-`InVault`) case must
    // not pay for a full `KeyStorage` clone — that clone exists only to restore
    // the resident plaintext on a vault-write failure.
    if !qi.private_keys.has_plaintext_for_vault() {
        return KeystoreMigration::Nothing;
    }
    // Fail-closed: never migrate a protected identity's resident
    // plaintext to a KEYLESS vault entry — that would silently strip protection
    // off a new key. Leave it resident (it still signs this session) and persist
    // nothing; the add-key path seals new keys Tier-2 under the identity password.
    if find_protected_identity_key_scope(secret_store, id, qi).is_some() {
        tracing::warn!(
            target = "context::identity_db",
            identity = %hex::encode(id),
            "Skipped keyless migration of a resident key on a password-protected identity",
        );
        return KeystoreMigration::ProtectedSkipped;
    }
    let mut before = qi.private_keys.clone();
    let taken = qi.private_keys.take_plaintext_for_vault();
    let view = crate::wallet_backend::IdentityKeyView::new(secret_store, *id);
    if let Err(e) = view.store_all(&taken) {
        qi.private_keys = before;
        tracing::warn!(
            target = "context::identity_db",
            identity = %hex::encode(id),
            error = ?e,
            "Identity-key vault migration deferred (vault write failed)",
        );
        return KeystoreMigration::VaultWriteFailed;
    }
    let migrated = taken.len();
    // The migrated plaintext now lives only in the vault; drop the `taken` copy
    // (it zeroizes on drop) so its key bytes do not linger across the DB write.
    drop(taken);
    // The vault write succeeded — the rollback clone is no longer
    // needed. Zeroize its plaintext bytes (Clear/AlwaysClear) before it drops
    // so no identity private key lingers in freed heap.
    let _ = before.take_plaintext_for_vault();
    if let Err(e) = persist(qi) {
        tracing::warn!(
            target = "context::identity_db",
            identity = %hex::encode(id),
            error = ?e,
            "Identity-key blob rewrite deferred after vault migration",
        );
    } else {
        tracing::info!(
            target = "context::identity_db",
            identity = %hex::encode(id),
            migrated,
            "Migrated identity keys to the secret vault",
        );
    }
    KeystoreMigration::Migrated(migrated)
}

/// Encode `qi` for at-rest storage with every resident plaintext private key
/// moved into the secret vault FIRST, leaving `InVault` placeholders in the
/// returned blob. This is the write-path twin of [`migrate_keystore_to_vault`]
/// (the load-path migration): a freshly inserted or updated identity never
/// writes `Clear` / `AlwaysClear` key bytes to `det-app.sqlite`.
///
/// Funds-safe ordering: the vault `store_all` happens BEFORE the bytes are
/// produced. On a vault-write failure the error propagates and the caller
/// persists nothing — never plaintext, never `InVault` placeholders without the
/// backing vault entries. Operates on a clone so the caller's in-memory
/// identity keeps its resident keys (signing continues this session). A blob
/// with no plaintext keys (already migrated / watch-only) encodes unchanged.
fn encode_identity_blob_vault_first(
    secret_store: &Arc<platform_wallet_storage::secrets::SecretStore>,
    id: &[u8; 32],
    qi: &QualifiedIdentity,
) -> std::result::Result<Vec<u8>, TaskError> {
    // No resident plaintext ⇒ nothing to vault and nothing to rewrite; encode
    // the borrow directly without a clone (the steady-state, already-`InVault`
    // identity that callers re-save unchanged).
    if !qi.private_keys.has_plaintext_for_vault() {
        return Ok(qi.to_bytes());
    }
    // Fail-closed: a password-protected identity must NEVER acquire a
    // keyless key. If any existing key is Tier-2, refuse to move new plaintext
    // into the vault keyless — the add-key path seals the new key Tier-2 under
    // the identity's password and marks it `InVault` first, so a correctly-sealed
    // add never reaches this branch. This closes the silent-plaintext-key leak.
    //
    // A Mixed identity (some keys Tier-2, some still resident plaintext) hits
    // this same guard on a plain re-save — e.g. an alias edit — so the re-save
    // fails closed until "Finish protecting" reseals the remaining keys under
    // the identity password. This is intended secure behavior, not a regression.
    if find_protected_identity_key_scope(secret_store, id, qi).is_some() {
        return Err(TaskError::IdentityKeyProtectionDowngrade);
    }
    let mut qi = qi.clone();
    let taken = qi.private_keys.take_plaintext_for_vault();
    crate::wallet_backend::IdentityKeyView::new(secret_store, *id).store_all(&taken)?;
    Ok(qi.to_bytes())
}

fn purge_identity_scope(kv: &DetKv, id: &[u8; 32]) -> std::result::Result<(), TaskError> {
    let scope = DetScope::Identity(id);
    // The identity blob is the recovery inventory for its vault keys. Delete
    // auxiliary records first and the blob last so a partial purge remains
    // discoverable and retryable.
    kv.delete(scope, TOP_UPS_KEY).map_err(top_up_err)?;
    delete_scheduled_votes_for_voter(kv, id)?;
    kv.delete(scope, IDENTITY_KEY).map_err(identity_err)
}

/// Read the Global scheduled-vote voter index. Returns an empty vector
/// when no voter has ever queued a scheduled vote.
fn load_scheduled_vote_voters(kv: &DetKv) -> std::result::Result<Vec<[u8; 32]>, TaskError> {
    Ok(kv
        .get::<Vec<[u8; 32]>>(DetScope::Global, SCHEDULED_VOTE_VOTERS_KEY)
        .map_err(scheduled_vote_err)?
        .unwrap_or_default())
}

/// Add `voter` to the Global scheduled-vote voter index if absent.
fn index_add_vote_voter(kv: &DetKv, voter: &[u8; 32]) -> std::result::Result<(), TaskError> {
    let mut voters = load_scheduled_vote_voters(kv)?;
    if voters.contains(voter) {
        return Ok(());
    }
    voters.push(*voter);
    kv.put(DetScope::Global, SCHEDULED_VOTE_VOTERS_KEY, &voters)
        .map_err(scheduled_vote_err)
}

/// List the scheduled-vote entry keys queued under `voter`'s Identity scope.
fn scheduled_vote_keys(
    kv: &DetKv,
    voter: &[u8; 32],
) -> std::result::Result<Vec<String>, TaskError> {
    kv.list(DetScope::Identity(voter), Some(SCHEDULED_VOTE_KEY_PREFIX))
        .map_err(scheduled_vote_err)
}

/// Drop `voter` from the Global scheduled-vote voter index. No-op when the
/// voter is not present, so repeated calls stay idempotent.
fn remove_vote_voter_from_index(
    kv: &DetKv,
    voter: &[u8; 32],
) -> std::result::Result<(), TaskError> {
    let mut voters = load_scheduled_vote_voters(kv)?;
    let before = voters.len();
    voters.retain(|v| v != voter);
    if voters.len() == before {
        return Ok(());
    }
    kv.put(DetScope::Global, SCHEDULED_VOTE_VOTERS_KEY, &voters)
        .map_err(scheduled_vote_err)
}

/// Prune `voter` from the Global scheduled-vote voter index when it no
/// longer has any scheduled votes left in its Identity scope. Keeps the
/// index from accumulating dangling voter entries.
fn prune_vote_voter_if_empty(kv: &DetKv, voter: &[u8; 32]) -> std::result::Result<(), TaskError> {
    if scheduled_vote_keys(kv, voter)?.is_empty() {
        remove_vote_voter_from_index(kv, voter)
    } else {
        Ok(())
    }
}

/// Delete every scheduled vote queued under `voter`'s Identity scope and
/// drop the voter from the index. Used by the identity-removal cleanup
/// path.
fn delete_scheduled_votes_for_voter(
    kv: &DetKv,
    voter: &[u8; 32],
) -> std::result::Result<(), TaskError> {
    let scope = DetScope::Identity(voter);
    for key in scheduled_vote_keys(kv, voter)? {
        kv.delete(scope, &key).map_err(scheduled_vote_err)?;
    }
    remove_vote_voter_from_index(kv, voter)
}

impl AppContext {
    /// Insert (or replace) a local qualified identity in the per-network
    /// wallet k/v store under [`DetScope::Identity`]. Mirrors pre-C7
    /// `INSERT OR REPLACE` semantics — wallet association is overwritten
    /// from the passed-in hint. Also registers the id in the Global
    /// enumeration index so the load-all paths can find it.
    ///
    /// The underlying k/v store offers no multi-key transaction, so the
    /// enumeration index is written *before* the blob. The ordering makes a
    /// mid-operation failure self-healing: a dangling index entry that points
    /// at a not-yet-written blob is skipped by every reader
    /// ([`Self::load_identities_filtered`] / [`Self::load_identity_order`]
    /// `continue` on a missing blob), and the next successful insert fills it
    /// in. The reverse order would instead hide a written identity — and its
    /// keys and balances — until an unrelated update happened to re-index it.
    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_and_identity_id_info: &Option<(WalletSeedHash, u32)>,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let (wallet_hash, wallet_index) = match wallet_and_identity_id_info {
            Some((seed, idx)) => (Some(*seed), Some(*idx)),
            None => {
                // Masternodes and evonodes are loaded by ProTxHash and have no
                // associated HD wallet by design, so a missing wallet is normal
                // for them — only a wallet-less User identity is worth flagging.
                if qualified_identity.identity_type == IdentityType::User {
                    tracing::warn!(
                        identity_id = %qualified_identity.identity.id(),
                        alias = ?qualified_identity.alias,
                        "saving identity without wallet; this needs investigating",
                    );
                } else {
                    tracing::debug!(
                        identity_id = %qualified_identity.identity.id(),
                        alias = ?qualified_identity.alias,
                        identity_type = ?qualified_identity.identity_type,
                        "saving masternode/evonode identity without wallet (expected)",
                    );
                }
                (None, None)
            }
        };
        let id = qualified_identity.identity.id().to_buffer();
        // Vault-first: move any plaintext keys into the vault before encoding, so
        // the at-rest blob carries only `InVault` placeholders. A vault-write
        // failure aborts the insert (nothing is persisted).
        let qi_bytes =
            encode_identity_blob_vault_first(&self.secret_store, &id, qualified_identity)?;
        let stored = StoredQualifiedIdentity {
            qi_bytes,
            status: qualified_identity.status.as_u8(),
            identity_type: qualified_identity.identity_type.as_tag().to_string(),
            wallet_hash,
            wallet_index,
        };
        index_add_identity(&kv, &id)?;
        kv.put(DetScope::Identity(&id), IDENTITY_KEY, &stored)
            .map_err(identity_err)
    }

    /// Update a local qualified identity in place. Wallet association
    /// (`wallet_hash` / `wallet_index`) is preserved from the existing
    /// record — pre-C7 `update_local_qualified_identity` had the same
    /// behaviour by virtue of omitting those columns from its `UPDATE`.
    pub fn update_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let id = qualified_identity.identity.id().to_buffer();
        let scope = DetScope::Identity(&id);
        let existing: Option<StoredQualifiedIdentity> =
            kv.get(scope, IDENTITY_KEY).map_err(identity_err)?;
        let (wallet_hash, wallet_index) = existing
            .as_ref()
            .map(|s| (s.wallet_hash, s.wallet_index))
            .unwrap_or((None, None));
        // Vault-first: move any plaintext keys into the vault before encoding, so
        // an update never lands `Clear` / `AlwaysClear` key bytes on disk.
        let qi_bytes =
            encode_identity_blob_vault_first(&self.secret_store, &id, qualified_identity)?;
        let stored = StoredQualifiedIdentity {
            qi_bytes,
            status: qualified_identity.status.as_u8(),
            identity_type: qualified_identity.identity_type.as_tag().to_string(),
            wallet_hash,
            wallet_index,
        };
        kv.put(scope, IDENTITY_KEY, &stored).map_err(identity_err)?;
        // Keep the enumeration index consistent even if a caller updates
        // an identity the index never learned about.
        index_add_identity(&kv, &id)
    }

    /// Update only the user-facing alias on a stored identity. Returns
    /// `Ok(())` when the identity is unknown — alias is metadata, not a
    /// load-bearing identifier.
    pub fn set_identity_alias(
        &self,
        identifier: &Identifier,
        new_alias: Option<&str>,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let id = identifier.to_buffer();
        let scope = DetScope::Identity(&id);
        let Some(mut stored) = kv
            .get::<StoredQualifiedIdentity>(scope, IDENTITY_KEY)
            .map_err(identity_err)?
        else {
            return Ok(());
        };
        let mut qi = decode_stored_identity(&stored.qi_bytes, self.network)?;
        qi.alias = new_alias.map(str::to_string);
        // Re-encode vault-first so an alias edit on a not-yet-migrated blob does
        // not rewrite resident plaintext keys back to disk.
        stored.qi_bytes = encode_identity_blob_vault_first(&self.secret_store, &id, &qi)?;
        kv.put(scope, IDENTITY_KEY, &stored).map_err(identity_err)
    }

    /// Read the user-facing alias for a stored identity, if any.
    pub fn get_identity_alias(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<Option<String>, TaskError> {
        let kv = self.det_kv()?;
        let id = identifier.to_buffer();
        let Some(stored) = kv
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id), IDENTITY_KEY)
            .map_err(identity_err)?
        else {
            return Ok(None);
        };
        let qi = decode_stored_identity(&stored.qi_bytes, self.network)?;
        Ok(qi.alias)
    }

    /// Fetches all local qualified identities from the k/v store, then
    /// hydrates each identity's top-up history.
    ///
    /// Stops on the first corrupted identity blob and returns an error.
    /// This is intentional — identities hold private keys and balance data,
    /// so skipping a corrupted entry could cause loss of funds.
    pub fn load_local_qualified_identities(
        &self,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let mut identities = self.load_identities_filtered(&wallets, |_| true)?;
        for identity in &mut identities {
            self.hydrate_top_ups(identity);
        }
        Ok(identities)
    }

    /// Load the DET-known, wallet-owned qualified identities for one wallet —
    /// every sidecar entry that carries a `wallet_index` and matches
    /// `seed_hash`. Drives the cold-boot/unlock reconcile that registers them
    /// into the upstream `IdentityManager`. Top-up history is intentionally not
    /// hydrated: the reconcile needs only the identity and its wallet index.
    pub(crate) fn load_local_qualified_identities_for_wallet(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let target = Some(*seed_hash);
        self.load_identities_filtered(&wallets, |s| {
            s.wallet_index.is_some() && s.wallet_hash == target
        })
    }

    /// The masternode/evonode identities for the active network — the
    /// Masternodes-page card list and the page-scoped masternode pill source.
    /// The complement of [`Self::load_local_user_identities`] over the FR-6 type
    /// boundary. Filters the hydrated full load, so each card's top-up history
    /// is available (unlike the pre-decode [`Self::load_local_voting_identities`],
    /// which is un-hydrated and named for the DPNS voting flows).
    pub fn load_local_masternode_identities(
        &self,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError> {
        Ok(self
            .load_local_qualified_identities()?
            .into_iter()
            .filter(|qi| {
                matches!(
                    qi.identity_type,
                    IdentityType::Masternode | IdentityType::Evonode
                )
            })
            .collect())
    }

    /// Read one stored qualified identity by id, hydrated like the list loads
    /// (status, wallet index, network, wallets, secret access). `None` when no
    /// identity with `id` is stored. Backs the load-path existence check
    /// (duplicate-ProTxHash rejection) and the in-place voter-key merge.
    pub fn get_local_qualified_identity(
        &self,
        id: &Identifier,
    ) -> std::result::Result<Option<QualifiedIdentity>, TaskError> {
        let kv = self.det_kv()?;
        let id_buf = id.to_buffer();
        let Some(stored) = kv
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id_buf), IDENTITY_KEY)
            .map_err(|source| TaskError::IdentityStorage { source })?
        else {
            return Ok(None);
        };
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let mut qi = decode_stored_identity(&stored.qi_bytes, self.network)?;
        qi.status = IdentityStatus::from_u8(stored.status);
        qi.wallet_index = stored.wallet_index;
        qi.network = self.network;
        qi.associated_wallets = wallets.clone();
        qi.secret_access = self.wallet_backend().ok().map(|b| b.secret_access());
        qi.top_ups = BTreeMap::new();
        self.migrate_identity_keys_to_vault(&kv, &id_buf, &mut qi);
        Ok(Some(qi))
    }

    /// Returns whether an identity blob is stored under `id` without decoding it.
    pub(crate) fn has_local_qualified_identity(
        &self,
        id: &Identifier,
    ) -> std::result::Result<bool, TaskError> {
        let kv = self.det_kv()?;
        let id_buf = id.to_buffer();
        Ok(kv
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id_buf), IDENTITY_KEY)
            .map_err(identity_err)?
            .is_some())
    }

    /// Internal: read every stored identity via the Global enumeration
    /// index, decode it, rehydrate the metadata kept outside the bincode
    /// blob, and apply `keep` as a pre-decode filter on the wrapper.
    /// Sorted by identity ID for deterministic output — mirrors the
    /// pre-C7 `ORDER BY id`.
    ///
    /// Under [`DetScope::Identity`] there is no cross-identity listing, so
    /// the index ([`IDENTITY_INDEX_KEY`]) is the roster: read the ids,
    /// then fetch each blob from its own identity scope.
    fn load_identities_filtered<F>(
        &self,
        wallets: &BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>,
        keep: F,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError>
    where
        F: Fn(&StoredQualifiedIdentity) -> bool,
    {
        let kv = self.det_kv()?;
        let mut ids = load_identity_index(&kv)?;
        ids.sort_unstable();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(stored) = kv
                .get::<StoredQualifiedIdentity>(DetScope::Identity(&id), IDENTITY_KEY)
                .map_err(identity_err)?
            else {
                continue;
            };
            if !keep(&stored) {
                continue;
            }
            out.push(self.hydrate_stored_identity(&kv, &id, &stored, wallets)?);
        }
        // Seed the JIT chokepoint's identity prompt-copy index (alias + hint)
        // so the sign-time prompt for an opted-in (Tier-2) identity shows the
        // right label and hint. Display-only and best-effort — the vault scheme,
        // not this index, decides whether a prompt fires.
        if let Ok(backend) = self.wallet_backend() {
            backend.seed_identity_prompt_index(&out);
        }
        Ok(out)
    }

    /// Decode a stored blob and rehydrate the runtime-only fields the encoder
    /// skips — status, wallet index, network, wallet map, secret access — then
    /// run the crash-safe vault migration. Shared by the bulk-load and
    /// single-get paths so both reconstruct an identity identically. Top-up
    /// history is left empty; callers hydrate it separately when needed.
    fn hydrate_stored_identity(
        &self,
        kv: &DetKv,
        id: &[u8; 32],
        stored: &StoredQualifiedIdentity,
        wallets: &BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>,
    ) -> std::result::Result<QualifiedIdentity, TaskError> {
        let mut qi = decode_stored_identity(&stored.qi_bytes, self.network)?;
        qi.status = IdentityStatus::from_u8(stored.status);
        qi.wallet_index = stored.wallet_index;
        qi.network = self.network;
        qi.associated_wallets = wallets.clone();
        qi.secret_access = self.wallet_backend().ok().map(|b| b.secret_access());
        qi.top_ups = BTreeMap::new();
        self.migrate_identity_keys_to_vault(kv, id, &mut qi);
        Ok(qi)
    }

    /// Populate `identity.top_ups` from the per-network wallet k/v
    /// store. A missing or unreadable entry is logged and treated as an
    /// empty map; pre-C5 SQLite data is intentionally not migrated and
    /// surfaces as empty under the "empty start" policy.
    fn hydrate_top_ups(&self, identity: &mut QualifiedIdentity) {
        let Ok(kv) = self.det_kv() else {
            return;
        };
        let id = identity.identity.id().to_buffer();
        match kv.get::<std::collections::BTreeMap<u32, u64>>(DetScope::Identity(&id), TOP_UPS_KEY) {
            Ok(Some(map)) => identity.top_ups = map,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    identity = %identity.identity.id(),
                    error = ?e,
                    "Failed to load top-up history from wallet k/v"
                );
            }
        }
    }

    /// Persist the running top-up history for an identity into the
    /// per-network wallet k/v store. Merges — see [`save_top_ups_in`].
    pub fn save_top_ups(
        &self,
        identity_id: &Identifier,
        top_ups: &std::collections::BTreeMap<u32, u64>,
    ) -> std::result::Result<(), TaskError> {
        save_top_ups_in(&self.det_kv()?, &identity_id.to_buffer(), top_ups)
    }

    pub fn get_identity_by_id(
        &self,
        identity_id: &Identifier,
    ) -> std::result::Result<Option<QualifiedIdentity>, TaskError> {
        let kv = self.det_kv()?;
        let id = identity_id.to_buffer();
        let Some(stored) = kv
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id), IDENTITY_KEY)
            .map_err(identity_err)?
        else {
            return Ok(None);
        };
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        // Shared with the bulk-load path: rehydrates the skipped fields and runs
        // the crash-safe vault migration so single-get consumers (the identity
        // key password tasks and others) see vault-backed schemes rather than
        // re-persisting resident plaintext.
        let mut qi = self.hydrate_stored_identity(&kv, &id, &stored, &wallets)?;
        self.hydrate_top_ups(&mut qi);
        Ok(Some(qi))
    }

    /// The [`SecretScope`](crate::wallet_backend::secret_prompt::SecretScope) of
    /// an existing password-protected key of `qi`, used to verify the identity's
    /// password when sealing a newly-added key, or `None` when the
    /// identity is not password-protected (the default keyless add applies).
    pub(crate) fn protected_identity_verify_scope(
        &self,
        qi: &QualifiedIdentity,
    ) -> std::result::Result<Option<crate::wallet_backend::secret_prompt::SecretScope>, TaskError>
    {
        let backend = self.wallet_backend()?;
        let id = qi.identity.id().to_buffer();
        Ok(find_protected_identity_key_scope(
            backend.secret_store(),
            &id,
            qi,
        ))
    }

    /// Fetches every locally-stored identity whose `identity_type` is
    /// not `User` — used by the DPNS contest voting flows.
    pub fn load_local_voting_identities(
        &self,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        self.load_identities_filtered(&wallets, |s| {
            !matches!(
                IdentityType::from_tag(&s.identity_type),
                Some(IdentityType::User)
            )
        })
    }

    /// Fetches every locally-stored identity whose `identity_type` is
    /// `User`. Top-up history is *not* loaded here — matching the
    /// pre-C7 query shape that the consumer screens depend on.
    pub fn load_local_user_identities(
        &self,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        self.load_identities_filtered(&wallets, |s| {
            matches!(
                IdentityType::from_tag(&s.identity_type),
                Some(IdentityType::User)
            )
        })
    }

    /// Return the raw ids of every locally-stored identity, read from the
    /// Global enumeration index. Cheap — no blob decode. Used by callers
    /// (e.g. the network-clear sweep) that need to fan an operation out
    /// over each identity's [`DetScope::Identity`] scope.
    pub fn local_identity_ids(&self) -> std::result::Result<Vec<Identifier>, TaskError> {
        let kv = self.det_kv()?;
        Ok(load_identity_index(&kv)?
            .into_iter()
            .map(Identifier::from)
            .collect())
    }

    /// Remove a locally-stored identity and its owner-attributable local state.
    /// Returns `Ok(())` even when the identity is unknown —
    /// mirrors the pre-C7 `DELETE` which silently no-ops on missing rows.
    ///
    /// Cleanup verdict: explicit. DET never deletes the upstream
    /// `identities` row (that table is owned by the upstream sync layer;
    /// DET stores the qualified-identity blob in the `meta_identity` k/v
    /// scope only), so the upstream `cascade_meta_identity_on_identity_delete`
    /// trigger never fires for this path. This method therefore drains the
    /// identity blob, keys, top-up history, scheduled votes, DashPay overlays,
    /// entity timestamp, reverse address mappings, metadata, and Global identity
    /// index entry itself.
    /// Global payment timestamp entries keyed only by transaction id cannot be
    /// attributed to one owner and remain until full-wallet teardown. Upstream
    /// `platform-wallet`'s `IdentitySyncManager`, exposed through
    /// `identity_sync()`, also has no per-identity forget operation, so token sync
    /// continues tracking a removed identity until upstream adds one.
    pub fn delete_local_qualified_identity(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<(), TaskError> {
        self.delete_local_qualified_identity_inner(identifier, false)
    }

    /// Delete an identity and remember a wallet-derived user's unload choice.
    pub(crate) fn unload_local_qualified_identity(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<(), TaskError> {
        self.delete_local_qualified_identity_inner(identifier, true)
    }

    /// Whether automatic discovery must leave this identity unloaded.
    pub(crate) fn is_identity_forgotten(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<bool, TaskError> {
        self.db
            .is_identity_forgotten(self.network, identifier)
            .map_err(|source| TaskError::ForgottenIdentityStorage { source })
    }

    /// Clear the discovery block after a user-requested load succeeds.
    pub(crate) fn clear_forgotten_identity_after_explicit_load(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<(), TaskError> {
        self.db
            .clear_forgotten_identity(self.network, identifier)
            .map_err(|source| TaskError::ForgottenIdentityStorage { source })
    }

    /// Finish cleanup for a forgotten, unindexed identity whose blob remains.
    ///
    /// Returns `Ok(true)` after cleanup, `Ok(false)` for a non-ghost state, and
    /// preserves the blob and marker when vault-key clearing or purging fails.
    pub(crate) fn retry_stuck_unload_cleanup(
        &self,
        id: &[u8; 32],
    ) -> std::result::Result<bool, TaskError> {
        self.retry_stuck_unload_cleanup_inner(id, true)
    }

    /// Load-path form of [`Self::retry_stuck_unload_cleanup`]. The forgotten
    /// marker remains in place across the later network fetch and persistence;
    /// [`Self::finish_identity_load_after_persist`] clears it only after the
    /// replacement identity is durably stored.
    pub(crate) fn prepare_stuck_unload_cleanup_for_reload(
        &self,
        id: &[u8; 32],
    ) -> std::result::Result<bool, TaskError> {
        let identifier = Identifier::from(*id);
        if !self.is_identity_forgotten(&identifier)?
            || self.local_identity_ids()?.contains(&identifier)
        {
            return Ok(false);
        }

        // This also handles a marker-only unload residue whose vault/blob tail
        // succeeded after an earlier sidecar failure. Cleanup is idempotent, and
        // the marker deliberately remains until the fresh load is persisted.
        self.cleanup_identity_after_index_removal(&identifier)?;
        Ok(true)
    }

    fn retry_stuck_unload_cleanup_inner(
        &self,
        id: &[u8; 32],
        clear_forgotten_marker: bool,
    ) -> std::result::Result<bool, TaskError> {
        let identifier = Identifier::from(*id);
        if !self.is_identity_forgotten(&identifier)?
            || !self.has_local_qualified_identity(&identifier)?
            || self.local_identity_ids()?.contains(&identifier)
        {
            return Ok(false);
        }

        self.cleanup_identity_after_index_removal(&identifier)?;
        if clear_forgotten_marker {
            self.db
                .clear_forgotten_identity(self.network, &identifier)
                .map_err(|source| TaskError::ForgottenIdentityStorage { source })?;
        }
        Ok(true)
    }

    /// Remove identity-scoped residue for a forgotten identity whose recovery
    /// blob is already gone. Vault cleanup already completed before that blob
    /// could be purged; this finishes best-effort sidecar and metadata cleanup.
    pub(crate) fn purge_forgotten_identity_residue(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<(), TaskError> {
        self.cleanup_identity_after_index_removal(identifier)
    }

    /// Idempotent cleanup tail shared by normal deletion, ghost recovery, and
    /// marker-only full-wipe recovery. The blob is retained whenever its vault
    /// inventory could not be cleared.
    fn cleanup_identity_after_index_removal(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let backend = self.wallet_backend()?;
        let id = identifier.to_buffer();
        let mut cleanup_error = None;
        keep_first_unload_cleanup_error(
            &mut cleanup_error,
            *identifier,
            backend.dashpay_clear_owner_overlays(identifier),
        );
        keep_first_unload_cleanup_error(
            &mut cleanup_error,
            *identifier,
            backend.dashpay_clear_identity_timestamps(identifier),
        );
        keep_first_unload_cleanup_error(
            &mut cleanup_error,
            *identifier,
            backend.dashpay_clear_identity_addr_map(identifier),
        );
        keep_first_unload_cleanup_error(
            &mut cleanup_error,
            *identifier,
            backend.identity_meta().delete(self.network, &id),
        );
        match self.clear_identity_vault_keys(&kv, &id) {
            Ok(()) => keep_first_unload_cleanup_error(
                &mut cleanup_error,
                *identifier,
                purge_identity_scope(&kv, &id),
            ),
            Err(error) => {
                keep_first_unload_cleanup_error(&mut cleanup_error, *identifier, Err(error))
            }
        }
        cleanup_error.map_or(Ok(()), Err)
    }

    fn delete_local_qualified_identity_inner(
        &self,
        identifier: &Identifier,
        remember_unload: bool,
    ) -> std::result::Result<(), TaskError> {
        let load_guard = self.begin_identity_cleanup_claim(identifier)?;
        self.delete_local_qualified_identity_with_claim(identifier, remember_unload)?;
        load_guard.loaded();
        Ok(())
    }

    /// Full-wipe deletion that returns its exclusive identity claim so the
    /// caller can retain it through marker retirement and wipe completion.
    pub(crate) fn delete_local_qualified_identity_retaining_claim(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<crate::context::identity_load_registry::IdentityLoadGuard, TaskError>
    {
        let load_guard = self.begin_identity_cleanup_claim(identifier)?;
        self.delete_local_qualified_identity_with_claim(identifier, false)?;
        Ok(load_guard)
    }

    fn begin_identity_cleanup_claim(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<crate::context::identity_load_registry::IdentityLoadGuard, TaskError>
    {
        // The load registry provides the existing per-identity exclusive claim.
        self.begin_identity_load(*identifier, None)
            .map_err(|error| match error {
                TaskError::IdentityLoadInProgress { identity_id } => {
                    TaskError::IdentityBusyWithLoad { identity_id }
                }
                other => other,
            })
    }

    fn delete_local_qualified_identity_with_claim(
        &self,
        identifier: &Identifier,
        remember_unload: bool,
    ) -> std::result::Result<(), TaskError> {
        let _migration_guard = self
            .migration_run
            .try_lock()
            .map_err(|_| TaskError::WalletStorageNotReady)?;
        if self.migration_status().state().is_in_progress() {
            return Err(TaskError::WalletStorageNotReady);
        }
        let kv = self.det_kv()?;
        let id = identifier.to_buffer();
        crate::backend_task::migration::finish_unwire::record_identity_deletion(self, id).map_err(
            |source| TaskError::IdentityDeletionMigrationRecord {
                source: Arc::new(source),
            },
        )?;
        if remember_unload {
            self.db
                .record_forgotten_identity(self.network, identifier)
                .map_err(|source| TaskError::ForgottenIdentityStorage { source })?;
        }
        // Drop the identity from the index BEFORE the irreversible vault-key
        // clear: a fault in either of the next two steps must never leave a
        // "zombie" identity that is still visible but already missing its
        // keys. `clear_identity_vault_keys` must still run before
        // `purge_identity_scope`, since it reads the identity blob that
        // `purge_identity_scope` deletes.
        if let Err(error) = index_remove_identity(&kv, &id) {
            if remember_unload
                && let Err(rollback_error) =
                    self.db.clear_forgotten_identity(self.network, identifier)
            {
                tracing::warn!(
                    identity_id = %identifier,
                    original_error = ?error,
                    rollback_error = ?rollback_error,
                    "Identity unload marker rollback failed after the removal commit failed"
                );
            }
            return Err(error);
        }
        self.cleanup_identity_after_index_removal(identifier)?;
        Ok(())
    }

    /// EAGER identity-key migration (dialog-free): move any plaintext
    /// `Clear`/`AlwaysClear` identity keys into the vault as raw bytes and
    /// rewrite the blob with `InVault` placeholders so the keys are never
    /// resident.
    ///
    /// Crash-safe ordering: vault `store_all` FIRST, then blob rewrite. If the
    /// vault write fails the blob is left untouched (the in-memory `qi` is
    /// restored to its resident plaintext for this session) and the next load
    /// retries — keys are never lost. Idempotent: a blob already all-`InVault`
    /// has nothing to take and is skipped. Best-effort: a blob-rewrite failure
    /// is logged; the next load re-detects the plaintext and retries.
    fn migrate_identity_keys_to_vault(
        &self,
        kv: &DetKv,
        id: &[u8; 32],
        qi: &mut QualifiedIdentity,
    ) {
        let _ = migrate_keystore_to_vault(&self.secret_store, id, qi, |migrated| {
            self.persist_identity_blob(kv, id, migrated)
        });
    }

    /// Re-persist `qi`'s blob in place, preserving the stored wallet
    /// association and status. Used by the eager identity-key migration.
    fn persist_identity_blob(
        &self,
        kv: &DetKv,
        id: &[u8; 32],
        qi: &QualifiedIdentity,
    ) -> std::result::Result<(), TaskError> {
        let scope = DetScope::Identity(id);
        let existing: Option<StoredQualifiedIdentity> =
            kv.get(scope, IDENTITY_KEY).map_err(identity_err)?;
        let (wallet_hash, wallet_index, status) = existing
            .as_ref()
            .map(|s| (s.wallet_hash, s.wallet_index, s.status))
            .unwrap_or((None, None, qi.status.as_u8()));
        let stored = StoredQualifiedIdentity {
            qi_bytes: qi.to_bytes(),
            status,
            identity_type: qi.identity_type.as_tag().to_string(),
            wallet_hash,
            wallet_index,
        };
        kv.put(scope, IDENTITY_KEY, &stored).map_err(identity_err)
    }

    /// Delete every identity-key raw secret for `id` from the vault.
    /// Idempotent when the identity or an individual vault label is absent.
    fn clear_identity_vault_keys(
        &self,
        kv: &DetKv,
        id: &[u8; 32],
    ) -> std::result::Result<(), TaskError> {
        let Some(stored) = kv
            .get::<StoredQualifiedIdentity>(DetScope::Identity(id), IDENTITY_KEY)
            .map_err(identity_err)?
        else {
            return Ok(());
        };
        let qi = decode_stored_identity(&stored.qi_bytes, self.network)?;
        let view = crate::wallet_backend::IdentityKeyView::new(&self.secret_store, *id);
        view.delete_all(qi.private_keys.keys_set())
    }

    /// Devnet-only sweep: drop every locally-stored identity for the
    /// current network. Matches the pre-C7
    /// `delete_all_local_qualified_identities_in_devnet` guard — no-op on
    /// non-devnet networks.
    pub fn delete_all_local_qualified_identities_in_devnet(
        &self,
    ) -> std::result::Result<(), TaskError> {
        if self.network != Network::Devnet {
            return Ok(());
        }
        let kv = self.det_kv()?;
        let ids = load_identity_index(&kv)?;
        for id in &ids {
            self.clear_identity_vault_keys(&kv, id)?;
            purge_identity_scope(&kv, id)?;
        }
        kv.delete(DetScope::Global, IDENTITY_INDEX_KEY)
            .map_err(identity_err)
    }

    /// Persist the user-chosen identity ordering at `det:identity_order:v1`.
    /// Overwrites the previous list — matches pre-C7 semantics.
    pub fn save_identity_order(
        &self,
        all_ids: Vec<Identifier>,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let payload: Vec<[u8; 32]> = all_ids.iter().map(Identifier::to_buffer).collect();
        kv.put(DetScope::Global, IDENTITY_ORDER_KEY, &payload)
            .map_err(identity_err)
    }

    /// Load the user-chosen identity ordering, dropping any references
    /// that no longer point at a stored identity.
    pub fn load_identity_order(&self) -> std::result::Result<Vec<Identifier>, TaskError> {
        let kv = self.det_kv()?;
        let Some(payload): Option<Vec<[u8; 32]>> = kv
            .get(DetScope::Global, IDENTITY_ORDER_KEY)
            .map_err(identity_err)?
        else {
            return Ok(Vec::new());
        };
        let mut kept = Vec::with_capacity(payload.len());
        let mut needs_rewrite = false;
        for buf in payload {
            let exists = kv
                .get::<StoredQualifiedIdentity>(DetScope::Identity(&buf), IDENTITY_KEY)
                .map_err(identity_err)?
                .is_some();
            if exists {
                kept.push(Identifier::from(buf));
            } else {
                needs_rewrite = true;
            }
        }
        if needs_rewrite {
            let payload: Vec<[u8; 32]> = kept.iter().map(Identifier::to_buffer).collect();
            kv.put(DetScope::Global, IDENTITY_ORDER_KEY, &payload)
                .map_err(identity_err)?;
        }
        Ok(kept)
    }

    /// Persist a batch of scheduled votes in the per-network wallet k/v
    /// store. Each vote is scoped to [`DetScope::Identity`] of its voter;
    /// existing entries with the same `(voter, contested_name)` are
    /// overwritten — matching the pre-C5 `INSERT OR REPLACE` semantics.
    /// Voters are tracked in a Global index so the network-wide read /
    /// clear paths can find them.
    pub fn insert_scheduled_votes(
        &self,
        scheduled_votes: &[ScheduledDPNSVote],
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        for vote in scheduled_votes {
            let voter = vote.voter_id.to_buffer();
            let stored = StoredScheduledVote::from(vote);
            kv.put(
                DetScope::Identity(&voter),
                &scheduled_vote_key(&vote.contested_name),
                &stored,
            )
            .map_err(scheduled_vote_err)?;
            index_add_vote_voter(&kv, &voter)?;
        }
        Ok(())
    }

    /// Count the scheduled votes queued for one identity on this network.
    pub fn scheduled_vote_count_for_identity(
        &self,
        voter: &Identifier,
    ) -> std::result::Result<usize, TaskError> {
        let kv = self.det_kv()?;
        Ok(scheduled_vote_keys(&kv, &voter.to_buffer())?.len())
    }

    /// Fetch every scheduled vote queued for this network from the
    /// wallet k/v store, across all voters in the Global voter index.
    pub fn get_scheduled_votes(&self) -> std::result::Result<Vec<ScheduledDPNSVote>, TaskError> {
        let kv = self.det_kv()?;
        let voters = load_scheduled_vote_voters(&kv)?;
        let mut out = Vec::new();
        for voter in voters {
            let scope = DetScope::Identity(&voter);
            for key in scheduled_vote_keys(&kv, &voter)? {
                match kv.get::<StoredScheduledVote>(scope, &key) {
                    Ok(Some(stored)) => out.push(stored.into()),
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(
                            key = %key,
                            error = ?e,
                            "Skipping unreadable scheduled vote entry"
                        );
                    }
                }
            }
        }
        Ok(out)
    }

    /// Drop every scheduled vote queued for this network.
    pub fn clear_all_scheduled_votes(&self) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let voters = load_scheduled_vote_voters(&kv)?;
        for voter in &voters {
            let scope = DetScope::Identity(voter);
            for key in scheduled_vote_keys(&kv, voter)? {
                kv.delete(scope, &key).map_err(scheduled_vote_err)?;
            }
        }
        kv.delete(DetScope::Global, SCHEDULED_VOTE_VOTERS_KEY)
            .map_err(scheduled_vote_err)
    }

    /// Drop every scheduled vote that has already been cast successfully.
    pub fn clear_executed_scheduled_votes(&self) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let voters = load_scheduled_vote_voters(&kv)?;
        for voter in &voters {
            let scope = DetScope::Identity(voter);
            for key in scheduled_vote_keys(&kv, voter)? {
                match kv.get::<StoredScheduledVote>(scope, &key) {
                    Ok(Some(stored)) if stored.executed_successfully => {
                        kv.delete(scope, &key).map_err(scheduled_vote_err)?;
                    }
                    _ => {}
                }
            }
            prune_vote_voter_if_empty(&kv, voter)?;
        }
        Ok(())
    }

    /// Drop a single scheduled vote keyed by `(voter_id, contested_name)`.
    pub fn delete_scheduled_vote(
        &self,
        identity_id: &[u8],
        contested_name: &str,
    ) -> std::result::Result<(), TaskError> {
        let voter = voter_buffer(identity_id)?;
        let kv = self.det_kv()?;
        kv.delete(
            DetScope::Identity(&voter),
            &scheduled_vote_key(contested_name),
        )
        .map_err(scheduled_vote_err)?;
        prune_vote_voter_if_empty(&kv, &voter)
    }

    /// Mark a single scheduled vote as executed so future cast loops skip it.
    pub fn mark_vote_executed(
        &self,
        identity_id: &[u8],
        contested_name: String,
    ) -> std::result::Result<(), TaskError> {
        let voter = voter_buffer(identity_id)?;
        let key = scheduled_vote_key(&contested_name);
        let scope = DetScope::Identity(&voter);
        let kv = self.det_kv()?;
        let Some(mut stored): Option<StoredScheduledVote> =
            kv.get(scope, &key).map_err(scheduled_vote_err)?
        else {
            return Ok(());
        };
        stored.executed_successfully = true;
        kv.put(scope, &key, &stored).map_err(scheduled_vote_err)
    }

    /// Fetches the local identities from the k/v store and maps them to their DPNS names.
    pub fn local_dpns_names(
        &self,
    ) -> std::result::Result<Vec<(Identifier, DPNSNameInfo)>, TaskError> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let qualified_identities = self.load_identities_filtered(&wallets, |_| true)?;

        // Map each identity's DPNS names to (Identifier, DPNSNameInfo) tuples
        let dpns_names = qualified_identities
            .iter()
            .flat_map(|qualified_identity| {
                qualified_identity.dpns_names.iter().map(|dpns_name_info| {
                    (
                        qualified_identity.identity.id(),
                        DPNSNameInfo {
                            name: dpns_name_info.name.clone(),
                            acquired_at: dpns_name_info.acquired_at,
                        },
                    )
                })
            })
            .collect::<Vec<(Identifier, DPNSNameInfo)>>();

        Ok(dpns_names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::BackendTaskSuccessResult;
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use DetKv;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    fn empty_kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    fn id(b: u8) -> [u8; 32] {
        [b; 32]
    }

    /// Minimal stored-identity blob carrying a synthetic `qi_bytes`
    /// payload and the chosen type label. Avoids constructing a full
    /// `QualifiedIdentity` (which needs an SDK identity) for storage-layer
    /// tests.
    fn stored(identity_type: &str) -> StoredQualifiedIdentity {
        StoredQualifiedIdentity {
            qi_bytes: vec![0xAB; 16],
            status: 0,
            identity_type: identity_type.to_string(),
            wallet_hash: None,
            wallet_index: None,
        }
    }

    fn put_identity(kv: &DetKv, id: &[u8; 32], identity_type: &str) {
        kv.put(DetScope::Identity(id), IDENTITY_KEY, &stored(identity_type))
            .unwrap();
        index_add_identity(kv, id).unwrap();
    }

    #[derive(Clone, Default)]
    struct SharedLog(Arc<Mutex<Vec<u8>>>);

    struct SharedLogWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedLogWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("lock captured log")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLog {
        type Writer = SharedLogWriter;

        fn make_writer(&'a self) -> Self::Writer {
            SharedLogWriter(Arc::clone(&self.0))
        }
    }

    #[test]
    fn keep_first_unload_cleanup_error_logs_every_later_failure() {
        let identity_id = Identifier::from([0x01; 32]);
        let mut cleanup_error = None;
        let captured = SharedLog::default();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(captured.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            keep_first_unload_cleanup_error(
                &mut cleanup_error,
                identity_id,
                Err(TaskError::IdentityNotFound),
            );
            keep_first_unload_cleanup_error(
                &mut cleanup_error,
                identity_id,
                Err(TaskError::InternalSendError),
            );
            keep_first_unload_cleanup_error(
                &mut cleanup_error,
                identity_id,
                Err(TaskError::InternalSendError),
            );
        });

        match cleanup_error.expect("a cleanup error must be recorded") {
            TaskError::IdentityUnloadCleanupFailed {
                identity_id: id,
                source,
            } => {
                assert_eq!(id, identity_id);
                assert!(
                    matches!(*source, TaskError::IdentityNotFound),
                    "the first failure remains the primary source, got {source:?}"
                );
            }
            other => panic!("expected IdentityUnloadCleanupFailed, got {other:?}"),
        }
        let output =
            String::from_utf8(captured.0.lock().expect("lock captured log").clone()).unwrap();
        assert_eq!(
            output
                .matches("Additional identity unload cleanup step failed")
                .count(),
            2,
            "the second and third failures must each be visible in logs: {output}"
        );
    }

    // ---------------------------------------------------------------
    // SEC: the redacting Debug must never print the private-key blob.
    // ---------------------------------------------------------------

    #[test]
    fn stored_identity_debug_redacts_qi_bytes() {
        let s = StoredQualifiedIdentity {
            qi_bytes: vec![0x42; 48],
            status: 1,
            identity_type: "User".to_string(),
            wallet_hash: Some([9u8; 32]),
            wallet_index: Some(3),
        };
        let dbg = format!("{s:?}");
        assert!(dbg.contains("[redacted]"), "expected redaction: {dbg}");
        assert!(dbg.contains("qi_bytes_len"), "expected length field: {dbg}");
        assert!(!dbg.contains("66, 66, 66"), "leaked qi bytes: {dbg}");
    }

    // ---------------------------------------------------------------
    // Identity blob: Identity-scoped round-trip + index enumeration.
    // ---------------------------------------------------------------

    #[test]
    fn identity_blob_round_trips_in_identity_scope() {
        let kv = empty_kv();
        let a = id(1);
        put_identity(&kv, &a, "User");
        let got: StoredQualifiedIdentity = kv
            .get(DetScope::Identity(&a), IDENTITY_KEY)
            .unwrap()
            .unwrap();
        assert_eq!(got.identity_type, "User");
        // The fixed slot is invisible from the Global scope.
        assert!(
            kv.get::<StoredQualifiedIdentity>(DetScope::Global, IDENTITY_KEY)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn distinct_identities_do_not_alias_in_identity_scope() {
        let kv = empty_kv();
        let a = id(1);
        let b = id(2);
        put_identity(&kv, &a, "User");
        put_identity(&kv, &b, "Masternode");
        let got_a: StoredQualifiedIdentity = kv
            .get(DetScope::Identity(&a), IDENTITY_KEY)
            .unwrap()
            .unwrap();
        let got_b: StoredQualifiedIdentity = kv
            .get(DetScope::Identity(&b), IDENTITY_KEY)
            .unwrap()
            .unwrap();
        assert_eq!(got_a.identity_type, "User");
        assert_eq!(got_b.identity_type, "Masternode");
    }

    #[test]
    fn identity_index_enumerates_all_identities() {
        let kv = empty_kv();
        put_identity(&kv, &id(1), "User");
        put_identity(&kv, &id(2), "User");
        put_identity(&kv, &id(3), "Masternode");
        let mut index = load_identity_index(&kv).unwrap();
        index.sort_unstable();
        assert_eq!(index, vec![id(1), id(2), id(3)]);
    }

    #[test]
    fn identity_index_add_is_idempotent() {
        let kv = empty_kv();
        index_add_identity(&kv, &id(1)).unwrap();
        index_add_identity(&kv, &id(1)).unwrap();
        assert_eq!(load_identity_index(&kv).unwrap(), vec![id(1)]);
    }

    #[test]
    fn identity_index_remove_drops_only_the_target() {
        let kv = empty_kv();
        index_add_identity(&kv, &id(1)).unwrap();
        index_add_identity(&kv, &id(2)).unwrap();
        index_remove_identity(&kv, &id(1)).unwrap();
        assert_eq!(load_identity_index(&kv).unwrap(), vec![id(2)]);
        // Removing an absent id is a no-op.
        index_remove_identity(&kv, &id(9)).unwrap();
        assert_eq!(load_identity_index(&kv).unwrap(), vec![id(2)]);
    }

    // ---------------------------------------------------------------
    // Identity-type tag: writer and filter share one stable mapping.
    // ---------------------------------------------------------------

    #[test]
    fn identity_type_tag_round_trips_writer_to_filter() {
        // The writer stores `as_tag()`; the load filters classify via
        // `from_tag()`. They must agree for every variant, independent of the
        // derived `Debug` representation.
        for ty in [
            IdentityType::User,
            IdentityType::Masternode,
            IdentityType::Evonode,
        ] {
            assert_eq!(IdentityType::from_tag(ty.as_tag()), Some(ty));
        }
        // Tags are fixed string constants, not the `Debug` output.
        assert_eq!(IdentityType::User.as_tag(), "User");
        assert_eq!(IdentityType::Masternode.as_tag(), "Masternode");
        assert_eq!(IdentityType::Evonode.as_tag(), "Evonode");
        // The user / non-user split the load filters depend on. An unknown tag
        // decodes to `None`, which the filters treat as non-user (voting) —
        // preserving the pre-tag string-compare behaviour.
        assert_eq!(IdentityType::from_tag("Bogus"), None);
    }

    // ---------------------------------------------------------------
    // Top-ups: Identity-scoped round-trip.
    // ---------------------------------------------------------------

    #[test]
    fn top_ups_round_trip_in_identity_scope() {
        let kv = empty_kv();
        let a = id(1);
        let mut map = std::collections::BTreeMap::new();
        map.insert(0u32, 100u64);
        map.insert(1u32, 250u64);
        kv.put(DetScope::Identity(&a), TOP_UPS_KEY, &map).unwrap();
        let got: std::collections::BTreeMap<u32, u64> = kv
            .get(DetScope::Identity(&a), TOP_UPS_KEY)
            .unwrap()
            .unwrap();
        assert_eq!(got, map);
    }

    /// A legacy import carries only the entries it found in `data.db`. It must
    /// union them into whatever the user has recorded since — a top-up made
    /// between two migration passes is real money moved, and a replacing write
    /// would erase its record.
    #[test]
    fn save_top_ups_merges_into_the_stored_history() {
        let kv = empty_kv();
        let a = id(1);

        // The user tops up in the new build; the entry lands at index 1.
        save_top_ups_in(&kv, &a, &std::collections::BTreeMap::from([(1u32, 250u64)])).unwrap();
        // A late migration pass replays the legacy history, which knows only index 0.
        save_top_ups_in(&kv, &a, &std::collections::BTreeMap::from([(0u32, 100u64)])).unwrap();

        let got: std::collections::BTreeMap<u32, u64> = kv
            .get(DetScope::Identity(&a), TOP_UPS_KEY)
            .unwrap()
            .unwrap();
        assert_eq!(
            got,
            std::collections::BTreeMap::from([(0u32, 100u64), (1u32, 250u64)]),
            "the entry recorded between the passes must survive the import",
        );
    }

    /// On a colliding index the caller's value wins: the top-up flow writes the
    /// amount it just confirmed on-chain, which is fresher than any stored copy.
    #[test]
    fn save_top_ups_incoming_value_wins_on_a_colliding_index() {
        let kv = empty_kv();
        let a = id(1);

        save_top_ups_in(&kv, &a, &std::collections::BTreeMap::from([(0u32, 100u64)])).unwrap();
        save_top_ups_in(&kv, &a, &std::collections::BTreeMap::from([(0u32, 999u64)])).unwrap();

        let got: std::collections::BTreeMap<u32, u64> = kv
            .get(DetScope::Identity(&a), TOP_UPS_KEY)
            .unwrap()
            .unwrap();
        assert_eq!(got, std::collections::BTreeMap::from([(0u32, 999u64)]));
    }

    // ---------------------------------------------------------------
    // Cleanup: purge drains the whole Identity scope.
    // ---------------------------------------------------------------

    #[test]
    fn purge_identity_scope_drains_blob_top_ups_and_votes() {
        let kv = empty_kv();
        let a = id(1);
        put_identity(&kv, &a, "Masternode");
        kv.put(
            DetScope::Identity(&a),
            TOP_UPS_KEY,
            &std::collections::BTreeMap::from([(0u32, 5u64)]),
        )
        .unwrap();
        kv.put(
            DetScope::Identity(&a),
            &scheduled_vote_key("alice"),
            &StoredScheduledVote {
                voter_id: a,
                contested_name: "alice".to_string(),
                choice: StoredVoteChoice::Lock,
                unix_timestamp: 0,
                executed_successfully: false,
            },
        )
        .unwrap();
        index_add_vote_voter(&kv, &a).unwrap();

        purge_identity_scope(&kv, &a).unwrap();

        assert!(
            kv.get::<StoredQualifiedIdentity>(DetScope::Identity(&a), IDENTITY_KEY)
                .unwrap()
                .is_none()
        );
        assert!(
            kv.get::<std::collections::BTreeMap<u32, u64>>(DetScope::Identity(&a), TOP_UPS_KEY)
                .unwrap()
                .is_none()
        );
        assert!(
            kv.list(DetScope::Identity(&a), Some(SCHEDULED_VOTE_KEY_PREFIX))
                .unwrap()
                .is_empty()
        );
        // The voter index is pruned by the cascade-free cleanup path.
        assert!(load_scheduled_vote_voters(&kv).unwrap().is_empty());
    }

    // ---------------------------------------------------------------
    // Scheduled votes: per-voter Identity scope + Global voter index.
    // ---------------------------------------------------------------

    #[test]
    fn scheduled_vote_round_trips_in_voter_scope() {
        let kv = empty_kv();
        let voter = id(1);
        let key = scheduled_vote_key("dash");
        kv.put(
            DetScope::Identity(&voter),
            &key,
            &StoredScheduledVote {
                voter_id: voter,
                contested_name: "dash".to_string(),
                choice: StoredVoteChoice::Abstain,
                unix_timestamp: 42,
                executed_successfully: false,
            },
        )
        .unwrap();
        index_add_vote_voter(&kv, &voter).unwrap();

        let got: StoredScheduledVote = kv.get(DetScope::Identity(&voter), &key).unwrap().unwrap();
        assert_eq!(got.contested_name, "dash");
        assert_eq!(got.unix_timestamp, 42);
        // Voter index tracks the single voter.
        assert_eq!(load_scheduled_vote_voters(&kv).unwrap(), vec![voter]);
    }

    #[test]
    fn scheduled_votes_for_two_voters_share_a_contested_name_without_aliasing() {
        let kv = empty_kv();
        let v1 = id(1);
        let v2 = id(2);
        let key = scheduled_vote_key("contested");
        for (v, ts) in [(v1, 10u64), (v2, 20u64)] {
            kv.put(
                DetScope::Identity(&v),
                &key,
                &StoredScheduledVote {
                    voter_id: v,
                    contested_name: "contested".to_string(),
                    choice: StoredVoteChoice::Lock,
                    unix_timestamp: ts,
                    executed_successfully: false,
                },
            )
            .unwrap();
            index_add_vote_voter(&kv, &v).unwrap();
        }
        let got1: StoredScheduledVote = kv.get(DetScope::Identity(&v1), &key).unwrap().unwrap();
        let got2: StoredScheduledVote = kv.get(DetScope::Identity(&v2), &key).unwrap().unwrap();
        assert_eq!(got1.unix_timestamp, 10);
        assert_eq!(got2.unix_timestamp, 20);
        let mut voters = load_scheduled_vote_voters(&kv).unwrap();
        voters.sort_unstable();
        assert_eq!(voters, vec![v1, v2]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scheduled_vote_count_for_identity_counts_only_that_voters_queue() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let target = id(0x61);
        let other = id(0x62);
        let kv = ctx.det_kv().expect("det kv");
        for (voter, name) in [(target, "alpha"), (target, "beta"), (other, "gamma")] {
            kv.put(
                DetScope::Identity(&voter),
                &scheduled_vote_key(name),
                &StoredScheduledVote {
                    voter_id: voter,
                    contested_name: name.to_string(),
                    choice: StoredVoteChoice::Lock,
                    unix_timestamp: 0,
                    executed_successfully: false,
                },
            )
            .expect("store scheduled vote");
        }

        assert_eq!(
            ctx.scheduled_vote_count_for_identity(&Identifier::from(target))
                .expect("count target votes"),
            2
        );
        assert_eq!(
            ctx.scheduled_vote_count_for_identity(&Identifier::from(id(0x63)))
                .expect("count absent voter"),
            0
        );

        ctx.wallet_backend()
            .expect("backend wired")
            .shutdown()
            .await;
    }

    #[test]
    fn delete_scheduled_votes_for_voter_drains_scope_and_prunes_index() {
        let kv = empty_kv();
        let voter = id(1);
        for name in ["a", "b"] {
            kv.put(
                DetScope::Identity(&voter),
                &scheduled_vote_key(name),
                &StoredScheduledVote {
                    voter_id: voter,
                    contested_name: name.to_string(),
                    choice: StoredVoteChoice::Lock,
                    unix_timestamp: 0,
                    executed_successfully: false,
                },
            )
            .unwrap();
        }
        index_add_vote_voter(&kv, &voter).unwrap();

        delete_scheduled_votes_for_voter(&kv, &voter).unwrap();
        assert!(
            kv.list(DetScope::Identity(&voter), Some(SCHEDULED_VOTE_KEY_PREFIX))
                .unwrap()
                .is_empty()
        );
        assert!(load_scheduled_vote_voters(&kv).unwrap().is_empty());
    }

    #[test]
    fn prune_vote_voter_keeps_voter_with_remaining_votes() {
        let kv = empty_kv();
        let voter = id(1);
        kv.put(
            DetScope::Identity(&voter),
            &scheduled_vote_key("still-here"),
            &StoredScheduledVote {
                voter_id: voter,
                contested_name: "still-here".to_string(),
                choice: StoredVoteChoice::Lock,
                unix_timestamp: 0,
                executed_successfully: false,
            },
        )
        .unwrap();
        index_add_vote_voter(&kv, &voter).unwrap();

        prune_vote_voter_if_empty(&kv, &voter).unwrap();
        assert_eq!(load_scheduled_vote_voters(&kv).unwrap(), vec![voter]);
    }

    // ---------------------------------------------------------------
    // dashpay private / address_index Identity-scope contracts are
    // covered in `src/wallet_backend/dashpay.rs`; here we assert the
    // identity domain's own scope isolation against a foreign scope.
    // ---------------------------------------------------------------

    #[test]
    fn identity_blob_is_isolated_from_a_different_identity_scope() {
        let kv = empty_kv();
        let a = id(1);
        let b = id(2);
        put_identity(&kv, &a, "User");
        assert!(
            kv.get::<StoredQualifiedIdentity>(DetScope::Identity(&b), IDENTITY_KEY)
                .unwrap()
                .is_none(),
            "identity b must not see identity a's blob"
        );
    }

    // ---------------------------------------------------------------
    // F43: a wrong-length voter id surfaces a typed variant carrying the
    // upstream error as a `#[source]`, not a stringified detail.
    // ---------------------------------------------------------------

    #[test]
    fn voter_buffer_accepts_a_32_byte_id() {
        let bytes = [7u8; 32];
        assert_eq!(voter_buffer(&bytes).unwrap(), bytes);
    }

    #[test]
    fn voter_buffer_rejects_short_id_with_typed_source() {
        let err = voter_buffer(&[0u8; 5]).expect_err("a 5-byte voter id must be rejected");
        assert!(
            matches!(err, TaskError::InvalidVoterIdentifier { .. }),
            "expected InvalidVoterIdentifier, got {err:?}"
        );
        assert!(
            std::error::Error::source(&err).is_some(),
            "the typed upstream error must be preserved as the source"
        );
    }

    // ---------------------------------------------------------------
    // F63: the index is written before the blob, so a reader tolerates a
    // dangling index entry rather than hiding a written identity.
    // ---------------------------------------------------------------

    #[test]
    fn dangling_index_entry_without_blob_is_skipped_by_readers() {
        let kv = empty_kv();
        let present = id(1);
        let dangling = id(2);
        put_identity(&kv, &present, "User");
        // Simulate the post-`index_add_identity`, pre-blob-write window.
        index_add_identity(&kv, &dangling).unwrap();

        // The enumeration index lists both ids...
        let mut listed = load_identity_index(&kv).unwrap();
        listed.sort_unstable();
        assert_eq!(listed, vec![present, dangling]);

        // ...but a blob read for the dangling id finds nothing, which the
        // load paths treat as "skip" (they `continue` on a missing blob).
        assert!(
            kv.get::<StoredQualifiedIdentity>(DetScope::Identity(&dangling), IDENTITY_KEY)
                .unwrap()
                .is_none(),
            "a dangling index entry must not resolve to a blob"
        );
    }

    // ---------------------------------------------------------------
    // Identity-key vault migration + deletion (funds-safety).
    // ---------------------------------------------------------------

    use crate::model::qualified_identity::encrypted_key_storage::{
        KeyStorage, PrivateKeyData, WalletDerivationPath,
    };
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{IdentityType, PrivateKeyTarget};
    use crate::wallet_backend::IdentityKeyView;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, IdentityPublicKey};

    fn fresh_vault(dir: &std::path::Path) -> Arc<platform_wallet_storage::secrets::SecretStore> {
        let path = dir.join("secrets.pwsvault");
        Arc::new(crate::wallet_backend::single_key::open_secret_store(&path).expect("open vault"))
    }

    /// A `QualifiedIdentity` carrying one `Clear` (HIGH), one `AlwaysClear`
    /// (MEDIUM), and one `AtWalletDerivationPath` key. Returns the QI plus the
    /// `(target, key_id)` of each plaintext key for assertions.
    fn qi_with_id_plaintext_and_derived(
        identity_id: Identifier,
        secret_high: [u8; 32],
        secret_medium: [u8; 32],
    ) -> QualifiedIdentity {
        let pv = PlatformVersion::latest();
        let mut ks = KeyStorage::default();
        let high = IdentityPublicKey::random_key(1, Some(1), pv);
        ks.private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, high.id()),
            (
                QualifiedIdentityPublicKey::from(high),
                PrivateKeyData::Clear(secret_high),
            ),
        );
        let medium = IdentityPublicKey::random_key(2, Some(2), pv);
        ks.private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, medium.id()),
            (
                QualifiedIdentityPublicKey::from(medium),
                PrivateKeyData::AlwaysClear(secret_medium),
            ),
        );
        let derived = IdentityPublicKey::random_key(3, Some(3), pv);
        ks.private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, derived.id()),
            (
                QualifiedIdentityPublicKey::from(derived),
                PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                    wallet_seed_hash: [0x07; 32],
                    derivation_path: DerivationPath::from(vec![]),
                }),
            ),
        );
        let identity = Identity::create_basic_identity(identity_id, pv).expect("basic identity");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: ks,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    fn qi_with_plaintext_and_derived(
        secret_high: [u8; 32],
        secret_medium: [u8; 32],
    ) -> QualifiedIdentity {
        qi_with_id_plaintext_and_derived(Identifier::default(), secret_high, secret_medium)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identity_unload_removes_all_owned_state_and_preserves_siblings() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::model::dashpay::{ContactAddressIndex, ContactPrivateInfo};
        use crate::model::qualified_identity::identity_meta::IdentityMeta;
        use crate::utils::egui_mpsc::SenderAsync;
        use crate::wallet_backend::WalletSeedView;
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");

        let target_id = Identifier::from([0x11; 32]);
        let sibling_id = Identifier::from([0x22; 32]);
        let contact_id = Identifier::from([0x33; 32]);
        let target = qi_with_id_plaintext_and_derived(target_id, [0x41; 32], [0x42; 32]);
        let sibling = qi_with_id_plaintext_and_derived(sibling_id, [0x51; 32], [0x52; 32]);
        ctx.insert_local_qualified_identity(&target, &None)
            .expect("insert target identity");
        ctx.insert_local_qualified_identity(&sibling, &None)
            .expect("insert sibling identity");

        let target_buf = target_id.to_buffer();
        let sibling_buf = sibling_id.to_buffer();
        let target_vault = IdentityKeyView::new(backend.secret_store(), target_buf);
        let sibling_vault = IdentityKeyView::new(backend.secret_store(), sibling_buf);
        assert!(
            target_vault
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .expect("read target key")
                .is_some(),
            "target key must exist before removal"
        );

        backend
            .dashpay_set_private_info(
                &target_id,
                &contact_id,
                &ContactPrivateInfo {
                    nickname: "target contact".into(),
                    notes: "target note".into(),
                    is_hidden: false,
                },
            )
            .expect("seed target private memo");
        backend
            .dashpay_set_address_index(
                &target_id,
                &contact_id,
                &ContactAddressIndex {
                    owner_identity_id: target_buf.to_vec(),
                    contact_identity_id: contact_id.to_buffer().to_vec(),
                    next_send_index: 3,
                    highest_receive_index: 2,
                    bloom_registered_count: 1,
                },
            )
            .expect("seed target address index");
        backend
            .dashpay_mark_blocked(&target_id, &contact_id)
            .expect("seed target blocked marker");
        backend
            .dashpay_mark_declined(&target_id, &contact_id)
            .expect("seed target declined marker");
        backend
            .dashpay_mark_withdrawn(&target_id, &contact_id)
            .expect("seed target withdrawn marker");
        let contact_b58 = contact_id.to_string(Encoding::Base58);
        backend
            .kv()
            .put::<()>(
                DetScope::Identity(&target_buf),
                &format!("det:dashpay:request_action:decline:{contact_b58}"),
                &(),
            )
            .expect("seed target request-action journal");

        backend
            .dashpay_set_private_info(
                &sibling_id,
                &contact_id,
                &ContactPrivateInfo {
                    nickname: "sibling contact".into(),
                    notes: "sibling note".into(),
                    is_hidden: true,
                },
            )
            .expect("seed sibling private memo");
        backend
            .dashpay_mark_blocked(&sibling_id, &contact_id)
            .expect("seed sibling blocked marker");

        backend
            .dashpay_set_timestamps(&target_id, 11, 12)
            .expect("seed target timestamps");
        backend
            .dashpay_set_timestamps(&sibling_id, 21, 22)
            .expect("seed sibling timestamps");
        backend
            .dashpay_set_address_mapping(&target_id, "target-address-1", &contact_id, 1)
            .expect("seed first target address mapping");
        backend
            .dashpay_set_address_mapping(&target_id, "target-address-2", &contact_id, 2)
            .expect("seed second target address mapping");
        backend
            .dashpay_set_address_mapping(&sibling_id, "sibling-address", &contact_id, 3)
            .expect("seed sibling address mapping");
        backend
            .identity_meta()
            .set(
                ctx.network(),
                &target_buf,
                &IdentityMeta {
                    password_hint: Some("target hint".into()),
                },
            )
            .expect("seed target identity metadata");

        let wallet_seed_hash = [0x77; 32];
        let wallet_seed = [0x88; 64];
        WalletSeedView::new(backend.secret_store())
            .set_raw(&wallet_seed_hash, &wallet_seed)
            .expect("seed wallet secret");

        let kv = backend.kv();
        assert_eq!(
            kv.list(DetScope::Identity(&target_buf), Some("det:dashpay:"))
                .expect("list target overlays")
                .len(),
            6,
            "all six owner-overlay families must be seeded"
        );

        ctx.delete_local_qualified_identity(&target_id)
            .expect("remove target identity");

        assert!(
            ctx.get_local_qualified_identity(&target_id)
                .expect("read removed identity")
                .is_none(),
            "target identity blob must be removed"
        );
        assert_eq!(
            ctx.local_identity_ids().expect("read identity index"),
            vec![sibling_id],
            "identity index must retain only the sibling"
        );
        assert!(
            target_vault
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .expect("read removed target key")
                .is_none(),
            "target vault keys must be removed"
        );
        assert!(
            sibling_vault
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .expect("read sibling key")
                .is_some(),
            "sibling vault keys must survive"
        );
        assert!(
            kv.list(DetScope::Identity(&target_buf), Some("det:dashpay:"))
                .expect("list removed target overlays")
                .is_empty(),
            "all target owner overlays must be removed"
        );
        assert_eq!(
            kv.list(DetScope::Identity(&sibling_buf), Some("det:dashpay:"))
                .expect("list sibling overlays")
                .len(),
            2,
            "sibling owner overlays must survive"
        );

        let target_timestamps = format!(
            "det:dashpay:timestamps:{}",
            target_id.to_string(Encoding::Base58)
        );
        let sibling_timestamps = format!(
            "det:dashpay:timestamps:{}",
            sibling_id.to_string(Encoding::Base58)
        );
        assert!(
            kv.get::<(i64, i64)>(DetScope::Global, &target_timestamps)
                .expect("read target timestamps")
                .is_none(),
            "target entity timestamps must be removed"
        );
        assert_eq!(
            kv.get::<(i64, i64)>(DetScope::Global, &sibling_timestamps)
                .expect("read sibling timestamps"),
            Some((21, 22)),
            "sibling entity timestamps must survive"
        );
        assert!(
            backend
                .dashpay_get_address_mapping(&target_id, "target-address-1")
                .expect("read first removed target address mapping")
                .is_none(),
            "the first target address mapping must be removed"
        );
        assert!(
            backend
                .dashpay_get_address_mapping(&target_id, "target-address-2")
                .expect("read second removed target address mapping")
                .is_none(),
            "the second target address mapping must be removed"
        );
        assert_eq!(
            backend
                .dashpay_get_address_mapping(&sibling_id, "sibling-address")
                .expect("read sibling address mapping"),
            Some((contact_id, 3)),
            "the sibling address mapping must survive"
        );
        assert!(
            backend
                .identity_meta()
                .get(ctx.network(), &target_buf)
                .is_none(),
            "target identity metadata must be removed"
        );
        assert_eq!(
            WalletSeedView::new(backend.secret_store())
                .get_raw(&wallet_seed_hash)
                .expect("read wallet seed")
                .as_deref(),
            Some(&wallet_seed),
            "wallet seed.raw.v1 must survive identity removal"
        );

        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identity_unload_reports_failure_when_wallet_backend_is_unwired() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");
        let target_id = Identifier::from([0xB1; 32]);
        let target = qi_with_id_plaintext_and_derived(target_id, [0xB2; 32], [0xB3; 32]);
        ctx.insert_local_qualified_identity(&target, &None)
            .expect("insert target identity");

        ctx.wallet_backend.store(None);

        assert!(
            matches!(
                ctx.unload_local_qualified_identity(&target_id),
                Err(TaskError::WalletBackendNotYetWired)
            ),
            "unload must not report success when its cleanup backend is unavailable"
        );
        assert!(
            backend
                .kv()
                .get::<StoredQualifiedIdentity>(
                    DetScope::Identity(&target_id.to_buffer()),
                    IDENTITY_KEY,
                )
                .expect("read retained identity")
                .is_some(),
            "an unload rejected before its commit point must retain the identity"
        );

        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_unload_before_commit_preserves_dashpay_overlays() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::model::dashpay::ContactPrivateInfo;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");
        let target_id = Identifier::from([0xB4; 32]);
        let contact_id = Identifier::from([0xB5; 32]);
        let target = qi_with_id_plaintext_and_derived(target_id, [0xB6; 32], [0xB7; 32]);
        ctx.insert_local_qualified_identity(&target, &Some(([0xB8; 32], 1)))
            .expect("insert target identity");
        backend
            .dashpay_set_private_info(
                &target_id,
                &contact_id,
                &ContactPrivateInfo {
                    nickname: "retained contact".into(),
                    notes: "retained note".into(),
                    is_hidden: false,
                },
            )
            .expect("seed owner overlay");

        let persister_path = backend.spv_storage_dir().join("platform-wallet.sqlite");
        let fault_connection =
            rusqlite::Connection::open(&persister_path).expect("open persister second handle");
        fault_connection
            .execute_batch(
                "CREATE TRIGGER fail_identity_index_commit
                 BEFORE INSERT ON meta_global
                 WHEN NEW.key = 'det:identity_index:v1'
                 BEGIN
                     SELECT RAISE(FAIL, 'injected identity-index failure');
                 END;",
            )
            .expect("install identity-index trigger");

        assert!(
            matches!(
                ctx.unload_local_qualified_identity(&target_id),
                Err(TaskError::IdentityStorage { .. })
            ),
            "the failed commit must surface its identity-storage error"
        );
        assert!(
            backend
                .dashpay_get_private_info(&target_id, &contact_id)
                .expect("read retained owner overlay")
                .is_some(),
            "cleanup must not destroy DashPay overlays before the unload commits"
        );

        fault_connection
            .execute_batch("DROP TRIGGER fail_identity_index_commit;")
            .expect("remove identity-index trigger");
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rollback_failure_does_not_replace_original_unload_error() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");
        let target_id = Identifier::from([0xB9; 32]);
        let target = qi_with_id_plaintext_and_derived(target_id, [0xBA; 32], [0xBB; 32]);
        ctx.insert_local_qualified_identity(&target, &Some(([0xBC; 32], 2)))
            .expect("insert target identity");

        let persister_path = backend.spv_storage_dir().join("platform-wallet.sqlite");
        let fault_connection =
            rusqlite::Connection::open(&persister_path).expect("open persister second handle");
        fault_connection
            .execute_batch(
                "CREATE TRIGGER fail_identity_index_commit
                 BEFORE INSERT ON meta_global
                 WHEN NEW.key = 'det:identity_index:v1'
                 BEGIN
                     SELECT RAISE(FAIL, 'injected identity-index failure');
                 END;",
            )
            .expect("install identity-index trigger");
        ctx.db()
            .locked_conn()
            .execute_batch(
                "CREATE TRIGGER fail_forgotten_marker_rollback
                 BEFORE DELETE ON forgotten_identities
                 BEGIN
                     SELECT RAISE(FAIL, 'injected marker-rollback failure');
                 END;",
            )
            .expect("install marker-rollback trigger");

        let error = ctx
            .unload_local_qualified_identity(&target_id)
            .expect_err("the unload commit must fail");
        assert!(
            matches!(error, TaskError::IdentityStorage { .. }),
            "the original commit failure must remain primary, got {error:?}"
        );

        fault_connection
            .execute_batch("DROP TRIGGER fail_identity_index_commit;")
            .expect("remove identity-index trigger");
        ctx.db()
            .locked_conn()
            .execute_batch("DROP TRIGGER fail_forgotten_marker_rollback;")
            .expect("remove marker-rollback trigger");
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identity_unload_continues_cleanup_and_wipes_vault_after_overlay_failure() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::model::dashpay::ContactPrivateInfo;
        use crate::model::qualified_identity::identity_meta::IdentityMeta;
        use crate::utils::egui_mpsc::SenderAsync;
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");

        let target_id = Identifier::from([0xA1; 32]);
        let contact_id = Identifier::from([0xA2; 32]);
        let target = qi_with_id_plaintext_and_derived(target_id, [0xA3; 32], [0xA4; 32]);
        ctx.insert_local_qualified_identity(&target, &None)
            .expect("insert target identity");

        let target_buf = target_id.to_buffer();
        let target_vault = IdentityKeyView::new(backend.secret_store(), target_buf);
        assert!(
            target_vault
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .expect("read target key")
                .is_some(),
            "target key must exist before removal"
        );

        backend
            .dashpay_set_private_info(
                &target_id,
                &contact_id,
                &ContactPrivateInfo {
                    nickname: "target contact".into(),
                    notes: "target note".into(),
                    is_hidden: false,
                },
            )
            .expect("seed target owner overlay");
        backend
            .dashpay_set_timestamps(&target_id, 11, 12)
            .expect("seed target timestamps");
        backend
            .dashpay_set_address_mapping(&target_id, "target-address", &contact_id, 1)
            .expect("seed target address mapping");
        backend
            .identity_meta()
            .set(
                ctx.network(),
                &target_buf,
                &IdentityMeta {
                    password_hint: Some("target hint".into()),
                },
            )
            .expect("seed target identity metadata");

        let kv = backend.kv();
        let overlay_keys = kv
            .list(
                DetScope::Identity(&target_buf),
                Some("det:dashpay:private:"),
            )
            .expect("list target owner overlays");
        let [overlay_key] = overlay_keys.as_slice() else {
            panic!("exactly one target owner overlay must be seeded");
        };
        let timestamps_key = format!(
            "det:dashpay:timestamps:{}",
            target_id.to_string(Encoding::Base58)
        );

        let persister_path = backend.spv_storage_dir().join("platform-wallet.sqlite");
        let fault_connection =
            rusqlite::Connection::open(&persister_path).expect("open persister second handle");
        let trigger_name = "fail_target_owner_overlay_delete";
        let trigger_sql = format!(
            "CREATE TRIGGER {trigger_name}
             BEFORE DELETE ON meta_identity
             WHEN OLD.identity_id = X'{}' AND OLD.key = '{}'
             BEGIN
                 SELECT RAISE(FAIL, 'injected owner overlay delete failure');
             END;",
            hex::encode(target_buf),
            overlay_key.replace('\'', "''"),
        );
        fault_connection
            .execute_batch(&trigger_sql)
            .expect("install owner-overlay delete trigger");

        match ctx.unload_local_qualified_identity(&target_id) {
            Err(TaskError::IdentityUnloadCleanupFailed {
                identity_id,
                source,
            }) => {
                assert_eq!(identity_id, target_id);
                assert!(
                    matches!(*source, TaskError::DashpaySidecarStorage { .. }),
                    "the first cleanup failure must preserve its DashPay source"
                );
            }
            other => panic!("expected identity-unload cleanup failure, got {other:?}"),
        }

        assert!(
            kv.get::<ContactPrivateInfo>(DetScope::Identity(&target_buf), overlay_key)
                .expect("read retained owner overlay")
                .is_some(),
            "the targeted owner overlay must survive its injected delete failure"
        );
        assert!(
            kv.get::<(i64, i64)>(DetScope::Global, &timestamps_key)
                .expect("read target timestamps")
                .is_none(),
            "timestamp cleanup must continue after the owner-overlay failure"
        );
        assert!(
            backend
                .dashpay_get_address_mapping(&target_id, "target-address")
                .expect("read target address mapping")
                .is_none(),
            "address-map cleanup must continue after the owner-overlay failure"
        );
        assert!(
            backend
                .identity_meta()
                .get(ctx.network(), &target_buf)
                .is_none(),
            "identity-metadata cleanup must continue after the owner-overlay failure"
        );
        assert!(
            target_vault
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .expect("read removed target key")
                .is_none(),
            "target vault keys must be removed despite the cleanup failure"
        );
        assert!(
            ctx.is_identity_forgotten(&target_id)
                .expect("read marker after partial cleanup"),
            "the partial unload must remain discoverable after its blob is purged"
        );
        assert!(
            !ctx.has_local_qualified_identity(&target_id)
                .expect("read blob after partial cleanup"),
            "vault cleanup success allows the recovery blob to be purged"
        );

        fault_connection
            .execute_batch(&format!("DROP TRIGGER {trigger_name};"))
            .expect("remove owner-overlay delete trigger");
        ctx.clear_network_database()
            .await
            .expect("full wipe must retry marker-only identity residue");
        assert!(
            kv.get::<ContactPrivateInfo>(DetScope::Identity(&target_buf), overlay_key)
                .expect("read owner overlay after full wipe")
                .is_none(),
            "the full wipe must remove owner residue even when no blob remains"
        );
        assert!(
            !ctx.is_identity_forgotten(&target_id)
                .expect("read marker after full wipe"),
            "the marker must clear only after its residue cleanup succeeds"
        );
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identity_unload_respects_an_in_flight_load_claim() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");
        let target_id = Identifier::from([0x71; 32]);
        let target = qi_with_id_plaintext_and_derived(target_id, [0x72; 32], [0x73; 32]);
        ctx.insert_local_qualified_identity(&target, &None)
            .expect("insert target identity");
        let load_guard = ctx
            .begin_identity_load(target_id, None)
            .expect("claim target for an in-flight load");

        assert!(
            matches!(
                ctx.delete_local_qualified_identity(&target_id),
                Err(TaskError::IdentityBusyWithLoad { identity_id }) if identity_id == target_id
            ),
            "deletion must surface an unload-specific error without racing the load"
        );
        assert!(
            ctx.get_local_qualified_identity(&target_id)
                .expect("read retained identity")
                .is_some(),
            "a rejected deletion must not mutate the identity"
        );

        drop(load_guard);
        backend.shutdown().await;
    }

    /// A failure partway through deletion must never leave a "zombie" identity:
    /// still indexed (so still visible in the UI) with its vault keys already
    /// gone. `index_remove_identity` must run before the irreversible vault-key
    /// clear, so a fault anywhere from that point on still leaves the identity
    /// hidden rather than visibly broken.
    ///
    /// The fault is a corrupted stored blob (bad `qi_bytes`) written directly
    /// after a real insert — a real, naturally-occurring failure of
    /// `clear_identity_vault_keys`'s `decode_stored_identity` call, not a
    /// simulated one — driving the actual `delete_local_qualified_identity`
    /// entry point end to end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deletion_fault_after_index_removal_leaves_no_visible_zombie() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");

        let target_id = Identifier::from([0x91; 32]);
        let target = qi_with_id_plaintext_and_derived(target_id, [0x92; 32], [0x93; 32]);
        ctx.insert_local_qualified_identity(&target, &None)
            .expect("insert target identity");
        assert!(
            ctx.local_identity_ids()
                .expect("read index")
                .contains(&target_id),
            "the identity must be indexed before deletion"
        );

        // Corrupt the stored blob in place: the index and wallet association
        // are untouched, but `clear_identity_vault_keys`'s decode will fail.
        let id_buf = target_id.to_buffer();
        let kv = ctx.det_kv().expect("det kv");
        kv.put(DetScope::Identity(&id_buf), IDENTITY_KEY, &stored("User"))
            .expect("corrupt the stored blob");

        let result = ctx.delete_local_qualified_identity(&target_id);
        assert!(
            result.is_err(),
            "the corrupted blob must surface as an error"
        );

        assert!(
            !ctx.local_identity_ids()
                .expect("read index")
                .contains(&target_id),
            "the identity must already be hidden from the index even though a \
             later cleanup step failed — never a visible entry with its keys \
             already gone"
        );

        backend.shutdown().await;
    }

    /// A failed vault-key clear must retain the identity blob that inventories
    /// its labels, allowing a later retry to finish without orphaning the key.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deletion_fault_after_index_removal_must_not_permanently_orphan_vault_key() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");

        let target_id = Identifier::from([0xA1; 32]);
        let target = qi_with_id_plaintext_and_derived(target_id, [0xA2; 32], [0xA3; 32]);
        ctx.insert_local_qualified_identity(&target, &None)
            .expect("insert target identity");

        let target_buf = target_id.to_buffer();
        let target_vault = IdentityKeyView::new(backend.secret_store(), target_buf);
        assert!(
            target_vault
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .expect("read target key before deletion")
                .is_some(),
            "the vault key must exist before the faulted deletion"
        );

        let id_buf = target_id.to_buffer();
        let kv = ctx.det_kv().expect("det kv");
        let stored_before_fault = kv
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id_buf), IDENTITY_KEY)
            .expect("read stored identity")
            .expect("stored identity exists");
        // Corrupt the stored blob in place so the first clear cannot decode
        // the inventory of vault labels.
        kv.put(DetScope::Identity(&id_buf), IDENTITY_KEY, &stored("User"))
            .expect("corrupt the stored blob");

        assert!(matches!(
            ctx.unload_local_qualified_identity(&target_id),
            Err(TaskError::IdentityUnloadCleanupFailed { .. })
        ));
        assert!(
            !ctx.local_identity_ids()
                .expect("read index")
                .contains(&target_id),
            "precondition: the identity is already hidden from the index"
        );
        assert!(
            kv.get::<StoredQualifiedIdentity>(DetScope::Identity(&id_buf), IDENTITY_KEY)
                .expect("read retained inventory")
                .is_some(),
            "a vault-clear failure must retain the only on-disk inventory of key labels"
        );
        assert!(
            ctx.is_identity_forgotten(&target_id)
                .expect("read forgotten marker"),
            "the interrupted unload must remain discoverable for retry"
        );
        assert!(matches!(
            ctx.retry_stuck_unload_cleanup(&id_buf),
            Err(TaskError::IdentityUnloadCleanupFailed { .. })
        ));
        assert!(
            ctx.has_local_qualified_identity(&target_id)
                .expect("read retained identity after failed retry"),
            "a failed retry must preserve the identity blob"
        );
        assert!(
            ctx.is_identity_forgotten(&target_id)
                .expect("read retained marker after failed retry"),
            "a failed retry must preserve the forgotten marker"
        );

        kv.put(
            DetScope::Identity(&id_buf),
            IDENTITY_KEY,
            &stored_before_fault,
        )
        .expect("restore stored identity");
        assert!(
            ctx.retry_stuck_unload_cleanup(&id_buf)
                .expect("retry repaired cleanup"),
            "repairing the inventory must let the retry finish"
        );

        assert!(
            target_vault
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .expect("read target key after retry")
                .is_none(),
            "the retained inventory must let a retry delete the vault key"
        );
        assert!(
            !ctx.has_local_qualified_identity(&target_id)
                .expect("read identity after successful retry"),
            "a successful retry must purge the retained blob"
        );
        assert!(
            !ctx.is_identity_forgotten(&target_id)
                .expect("read marker after successful retry"),
            "a successful retry must clear the forgotten marker"
        );

        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stuck_unload_retry_ignores_non_ghost_states() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");

        let stored_id = Identifier::from([0xA4; 32]);
        let stored_identity = qi_with_id_plaintext_and_derived(stored_id, [0xA5; 32], [0xA6; 32]);
        ctx.insert_local_qualified_identity(&stored_identity, &None)
            .expect("insert non-forgotten identity");
        assert!(
            !ctx.retry_stuck_unload_cleanup(&stored_id.to_buffer())
                .expect("check non-forgotten identity"),
            "an identity that was never forgotten is not a cleanup ghost"
        );
        assert!(
            ctx.has_local_qualified_identity(&stored_id)
                .expect("read non-forgotten identity"),
            "the non-forgotten identity must remain stored"
        );

        let purged_id = Identifier::from([0xA7; 32]);
        ctx.db()
            .record_forgotten_identity(Network::Testnet, &purged_id)
            .expect("record marker without a blob");
        assert!(
            !ctx.retry_stuck_unload_cleanup(&purged_id.to_buffer())
                .expect("check marker without blob"),
            "a forgotten identity with no residual blob needs no retry"
        );
        assert!(
            ctx.is_identity_forgotten(&purged_id)
                .expect("read marker without blob"),
            "normal marker lifecycle remains the caller's responsibility"
        );

        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reject_if_exists_retries_a_repaired_unload_ghost() {
        use crate::app::TaskResult;
        use crate::backend_task::identity::{IdentityInputToLoad, IdentityLoadMode, IdentityTask};
        use crate::context::test_support::test_app_context;
        use crate::model::secret::Secret;
        use crate::utils::egui_mpsc::SenderAsync;
        use dash_sdk::SdkBuilder;
        use dash_sdk::dpp::platform_value::Value;
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        use dash_sdk::dpp::version::PlatformVersion;
        use dash_sdk::drive::query::{SelectProjection, WhereClause, WhereOperator};
        use dash_sdk::platform::{DocumentQuery, Identifier, Identity};

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");

        let target_id = Identifier::from([0xA8; 32]);
        let target = qi_with_id_plaintext_and_derived(target_id, [0xA9; 32], [0xAA; 32]);
        ctx.insert_local_qualified_identity(&target, &None)
            .expect("insert target identity");
        let id_buf = target_id.to_buffer();
        let kv = ctx.det_kv().expect("det kv");
        let stored_before_fault = kv
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id_buf), IDENTITY_KEY)
            .expect("read stored identity")
            .expect("stored identity exists");
        kv.put(DetScope::Identity(&id_buf), IDENTITY_KEY, &stored("User"))
            .expect("corrupt the stored blob");
        assert!(matches!(
            ctx.unload_local_qualified_identity(&target_id),
            Err(TaskError::IdentityUnloadCleanupFailed { .. })
        ));
        let failed_wipe = ctx.clear_network_database().await;
        assert!(
            matches!(
                failed_wipe,
                Err(TaskError::WalletDataClearIncomplete { .. })
            ),
            "an unreadable ghost must make the full wipe report incomplete"
        );
        assert!(
            ctx.has_local_qualified_identity(&target_id)
                .expect("read retained ghost after failed wipe"),
            "a failed full-wipe retry must preserve the ghost blob"
        );
        assert!(
            ctx.is_identity_forgotten(&target_id)
                .expect("read retained marker after failed wipe"),
            "a failed full-wipe retry must preserve the forgotten marker"
        );
        kv.put(
            DetScope::Identity(&id_buf),
            IDENTITY_KEY,
            &stored_before_fault,
        )
        .expect("restore stored identity");

        let input = IdentityInputToLoad {
            identity_id_input: target_id.to_string(Encoding::Hex),
            identity_type: IdentityType::User,
            alias_input: String::new(),
            voting_private_key_input: Secret::new(""),
            owner_private_key_input: Secret::new(""),
            payout_address_private_key_input: Secret::new(""),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: None,
            load_mode: IdentityLoadMode::RejectIfExists,
            load_token: None,
        };
        let (task_tx, _task_rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let task_sender = SenderAsync::new(task_tx, ctx.egui_ctx().clone());
        let mut missing_sdk = SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .expect("build pinned mock SDK");
        missing_sdk
            .mock()
            .expect_fetch(target_id, None::<Identity>)
            .await
            .expect("mock missing identity fetch");
        let failed_result = ctx
            .run_identity_task(
                IdentityTask::LoadIdentity(input.clone()),
                &missing_sdk,
                task_sender.clone(),
            )
            .await;
        assert!(
            matches!(failed_result, Err(TaskError::IdentityNotFound)),
            "a missing network identity must fail the reload: {failed_result:?}"
        );
        assert!(
            ctx.is_identity_forgotten(&target_id)
                .expect("read marker after failed reload"),
            "a failed reload must preserve the user's unload marker"
        );
        assert!(
            !ctx.has_local_qualified_identity(&target_id)
                .expect("read slot after failed reload"),
            "the repaired ghost blob should remain purged"
        );

        let mut sdk = SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .expect("build pinned mock SDK");
        sdk.mock()
            .expect_fetch(target_id, Some(target.identity.clone()))
            .await
            .expect("mock identity fetch");
        let dpns_query = DocumentQuery {
            select: SelectProjection::documents(),
            data_contract: ctx.dpns_contract.clone(),
            document_type_name: "domain".to_string(),
            where_clauses: vec![WhereClause {
                field: "records.identity".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(target_id.into()),
            }],
            group_by: Vec::new(),
            having: Vec::new(),
            order_by_clauses: vec![],
            limit: 100,
            start: None,
        };
        sdk.mock()
            .expect_fetch_many(
                dpns_query,
                Some(dash_sdk::query_types::Documents::default()),
            )
            .await
            .expect("mock DPNS fetch");
        let result = ctx
            .run_identity_task(IdentityTask::LoadIdentity(input), &sdk, task_sender)
            .await;

        assert!(
            matches!(
                result,
                Ok(BackendTaskSuccessResult::LoadedIdentity(ref identity))
                    if identity.identity.id() == target_id
            ),
            "a repaired ghost must load successfully: {result:?}"
        );
        assert!(
            ctx.has_local_qualified_identity(&target_id)
                .expect("read identity after retry"),
            "the fresh identity must be stored after the ghost is purged"
        );
        assert!(
            !ctx.is_identity_forgotten(&target_id)
                .expect("read marker after retry"),
            "the fresh load must clear the recovered ghost marker"
        );

        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn clear_network_database_purges_a_repaired_unload_ghost() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");

        let target_id = Identifier::from([0xAB; 32]);
        let target = qi_with_id_plaintext_and_derived(target_id, [0xAC; 32], [0xAD; 32]);
        ctx.insert_local_qualified_identity(&target, &None)
            .expect("insert target identity");
        let id_buf = target_id.to_buffer();
        let target_vault = IdentityKeyView::new(backend.secret_store(), id_buf);
        let kv = ctx.det_kv().expect("det kv");
        let stored_before_fault = kv
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id_buf), IDENTITY_KEY)
            .expect("read stored identity")
            .expect("stored identity exists");
        kv.put(DetScope::Identity(&id_buf), IDENTITY_KEY, &stored("User"))
            .expect("corrupt the stored blob");
        assert!(matches!(
            ctx.unload_local_qualified_identity(&target_id),
            Err(TaskError::IdentityUnloadCleanupFailed { .. })
        ));
        kv.put(
            DetScope::Identity(&id_buf),
            IDENTITY_KEY,
            &stored_before_fault,
        )
        .expect("restore stored identity");

        ctx.clear_network_database()
            .await
            .expect("full wipe must recover and purge the ghost");

        assert!(
            !ctx.has_local_qualified_identity(&target_id)
                .expect("read identity after full wipe"),
            "the full wipe must purge the ghost blob"
        );
        assert!(
            !ctx.is_identity_forgotten(&target_id)
                .expect("read marker after full wipe"),
            "the full wipe must clear the recovered ghost marker"
        );
        assert!(
            target_vault
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .expect("read vault after full wipe")
                .is_none(),
            "the full wipe must remove the ghost's vault key"
        );

        backend.shutdown().await;
    }

    /// Load-path migration — `migrate_keystore_to_vault` content-detects Clear/AlwaysClear,
    /// stores them in the vault FIRST, then rewrites the blob to InVault.
    /// Asserts: vault-first (the raw bytes are present), the wallet-derived key
    /// is untouched, zero plaintext remains, and the persist closure ran AFTER
    /// the vault holds the keys.
    #[test]
    fn qa_002_migrate_keystore_to_vault_vault_first_then_blob() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_vault(dir.path());
        let id = id(0x11);
        let high = [0xAA; 32];
        let medium = [0xBB; 32];
        let mut qi = qi_with_plaintext_and_derived(high, medium);

        let view = IdentityKeyView::new(&store, id);
        let mut persisted = false;
        let outcome = migrate_keystore_to_vault(&store, &id, &mut qi, |migrated| {
            // Vault-FIRST: by the time persist runs, the raw keys are stored.
            assert!(
                view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                    .unwrap()
                    .is_some(),
                "vault must hold the keys before the blob is rewritten"
            );
            // And the in-memory blob being persisted is already InVault-only.
            assert!(
                migrated
                    .private_keys
                    .private_keys
                    .values()
                    .all(|(_, d)| !matches!(
                        d,
                        PrivateKeyData::Clear(_) | PrivateKeyData::AlwaysClear(_)
                    )),
                "persisted blob must carry no plaintext"
            );
            persisted = true;
            Ok(())
        });

        assert_eq!(outcome, KeystoreMigration::Migrated(2));
        assert!(persisted, "persist closure ran");
        // Both plaintext keys are in the vault and equal the originals.
        assert_eq!(
            *view
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .unwrap()
                .unwrap(),
            high
        );
        assert_eq!(
            *view
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 2)
                .unwrap()
                .unwrap(),
            medium
        );
        // The wallet-derived key (key_id 3) was never plaintext → not stored.
        assert!(
            view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3)
                .unwrap()
                .is_none(),
            "AtWalletDerivationPath key must be untouched (not vaulted)"
        );
        // KeyStorage now has zero Clear/AlwaysClear; the derived key remains.
        let mut derived = 0;
        for (_, d) in qi.private_keys.private_keys.values() {
            match d {
                PrivateKeyData::Clear(_) | PrivateKeyData::AlwaysClear(_) => {
                    panic!("plaintext survived migration")
                }
                PrivateKeyData::AtWalletDerivationPath(_) => derived += 1,
                _ => {}
            }
        }
        assert_eq!(derived, 1, "wallet-derived key preserved");

        // Idempotent: a second run finds nothing to migrate.
        assert_eq!(
            migrate_keystore_to_vault(&store, &id, &mut qi, |_| Ok(())),
            KeystoreMigration::Nothing
        );
    }

    /// Regression: the single-get `get_identity_by_id` path
    /// must run the SAME vault migration the bulk `load_identities_filtered`
    /// path runs, so a legacy blob with resident `Clear`/`AlwaysClear` keys is
    /// migrated to the vault on read instead of returning (and re-persisting)
    /// resident plaintext. Before the fix this path called only `hydrate_top_ups`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_identity_by_id_migrates_legacy_resident_keys_to_vault() {
        use crate::app::TaskResult;
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::utils::egui_mpsc::SenderAsync;
        use crate::utils::tasks::TaskManager;

        // Offline wired AppContext (no network I/O) so `secret_store` is a real,
        // writable vault and `get_identity_by_id` can migrate into it.
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
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
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");

        // Stage a LEGACY blob: resident Clear/AlwaysClear keys written WITHOUT
        // the vault-first encode (bypassing `insert_local_qualified_identity`).
        let high = [0xAA; 32];
        let medium = [0xBB; 32];
        let qi = qi_with_plaintext_and_derived(high, medium);
        let identity_id = qi.identity.id();
        let id_buf = identity_id.to_buffer();
        let kv = ctx.det_kv().expect("identity kv");
        kv.put(
            DetScope::Identity(&id_buf),
            IDENTITY_KEY,
            &StoredQualifiedIdentity {
                qi_bytes: qi.to_bytes(),
                status: qi.status.as_u8(),
                identity_type: qi.identity_type.as_tag().to_string(),
                wallet_hash: None,
                wallet_index: None,
            },
        )
        .expect("stage legacy blob");
        index_add_identity(&kv, &id_buf).expect("index legacy identity");

        // Precondition: the vault holds nothing yet for this identity.
        let store = ctx.secret_store();
        let view = IdentityKeyView::new(&store, id_buf);
        assert!(
            view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .unwrap()
                .is_none(),
            "vault must be empty before the read-path migration"
        );

        // The single-get read MUST migrate the resident plaintext.
        let loaded = ctx
            .get_identity_by_id(&identity_id)
            .expect("load identity")
            .expect("identity present");
        assert!(
            !loaded.private_keys.has_plaintext_for_vault(),
            "returned identity must carry no resident plaintext after migration"
        );

        // The plaintext keys now live in the vault as raw bytes.
        assert_eq!(
            *view
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .unwrap()
                .expect("Clear key migrated to vault"),
            high
        );
        assert_eq!(
            *view
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 2)
                .unwrap()
                .expect("AlwaysClear key migrated to vault"),
            medium
        );

        // The persisted blob was rewritten to InVault placeholders — a re-read
        // no longer re-exposes resident plaintext.
        let raw: StoredQualifiedIdentity = kv
            .get(DetScope::Identity(&id_buf), IDENTITY_KEY)
            .unwrap()
            .expect("blob present");
        let redecoded =
            decode_stored_identity(&raw.qi_bytes, Network::Testnet).expect("decode rewritten blob");
        assert!(
            !redecoded.private_keys.has_plaintext_for_vault(),
            "rewritten blob must carry only InVault placeholders"
        );

        if let Ok(backend) = ctx.wallet_backend() {
            backend.shutdown().await;
        }
    }

    /// Helper: a QI with one `InVault` key (key_id 1, sealed
    /// Tier-2 in the vault by the caller) and one freshly-added `Clear` key
    /// (key_id 2) — i.e. a new key added to a password-protected identity.
    fn qi_invault_plus_new_clear() -> (QualifiedIdentity, dash_sdk::dpp::identity::KeyID) {
        let pv = PlatformVersion::latest();
        let mut ks = KeyStorage::default();
        let existing = IdentityPublicKey::random_key(1, Some(1), pv);
        ks.private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, existing.id()),
            (
                QualifiedIdentityPublicKey::from(existing),
                PrivateKeyData::InVault,
            ),
        );
        let added = IdentityPublicKey::random_key(2, Some(2), pv);
        let added_id = added.id();
        ks.private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, added_id),
            (
                QualifiedIdentityPublicKey::from(added),
                PrivateKeyData::Clear([0xCC; 32]),
            ),
        );
        let identity =
            Identity::create_basic_identity(Identifier::default(), pv).expect("basic identity");
        let qi = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: ks,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        };
        (qi, added_id)
    }

    /// The at-rest encode path REFUSES to write a new keyless
    /// key onto a password-protected identity (a silent-plaintext leak). The
    /// encode fails closed and the new key lands NOWHERE — not keyless, not
    /// Tier-2.
    #[test]
    fn encode_refuses_keyless_key_on_protected_identity() {
        use crate::wallet_backend::secret_seam::SecretScheme;
        use platform_wallet_storage::secrets::SecretString;

        let dir = tempfile::tempdir().unwrap();
        let store = fresh_vault(dir.path());
        let id = id(0x71);
        let (qi, added_id) = qi_invault_plus_new_clear();
        // Seal the existing key Tier-2 so the identity is password-protected.
        IdentityKeyView::new(&store, id)
            .store_protected(
                &PrivateKeyTarget::PrivateKeyOnMainIdentity,
                1,
                &[0x10; 32],
                &SecretString::new("identity-password-xx"),
            )
            .expect("seal existing key");

        let err = encode_identity_blob_vault_first(&store, &id, &qi)
            .expect_err("must refuse to keyless-store a new key on a protected identity");
        assert!(
            matches!(err, TaskError::IdentityKeyProtectionDowngrade),
            "expected IdentityKeyProtectionDowngrade, got {err:?}"
        );
        // The new key was NOT written keyless (or at all).
        assert_eq!(
            IdentityKeyView::new(&store, id)
                .scheme(&PrivateKeyTarget::PrivateKeyOnMainIdentity, added_id)
                .unwrap(),
            SecretScheme::Absent,
            "no keyless key landed for the newly-added id",
        );
    }

    /// The load-path migration likewise skips a protected identity's
    /// resident plaintext rather than writing it keyless — fail closed, persist
    /// nothing, leave it resident for the session.
    #[test]
    fn migrate_skips_keyless_on_protected_identity() {
        use platform_wallet_storage::secrets::SecretString;

        let dir = tempfile::tempdir().unwrap();
        let store = fresh_vault(dir.path());
        let id = id(0x72);
        let (mut qi, added_id) = qi_invault_plus_new_clear();
        IdentityKeyView::new(&store, id)
            .store_protected(
                &PrivateKeyTarget::PrivateKeyOnMainIdentity,
                1,
                &[0x10; 32],
                &SecretString::new("identity-password-xx"),
            )
            .expect("seal existing key");

        let mut persisted = false;
        let outcome = migrate_keystore_to_vault(&store, &id, &mut qi, |_| {
            persisted = true;
            Ok(())
        });
        assert_eq!(outcome, KeystoreMigration::ProtectedSkipped);
        assert!(!persisted, "a protected-skip must persist nothing");
        // No keyless key written for the resident plaintext key.
        assert_eq!(
            IdentityKeyView::new(&store, id)
                .scheme(&PrivateKeyTarget::PrivateKeyOnMainIdentity, added_id)
                .unwrap(),
            crate::wallet_backend::secret_seam::SecretScheme::Absent,
        );
        // The resident plaintext is preserved (it still signs this session).
        assert!(
            qi.private_keys
                .is_in_vault(&(PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)),
        );
    }

    /// Write-path twin of the load-path migration: the insert/update encoder
    /// (`encode_identity_blob_vault_first`) moves plaintext keys into the vault
    /// FIRST and returns an `InVault`-only blob, so a freshly inserted or
    /// updated identity never lands `Clear` / `AlwaysClear` key bytes in
    /// `det-app.sqlite`. Regression for the gap where the migration only ran on
    /// bulk load while the write paths still serialized plaintext.
    #[test]
    fn write_path_encodes_invault_only_and_vaults_plaintext() {
        use crate::wallet_backend::leak_test_support::assert_no_leak_bytes;

        let dir = tempfile::tempdir().unwrap();
        let store = fresh_vault(dir.path());
        let id = id(0x55);
        let high = [0xA1; 32];
        let medium = [0xB2; 32];
        let qi = qi_with_plaintext_and_derived(high, medium);

        let blob = encode_identity_blob_vault_first(&store, &id, &qi).expect("encode");

        // The persisted blob carries neither plaintext key in any rendered form.
        let rendered = format!("{blob:?}");
        assert_no_leak_bytes(&rendered, &high, "identity write-path blob (HIGH)");
        assert_no_leak_bytes(&rendered, &medium, "identity write-path blob (MEDIUM)");

        // Decoding the stored blob yields no plaintext key variant at all.
        let decoded = QualifiedIdentity::from_bytes(&blob).expect("decode");
        for (_, d) in decoded.private_keys.private_keys.values() {
            assert!(
                !matches!(d, PrivateKeyData::Clear(_) | PrivateKeyData::AlwaysClear(_)),
                "persisted write-path blob must carry no plaintext key",
            );
        }

        // The plaintext bytes live in the vault, retrievable per (target, key_id).
        let view = IdentityKeyView::new(&store, id);
        assert_eq!(
            *view
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .unwrap()
                .unwrap(),
            high
        );
        assert_eq!(
            *view
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 2)
                .unwrap()
                .unwrap(),
            medium
        );

        // The caller's in-memory identity keeps its resident keys (signing still
        // works this session) — the encoder operates on a clone.
        let clear_in_caller = qi
            .private_keys
            .private_keys
            .values()
            .filter(|(_, d)| matches!(d, PrivateKeyData::Clear(_) | PrivateKeyData::AlwaysClear(_)))
            .count();
        assert_eq!(
            clear_in_caller, 2,
            "the caller's identity must keep its resident plaintext for this session",
        );
    }

    /// Write-fault no-loss ordering. With the vault made unwritable so
    /// `store_all` fails, the migration restores the resident plaintext, does
    /// NOT call persist, and reports `VaultWriteFailed` — keys are never lost on
    /// a mid-write fault (the write half CRASH-01's read half does not cover).
    #[cfg(unix)]
    #[test]
    fn qa_005_vault_write_fault_leaves_keystore_intact_and_skips_persist() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let store = fresh_vault(dir.path());
        let id = id(0x22);
        let high = [0xCC; 32];
        let medium = [0xDD; 32];
        let mut qi = qi_with_plaintext_and_derived(high, medium);
        let before = qi.private_keys.clone();

        // Make the vault's parent dir read-only so the atomic rename-replace
        // `set` fails. (The file backend rewrites the whole file on set.)
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500))
            .expect("chmod ro");

        let mut persisted = false;
        let outcome = migrate_keystore_to_vault(&store, &id, &mut qi, |_| {
            persisted = true;
            Ok(())
        });

        // Restore perms so tempdir cleanup works.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).ok();

        assert_eq!(outcome, KeystoreMigration::VaultWriteFailed);
        assert!(
            !persisted,
            "persist must NOT run when the vault write failed"
        );
        assert_eq!(
            qi.private_keys, before,
            "the resident plaintext keystore must be restored on vault failure"
        );
    }

    /// Scoped key deletion — `clear_identity_vault_keys` removes the deleted identity's vault
    /// keys AND leaves other identities' keys untouched (isolation), via the
    /// public delete entry point. Builds a real `AppContext`-free vault and
    /// drives the free `IdentityKeyView` the deletion uses.
    #[test]
    fn qa_003_identity_key_deletion_is_scoped_and_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_vault(dir.path());
        let victim = id(0x33);
        let bystander = id(0x44);

        // Both identities have a vaulted key under the same (target, key_id).
        IdentityKeyView::new(&store, victim)
            .store(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0, &[0x01; 32])
            .unwrap();
        IdentityKeyView::new(&store, bystander)
            .store(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0, &[0x02; 32])
            .unwrap();

        // Delete the victim's keys the way clear_identity_vault_keys does:
        // enumerate the keystore's (target,key_id) set and delete_all.
        let mut ks = KeyStorage::default();
        let pv = PlatformVersion::latest();
        let pk = IdentityPublicKey::random_key(0, Some(0), pv);
        ks.private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, 0),
            (
                QualifiedIdentityPublicKey::from(pk),
                PrivateKeyData::InVault,
            ),
        );
        IdentityKeyView::new(&store, victim)
            .delete_all(ks.keys_set())
            .unwrap();

        assert!(
            IdentityKeyView::new(&store, victim)
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0)
                .unwrap()
                .is_none(),
            "victim's vault key must be gone"
        );
        assert_eq!(
            *IdentityKeyView::new(&store, bystander)
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0)
                .unwrap()
                .unwrap(),
            [0x02; 32],
            "a different identity's vault key must be untouched (isolation)"
        );
    }
}
