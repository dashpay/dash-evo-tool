use super::AppContext;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

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

/// Durable manifest of vault-key placements still pending deletion for one
/// identity, keyed by that identity's id in the key itself.
/// [`DetScope::Global`] is deliberate, not incidental: `purge_identity_scope`
/// only ever mutates [`DetScope::Identity`], so a manifest filed there would
/// share fate with the very state a partial `purge_identity_scope` failure
/// can destroy — the one case this manifest exists to survive. Key shape:
/// `det:vault_cleanup_pending:v1:<identity_b58>`.
const VAULT_CLEANUP_PENDING_PREFIX: &str = "det:vault_cleanup_pending:v1:";

fn vault_cleanup_pending_key(id: &[u8; 32]) -> String {
    format!(
        "{VAULT_CLEANUP_PENDING_PREFIX}{}",
        Identifier::from(*id).to_string(Encoding::Base58)
    )
}

/// Recover the identity id named by a vault-cleanup manifest key, for the
/// boot-time sweep that enumerates every manifest without already knowing
/// which identities they belong to. `None` for anything that is not a
/// well-formed manifest key — a corrupt or foreign entry is skipped rather
/// than treated as fatal, so it never blocks the sweep from resuming every
/// other pending cleanup.
fn parse_vault_cleanup_pending_key(key: &str) -> Option<[u8; 32]> {
    let suffix = key.strip_prefix(VAULT_CLEANUP_PENDING_PREFIX)?;
    Identifier::from_string(suffix, Encoding::Base58)
        .ok()
        .map(|id| id.to_buffer())
}

/// Serializable mirror of [`PrivateKeyTarget`] for the vault-cleanup
/// manifest. [`PrivateKeyTarget`] itself derives `bincode::{Encode, Decode}`
/// for the vault blob's own wire format, not `serde` — the k/v sidecar is
/// serde-based, so the manifest needs its own small serializable shape.
///
/// `DetKv` encodes values with `bincode::serde::encode_to_vec`, which is
/// positional (see `wallet_backend/kv.rs`'s `SCHEMA_VERSION` doc comment):
/// this enum's wire representation is its variants' *declaration order*,
/// not their names. Adding a variant is fine (compiler-caught at every
/// match site); reordering or removing an existing one silently re-labels
/// every already-persisted manifest entry to the wrong key target —
/// nothing decoding it would notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum StoredPrivateKeyTarget {
    Main,
    Voter,
    Operator,
}

impl From<PrivateKeyTarget> for StoredPrivateKeyTarget {
    fn from(target: PrivateKeyTarget) -> Self {
        match target {
            PrivateKeyTarget::PrivateKeyOnMainIdentity => Self::Main,
            PrivateKeyTarget::PrivateKeyOnVoterIdentity => Self::Voter,
            PrivateKeyTarget::PrivateKeyOnOperatorIdentity => Self::Operator,
        }
    }
}

impl From<StoredPrivateKeyTarget> for PrivateKeyTarget {
    fn from(target: StoredPrivateKeyTarget) -> Self {
        match target {
            StoredPrivateKeyTarget::Main => Self::PrivateKeyOnMainIdentity,
            StoredPrivateKeyTarget::Voter => Self::PrivateKeyOnVoterIdentity,
            StoredPrivateKeyTarget::Operator => Self::PrivateKeyOnOperatorIdentity,
        }
    }
}

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

/// What an identity's stored record already claims about its wallet scope.
///
/// Read by [`AppContext::stored_wallet_scope`], whose docs explain why the
/// wallet-free and no-record cases must not be answered alike.
#[derive(Debug)]
enum StoredWalletScope {
    /// A record naming the wallet that owns this identity, and its index in it.
    Linked(WalletSeedHash, u32),
    /// A record that already claims this identity belongs to no wallet.
    WalletLess,
    /// No record, or one whose wallet fields do not amount to a link — either
    /// way, nothing on file says where this identity belongs.
    Unestablished,
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

/// Serializes every read-modify-write of [`IDENTITY_INDEX_KEY`].
///
/// The roster is a single blob rewritten wholesale, so two identities being
/// listed or delisted at the same time race on it: each pass reads the whole
/// index, edits its own entry, and writes the result back over its peer's.
/// The per-identity `identity_record_lock` cannot close that window — it
/// serializes writers of the *same* identity, while this race is between
/// writers of *different* ones.
///
/// A lost entry is not cosmetic. [`AppContext::resume_pending_vault_cleanups`]
/// treats absence from this roster as proof an identity was removed and
/// deletes its vault keys, so an import whose write is clobbered hands a live
/// identity's private keys to the next boot's sweep. One process-wide lock
/// rather than a per-context field because the mutators are free functions
/// reached from contexts and bare [`DetKv`] handles alike; the writes are
/// user-paced (import, removal), so serializing them globally costs nothing.
///
/// # Lock order
///
/// An identity's `identity_record_lock` is OUTER, this lock is INNER —
/// [`AppContext::insert_local_qualified_identity`] and
/// [`AppContext::delete_local_qualified_identity`] both already hold the
/// record lock when their index write takes this one, so that is the order
/// every path must keep. Never acquire a record lock while holding this one,
/// and never hold it across an `await` or any call that can reach the index
/// again — `std::sync::Mutex` is not reentrant, so a second acquisition on
/// this thread self-deadlocks. Only
/// [`AppContext::delete_all_local_qualified_identities_in_devnet`] holds it
/// across other work, and every call it makes there (`clear_identity_vault_keys`,
/// `purge_identity_scope`) touches the vault and Identity-scoped keys only,
/// never the roster and never a record lock.
static IDENTITY_INDEX_LOCK: Mutex<()> = Mutex::new(());

/// Threads currently blocked acquiring [`IDENTITY_INDEX_LOCK`].
///
/// Test-only instrumentation, and the thing that makes serialization
/// *observable* rather than merely inferable from elapsed time: a thread
/// counted here has reached the roster read and cannot proceed, which is
/// precisely what a timing-based test can only guess at. Read by the
/// rendezvous fixture in the roster race tests.
#[cfg(test)]
pub(crate) static IDENTITY_INDEX_LOCK_CONTENDERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Acquire [`IDENTITY_INDEX_LOCK`]. A poisoned lock guards no invariant of its
/// own — the k/v store holds the state — so the guard is taken regardless.
fn lock_identity_index() -> MutexGuard<'static, ()> {
    #[cfg(test)]
    IDENTITY_INDEX_LOCK_CONTENDERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let guard = IDENTITY_INDEX_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    #[cfg(test)]
    IDENTITY_INDEX_LOCK_CONTENDERS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    guard
}

/// Read the Global identity-id enumeration index. Returns an empty
/// vector when the index has never been written.
///
/// Callers that go on to write the index back, or that act irreversibly on
/// the answer, must hold [`lock_identity_index`] across both halves.
fn load_identity_index(kv: &DetKv) -> std::result::Result<Vec<[u8; 32]>, TaskError> {
    Ok(kv
        .get::<Vec<[u8; 32]>>(DetScope::Global, IDENTITY_INDEX_KEY)
        .map_err(identity_err)?
        .unwrap_or_default())
}

/// Add `identity_id` to the Global enumeration index if absent. No-op
/// when the id is already tracked, so repeated inserts stay idempotent.
fn index_add_identity(kv: &DetKv, identity_id: &[u8; 32]) -> std::result::Result<(), TaskError> {
    let _index_guard = lock_identity_index();
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
    let _index_guard = lock_identity_index();
    let mut index = load_identity_index(kv)?;
    let before = index.len();
    index.retain(|id| id != identity_id);
    if index.len() == before {
        return Ok(());
    }
    kv.put(DetScope::Global, IDENTITY_INDEX_KEY, &index)
        .map_err(identity_err)
}

