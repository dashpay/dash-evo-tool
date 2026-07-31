use super::AppContext;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::KeyID;
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
    kv.delete(scope, IDENTITY_KEY).map_err(identity_err)?;
    kv.delete(scope, TOP_UPS_KEY).map_err(top_up_err)?;
    delete_scheduled_votes_for_voter(kv, id)
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
    /// Whether the previous version's preserved `data.db` holds any identity of
    /// this network.
    ///
    /// The gate on the stranded-key recovery offer, and the only condition
    /// under which that offer can ever find something. A fresh install answers
    /// `false` — its `data.db` either does not exist or was created without the
    /// legacy `identity` table — while an upgraded one answers `true` for as
    /// long as the rows are there, which is forever: recovery never deletes
    /// them.
    ///
    /// Probed at most once per context: the file is a read-only artifact, so
    /// the answer cannot change under the session. A probe that fails arms the
    /// offer rather than retiring it — the detection task reports its own typed
    /// error, whereas a silent `false` would withdraw a recovery the user's
    /// data still supports.
    pub(crate) fn has_legacy_identities(&self) -> bool {
        *self.legacy_identities_present.get_or_init(|| {
            match crate::database::legacy_import::local_identities_exist(
                &self.db.locked_conn(),
                self.network,
            ) {
                Ok(found) => found,
                Err(error) => {
                    tracing::warn!(
                        target = "context::identity_db",
                        network = %self.network,
                        error = ?error,
                        "Could not tell whether the previous version's data holds identities; offering key recovery anyway",
                    );
                    true
                }
            }
        })
    }

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
    ///
    /// Serialized against every other whole-record writer of this identity by
    /// [`Self::identity_record_lock`].
    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_and_identity_id_info: &Option<(WalletSeedHash, u32)>,
    ) -> std::result::Result<(), TaskError> {
        let lock = self.identity_record_lock(qualified_identity.identity.id());
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    ///
    /// Takes [`Self::identity_record_lock`] for the write, so a caller whose
    /// snapshot came from an earlier read cannot interleave with another
    /// writer's read-modify-write. A caller that must keep its own read and
    /// this write atomic holds that guard itself and calls
    /// [`Self::write_local_qualified_identity_locked`] instead.
    pub fn update_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
    ) -> std::result::Result<(), TaskError> {
        let lock = self.identity_record_lock(qualified_identity.identity.id());
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.write_local_qualified_identity_locked(qualified_identity)
    }

    /// Read-modify-write one stored identity under its record guard.
    ///
    /// Re-reads the record inside [`Self::identity_record_lock`], hands the
    /// fresh copy to `edit`, and persists the result through
    /// [`Self::write_local_qualified_identity_locked`] — so the edit applies
    /// to what is on disk *now*, never to a caller's earlier snapshot, and a
    /// concurrent writer's change cannot be silently written away. Returns
    /// the persisted record for the caller to adopt in place of any clone it
    /// holds.
    ///
    /// # Errors
    ///
    /// [`TaskError::IdentityNotFoundLocally`] when nothing is stored under
    /// `identity_id`, and whatever `edit` returns — in both cases nothing is
    /// persisted.
    pub fn edit_local_qualified_identity(
        &self,
        identity_id: &Identifier,
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
        self.write_local_qualified_identity_locked(&fresh)?;
        Ok(fresh)
    }

    /// The write half of [`Self::update_local_qualified_identity`], for a
    /// caller that already holds this identity's
    /// [`identity_record_lock`](Self::identity_record_lock) across a wider
    /// read-modify-write span. Calling it without that guard reopens the
    /// lost-update race the guard exists to close.
    pub(crate) fn write_local_qualified_identity_locked(
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
    ///
    /// Read-modify-writes the whole blob, so it holds
    /// [`Self::identity_record_lock`] across both halves.
    pub fn set_identity_alias(
        &self,
        identifier: &Identifier,
        new_alias: Option<&str>,
    ) -> std::result::Result<(), TaskError> {
        let lock = self.identity_record_lock(*identifier);
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    /// The encoded identity blob stored under `id`, exactly as it sits at
    /// rest. Test-only: it is how an assertion proves what a write actually
    /// landed on disk (that no plaintext key survived, or that a re-run changed
    /// nothing), which the hydrated [`QualifiedIdentity`] cannot show.
    #[cfg(test)]
    pub(crate) fn stored_identity_blob(
        &self,
        id: &Identifier,
    ) -> std::result::Result<Option<Vec<u8>>, TaskError> {
        Ok(self
            .det_kv()?
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id.to_buffer()), IDENTITY_KEY)
            .map_err(identity_err)?
            .map(|stored| stored.qi_bytes))
    }

    /// The wallet link recorded for `id`, or `None` when the identity is not
    /// stored or was never linked to a wallet.
    ///
    /// This link — not [`QualifiedIdentity::associated_wallets`], which
    /// hydration fills with every loaded wallet — is what "this wallet owns
    /// this identity" means in DET. It lives beside the blob rather than inside
    /// it, so only a direct read sees it.
    pub(crate) fn stored_identity_wallet_link(
        &self,
        id: &Identifier,
    ) -> std::result::Result<Option<(WalletSeedHash, u32)>, TaskError> {
        Ok(self
            .det_kv()?
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id.to_buffer()), IDENTITY_KEY)
            .map_err(identity_err)?
            .and_then(|stored| Some((stored.wallet_hash?, stored.wallet_index?))))
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

    /// Remove a locally-stored identity and all of its Identity-scoped
    /// children. Returns `Ok(())` even when the identity is unknown —
    /// mirrors the pre-C7 `DELETE` which silently no-ops on missing rows.
    ///
    /// Cleanup verdict: explicit. DET never deletes the upstream
    /// `identities` row (that table is owned by the upstream sync layer;
    /// DET stores the qualified-identity blob in the `meta_identity` k/v
    /// scope only), so the upstream `cascade_meta_identity_on_identity_delete`
    /// trigger never fires for this path. This method therefore drains the
    /// Identity scope itself — the blob, the top-up history, and every
    /// scheduled vote queued for this identity — and removes the Global
    /// index entries that the trigger would not touch.
    pub fn delete_local_qualified_identity(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<(), TaskError> {
        let _migration_guard = self
            .migration_run
            .try_lock()
            .map_err(|_| TaskError::WalletStorageNotReady)?;
        if self.migration_status().state().is_in_progress() {
            return Err(TaskError::WalletStorageNotReady);
        }
        // Lock order: the storage-migration mutex above, then this identity's
        // record guard — the same order the legacy-recovery write takes.
        let lock = self.identity_record_lock(*identifier);
        let _record_guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kv = self.det_kv()?;
        let id = identifier.to_buffer();
        crate::backend_task::migration::finish_unwire::record_identity_deletion(self, id).map_err(
            |source| TaskError::IdentityDeletionMigrationRecord {
                source: Arc::new(source),
            },
        )?;
        self.clear_identity_vault_keys(&kv, &id)?;
        purge_identity_scope(&kv, &id)?;
        index_remove_identity(&kv, &id)
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
    ///
    /// The one whole-record write that does NOT take
    /// [`Self::identity_record_lock`]: it runs inside the *read* path
    /// ([`Self::hydrate_stored_identity`]), which a caller already holding that
    /// guard calls, so taking it here would self-deadlock. The write is
    /// idempotent — it rewrites the blob this call just read, replacing
    /// plaintext keys with vault placeholders — so a concurrent whole-record
    /// writer loses only the placeholder rewrite, and the next read redoes it.
    // TODO(#889 follow-up): fold this rewrite into the guarded write path (e.g.
    // by having the read return the migration for the caller to persist) so
    // every blob write is serialized, not merely every deliberate one.
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

    /// Delete the vault secrets filed at `placements` for `identity_id`, leaving
    /// the identity's other keys in place.
    ///
    /// The per-key counterpart of the whole-identity sweep
    /// [`Self::clear_identity_vault_keys`], for a caller dropping this device's
    /// copy of a single key. Call it *before* the placement leaves the stored
    /// key map: that map is where the sweep reads its delete set, so a secret
    /// orphaned by an earlier map eviction is reachable by nothing afterwards.
    ///
    /// Idempotent — a placement whose vault label is already absent is not an
    /// error, so a caller need not know whether the key was vault-backed.
    pub fn delete_identity_key_secrets(
        &self,
        identity_id: &Identifier,
        placements: impl IntoIterator<Item = (PrivateKeyTarget, KeyID)>,
    ) -> std::result::Result<(), TaskError> {
        crate::wallet_backend::IdentityKeyView::new(&self.secret_store, identity_id.to_buffer())
            .delete_all(placements)
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
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use DetKv;
    use std::sync::Arc;

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
    fn qi_with_plaintext_and_derived(
        secret_high: [u8; 32],
        secret_medium: [u8; 32],
    ) -> QualifiedIdentity {
        let pv = PlatformVersion::latest();
        let mut ks = KeyStorage::default();
        let high = IdentityPublicKey::random_key(1, Some(1), pv);
        ks.insert_at(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, high.id()),
            (
                QualifiedIdentityPublicKey::from(high),
                PrivateKeyData::Clear(secret_high),
            ),
        );
        let medium = IdentityPublicKey::random_key(2, Some(2), pv);
        ks.insert_at(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, medium.id()),
            (
                QualifiedIdentityPublicKey::from(medium),
                PrivateKeyData::AlwaysClear(secret_medium),
            ),
        );
        let derived = IdentityPublicKey::random_key(3, Some(3), pv);
        ks.insert_at(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, derived.id()),
            (
                QualifiedIdentityPublicKey::from(derived),
                PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                    wallet_seed_hash: [0x07; 32],
                    derivation_path: DerivationPath::from(vec![]),
                }),
            ),
        );
        let identity =
            Identity::create_basic_identity(Identifier::default(), pv).expect("basic identity");
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
                migrated.private_keys.values().all(|(_, d)| !matches!(
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
        for (_, d) in qi.private_keys.values() {
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
        ks.insert_at(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, existing.id()),
            (
                QualifiedIdentityPublicKey::from(existing),
                PrivateKeyData::InVault,
            ),
        );
        let added = IdentityPublicKey::random_key(2, Some(2), pv);
        let added_id = added.id();
        ks.insert_at(
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
        for (_, d) in decoded.private_keys.values() {
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
        ks.insert_at(
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

    /// An offline `AppContext` over a throwaway data dir, plus the very vault it
    /// was built on so a test can probe what the context wrote.
    async fn ctx_with_vault() -> (
        Arc<AppContext>,
        Arc<platform_wallet_storage::secrets::SecretStore>,
        tempfile::TempDir,
    ) {
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::utils::tasks::TaskManager;

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
            Arc::clone(&secret_store),
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext::new");
        (ctx, secret_store, temp_dir)
    }

    /// Per-key vault deletion drops the placements it is given and nothing else.
    /// That is what separates it from `clear_identity_vault_keys`, which empties
    /// the identity: dropping one key must leave the identity's remaining keys —
    /// and every other identity's — exactly where they were. Idempotent, because
    /// the remove path calls it without first knowing whether the label is there.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_identity_key_secrets_drops_only_the_named_placement() {
        const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;

        let (ctx, store, _dir) = ctx_with_vault().await;
        let victim = Identifier::from(id(0x61));
        let bystander = Identifier::from(id(0x62));
        for owner in [victim, bystander] {
            let view = IdentityKeyView::new(&store, owner.to_buffer());
            view.store(&MAIN, 0, &[0x01; 32]).unwrap();
            view.store(&MAIN, 1, &[0x02; 32]).unwrap();
        }

        ctx.delete_identity_key_secrets(&victim, [(MAIN, 0)])
            .expect("delete the named placement");

        let victim_view = IdentityKeyView::new(&store, victim.to_buffer());
        assert!(
            victim_view.get(&MAIN, 0).unwrap().is_none(),
            "the named placement's secret must be gone",
        );
        assert!(
            victim_view.get(&MAIN, 1).unwrap().is_some(),
            "the identity's other key must survive a single-key removal",
        );
        assert!(
            IdentityKeyView::new(&store, bystander.to_buffer())
                .get(&MAIN, 0)
                .unwrap()
                .is_some(),
            "another identity's key at the same placement must be untouched",
        );

        ctx.delete_identity_key_secrets(&victim, [(MAIN, 0)])
            .expect("deleting an already-gone placement is not an error");
    }
}