/// Run the irreversible tail of an identity removal: delete `vault_keys`
/// from the vault, then drop the manifest that recorded them.
///
/// Returns `Err` only while key material may still be on the device. A vault
/// delete that fails is propagated, never swallowed — leaving keys on a device
/// the user asked to clear is the one part of a removal they must not be told
/// succeeded. A manifest clear that fails afterwards is not that: the keys are
/// already gone and only Global bookkeeping is stale, so it is logged and the
/// removal counts as complete. Reporting it as a failure would tell the user
/// their private keys are still here — the opposite of what happened — and
/// would surface a cleanup-pending warning for nothing. The stale manifest is
/// harmless: the next boot's sweep re-runs the same idempotent deletes and
/// clears it.
fn finish_vault_cleanup(
    kv: &DetKv,
    secret_store: &Arc<platform_wallet_storage::secrets::SecretStore>,
    id: &[u8; 32],
    vault_keys: impl IntoIterator<Item = (PrivateKeyTarget, KeyID)>,
) -> std::result::Result<(), TaskError> {
    crate::wallet_backend::IdentityKeyView::new(secret_store, *id).delete_all(vault_keys)?;
    if let Err(error) = kv
        .delete(DetScope::Global, &vault_cleanup_pending_key(id))
        .map_err(identity_err)
    {
        tracing::warn!(
            identity = %Identifier::from(*id),
            %error,
            "Identity keys deleted but their cleanup manifest could not be cleared; the next boot re-runs the deletes and clears it"
        );
    }
    Ok(())
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
    /// One condition on that overwrite: `wallet_hash: None` is written against
    /// a *verified* unowned mirror — [`Self::mirror_identity_unowned`]
    /// returning `Ok`, which means the wallet store was read back and holds
    /// the wallet-free row — or, failing that, only where the record already
    /// claims to be wallet-free (the branches below, which is where an
    /// unverified rewrite is confined). Upstream files one row per identity
    /// and will not re-scope a wallet-owned one, so a `wallet_hash: None`
    /// record written without that proof may sit under some wallet's
    /// `ON DELETE CASCADE` while claiming to be out of its reach — removing
    /// that wallet would then take this record with it.
    ///
    /// No mirror error carries that proof, so none of them is trusted with the
    /// claim. What the failure costs depends on what the identity's record
    /// ([`Self::stored_wallet_scope`]) already says:
    ///
    /// - **Linked to a wallet** — the link is kept against the caller's `None`
    ///   hint, and the user is told. The outcome differs from what was asked
    ///   for and only a retry (or loading the identity from its wallet)
    ///   changes it, because a kept association takes the record out of the
    ///   wallet-less set `AppContext::reconcile_unowned_identities` drives
    ///   off, so no boot revisits it. This is at worst a **mislabel, not a
    ///   loss**: if the mirror write was merely lost (upstream swallows a
    ///   persist failure into `Ok(())`, so a lost write and a refused one are
    ///   indistinguishable here) there is no upstream row for any `ON DELETE
    ///   CASCADE` to reach, and the identity keeps a wallet link it may not
    ///   deserve until the operation is retried.
    /// - **Already wallet-free** — the write proceeds, and once it lands the
    ///   user is told. It restates a claim the record already carries rather than
    ///   making one, so the field the mirror guards does not change and
    ///   refusing would only forfeit the rest of the write; the divergence it
    ///   leaves — DET calling the identity wallet-free where the wallet store
    ///   will not — outlives the write regardless, so it is reported rather
    ///   than left silent. Masternodes and evonodes are wallet-less by design
    ///   and refresh through here, so this is the common path.
    /// - **Nothing on file** — the write is refused with the mirror's own
    ///   error and nothing is persisted, since a wallet-free record here would
    ///   be this method's invention rather than anything it can stand on.
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
    ///
    /// # Errors
    /// The mirror's own error, when a wallet-free record cannot be shown to be
    /// durable and no earlier record stands in for the proof. Also whatever
    /// the vault write or the k/v writes below return.
    ///
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
        let mut wallet_link = match wallet_and_identity_id_info {
            Some((seed, idx)) => Some((*seed, *idx)),
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
                None
            }
        };
        let identity_id = qualified_identity.identity.id();
        let id = identity_id.to_buffer();
        // Mirror first: its outcome decides what to write (see this method's
        // docs), and a refusal returns before the vault write below rather than
        // stranding placeholders no record points at. An orphan mirror left by
        // a failure of the writes below is withdrawn by the next boot's
        // reconcile.
        let mut unconfirmed_wallet_free = None;
        if wallet_link.is_none()
            && let Err(error) = self.mirror_identity_unowned(qualified_identity)
        {
            match self.stored_wallet_scope(&identity_id) {
                Ok(StoredWalletScope::Linked(seed, index)) => {
                    wallet_link = Some((seed, index));
                    // Logged here rather than left to the banner: a banner logs
                    // only when a frame renders it, which never happens headless
                    // (MCP/CLI) and need not happen in the GUI, where it is an
                    // evictable toast. No boot revisits this record, so it gets
                    // the durable record and a banner that stays put.
                    tracing::warn!(
                        identity_id = %identity_id,
                        wallet = %hex::encode(seed),
                        error = %error,
                        "Identity kept its wallet link: the wallet store could not confirm a wallet-free row for it, so a wallet-free record would not be durable. Retry, or load the identity from the wallet that holds it."
                    );
                    let banner =
                        MessageBanner::set_global(self.egui_ctx(), &error, MessageType::Warning);
                    banner.with_details(error);
                    banner.disable_auto_dismiss();
                    self.egui_ctx().request_repaint();
                }
                // The record already claims no wallet, so this write restates
                // that claim rather than making it. Nothing is at stake in the
                // field the mirror guards, and refusing would forfeit the rest
                // of the write to repair a divergence it cannot repair. Said
                // out loud all the same — accepted, not unnoticed — because no
                // boot revisits an accepted write.
                Ok(StoredWalletScope::WalletLess) => {
                    tracing::warn!(
                        identity_id = %identity_id,
                        error = %error,
                        "Identity already recorded as belonging to no wallet; storing it again leaves that unchanged, but the wallet store still could not confirm a wallet-free row for it. Retry, or restart the application."
                    );
                    // Reported once the record is on disk, not here: the writes
                    // below can still fail.
                    unconfirmed_wallet_free = Some(error);
                }
                // Nothing on file to restate, so a wallet-free record here
                // would be this method's own invention. Refuse rather than
                // persist a claim the wallet store contradicts.
                Ok(StoredWalletScope::Unestablished) => {
                    tracing::warn!(
                        identity_id = %identity_id,
                        error = %error,
                        "Identity not stored: the wallet store could not confirm a wallet-free row for it, and no earlier wallet scope is on file to stand on. Retry, or load the identity from the wallet that holds it."
                    );
                    return Err(error);
                }
                // Two storage failures, one cause worth reporting: the mirror
                // is why a wallet-free record is in question at all, so the
                // readback that also failed is logged rather than returned.
                Err(readback_error) => {
                    tracing::warn!(
                        identity_id = %identity_id,
                        error = %error,
                        readback_error = %readback_error,
                        "Identity not stored: neither the wallet store nor this identity's own record could be read. Retry, or load the identity from the wallet that holds it."
                    );
                    return Err(error);
                }
            }
        }
        // Vault-first: move any plaintext keys into the vault before encoding, so
        // the at-rest blob carries only `InVault` placeholders. A vault-write
        // failure aborts the insert (nothing is persisted).
        let qi_bytes =
            encode_identity_blob_vault_first(&self.secret_store, &id, qualified_identity)?;
        let (wallet_hash, wallet_index) = match wallet_link {
            Some((seed, idx)) => (Some(seed), Some(idx)),
            None => (None, None),
        };
        let stored = StoredQualifiedIdentity {
            qi_bytes,
            status: qualified_identity.status.as_u8(),
            identity_type: qualified_identity.identity_type.as_tag().to_string(),
            wallet_hash,
            wallet_index,
        };
        index_add_identity(&kv, &id)?;
        kv.put(DetScope::Identity(&id), IDENTITY_KEY, &stored)
            .map_err(identity_err)?;
        // Raised only now, because the writes above can still fail. Carries no
        // identity id — the tray holds five, this banner never auto-dismisses
        // and dedup is by exact text, so one per identity would evict whatever
        // the user was reading. The id is in the log line above.
        if let Some(error) = unconfirmed_wallet_free {
            let banner = MessageBanner::set_global(
                self.egui_ctx(),
                "An identity was saved, but this device's wallet data does not yet confirm that \
                 it belongs to no wallet. Restart the application to complete the update.",
                MessageType::Warning,
            );
            banner.with_details(error);
            banner.disable_auto_dismiss();
            self.egui_ctx().request_repaint();
        }
        Ok(())
    }

    /// Test-only: write a wallet-less identity's sidecar record WITHOUT the
    /// upstream unowned-scope mirror [`Self::insert_local_qualified_identity`]
    /// always performs for one. Simulates the genuine pre-#955 on-disk shape
    /// — a sidecar record that predates the mirror existing at all — which a
    /// [`WalletBackend::remove_unowned_identity`](crate::wallet_backend::WalletBackend::remove_unowned_identity)
    /// call cannot: that leaves a *tombstoned* upstream row (an add-then-
    /// remove path), not the *absent* row (a row that was never added) an
    /// upgrading pre-#955 install actually has, and upstream's upsert may
    /// treat reviving a tombstone differently from a first insert.
    #[cfg(test)]
    pub(crate) fn insert_local_qualified_identity_sidecar_only(
        &self,
        qualified_identity: &QualifiedIdentity,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let id = qualified_identity.identity.id().to_buffer();
        let qi_bytes =
            encode_identity_blob_vault_first(&self.secret_store, &id, qualified_identity)?;
        let stored = StoredQualifiedIdentity {
            qi_bytes,
            status: qualified_identity.status.as_u8(),
            identity_type: qualified_identity.identity_type.as_tag().to_string(),
            wallet_hash: None,
            wallet_index: None,
        };
        index_add_identity(&kv, &id)?;
        kv.put(DetScope::Identity(&id), IDENTITY_KEY, &stored)
            .map_err(identity_err)
    }

    /// Mirror a wallet-less identity into the wallet store's unowned scope, so
    /// upstream has a durable record of it instead of only DET's own sidecar
    /// holding it. Every wallet-less identity is mirrored here, not only
    /// masternodes/evonodes — a `User` identity DET loaded without an owning
    /// wallet (the case the warning above flags as unexpected) takes the same
    /// path.
    ///
    /// `Ok` is the only proof [`Self::insert_local_qualified_identity`] accepts
    /// that a wallet-free record would be durable — no failure distinguishes a
    /// refused row from a lost write — so the error is passed through for that
    /// caller to report rather than reduced to a flag. The mirrored row carries
    /// none of `qualified_identity`'s public keys — see
    /// [`WalletBackend::ensure_identity_unowned`](crate::wallet_backend::WalletBackend::ensure_identity_unowned)
    /// for why.
    ///
    /// A missing mirror is retried once. Of its two indistinguishable causes
    /// only one — upstream swallowing its own persist failure — is recoverable,
    /// and each attempt rebuilds its manager from a fresh read, so the second
    /// is a real attempt rather than a re-reading of the first. A refused row
    /// costs one extra no-op; a lost write costs the caller its insert, which
    /// is the trade this retry is priced against.
    fn mirror_identity_unowned(
        &self,
        qualified_identity: &QualifiedIdentity,
    ) -> std::result::Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        match backend.ensure_identity_unowned(&qualified_identity.identity) {
            Err(TaskError::UnownedIdentityMirrorMissing { .. }) => backend
                .ensure_identity_unowned(&qualified_identity.identity)
                .map(|_| ()),
            other => other.map(|_| ()),
        }
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

    /// The ids of every DET-known identity that belongs to no wallet — every
    /// sidecar entry without a `wallet_hash`. Masternode/evonode nodes are
    /// the expected case, but this partitions on `wallet_hash`, not
    /// `identity_type` — **not** the same set as
    /// [`Self::load_local_masternode_identities`], which partitions on
    /// `identity_type` and can both include a wallet-owned node and exclude
    /// a wallet-less `User` identity that this method would return.
    ///
    /// Cheap by design: reads only the un-hydrated [`StoredQualifiedIdentity`]
    /// wrapper for each id in the Global enumeration index — no blob decode,
    /// no vault key migration. Drives the boot reconcile
    /// (`AppContext::reconcile_unowned_identities`), which diffs this set
    /// against what's already registered upstream and hydrates
    /// (via [`Self::get_local_qualified_identity`]) only the ids it actually
    /// needs to act on — a steady-state boot with nothing to reconcile
    /// touches neither the blob decoder nor the secret vault.
    pub(crate) fn local_wallet_less_identity_ids(
        &self,
    ) -> std::result::Result<std::collections::BTreeSet<Identifier>, TaskError> {
        let kv = self.det_kv()?;
        let ids = load_identity_index(&kv)?;
        let mut out = std::collections::BTreeSet::new();
        for id in ids {
            let Some(stored) = kv
                .get::<StoredQualifiedIdentity>(DetScope::Identity(&id), IDENTITY_KEY)
                .map_err(identity_err)?
            else {
                continue;
            };
            if stored.wallet_hash.is_none() {
                out.insert(Identifier::from(id));
            }
        }
        Ok(out)
    }

    /// The masternode/evonode identities for the active network — the
    /// Masternodes-page card list and the page-scoped masternode pill source.
    /// The complement of [`Self::load_local_user_identities`] over the FR-6 type
    /// boundary (`identity_type`) — a different partition from
    /// [`Self::local_wallet_less_identity_ids`] (`wallet_hash`); the two
    /// sets need not match. Filters the hydrated full load, so each card's
    /// top-up history is available (unlike the pre-decode
    /// [`Self::load_local_voting_identities`], which is un-hydrated and named
    /// for the DPNS voting flows).
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

    /// Whether `id` is currently on the Global identity index — the roster
    /// every screen reads. Distinct from [`Self::get_local_qualified_identity`],
    /// which reads the per-identity blob: a [`Self::delete_local_qualified_identity`]
    /// failure can leave the blob already gone (an early `purge_identity_scope`
    /// step) while the index removal that actually delists the identity ran
    /// even earlier, so only the index membership answers "is this identity
    /// still reachable from the UI".
    pub(crate) fn is_identity_listed(
        &self,
        id: &Identifier,
    ) -> std::result::Result<bool, TaskError> {
        let kv = self.det_kv()?;
        Ok(load_identity_index(&kv)?.contains(&id.to_buffer()))
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

    /// The wallet scope `id`'s stored record already claims, as the three
    /// states [`Self::stored_identity_wallet_link`]'s `Option` collapses into
    /// two. Only [`Self::insert_local_qualified_identity`] needs them apart:
    /// "already wallet-free" is a claim it may restate unverified, while
    /// "nothing on file" is one it may not invent.
    ///
    /// A record holding a `wallet_hash` without a `wallet_index` names no
    /// position in that wallet and so cannot be kept as a link; it reads as
    /// [`StoredWalletScope::Unestablished`] rather than as wallet-free,
    /// because a `wallet_hash` is exactly what a wallet-free record must not
    /// have. No writer produces that pairing — the two fields are always set
    /// or cleared together — so this is a fail-closed reading, not a case.
    fn stored_wallet_scope(
        &self,
        id: &Identifier,
    ) -> std::result::Result<StoredWalletScope, TaskError> {
        let stored = self
            .det_kv()?
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id.to_buffer()), IDENTITY_KEY)
            .map_err(identity_err)?;
        Ok(match stored {
            Some(stored) => match (stored.wallet_hash, stored.wallet_index) {
                (Some(seed), Some(index)) => StoredWalletScope::Linked(seed, index),
                (None, _) => StoredWalletScope::WalletLess,
                (Some(_), None) => StoredWalletScope::Unestablished,
            },
            None => StoredWalletScope::Unestablished,
        })
    }

    /// Whether `id` is one of DET's wallet-less identities right now — the
    /// single-id form of [`Self::local_wallet_less_identity_ids`], and read
    /// the same way: on the enumeration index (the roster, so an id dropped
    /// from it is gone whatever its blob still says) and then on the record's
    /// `wallet_hash`.
    ///
    /// The boot reconcile re-checks this before withdrawing an upstream
    /// unowned registration: an identity stored after its id scan must keep
    /// its registration, while one that has since gained a wallet must still
    /// lose it — a distinction [`Self::has_local_qualified_identity`] cannot
    /// make. Two wrapper reads, the roster and the record: no blob decode, no
    /// vault touch.
    pub(crate) fn stored_identity_is_wallet_less(
        &self,
        id: &Identifier,
    ) -> std::result::Result<bool, TaskError> {
        let kv = self.det_kv()?;
        let id_buf = id.to_buffer();
        if !load_identity_index(&kv)?.contains(&id_buf) {
            return Ok(false);
        }
        Ok(kv
            .get::<StoredQualifiedIdentity>(DetScope::Identity(&id_buf), IDENTITY_KEY)
            .map_err(identity_err)?
            .is_some_and(|stored| stored.wallet_hash.is_none()))
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
    /// Cleanup verdict: explicit. DET never issues a row `DELETE` against the
    /// upstream `identities` table (that table is owned by the upstream sync
    /// layer; DET stores the qualified-identity blob in the `meta_identity`
    /// k/v scope only), so the upstream `cascade_meta_on_identity_delete`
    /// trigger — which fires on `DELETE`, not `UPDATE` — never reaches this
    /// path. This method therefore drains the Identity scope itself — the
    /// blob, the top-up history, and every scheduled vote queued for this
    /// identity — and removes the Global index entries that the trigger
    /// would not touch.
    ///
    /// For a wallet-less identity this also *tombstones* (never row-deletes)
    /// its mirrored row in the upstream unowned scope, via
    /// [`WalletBackend::remove_unowned_identity`](crate::wallet_backend::WalletBackend::remove_unowned_identity)
    /// below — so upstream stops advertising a node this device no longer
    /// has. Best-effort, and retried like registration is: a tombstone lost
    /// here is re-issued by the next boot's
    /// `AppContext::reconcile_unowned_identities`, which withdraws every
    /// unowned registration whose sidecar record is gone.
    ///
    /// **`Err` does not mean "nothing happened".** The Global index removal
    /// runs before the irreversible vault-key delete, so a failure can land
    /// strictly after `identifier` is already gone from every screen — a
    /// durable vault-cleanup manifest survives that and the next boot's
    /// [`Self::resume_pending_vault_cleanups`] finishes the job regardless.
    /// A caller that turns this `Err` straight into a user-facing "removal
    /// failed, please retry" message is wrong in exactly that case: the
    /// identity cannot be retried (it is unlisted) and nothing is actually
    /// still broken. Call [`Self::is_identity_listed`] first to tell the two
    /// outcomes apart — see `AppContext::remove_identity` (in
    /// `backend_task/identity/remove_identity.rs`) for the reference pattern.
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
        // Ordering is a safety property. The vault delete is the only step
        // nothing can undo — Platform can re-supply the identity, but no one
        // can re-supply its keys — so it runs last, once the identity is
        // already unlisted and drained. Its delete set is read up front,
        // because the blob `purge_identity_scope` drops is where that set is
        // recorded.
        //
        // `purge_identity_scope` is itself not atomic (three independent k/v
        // writes), so a failure inside it — after its own first write has
        // already dropped the blob — would leave a retry with nothing to
        // re-derive the delete set from. The manifest below is what survives
        // that: persisted before any mutation runs, retained across every
        // error, and cleared only once every listed key is confirmed absent.
        let vault_keys = self.pending_vault_key_placements(&kv, &id)?;
        self.persist_vault_cleanup_manifest(&kv, &id, &vault_keys)?;
        index_remove_identity(&kv, &id)?;
        purge_identity_scope(&kv, &id)?;
        finish_vault_cleanup(&kv, &self.secret_store, &id, vault_keys)?;
        // Mirror the removal into the wallet store's unowned scope, so a
        // deleted node does not linger there. Best-effort, and a no-op for a
        // wallet-owned identity, which is never registered unowned.
        if let Ok(backend) = self.wallet_backend()
            && let Err(error) = backend.remove_unowned_identity(identifier)
        {
            tracing::debug!(
                identity_id = %identifier,
                %error,
                "Deleted identity still registered with the wallet store"
            );
        }
        Ok(())
    }

    /// Boot-time sweep for vault-cleanup manifests left behind by a
    /// [`Self::delete_local_qualified_identity`] call that failed after
    /// `index_remove_identity` had already run. Once an identity leaves the
    /// Global index it renders on no screen, so nothing in the UI can ever
    /// call that method again for it — the manifest, and this sweep, are the
    /// only surviving path back to the orphaned vault keys.
    ///
    /// Resumes a manifest only while its identity is absent from that index,
    /// re-checked fresh under that identity's record lock (see below) rather
    /// than from one snapshot read before the loop — a snapshot would miss a
    /// concurrent re-import that lists the identity again mid-sweep. A
    /// manifest whose identity is listed belongs to a removal that never
    /// reached the irreversible step, or to an identity re-imported since:
    /// either way the identity is live and the user keeps a working retry, so
    /// deleting its keys here would strand exactly the identity this ordering
    /// exists to protect. That absence is only trustworthy because every
    /// mutation of the roster is serialized ([`lock_identity_index`]) and
    /// every mutation *of this identity* is serialized against this sweep by
    /// its record lock: without the first, another identity's concurrent
    /// import could silently drop this one's entry and fake the very evidence
    /// the delete below acts on.
    ///
    /// Also re-runs `purge_identity_scope` before deleting vault keys: the
    /// manifest is persisted one step *before* that purge, so a crash between
    /// the two leaves it incomplete, and every one of its steps is a delete-
    /// if-present or list-then-conditional-prune, so re-running it against an
    /// already-purged scope is a safe no-op.
    ///
    /// Best-effort and idempotent, like every other boot reconcile
    /// ([`super::wallet_lifecycle::bootstrap`]'s unowned-identity pass): a
    /// failure on one manifest is logged and retried next boot, and never
    /// blocks the sweep from resuming every other one.
    ///
    /// Guarded by the same `migration_run` lock and in-progress check as
    /// [`Self::delete_local_qualified_identity`]: a storage migration can be
    /// mid-rewrite of this same Identity scope around the same boot window
    /// this sweep runs in, and the sweep's purge/vault-delete pair is not
    /// safe to interleave with that.
    pub(crate) fn resume_pending_vault_cleanups(&self) {
        let Ok(_migration_guard) = self.migration_run.try_lock() else {
            tracing::debug!(
                "Pending vault-cleanup sweep skipped; a storage migration is running, will retry at next boot"
            );
            return;
        };
        if self.migration_status().state().is_in_progress() {
            tracing::debug!(
                "Pending vault-cleanup sweep skipped; a storage migration is in progress, will retry at next boot"
            );
            return;
        }
        let kv = match self.det_kv() {
            Ok(kv) => kv,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "Pending vault-cleanup sweep skipped; k/v store not ready, will retry at next boot"
                );
                return;
            }
        };
        let keys = match kv.list(DetScope::Global, Some(VAULT_CLEANUP_PENDING_PREFIX)) {
            Ok(keys) => keys,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "Pending vault-cleanup sweep skipped; listing manifests failed, will retry at next boot"
                );
                return;
            }
        };
        let mut resumed = 0usize;
        for key in keys {
            let Some(id) = parse_vault_cleanup_pending_key(&key) else {
                tracing::warn!(%key, "Skipping an unparsable vault-cleanup manifest key");
                continue;
            };
            // Held through the roster re-check, the purge, the vault delete,
            // and the manifest clear — the same lock `insert_local_qualified_identity`
            // takes before ever touching this identity's k/v, so a concurrent
            // re-import cannot land between the "still listed?" check below
            // and this sweep's own writes.
            let lock = self.identity_record_lock(Identifier::from(id));
            let _record_guard = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // The manifest is persisted one step *before* `index_remove_identity`,
            // so its presence alone does not mean the removal ever happened. An
            // unreadable index cannot prove any identity is gone, so resume nothing
            // rather than guess. Read fresh, under the lock, on every iteration —
            // never hoisted above the loop — so a re-import that lands between
            // manifests is always seen.
            let listed = match load_identity_index(&kv) {
                Ok(listed) => listed,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "Pending vault-cleanup sweep skipped; the identity index is unreadable, will retry at next boot"
                    );
                    return;
                }
            };
            if listed.contains(&id) {
                tracing::debug!(
                    identity = %Identifier::from(id),
                    "Pending vault-cleanup left alone; this identity is still listed and still usable, so removing it stays the user's call"
                );
                continue;
            }
            let placements: Vec<(StoredPrivateKeyTarget, KeyID)> =
                match kv.get(DetScope::Global, &key) {
                    Ok(Some(placements)) => placements,
                    // Raced with another clear of the same manifest; nothing left to do.
                    Ok(None) => continue,
                    Err(error) => {
                        tracing::warn!(
                            identity = %Identifier::from(id),
                            %error,
                            "Pending vault-cleanup manifest unreadable, will retry at next boot"
                        );
                        continue;
                    }
                };
            if let Err(error) = purge_identity_scope(&kv, &id) {
                tracing::warn!(
                    identity = %Identifier::from(id),
                    %error,
                    "Pending vault-cleanup deferred; scope purge incomplete, will retry at next boot"
                );
                continue;
            }
            let vault_keys = placements
                .into_iter()
                .map(|(target, key_id)| (target.into(), key_id));
            match finish_vault_cleanup(&kv, &self.secret_store, &id, vault_keys) {
                Ok(()) => resumed += 1,
                Err(error) => tracing::warn!(
                    identity = %Identifier::from(id),
                    %error,
                    "Pending vault-cleanup deferred; will retry at next boot"
                ),
            }
        }
        if resumed > 0 {
            tracing::info!(
                resumed,
                "Resumed vault-key cleanups left behind by an interrupted identity removal"
            );
        }
    }

    /// Test-only: remove `identifier` from the Global enumeration index
    /// without touching the upstream unowned scope or any other
    /// Identity-scoped data. Simulates a sidecar delete whose upstream
    /// tombstone never landed — e.g. a crash between
    /// [`Self::delete_local_qualified_identity`]'s sidecar drain and its
    /// [`WalletBackend::remove_unowned_identity`](crate::wallet_backend::WalletBackend::remove_unowned_identity)
    /// call, or the `wallet_backend()` guard above finding no backend wired
    /// yet.
    #[cfg(test)]
    pub(crate) fn remove_local_qualified_identity_from_index_only(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        index_remove_identity(&kv, &identifier.to_buffer())
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

    /// The vault placements holding `id`'s identity-key secrets, as recorded in
    /// its stored blob. Empty when the identity is not stored.
    ///
    /// Read this before anything drops the identity blob — the blob is the
    /// only record of which vault labels belong to `id`, so a delete set
    /// derived after it is gone is silently empty and strands every secret
    /// in the vault.
    fn identity_vault_key_placements(
        &self,
        kv: &DetKv,
        id: &[u8; 32],
    ) -> std::result::Result<std::collections::BTreeSet<(PrivateKeyTarget, KeyID)>, TaskError> {
        let Some(stored) = kv
            .get::<StoredQualifiedIdentity>(DetScope::Identity(id), IDENTITY_KEY)
            .map_err(identity_err)?
        else {
            return Ok(std::collections::BTreeSet::new());
        };
        Ok(decode_stored_identity(&stored.qi_bytes, self.network)?
            .private_keys
            .keys_set())
    }

    /// The full vault-key delete set for `id`: freshly-derived placements
    /// from the still-live blob (empty once it is gone), unioned with any
    /// manifest left behind by an earlier failed delete. The union — never a
    /// choice of one source over the other — means a placement discovered
    /// by either source is never dropped, whether this is a first attempt
    /// (manifest empty, blob live) or a retry after the blob was already
    /// purged (blob empty, manifest live).
    fn pending_vault_key_placements(
        &self,
        kv: &DetKv,
        id: &[u8; 32],
    ) -> std::result::Result<std::collections::BTreeSet<(PrivateKeyTarget, KeyID)>, TaskError> {
        let mut keys = self.identity_vault_key_placements(kv, id)?;
        let manifest: Vec<(StoredPrivateKeyTarget, KeyID)> = kv
            .get(DetScope::Global, &vault_cleanup_pending_key(id))
            .map_err(identity_err)?
            .unwrap_or_default();
        keys.extend(manifest.into_iter().map(|(t, k)| (t.into(), k)));
        Ok(keys)
    }

    /// Persist `keys` as the durable vault-cleanup manifest for `id`, so a
    /// failure anywhere between this call and the manifest clear in
    /// [`finish_vault_cleanup`] leaves a record a retry can recover from.
    fn persist_vault_cleanup_manifest(
        &self,
        kv: &DetKv,
        id: &[u8; 32],
        keys: &std::collections::BTreeSet<(PrivateKeyTarget, KeyID)>,
    ) -> std::result::Result<(), TaskError> {
        let stored: Vec<(StoredPrivateKeyTarget, KeyID)> =
            keys.iter().cloned().map(|(t, k)| (t.into(), k)).collect();
        kv.put(DetScope::Global, &vault_cleanup_pending_key(id), &stored)
            .map_err(identity_err)
    }

    /// Delete every identity-key raw secret for `id` from the vault.
    /// Idempotent when the identity or an individual vault label is absent.
    fn clear_identity_vault_keys(
        &self,
        kv: &DetKv,
        id: &[u8; 32],
    ) -> std::result::Result<(), TaskError> {
        let placements = self.identity_vault_key_placements(kv, id)?;
        crate::wallet_backend::IdentityKeyView::new(&self.secret_store, *id).delete_all(placements)
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
    ///
    /// Holds [`lock_identity_index`] across the whole wipe, the read and the
    /// index delete alike: an import that landed in between would have its
    /// roster entry dropped by the delete while its blob and vault keys
    /// survived — the unlisted-but-live shape the cleanup sweep destroys keys
    /// over.
    pub fn delete_all_local_qualified_identities_in_devnet(
        &self,
    ) -> std::result::Result<(), TaskError> {
        if self.network != Network::Devnet {
            return Ok(());
        }
        let kv = self.det_kv()?;
        let _index_guard = lock_identity_index();
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

/// Test-only staging fixtures for the identity-removal paths.
///
/// Lives outside `mod tests` so `backend_task::identity::remove_identity` can
/// stage the same on-disk shape this module's own tests do — the classifier it
/// owns reads exactly the state written here, and had no way to reach it while
/// these fixtures were private to this file's test module.
#[cfg(test)]
pub(crate) mod test_staging {
    use super::*;
    use crate::model::qualified_identity::encrypted_key_storage::{
        KeyStorage, PrivateKeyData, WalletDerivationPath,
    };
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType, PrivateKeyTarget};
    use crate::wallet_backend::IdentityKeyView;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::IdentityPublicKey;

    /// An offline `AppContext` over a throwaway data dir, plus the very vault it
    /// was built on so a test can probe what the context wrote.
    pub(crate) async fn ctx_with_vault() -> (
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

    /// A `QualifiedIdentity` carrying one `Clear` (HIGH), one `AlwaysClear`
    /// (MEDIUM), and one `AtWalletDerivationPath` key, under the default
    /// (all-zero) identity id.
    pub(crate) fn qi_with_plaintext_and_derived(
        secret_high: [u8; 32],
        secret_medium: [u8; 32],
    ) -> QualifiedIdentity {
        qi_with_plaintext_and_derived_at(Identifier::default(), secret_high, secret_medium)
    }

    /// [`qi_with_plaintext_and_derived`] under a caller-chosen identity id, for
    /// the fixtures that stage two related identities at once.
    pub(crate) fn qi_with_plaintext_and_derived_at(
        identity_id: Identifier,
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

    /// A stored identity whose plaintext keys the read path has already moved
    /// into the vault — the state a real delete runs against. Holds the temp dir
    /// and the event receiver so neither is dropped while the test runs.
    pub(crate) struct StagedIdentity {
        pub(crate) ctx: Arc<AppContext>,
        pub(crate) store: Arc<platform_wallet_storage::secrets::SecretStore>,
        pub(crate) id: Identifier,
        /// The associated voter identity, staged alongside `id` and named by
        /// its record. `None` unless the fixture staged a voter twin.
        pub(crate) voter_id: Option<Identifier>,
        _dir: tempfile::TempDir,
        _events: tokio::sync::mpsc::Receiver<crate::app::TaskResult>,
    }

    /// The two vault placements [`qi_with_plaintext_and_derived_at`] leaves
    /// behind once the read path has migrated its plaintext keys.
    const STAGED_PLACEMENTS: [(PrivateKeyTarget, dash_sdk::dpp::identity::KeyID); 2] = [
        (PrivateKeyTarget::PrivateKeyOnMainIdentity, 1),
        (PrivateKeyTarget::PrivateKeyOnMainIdentity, 2),
    ];

    /// An offline context with its wallet backend wired — `det_kv()` is only
    /// reachable once it is.
    async fn staged_context() -> (
        Arc<AppContext>,
        Arc<platform_wallet_storage::secrets::SecretStore>,
        tempfile::TempDir,
        tokio::sync::mpsc::Receiver<crate::app::TaskResult>,
    ) {
        let (ctx, store, dir) = ctx_with_vault().await;
        let (tx, events) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(32);
        ctx.ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
            tx,
            ctx.egui_ctx().clone(),
        ))
        .await
        .expect("wire the wallet backend offline");
        (ctx, store, dir, events)
    }

    /// Write `qi`'s blob, list it on the roster, and read it back once so the
    /// load path moves its plaintext keys into the vault. Asserts both keys
    /// landed there, since every fixture below is only meaningful if they did.
    fn stage_identity_record(
        ctx: &Arc<AppContext>,
        store: &Arc<platform_wallet_storage::secrets::SecretStore>,
        qi: &QualifiedIdentity,
    ) {
        let id = qi.identity.id();
        let id_buf = id.to_buffer();
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
        .expect("stage the identity blob");
        index_add_identity(&kv, &id_buf).expect("index the identity");
        ctx.get_local_qualified_identity(&id)
            .expect("hydrate the staged identity")
            .expect("identity present");

        let view = IdentityKeyView::new(store, id_buf);
        for (target, key_id) in STAGED_PLACEMENTS {
            assert!(
                view.get(&target, key_id).unwrap().is_some(),
                "precondition: key {key_id} must be in the vault before the delete"
            );
        }
    }

    /// One staged identity with no voter twin.
    pub(crate) async fn stage_identity_with_vaulted_keys(
        high: [u8; 32],
        medium: [u8; 32],
    ) -> StagedIdentity {
        let (ctx, store, dir, events) = staged_context().await;
        let qi = qi_with_plaintext_and_derived(high, medium);
        stage_identity_record(&ctx, &store, &qi);
        StagedIdentity {
            ctx,
            store,
            id: qi.identity.id(),
            voter_id: None,
            _dir: dir,
            _events: events,
        }
    }

    /// Break the Global scheduled-vote voter index, so the next
    /// `delete_local_qualified_identity` fails at the last step of
    /// `purge_identity_scope` — strictly *after* `index_remove_identity` has
    /// already delisted the identity. The reachable shape of a removal that
    /// failed past its point of no return: the identity is gone from every
    /// screen, and only its vault cleanup is outstanding.
    pub(crate) fn fail_removals_after_delisting(ctx: &Arc<AppContext>) {
        ctx.det_kv()
            .expect("identity kv")
            .put(
                DetScope::Global,
                SCHEDULED_VOTE_VOTERS_KEY,
                &"not a voter index".to_string(),
            )
            .expect("corrupt the scheduled-vote voter index");
    }

    /// Break `identity_id`'s vault-cleanup manifest slot, so its next
    /// `delete_local_qualified_identity` fails while reading the delete set —
    /// before any mutation, and before the identity is delisted. The reachable
    /// shape of a removal that never happened and can still be retried.
    pub(crate) fn fail_removal_before_delisting(ctx: &Arc<AppContext>, identity_id: &Identifier) {
        ctx.det_kv()
            .expect("identity kv")
            .put(
                DetScope::Global,
                &vault_cleanup_pending_key(&identity_id.to_buffer()),
                &"not a cleanup manifest".to_string(),
            )
            .expect("corrupt the vault-cleanup manifest");
    }

    /// An evonode identity whose record names a separate voter identity, with
    /// both staged and both holding vaulted keys — the shape
    /// `AppContext::remove_identity` walks when one removal has to delete two
    /// identities.
    pub(crate) async fn stage_identity_with_voter_twin(
        high: [u8; 32],
        medium: [u8; 32],
    ) -> StagedIdentity {
        let (ctx, store, dir, events) = staged_context().await;

        let voter = qi_with_plaintext_and_derived_at(Identifier::from([0x2A; 32]), high, medium);
        stage_identity_record(&ctx, &store, &voter);

        let mut primary =
            qi_with_plaintext_and_derived_at(Identifier::from([0x1B; 32]), high, medium);
        primary.identity_type = IdentityType::Evonode;
        primary.associated_voter_identity = Some((
            voter.identity.clone(),
            IdentityPublicKey::random_key(4, Some(4), PlatformVersion::latest()),
        ));
        stage_identity_record(&ctx, &store, &primary);

        StagedIdentity {
            ctx,
            store,
            id: primary.identity.id(),
            voter_id: Some(voter.identity.id()),
            _dir: dir,
            _events: events,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_staging::*;
    use super::*;
    use crate::wallet_backend::kv_test_support::{FailingKv, InMemoryKv, RendezvousKv};
    use DetKv;
    use std::sync::Arc;

    /// A store whose reads are held until every racing reader has snapshotted
    /// — or until the missing ones are provably stuck acquiring the roster
    /// lock — plus the [`DetKv`] over it. Arm it once the test's setup writes
    /// are done; until then reads pass straight through.
    /// Peers parked on the roster lock: they have reached the read and cannot
    /// proceed, so a reader waiting on them can stop waiting.
    fn roster_lock_contenders() -> usize {
        IDENTITY_INDEX_LOCK_CONTENDERS.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn rendezvous_kv() -> (Arc<RendezvousKv>, DetKv) {
        let store = Arc::new(RendezvousKv::default());
        let kv = DetKv::from_store(store.clone());
        (store, kv)
    }

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

    /// A roster whose entries can be lost is a roster that authorizes an
    /// irreversible delete on false evidence: `resume_pending_vault_cleanups`
    /// reads "absent from the index" as proof an identity was removed and
    /// destroys its vault keys. The index is a single blob rewritten
    /// wholesale, so listing two identities at once is a read-modify-write
    /// race — and the entry that loses it belongs to an identity that is
    /// live, listed everywhere else, and about to lose its private keys on
    /// the next boot. Every mutation of the key must be serialized.
    ///
    /// The rendezvous store makes that failure certain rather than likely:
    /// both adds are released only once both hold the pre-add roster, so
    /// unserialized code loses an entry on every run, on every runner.
    #[test]
    fn concurrently_listing_two_identities_keeps_both_on_the_roster() {
        let (store, kv) = rendezvous_kv();
        store.arm(2, roster_lock_contenders);

        std::thread::scope(|scope| {
            scope.spawn(|| index_add_identity(&kv, &id(1)).expect("list the re-imported identity"));
            scope.spawn(|| index_add_identity(&kv, &id(2)).expect("list the imported identity"));
        });

        let mut listed = load_identity_index(&kv).unwrap();
        listed.sort_unstable();
        assert_eq!(
            listed,
            vec![id(1), id(2)],
            "neither identity may lose its roster entry to the other's write"
        );
    }

    /// The same race in its mixed form, which corrupts the roster in both
    /// directions at once: the removal can be undone (a delisted identity
    /// reappears on screen with its keys already deleted) or the addition can
    /// be dropped (a live identity is handed to the cleanup sweep). Only one
    /// of the two writes survives unless they are serialized.
    #[test]
    fn a_concurrent_listing_and_delisting_both_take_effect() {
        let (store, kv) = rendezvous_kv();
        index_add_identity(&kv, &id(1)).unwrap();
        index_add_identity(&kv, &id(2)).unwrap();
        // Armed only now: the two seeding writes above are sequential, and an
        // armed read waits for a peer that would never come.
        store.arm(2, roster_lock_contenders);

        std::thread::scope(|scope| {
            scope
                .spawn(|| index_remove_identity(&kv, &id(1)).expect("delist the removed identity"));
            scope.spawn(|| index_add_identity(&kv, &id(3)).expect("list the imported identity"));
        });

        let mut listed = load_identity_index(&kv).unwrap();
        listed.sort_unstable();
        assert_eq!(
            listed,
            vec![id(2), id(3)],
            "the removal must not resurrect the delisted identity, and the \
             import must not vanish from the roster"
        );
    }

    // ---------------------------------------------------------------
    // StoredPrivateKeyTarget: wire encoding is positional, not named.
    // ---------------------------------------------------------------

    /// `DetKv` encodes with `bincode::serde::encode_to_vec`, which is
    /// positional — a variant's wire identity is its declaration order, not
    /// its name. This pins today's order so a future reorder or removal
    /// fails loudly here instead of silently re-labelling every
    /// already-persisted vault-cleanup manifest entry to the wrong key
    /// target. A new variant appended at the end is fine and needs no
    /// update; an insertion, removal, or reordering must update this test
    /// deliberately, with a migration plan for existing manifests.
    #[test]
    fn stored_private_key_target_wire_order_is_pinned() {
        for (variant, expected_index) in [
            (StoredPrivateKeyTarget::Main, 0u32),
            (StoredPrivateKeyTarget::Voter, 1u32),
            (StoredPrivateKeyTarget::Operator, 2u32),
        ] {
            let encoded =
                bincode::serde::encode_to_vec(variant, bincode::config::standard()).unwrap();
            // bincode's derive/serde-adapter encodes a unit-only enum's
            // discriminant as a `u32` varint prefix, with no payload after
            // it for a fieldless variant — decode that prefix directly
            // rather than asserting on raw bytes, which would be brittle
            // to bincode's own varint format.
            let (decoded_index, _): (u32, usize) =
                bincode::serde::decode_from_slice(&encoded, bincode::config::standard()).unwrap();
            assert_eq!(
                decoded_index, expected_index,
                "{variant:?} must encode at wire position {expected_index}"
            );
        }
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

    use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{IdentityType, PrivateKeyTarget};
    use crate::wallet_backend::IdentityKeyView;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, IdentityPublicKey};

    fn fresh_vault(dir: &std::path::Path) -> Arc<platform_wallet_storage::secrets::SecretStore> {
        let path = dir.join("secrets.pwsvault");
        Arc::new(crate::wallet_backend::single_key::open_secret_store(&path).expect("open vault"))
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

    /// Removal ordering is a safety property, not a style choice: the vault-key
    /// delete is the one irreversible step (Platform can re-supply the identity,
    /// nothing can re-supply the keys), so it must be unreachable until the
    /// identity is already gone from local storage. Failing a step in the middle
    /// of the delete must therefore leave every key where it was, rather than
    /// leaving a "zombie" — an identity still on file whose keys are gone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_identity_delete_never_destroys_the_vault_keys() {
        const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
        const HIGH: [u8; 32] = [0xAA; 32];
        const MEDIUM: [u8; 32] = [0xBB; 32];

        let staged = stage_identity_with_vaulted_keys(HIGH, MEDIUM).await;
        let kv = staged.ctx.det_kv().expect("identity kv");

        // Break the Global enumeration index so the delete fails partway: the
        // index read no longer decodes, which is the same shape as a store that
        // goes away mid-operation.
        kv.put(
            DetScope::Global,
            IDENTITY_INDEX_KEY,
            &"not an identity index".to_string(),
        )
        .expect("corrupt the enumeration index");

        let error = staged
            .ctx
            .delete_local_qualified_identity(&staged.id)
            .expect_err("an unreadable index must fail the delete");
        assert!(
            !matches!(error, TaskError::WalletStorageNotReady),
            "the delete must reach the index step, not stop at the migration guard: {error:?}"
        );

        let view = IdentityKeyView::new(&staged.store, staged.id.to_buffer());
        assert_eq!(
            *view
                .get(&MAIN, 1)
                .unwrap()
                .expect("the HIGH key must survive a failed delete"),
            HIGH,
        );
        assert_eq!(
            *view
                .get(&MAIN, 2)
                .unwrap()
                .expect("the MEDIUM key must survive a failed delete"),
            MEDIUM,
        );
        assert!(
            staged
                .ctx
                .stored_identity_blob(&staged.id)
                .expect("read the blob")
                .is_some(),
            "the identity record must survive a failed delete, so a retry still has something to delete"
        );
    }

    /// `is_identity_listed` is the primitive `remove_identity` uses to tell a
    /// benign "removed, cleanup still pending" failure apart from a real
    /// "never removed" one: it must track the Global index, not the blob —
    /// `purge_identity_scope`'s first step drops the blob before the index
    /// removal that actually delists the identity has even run for the case
    /// this distinction exists to catch (a failure strictly after delisting).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_identity_listed_tracks_the_index_not_the_blob() {
        let staged = stage_identity_with_vaulted_keys([0x77; 32], [0x88; 32]).await;
        let kv = staged.ctx.det_kv().expect("identity kv");

        assert!(
            staged
                .ctx
                .is_identity_listed(&staged.id)
                .expect("read listed state"),
            "a freshly staged identity must be listed"
        );

        // Drop the blob directly, leaving the index untouched — the inverse
        // of the failure window this helper exists to distinguish.
        kv.delete(DetScope::Identity(&staged.id.to_buffer()), IDENTITY_KEY)
            .expect("drop the blob");
        assert!(
            staged
                .ctx
                .is_identity_listed(&staged.id)
                .expect("read listed state"),
            "the index, not the blob, is authoritative: a blob-only removal \
             must still read as listed"
        );

        index_remove_identity(&kv, &staged.id.to_buffer()).expect("remove from the index");
        assert!(
            !staged
                .ctx
                .is_identity_listed(&staged.id)
                .expect("read listed state"),
            "once the index entry is gone, the identity must read as unlisted"
        );
    }

    /// The other half of the ordering contract. Deferring the vault delete must
    /// not quietly turn it into a no-op: the delete set is read from the blob,
    /// so reading it after `purge_identity_scope` drops that blob would strand
    /// every private key in the vault. A completed removal leaves nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_successful_identity_delete_leaves_no_orphaned_vault_key() {
        const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;

        let staged = stage_identity_with_vaulted_keys([0xCC; 32], [0xDD; 32]).await;

        staged
            .ctx
            .delete_local_qualified_identity(&staged.id)
            .expect("delete the staged identity");

        let view = IdentityKeyView::new(&staged.store, staged.id.to_buffer());
        for key_id in [1, 2] {
            assert!(
                view.get(&MAIN, key_id).unwrap().is_none(),
                "key {key_id} must not be stranded in the vault after a completed delete"
            );
        }
        assert!(
            staged
                .ctx
                .stored_identity_blob(&staged.id)
                .expect("read the blob")
                .is_none(),
            "the identity record must be gone after a completed delete"
        );
        assert!(
            !load_identity_index(&staged.ctx.det_kv().expect("identity kv"))
                .expect("read the index")
                .contains(&staged.id.to_buffer()),
            "the identity must be unlisted after a completed delete"
        );
    }

    /// `purge_identity_scope` is not atomic: it can fail on its own last step
    /// (pruning the scheduled-vote voter index) after its first step has
    /// already deleted `IDENTITY_KEY` — the blob `identity_vault_key_placements`
    /// needs to re-derive a delete set. Without a durable manifest, a retry
    /// after such a failure would read an empty placement set from the (now
    /// gone) blob and report success while every vault key stays orphaned.
    /// The manifest persisted before the first mutation must survive that
    /// failure and let a retry still find and delete every key.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_retry_after_purge_partially_fails_still_recovers_every_key() {
        const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
        const HIGH: [u8; 32] = [0xEE; 32];
        const LOW: [u8; 32] = [0xFF; 32];

        let staged = stage_identity_with_vaulted_keys(HIGH, LOW).await;
        let kv = staged.ctx.det_kv().expect("identity kv");

        // Break the Global scheduled-vote voter index so `purge_identity_scope`
        // fails at its LAST step (`delete_scheduled_votes_for_voter` ->
        // `remove_vote_voter_from_index`) — strictly after `IDENTITY_KEY` (the
        // blob) and `TOP_UPS_KEY` are already gone.
        kv.put(
            DetScope::Global,
            SCHEDULED_VOTE_VOTERS_KEY,
            &"not a voter index".to_string(),
        )
        .expect("corrupt the scheduled-vote voter index");

        let error = staged
            .ctx
            .delete_local_qualified_identity(&staged.id)
            .expect_err("a corrupt voter index must fail the delete");
        assert!(
            !matches!(error, TaskError::WalletStorageNotReady),
            "the delete must reach purge_identity_scope, not stop at the migration guard: {error:?}"
        );
        assert!(
            staged
                .ctx
                .stored_identity_blob(&staged.id)
                .expect("read the blob")
                .is_none(),
            "purge_identity_scope's first delete must already have removed the blob \
             before its own later step failed"
        );

        // Repair the index so the retry can actually complete.
        kv.delete(DetScope::Global, SCHEDULED_VOTE_VOTERS_KEY)
            .expect("repair the voter index");
        staged
            .ctx
            .delete_local_qualified_identity(&staged.id)
            .expect("the retry must complete now that the index is repaired");

        let view = IdentityKeyView::new(&staged.store, staged.id.to_buffer());
        for key_id in [1, 2] {
            assert!(
                view.get(&MAIN, key_id).unwrap().is_none(),
                "key {key_id} must not be stranded: the manifest from the failed \
                 attempt must have survived to inform the retry"
            );
        }
        assert!(
            kv.get::<Vec<(StoredPrivateKeyTarget, KeyID)>>(
                DetScope::Global,
                &vault_cleanup_pending_key(&staged.id.to_buffer())
            )
            .expect("read the manifest slot")
            .is_none(),
            "the manifest must be cleared once every key is confirmed deleted"
        );
    }

    /// The reachable form of the retry above: once `index_remove_identity`
    /// has run, the identity is gone from the Global index and therefore off
    /// every roster-backed screen — nothing in the UI can call
    /// `delete_local_qualified_identity` again for it. The manifest must be
    /// recoverable without that call: the boot-time sweep is the only
    /// reachable path back to keys orphaned this way.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resume_pending_vault_cleanups_recovers_a_manifest_no_ui_can_reach() {
        const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
        const HIGH: [u8; 32] = [0x11; 32];
        const LOW: [u8; 32] = [0x22; 32];

        let staged = stage_identity_with_vaulted_keys(HIGH, LOW).await;
        let kv = staged.ctx.det_kv().expect("identity kv");
        let id_buf = staged.id.to_buffer();

        // Simulate a crash between the manifest write and `purge_identity_scope`
        // ever starting — the manifest is durable and the identity is already
        // off the roster, but its blob, top-up history, and a scheduled vote
        // are all still sitting untouched in Identity scope. Built directly
        // (not via a failing `delete_local_qualified_identity` call) because
        // `purge_identity_scope`'s first two steps are unconditional deletes
        // that cannot be made to fail independently of its last step — going
        // through the real call would always leave the blob and top-ups
        // already gone, proving nothing about whether the sweep re-purges.
        let vault_keys = staged
            .ctx
            .identity_vault_key_placements(&kv, &id_buf)
            .expect("read the live placements before removing from the index");
        staged
            .ctx
            .persist_vault_cleanup_manifest(&kv, &id_buf, &vault_keys)
            .expect("persist the manifest");
        index_remove_identity(&kv, &id_buf).expect("remove from the roster");
        kv.put(
            DetScope::Identity(&id_buf),
            TOP_UPS_KEY,
            &std::collections::BTreeMap::from([(0u32, 5u64)]),
        )
        .expect("stage a top-up entry purge_identity_scope never reached");
        kv.put(
            DetScope::Identity(&id_buf),
            &scheduled_vote_key("alice"),
            &StoredScheduledVote {
                voter_id: id_buf,
                contested_name: "alice".to_string(),
                choice: StoredVoteChoice::Lock,
                unix_timestamp: 0,
                executed_successfully: false,
            },
        )
        .expect("stage a scheduled vote purge_identity_scope never reached");
        index_add_vote_voter(&kv, &id_buf).expect("add to the voter index");

        staged.ctx.resume_pending_vault_cleanups();

        let view = IdentityKeyView::new(&staged.store, id_buf);
        for key_id in [1, 2] {
            assert!(
                view.get(&MAIN, key_id).unwrap().is_none(),
                "key {key_id} must be recovered by the sweep even with no UI path left to reach it"
            );
        }
        assert!(
            staged
                .ctx
                .stored_identity_blob(&staged.id)
                .expect("read the blob")
                .is_none(),
            "the sweep's purge_identity_scope re-run must leave the blob gone"
        );
        assert!(
            kv.get::<std::collections::BTreeMap<u32, u64>>(
                DetScope::Identity(&id_buf),
                TOP_UPS_KEY
            )
            .expect("read top-ups")
            .is_none(),
            "the sweep must drain the top-up history left behind by the interrupted purge"
        );
        assert!(
            kv.list(DetScope::Identity(&id_buf), Some(SCHEDULED_VOTE_KEY_PREFIX))
                .expect("list scheduled votes")
                .is_empty(),
            "the sweep must drain any scheduled votes left behind by the interrupted purge"
        );
        assert!(
            load_scheduled_vote_voters(&kv)
                .expect("read the voter index")
                .is_empty(),
            "the sweep must prune this voter from the Global scheduled-vote index"
        );
        assert!(
            kv.get::<Vec<(StoredPrivateKeyTarget, KeyID)>>(
                DetScope::Global,
                &vault_cleanup_pending_key(&id_buf)
            )
            .expect("read the manifest slot")
            .is_none(),
            "the manifest must be cleared once the sweep confirms every key deleted"
        );
    }

    /// The sweep's mirror-image hazard. `index_remove_identity` runs *after*
    /// the manifest is persisted, so a failure in that write leaves a manifest
    /// behind for an identity that is still on the roster, still holding a live
    /// blob, and still fully usable. Resuming such a manifest would delete the
    /// keys of an identity the user can still see and was told was *not*
    /// removed — the exact zombie
    /// `a_failed_identity_delete_never_destroys_the_vault_keys` exists to
    /// forbid, arriving one boot later. The sweep must therefore resume only
    /// what the roster confirms is already gone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resume_pending_vault_cleanups_spares_an_identity_still_on_the_roster() {
        const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
        const HIGH: [u8; 32] = [0x33; 32];
        const LOW: [u8; 32] = [0x44; 32];

        let staged = stage_identity_with_vaulted_keys(HIGH, LOW).await;
        let kv = staged.ctx.det_kv().expect("identity kv");
        let id_buf = staged.id.to_buffer();

        // Fail the delete *at* `index_remove_identity`, after the manifest is
        // already durable: an unreadable index makes its own load step error.
        kv.put(
            DetScope::Global,
            IDENTITY_INDEX_KEY,
            &"not an identity index".to_string(),
        )
        .expect("corrupt the identity index");

        staged
            .ctx
            .delete_local_qualified_identity(&staged.id)
            .expect_err("index_remove_identity must fail on an unreadable index");
        assert!(
            kv.get::<Vec<(StoredPrivateKeyTarget, KeyID)>>(
                DetScope::Global,
                &vault_cleanup_pending_key(&id_buf)
            )
            .expect("read the manifest slot")
            .is_some(),
            "precondition: the failed delete must have left a manifest behind"
        );

        // The index write never landed, so on the next boot the identity is
        // still listed exactly as it was — the removal never happened.
        kv.put(DetScope::Global, IDENTITY_INDEX_KEY, &vec![id_buf])
            .expect("restore the untouched index");

        staged.ctx.resume_pending_vault_cleanups();

        let view = IdentityKeyView::new(&staged.store, id_buf);
        for key_id in [1, 2] {
            assert!(
                view.get(&MAIN, key_id).unwrap().is_some(),
                "key {key_id} belongs to an identity still on the roster; the sweep \
                 must not delete it"
            );
        }
        assert!(
            load_identity_index(&kv)
                .expect("read the index")
                .contains(&id_buf),
            "the identity must still be listed, making the UI retry path reachable"
        );
        assert!(
            kv.get::<Vec<(StoredPrivateKeyTarget, KeyID)>>(
                DetScope::Global,
                &vault_cleanup_pending_key(&id_buf)
            )
            .expect("read the manifest slot")
            .is_some(),
            "the manifest must be retained: its keys are not confirmed absent, so \
             clearing it here would discard the delete set a retry still needs"
        );
    }

    /// The sweep's "still listed?" check and a concurrent re-import's write
    /// both touch this identity's roster entry and vault keys; without a
    /// shared lock the two could interleave so the sweep deletes keys a
    /// re-import just wrote for a live identity. `identity_record_lock` — the
    /// same lock [`AppContext::insert_local_qualified_identity`] takes before
    /// touching this identity's k/v — must serialize the two: whichever one
    /// the sweep observes after acquiring it is the ground truth, so a
    /// re-import that lands first must be honored, never overwritten.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resume_pending_vault_cleanups_is_serialized_against_a_concurrent_reimport() {
        const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
        const HIGH: [u8; 32] = [0x55; 32];
        const LOW: [u8; 32] = [0x66; 32];

        let staged = stage_identity_with_vaulted_keys(HIGH, LOW).await;
        let kv = staged.ctx.det_kv().expect("identity kv");
        let id_buf = staged.id.to_buffer();

        // Leave a manifest behind for an unlisted identity — the sweep's
        // normal entry condition — via the same crash simulation as above:
        // the index write landed, but `purge_identity_scope` never got the
        // chance to run.
        let vault_keys = staged
            .ctx
            .identity_vault_key_placements(&kv, &id_buf)
            .expect("read the live placements before removing from the index");
        staged
            .ctx
            .persist_vault_cleanup_manifest(&kv, &id_buf, &vault_keys)
            .expect("persist the manifest");
        index_remove_identity(&kv, &id_buf).expect("remove from the roster");

        // Hold this identity's own record lock ourselves, simulating a
        // concurrent `insert_local_qualified_identity` re-import already in
        // flight when the boot sweep starts.
        let lock = staged.ctx.identity_record_lock(staged.id);
        let guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let sweep_ctx = Arc::clone(&staged.ctx);
        let sweep = std::thread::spawn(move || {
            sweep_ctx.resume_pending_vault_cleanups();
            let _ = done_tx.send(());
        });

        // While we hold the identity's record lock, the sweep must not be
        // able to finish at all — proving it genuinely blocks on the same
        // lock rather than racing straight through. 300ms is generous
        // headroom over how long an unblocked sweep of one manifest takes
        // (sub-millisecond), so this bound is not a source of flakiness.
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "the sweep must block on this identity's record lock while a \
             re-import holds it, not race ahead and delete its keys"
        );

        // The re-import completes: re-list the identity, then release the
        // lock — mirroring `insert_local_qualified_identity`'s own order of
        // operations under the same guard.
        index_add_identity(&kv, &id_buf).expect("re-list the identity");
        drop(guard);

        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the sweep must proceed and finish once the lock is released");
        sweep.join().expect("sweep thread must not panic");

        let view = IdentityKeyView::new(&staged.store, id_buf);
        for key_id in [1, 2] {
            assert!(
                view.get(&MAIN, key_id).unwrap().is_some(),
                "key {key_id} belongs to an identity the lock-holder re-listed \
                 before releasing; the sweep must see that fresh state — read \
                 after it acquires the lock, not before — and skip deleting it"
            );
        }
        assert!(
            load_identity_index(&kv)
                .expect("read the index")
                .contains(&id_buf),
            "the re-import must have won: the identity is listed again"
        );
    }

    /// A removal is reported by what happened to the keys, not by what happened
    /// to the bookkeeping. Once the vault delete lands the keys are gone, and a
    /// manifest clear that fails afterwards leaves only a stale Global record
    /// the next boot's sweep drops on its own. Propagating that failure would
    /// tell the user their private keys are still on this device — the opposite
    /// of the truth — and would raise a cleanup-pending warning for an identity
    /// with nothing left to clean up. The other half of the distinction, a
    /// cleanup that really is outstanding because the keys are still there, is
    /// asserted by `remove_identity`'s deferred-cleanup tests.
    #[test]
    fn a_manifest_clear_failure_after_the_keys_are_deleted_still_completes_the_removal() {
        const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;

        let dir = tempfile::tempdir().unwrap();
        let store = fresh_vault(dir.path());
        let failing = Arc::new(FailingKv::default());
        let kv = DetKv::from_store(failing.clone());
        let id_buf = id(1);
        let manifest_key = vault_cleanup_pending_key(&id_buf);

        let view = IdentityKeyView::new(&store, id_buf);
        view.store(&MAIN, 1, &[0x11; 32]).expect("vault the key");
        kv.put(
            DetScope::Global,
            &manifest_key,
            &vec![(StoredPrivateKeyTarget::from(MAIN), 1 as KeyID)],
        )
        .expect("stage the manifest the delete is meant to clear");

        failing.fail_deletes(true);
        finish_vault_cleanup(&kv, &store, &id_buf, [(MAIN, 1)])
            .expect("the keys are gone, so only bookkeeping failed: not a failed removal");

        assert!(
            view.get(&MAIN, 1).unwrap().is_none(),
            "the vault delete must have landed before the manifest clear was even attempted"
        );
        failing.fail_deletes(false);
        assert!(
            kv.get::<Vec<(StoredPrivateKeyTarget, KeyID)>>(DetScope::Global, &manifest_key)
                .expect("read the manifest slot")
                .is_some(),
            "the manifest must survive its failed clear, so the next boot's sweep still \
             finds it and re-runs the idempotent deletes"
        );
    }
}
