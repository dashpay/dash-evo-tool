//! Eligibility and merge rules for restoring identity data stranded in the
//! preserved legacy `data.db` (issue #889).
//!
//! The single source of truth for "what counts as genuinely missing": the
//! detection preview and the executing backend task both call these functions,
//! so what the user approves and what the task merges cannot drift. Pure — no
//! `AppContext`, `Sdk`, database, or vault.
//!
//! The absence-versus-deliberate-removal ambiguity is **not** resolved here.
//! [`compute_recovery_plan`] only *lists* what a legacy record holds and the
//! modern record does not; whether an item should come back is the user's
//! decision, carried into [`apply_recovery_plan`] as an item-level allowlist.

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Identity, KeyID, Purpose};
use dash_sdk::platform::IdentityPublicKey;

use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
use crate::model::qualified_identity::{PrivateKeyTarget, QualifiedIdentity};

/// One restorable piece of a legacy identity record.
///
/// A key is identified by its `(target, key_id)` map key — the exact key of
/// [`KeyStorage::private_keys`](crate::model::qualified_identity::encrypted_key_storage::KeyStorage::private_keys)
/// — so an item names the same slot in the legacy and the modern record. The
/// three association variants are the role links a masternode needs to present
/// and use its keys.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecoveryItem {
    /// A private key held at `(target, key_id)`.
    Key {
        target: PrivateKeyTarget,
        key_id: KeyID,
    },
    /// The `associated_voter_identity` link.
    VoterAssociation,
    /// The `associated_operator_identity` link.
    OperatorAssociation,
    /// The `associated_owner_key_id` link.
    OwnerKeyAssociation,
}

/// A [`RecoveryItem`] plus the public metadata a caller needs to label it.
///
/// Public data only: the key's on-chain [`Purpose`], never key bytes. This is
/// what travels in the task payload and reaches the UI, so a plan can be
/// previewed without any secret leaving the backend.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryItemDescriptor {
    /// The item this descriptor labels.
    pub item: RecoveryItem,
    /// The key's purpose, when the item is a key whose public metadata is
    /// known. `None` for associations and for a key present in neither record.
    pub purpose: Option<Purpose>,
}

impl RecoveryItemDescriptor {
    /// A key item, labelled from its public key metadata.
    fn key(target: PrivateKeyTarget, key_id: KeyID, purpose: Option<Purpose>) -> Self {
        Self {
            item: RecoveryItem::Key { target, key_id },
            purpose,
        }
    }

    /// An association item, which needs no key metadata to label.
    fn association(item: RecoveryItem) -> Self {
        Self {
            item,
            purpose: None,
        }
    }

    /// Whether this item is a key held on the identity's voter identity.
    ///
    /// The grouping invariant keys off the target, not the purpose: it is the
    /// target that decides which identity a key belongs to, and therefore
    /// whether the voter-identity link has to travel with it.
    pub fn is_on_voter_identity(&self) -> bool {
        matches!(
            &self.item,
            RecoveryItem::Key { target, .. } if *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity
        )
    }

    /// Whether this item plays the voting role, either because it sits on the
    /// voter identity or because its own purpose says so. Wider than
    /// [`Self::is_on_voter_identity`] — a voting-purpose key on the main
    /// identity also lets a node vote.
    pub fn is_voting_role(&self) -> bool {
        matches!(&self.item, RecoveryItem::Key { .. })
            && (self.is_on_voter_identity() || self.purpose == Some(Purpose::VOTING))
    }

    /// The key id this item names, or `None` for an association.
    pub fn key_id(&self) -> Option<KeyID> {
        match &self.item {
            RecoveryItem::Key { key_id, .. } => Some(*key_id),
            _ => None,
        }
    }
}

/// Why a legacy item this flow found cannot be restored by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionReason {
    /// The key uses the legacy per-key password envelope, which this version
    /// can neither open nor carry forward.
    LegacyEncryptedFormat,
    /// The legacy entry holds no key material to restore.
    NoMaterial,
    /// The saved key does not correspond to the key this identity uses now at
    /// the saved slot: the identity publishes no such key there, has retired
    /// it, the slot and the public half filed in it name different key ids, the
    /// saved private half does not derive that public half (or cannot be
    /// checked against it), or the entry names a wallet this install does not
    /// hold.
    KeyNoLongerOnIdentity,
    /// The item rests on a voter or operator identity that only the legacy file
    /// names — a key held there, or the operator link itself. Nothing outside
    /// that file vouches for it, and the file is exactly what cannot be taken
    /// at its word.
    LinkedIdentityUnverified,
    /// The voter-identity link would be restored with no voting key behind it,
    /// which would report the node as able to vote when it still cannot.
    VoterLinkWithoutVotingKey,
}

/// Everything a legacy record could restore into a modern one, split into what
/// this flow can restore and what it cannot.
///
/// An empty plan means there is nothing to offer — the affordance disappears
/// rather than nagging.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecoveryPlan {
    /// Restorable candidates, in a stable order: keys in `(target, key_id)`
    /// order, then associations.
    pub items: Vec<RecoveryItemDescriptor>,
    /// Legacy items found but not restorable by this flow, each with its
    /// reason. Never part of an allowlist.
    pub excluded: Vec<(RecoveryItemDescriptor, ExclusionReason)>,
}

impl RecoveryPlan {
    /// Whether there is anything at all to show the user.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.excluded.is_empty()
    }

    /// The full allowlist for "restore everything offered" — including a
    /// [`RecoveryItem::VoterAssociation`] that [`Self::preview_items`] folds
    /// into its voting key.
    pub fn approved_items(&self) -> Vec<RecoveryItem> {
        self.items.iter().map(|d| d.item.clone()).collect()
    }

    /// The rows a preview shows: every candidate except a
    /// [`RecoveryItem::VoterAssociation`] that a candidate voting key already
    /// implies. A voting key without its voter-identity link is unusable, so
    /// the two are one decision, not two.
    pub fn preview_items(&self) -> Vec<&RecoveryItemDescriptor> {
        let grouped = self.voter_association_is_grouped();
        self.items
            .iter()
            .filter(|d| !(grouped && d.item == RecoveryItem::VoterAssociation))
            .collect()
    }

    /// Whether a candidate is this identity's voting key. A node that cannot
    /// vote has a remedy that needs no key on hand, which changes what its
    /// missing-voter prompt should offer first.
    pub fn contains_voting_key(&self) -> bool {
        self.items
            .iter()
            .any(RecoveryItemDescriptor::is_voting_role)
    }

    /// Whether approving a candidate voting key also approves the candidate
    /// voter-identity link. Keyed on the voting *role*, the same predicate that
    /// decides the link is worth offering at all, so the two cannot disagree
    /// about which key the link belongs to.
    pub fn voter_association_is_grouped(&self) -> bool {
        self.contains(&RecoveryItem::VoterAssociation) && self.contains_voting_key()
    }

    fn contains(&self, item: &RecoveryItem) -> bool {
        self.items.iter().any(|d| &d.item == item)
    }

    fn is_excluded(&self, item: &RecoveryItem) -> bool {
        self.excluded.iter().any(|(d, _)| &d.item == item)
    }
}

/// The outcome of applying an approved allowlist to a modern record.
///
/// The three item lists are disjoint: an item this flow cannot restore at all
/// is reported in [`Self::excluded`] and never in [`Self::skipped_stale`],
/// which means only "already back in place, nothing to do".
#[derive(Debug, Clone)]
pub struct AppliedRecovery {
    /// The modern record with the approved candidates inserted. Never
    /// persisted unless [`Self::applied`] is non-empty.
    pub merged: QualifiedIdentity,
    /// Items that actually landed in `merged`.
    pub applied: Vec<RecoveryItemDescriptor>,
    /// Approved items that were no longer missing when the merge ran, so there
    /// was nothing to restore. Reported, not failed.
    pub skipped_stale: Vec<RecoveryItemDescriptor>,
    /// Legacy items this flow cannot restore, each with its reason — the
    /// `excluded` list of the plan recomputed inside the merge, so a caller
    /// never has to compute the same plan twice to report it.
    pub excluded: Vec<(RecoveryItemDescriptor, ExclusionReason)>,
}

/// Everything `legacy` holds that `modern` does not, with no judgement about
/// whether it should come back.
///
/// A key is a candidate when the modern key map has no entry at its
/// `(target, key_id)` **and** the legacy entry carries material this flow can
/// carry forward — plaintext (`Clear`/`AlwaysClear`) or a wallet derivation
/// reference. A legacy per-key password envelope and a bare vault placeholder
/// are listed as [`RecoveryPlan::excluded`] instead. The voter and owner-key
/// associations are candidates when modern is `None` and legacy is `Some`; the
/// operator link never is (see below).
///
/// Anything present in the modern record wins unconditionally and never
/// appears. `alias`, `status`, the dpp identity, DPNS names, and the wallet
/// link are never candidates: the modern copy of each is the newer one, and
/// refilling them from a stale legacy blob would resurrect a user's edit.
///
/// A candidate must also still *correspond* to the identity (see
/// [`ExclusionReason::KeyNoLongerOnIdentity`]), judged against the modern
/// record alone. The legacy file is an unauthenticated copy that can predate a
/// key rotation by months, and `masternode_key_presence` reports a role as held
/// from the record alone: a restored key that cannot sign would retire the very
/// remedy the operator needs, and the failure would surface as a rejected
/// transaction instead. A key on a voter or operator identity the modern record
/// does not link to therefore has no admissible witness at all and is excluded
/// as [`ExclusionReason::LinkedIdentityUnverified`]. The operator link itself
/// carries the same exclusion whenever the modern record has none: restoring it
/// would write the legacy file's own claim into the record, and a later pass
/// would read that claim back as the outside witness for the same file's
/// operator keys.
pub fn compute_recovery_plan(
    modern: &QualifiedIdentity,
    legacy: &QualifiedIdentity,
) -> RecoveryPlan {
    let mut plan = RecoveryPlan::default();

    for (map_key, (public_key, data)) in &legacy.private_keys.private_keys {
        if modern.private_keys.private_keys.contains_key(map_key) {
            continue;
        }
        let (target, key_id) = map_key.clone();
        let descriptor = RecoveryItemDescriptor::key(
            target.clone(),
            key_id,
            Some(public_key.identity_public_key.purpose()),
        );
        let plaintext = match data {
            PrivateKeyData::Clear(bytes) | PrivateKeyData::AlwaysClear(bytes) => Some(bytes),
            PrivateKeyData::AtWalletDerivationPath(_) => None,
            PrivateKeyData::Encrypted(_) => {
                plan.excluded
                    .push((descriptor, ExclusionReason::LegacyEncryptedFormat));
                continue;
            }
            PrivateKeyData::InVault => {
                plan.excluded
                    .push((descriptor, ExclusionReason::NoMaterial));
                continue;
            }
        };
        let derivable = match data {
            PrivateKeyData::AtWalletDerivationPath(path) => modern
                .associated_wallets
                .contains_key(&path.wallet_seed_hash),
            _ => true,
        };
        let exclusion = if derivable {
            key_exclusion(
                modern,
                &target,
                key_id,
                &public_key.identity_public_key,
                plaintext,
            )
        } else {
            Some(ExclusionReason::KeyNoLongerOnIdentity)
        };
        match exclusion {
            None => plan.items.push(descriptor),
            Some(reason) => plan.excluded.push((descriptor, reason)),
        }
    }

    if modern.associated_voter_identity.is_none() && legacy.associated_voter_identity.is_some() {
        let descriptor = RecoveryItemDescriptor::association(RecoveryItem::VoterAssociation);
        // The link alone reports the node as able to vote. It is worth
        // restoring only alongside a voting key the node will actually hold.
        if holds_a_voting_key(modern) || plan.contains_voting_key() {
            plan.items.push(descriptor);
        } else {
            plan.excluded
                .push((descriptor, ExclusionReason::VoterLinkWithoutVotingKey));
        }
    }
    // Only the legacy file names this operator identity. Writing that claim
    // into the modern record would give the next pass an "outside" witness for
    // the same file's operator keys, which is the check itself run backwards.
    if modern.associated_operator_identity.is_none()
        && legacy.associated_operator_identity.is_some()
    {
        plan.excluded.push((
            RecoveryItemDescriptor::association(RecoveryItem::OperatorAssociation),
            ExclusionReason::LinkedIdentityUnverified,
        ));
    }
    if modern.associated_owner_key_id.is_none() && legacy.associated_owner_key_id.is_some() {
        plan.items.push(RecoveryItemDescriptor::association(
            RecoveryItem::OwnerKeyAssociation,
        ));
    }

    plan
}

/// Merge the approved candidates of `legacy` into `modern`, additively.
///
/// The output starts as a clone of `modern` and only ever gains entries: a key
/// slot the modern record already fills is never replaced, and no field
/// outside the key map and the three association links has a write path here.
/// Candidacy is recomputed from `(modern, legacy)` rather than trusted from the
/// caller, so an approval that went stale between preview and execution merges
/// nothing and is reported in [`AppliedRecovery::skipped_stale`]; a candidate
/// the user did not approve is left alone. An approval naming something this
/// flow can never restore is reported in [`AppliedRecovery::excluded`] only —
/// "cannot be restored" and "already back in place" are not the same answer.
///
/// Approving a voting key also applies the candidate voter-identity link — a
/// voting key without it cannot vote.
///
/// Whatever plaintext the legacy record still holds when the merge finishes is
/// zeroized before it is dropped: the same record is decoded again on every
/// preview, so its unapproved keys must not accumulate in freed heap.
pub fn apply_recovery_plan(
    modern: &QualifiedIdentity,
    legacy: QualifiedIdentity,
    approved: &[RecoveryItem],
) -> AppliedRecovery {
    let plan = compute_recovery_plan(modern, &legacy);
    let mut legacy = legacy;
    let mut merged = modern.clone();

    // Plan order, not caller order: the merge is deterministic whatever
    // sequence the allowlist arrived in, and a duplicated approval applies once.
    let mut to_apply: Vec<RecoveryItemDescriptor> = plan
        .items
        .iter()
        .filter(|candidate| approved.contains(&candidate.item))
        .cloned()
        .collect();

    // A voting key is unusable without its voter-identity link, so approving
    // one approves both even if the caller listed only the key.
    if to_apply.iter().any(RecoveryItemDescriptor::is_voting_role)
        && !to_apply
            .iter()
            .any(|d| d.item == RecoveryItem::VoterAssociation)
        && let Some(link) = plan
            .items
            .iter()
            .find(|d| d.item == RecoveryItem::VoterAssociation)
    {
        to_apply.push(link.clone());
    }

    let mut skipped_stale: Vec<RecoveryItemDescriptor> = Vec::new();
    for item in approved {
        let still_a_candidate = plan.items.iter().any(|d| &d.item == item);
        let already_reported = skipped_stale.iter().any(|d| &d.item == item);
        // An excluded item is enumerated with its reason instead: it was never
        // restorable, which is the opposite of "nothing left to restore".
        if !still_a_candidate && !already_reported && !plan.is_excluded(item) {
            skipped_stale.push(stale_descriptor(modern, item));
        }
    }

    let mut applied = Vec::new();
    for descriptor in to_apply {
        match &descriptor.item {
            RecoveryItem::Key { target, key_id } => {
                let map_key = (target.clone(), *key_id);
                let Some(entry) = legacy.private_keys.private_keys.remove(&map_key) else {
                    continue;
                };
                // `or_insert`, never `insert`: the modern record wins every
                // collision by construction, not by the caller being careful.
                merged
                    .private_keys
                    .private_keys
                    .entry(map_key)
                    .or_insert(entry);
            }
            RecoveryItem::VoterAssociation => {
                if merged.associated_voter_identity.is_none() {
                    merged.associated_voter_identity = legacy.associated_voter_identity.take();
                }
            }
            RecoveryItem::OperatorAssociation => {
                if merged.associated_operator_identity.is_none() {
                    merged.associated_operator_identity =
                        legacy.associated_operator_identity.take();
                }
            }
            RecoveryItem::OwnerKeyAssociation => {
                if merged.associated_owner_key_id.is_none() {
                    merged.associated_owner_key_id = legacy.associated_owner_key_id.take();
                }
            }
        }
        applied.push(descriptor);
    }

    // Whatever was not moved into `merged` is about to be dropped; wipe its
    // resident arrays first. The returned bytes are already zeroize-on-drop.
    let _ = legacy.private_keys.take_plaintext_for_vault();

    AppliedRecovery {
        merged,
        applied,
        skipped_stale,
        excluded: plan.excluded,
    }
}

/// Label a stale approval from the modern record, which holds the key that
/// made it stale. Falls back to no metadata for an item present in neither
/// record — a caller-supplied allowlist entry that names nothing.
fn stale_descriptor(modern: &QualifiedIdentity, item: &RecoveryItem) -> RecoveryItemDescriptor {
    match item {
        RecoveryItem::Key { target, key_id } => RecoveryItemDescriptor::key(
            target.clone(),
            *key_id,
            modern
                .private_keys
                .public_key_for(&(target.clone(), *key_id))
                .map(|public_key| public_key.identity_public_key.purpose()),
        ),
        association => RecoveryItemDescriptor::association(association.clone()),
    }
}

/// Why a saved key cannot be treated as the identity's current key at
/// `key_id`, or `None` when it can.
///
/// Everything is judged against the one slot the entry occupies, because that
/// is the only slot it can ever answer for: the entry is stored under the
/// legacy `(target, key_id)` map key, and signing resolves private material by
/// the on-chain key id it was asked for. Three tests, all of which the manual
/// "type the WIF" path this flow replaces already enforces:
///
/// 1. the saved slot and the public half filed in it name the same key id;
/// 2. the saved private half derives that public half — a derivation this build
///    cannot perform is a rejection, not a pass, since nothing else here can
///    settle the correspondence (a key type with an independent correspondence
///    validator would be checked with that validator instead);
/// 3. the identity the target names still publishes exactly that key, at that
///    id, undisabled.
///
/// The third test is only worth anything when the identity it reads comes from
/// outside the legacy file, which is why [`reference_identity`] takes only the
/// modern record.
fn key_exclusion(
    modern: &QualifiedIdentity,
    target: &PrivateKeyTarget,
    key_id: KeyID,
    public_key: &IdentityPublicKey,
    plaintext: Option<&[u8; 32]>,
) -> Option<ExclusionReason> {
    if public_key.id() != key_id {
        return Some(ExclusionReason::KeyNoLongerOnIdentity);
    }
    if let Some(bytes) = plaintext {
        let derived = public_key
            .key_type()
            .public_key_data_from_private_key_data(bytes, modern.network);
        match derived {
            Ok(derived) if derived.as_slice() == public_key.data().as_slice() => {}
            _ => return Some(ExclusionReason::KeyNoLongerOnIdentity),
        }
    }
    let Some(identity) = reference_identity(modern, target) else {
        return Some(ExclusionReason::LinkedIdentityUnverified);
    };
    let live = identity.public_keys().get(&key_id).is_some_and(|current| {
        current.key_type() == public_key.key_type()
            && current.data() == public_key.data()
            && !current.is_disabled()
    });
    (!live).then_some(ExclusionReason::KeyNoLongerOnIdentity)
}

/// The identity whose key set decides whether a key at `target` still exists:
/// the node's own identity, or the voter/operator identity the *modern* record
/// links to.
///
/// The legacy file never stands in for a missing modern link. It supplies the
/// key under test, so letting it also supply the identity that vouches for that
/// key would validate untrusted data against itself, and pass for any
/// self-consistent pair — including a key the chain retired months before the
/// upgrade. `None` therefore means unverifiable
/// ([`ExclusionReason::LinkedIdentityUnverified`]), not merely unknown.
fn reference_identity<'a>(
    modern: &'a QualifiedIdentity,
    target: &PrivateKeyTarget,
) -> Option<&'a Identity> {
    let linked = |link: &'a Option<(Identity, IdentityPublicKey)>| {
        link.as_ref().map(|(identity, _)| identity)
    };
    match target {
        PrivateKeyTarget::PrivateKeyOnMainIdentity => Some(&modern.identity),
        PrivateKeyTarget::PrivateKeyOnVoterIdentity => linked(&modern.associated_voter_identity),
        PrivateKeyTarget::PrivateKeyOnOperatorIdentity => {
            linked(&modern.associated_operator_identity)
        }
    }
}

/// Whether `record` already holds a key that lets it vote — the same two
/// signals [`QualifiedIdentity::masternode_key_presence`] reads.
fn holds_a_voting_key(record: &QualifiedIdentity) -> bool {
    record
        .private_keys
        .private_keys
        .iter()
        .any(|((target, _), (public_key, _))| {
            *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity
                || public_key.identity_public_key.purpose() == Purpose::VOTING
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_identity::encrypted_key_storage::{
        KeyStorage, WalletDerivationPath,
    };
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType};
    use crate::model::wallet::test_support::open_wallet;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{KeyType, SecurityLevel};
    use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
    use dash_sdk::dpp::platform_value::BinaryData;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;
    use std::collections::BTreeMap;
    use std::sync::{Arc, RwLock};

    const M: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
    const V: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;
    const O: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnOperatorIdentity;

    /// A fixture key pair: a private half plus the public half genuinely
    /// derived from it. Real material, because the correspondence check the
    /// plan applies would reject any placeholder that cannot verify.
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

        /// The same key as the chain holds it after a rotation: still listed,
        /// no longer usable.
        fn retired(&self) -> Self {
            let IdentityPublicKey::V0(mut v0) = self.public.clone();
            v0.disabled_at = Some(1);
            Self {
                secret: self.secret,
                public: IdentityPublicKey::V0(v0),
            }
        }
    }

    /// A key pair for `purpose`, keyed by `secret_byte` so two fixtures in one
    /// test are genuinely different keys.
    fn test_key(id: KeyID, purpose: Purpose, secret_byte: u8) -> TestKey {
        let secret = [secret_byte; 32];
        let key_type = KeyType::ECDSA_HASH160;
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

    /// A `QualifiedIdentity` with no keys, no associations and no alias — the
    /// bare shape a ProTxHash-only masternode load produces.
    fn bare_identity(id: u8) -> QualifiedIdentity {
        let pv = PlatformVersion::latest();
        QualifiedIdentity {
            identity: Identity::create_basic_identity(Identifier::from([id; 32]), pv)
                .expect("basic identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    /// Register `key` on `identity` as one of the public keys the chain holds.
    fn publish_on(identity: &mut Identity, key: &IdentityPublicKey) {
        let mut keys = identity.public_keys().clone();
        keys.insert(key.id(), key.clone());
        identity.set_public_keys(keys);
    }

    /// Hold `key`'s private half at `(target, key id)` as `data`.
    fn hold(
        qi: &mut QualifiedIdentity,
        target: PrivateKeyTarget,
        key: &TestKey,
        data: PrivateKeyData,
    ) {
        qi.private_keys.private_keys.insert(
            (target, key.id()),
            (QualifiedIdentityPublicKey::from(key.public.clone()), data),
        );
    }

    /// A voter identity publishing `keys`, in the shape
    /// `associated_voter_identity` carries.
    fn voter_identity(id: u8, keys: &[&IdentityPublicKey]) -> (Identity, IdentityPublicKey) {
        let mut identity =
            Identity::create_basic_identity(Identifier::from([id; 32]), PlatformVersion::latest())
                .expect("voter identity");
        for key in keys {
            publish_on(&mut identity, key);
        }
        let paired = keys
            .first()
            .map(|key| (*key).clone())
            .unwrap_or_else(|| test_key(0, Purpose::VOTING, 0xF0).public);
        (identity, paired)
    }

    /// The partial-load shape this whole flow exists for: the modern record
    /// knows the key from the chain, the legacy file still holds its private
    /// half.
    fn strand(
        modern: &mut QualifiedIdentity,
        legacy: &mut QualifiedIdentity,
        target: PrivateKeyTarget,
        key: &TestKey,
        data: PrivateKeyData,
    ) {
        publish_on(&mut modern.identity, &key.public);
        hold(legacy, target, key, data);
    }

    fn key_item(target: PrivateKeyTarget, key_id: KeyID) -> RecoveryItem {
        RecoveryItem::Key { target, key_id }
    }

    fn plan_items(plan: &RecoveryPlan) -> Vec<RecoveryItem> {
        plan.items.iter().map(|d| d.item.clone()).collect()
    }

    fn excluded_items(plan: &RecoveryPlan) -> Vec<(RecoveryItem, ExclusionReason)> {
        plan.excluded
            .iter()
            .map(|(d, reason)| (d.item.clone(), *reason))
            .collect()
    }

    fn applied_items(applied: &AppliedRecovery) -> Vec<RecoveryItem> {
        applied.applied.iter().map(|d| d.item.clone()).collect()
    }

    fn stale_items(applied: &AppliedRecovery) -> Vec<RecoveryItem> {
        applied
            .skipped_stale
            .iter()
            .map(|d| d.item.clone())
            .collect()
    }

    fn key_data(qi: &QualifiedIdentity, target: PrivateKeyTarget, key_id: KeyID) -> PrivateKeyData {
        qi.private_keys
            .private_keys
            .get(&(target, key_id))
            .expect("key present")
            .1
            .clone()
    }

    /// M1 — a plaintext key only the legacy record holds is a candidate.
    #[test]
    fn m1_legacy_only_clear_key_is_a_candidate() {
        let mut modern = bare_identity(0x01);
        let mut legacy = bare_identity(0x01);
        let owner = test_key(1, Purpose::OWNER, 0xA0);
        strand(&mut modern, &mut legacy, M, &owner, owner.clear());

        let plan = compute_recovery_plan(&modern, &legacy);

        assert_eq!(plan_items(&plan), vec![key_item(M, 1)]);
        assert!(plan.excluded.is_empty());
    }

    /// M2 — a key both records hold is never a candidate, and the modern bytes
    /// survive even when a stale allowlist names it. Legacy must never win a
    /// collision.
    #[test]
    fn m2_key_present_in_both_keeps_the_modern_bytes() {
        let owner = test_key(1, Purpose::OWNER, 0x11);
        let rotated = test_key(1, Purpose::OWNER, 0x99);

        let mut modern = bare_identity(0x02);
        publish_on(&mut modern.identity, &owner.public);
        hold(&mut modern, M, &owner, owner.clear());
        let mut legacy = bare_identity(0x02);
        hold(&mut legacy, M, &rotated, rotated.clear());

        let plan = compute_recovery_plan(&modern, &legacy);
        assert!(
            plan.items.is_empty(),
            "a key the modern record already holds is not a candidate",
        );

        let applied = apply_recovery_plan(&modern, legacy, &[key_item(M, 1)]);
        assert_eq!(
            key_data(&applied.merged, M, 1),
            owner.clear(),
            "the modern key bytes must survive an approval that named the same slot",
        );
        assert!(applied.applied.is_empty(), "nothing was restorable");
    }

    /// M3 — a key only the modern record holds is never in the plan and comes
    /// through the merge byte-identical.
    #[test]
    fn m3_modern_only_key_is_untouched() {
        let held = test_key(7, Purpose::TRANSFER, 0x33);
        let stranded = test_key(8, Purpose::OWNER, 0x44);

        let mut modern = bare_identity(0x03);
        publish_on(&mut modern.identity, &held.public);
        hold(&mut modern, M, &held, held.clear());
        let mut legacy = bare_identity(0x03);
        strand(&mut modern, &mut legacy, M, &stranded, stranded.clear());

        let plan = compute_recovery_plan(&modern, &legacy);
        assert_eq!(plan_items(&plan), vec![key_item(M, 8)]);

        let applied = apply_recovery_plan(&modern, legacy, &plan.approved_items());
        assert_eq!(
            key_data(&applied.merged, M, 7),
            held.clear(),
            "the modern-only key must survive the merge byte-identical",
        );
        assert_eq!(key_data(&applied.merged, M, 8), stranded.clear());
    }

    /// M4 — the voter and owner-key links are candidates exactly when the modern
    /// record lacks them and the legacy record has them, across all four
    /// combinations. The operator link never is: only the legacy file names that
    /// identity, whatever the combination.
    #[test]
    fn m4_associations_are_candidates_only_when_modern_is_none() {
        for (modern_has, legacy_has, expected) in [
            (false, true, true),
            (false, false, false),
            (true, true, false),
            (true, false, false),
        ] {
            let mut modern = bare_identity(0x04);
            // The voter link is only worth restoring alongside a voting key,
            // so the node holds one already.
            let voting = test_key(1, Purpose::VOTING, 0xB1);
            publish_on(&mut modern.identity, &voting.public);
            hold(&mut modern, M, &voting, voting.clear());
            let mut legacy = bare_identity(0x04);
            if modern_has {
                modern.associated_voter_identity = Some(voter_identity(0xC0, &[]));
                modern.associated_operator_identity = Some(voter_identity(0xC1, &[]));
                modern.associated_owner_key_id = Some(5);
            }
            if legacy_has {
                legacy.associated_voter_identity = Some(voter_identity(0xD0, &[]));
                legacy.associated_operator_identity = Some(voter_identity(0xD1, &[]));
                legacy.associated_owner_key_id = Some(9);
            }

            let plan = compute_recovery_plan(&modern, &legacy);
            let items = plan_items(&plan);
            for item in [
                RecoveryItem::VoterAssociation,
                RecoveryItem::OwnerKeyAssociation,
            ] {
                assert_eq!(
                    items.contains(&item),
                    expected,
                    "{item:?}: modern_has={modern_has}, legacy_has={legacy_has}",
                );
            }
            assert!(
                !items.contains(&RecoveryItem::OperatorAssociation),
                "the operator link is never offered: modern_has={modern_has}, legacy_has={legacy_has}",
            );
        }
    }

    /// M5 — a legacy per-key password envelope is excluded with its own reason
    /// and never merged, even when an allowlist names it.
    #[test]
    fn m5_legacy_encrypted_key_is_excluded_and_never_applied() {
        let owner = test_key(1, Purpose::OWNER, 0xA0);
        let mut modern = bare_identity(0x05);
        let mut legacy = bare_identity(0x05);
        strand(
            &mut modern,
            &mut legacy,
            M,
            &owner,
            PrivateKeyData::Encrypted(vec![0x77; 48]),
        );

        let plan = compute_recovery_plan(&modern, &legacy);
        assert!(
            plan.items.is_empty(),
            "an unreadable key is not a candidate"
        );
        assert_eq!(
            excluded_items(&plan),
            vec![(key_item(M, 1), ExclusionReason::LegacyEncryptedFormat)],
        );

        let applied = apply_recovery_plan(&modern, legacy, &[key_item(M, 1)]);
        assert!(
            applied.applied.is_empty()
                && !applied
                    .merged
                    .private_keys
                    .private_keys
                    .contains_key(&(M, 1)),
            "an excluded key must never be merged, even if approved",
        );
    }

    /// M6 — a candidate voting key carries its voter-identity link: the two are
    /// one preview row, and approving the key alone still applies the link.
    #[test]
    fn m6_voting_key_groups_its_voter_association() {
        let voting = test_key(2, Purpose::VOTING, 0xB0);
        let mut modern = bare_identity(0x06);
        let mut legacy = bare_identity(0x06);
        strand(&mut modern, &mut legacy, M, &voting, voting.clear());
        legacy.associated_voter_identity = Some(voter_identity(0xD0, &[&voting.public]));

        let plan = compute_recovery_plan(&modern, &legacy);
        assert!(plan.voter_association_is_grouped());
        assert_eq!(
            plan.preview_items()
                .into_iter()
                .map(|d| d.item.clone())
                .collect::<Vec<_>>(),
            vec![key_item(M, 2)],
            "the voter link is folded into its voting key, not offered separately",
        );

        let applied = apply_recovery_plan(&modern, legacy, &[key_item(M, 2)]);
        assert!(
            applied.merged.associated_voter_identity.is_some(),
            "applying the voting key must apply its voter-identity link",
        );
        assert!(applied_items(&applied).contains(&RecoveryItem::VoterAssociation));
    }

    /// M7 — identical records yield an empty plan, so the affordance is never
    /// offered when there is nothing to restore.
    #[test]
    fn m7_identical_records_produce_an_empty_plan() {
        let owner = test_key(1, Purpose::OWNER, 0x55);
        let mut modern = bare_identity(0x07);
        publish_on(&mut modern.identity, &owner.public);
        hold(&mut modern, M, &owner, owner.clear());
        modern.associated_voter_identity = Some(voter_identity(0xE0, &[]));
        let legacy = modern.clone();

        let plan = compute_recovery_plan(&modern, &legacy);

        assert!(plan.is_empty(), "identical records have nothing to restore");
    }

    /// M8 — the case this flow exists for: a bare modern record (a ProTxHash-only
    /// load) offers the owner and payout keys it can vouch for, and reports the
    /// voter-identity key and link it cannot.
    ///
    /// Everything the modern record can vouch for, and nothing else: a bare
    /// record names no voter identity, so the key held on one is reported
    /// unverifiable and takes the link down with it.
    #[test]
    fn m8_bare_modern_record_offers_owner_and_payout_but_not_an_unlinked_voter_key() {
        let owner = test_key(1, Purpose::OWNER, 0xA0);
        let payout = test_key(2, Purpose::TRANSFER, 0xA1);
        let voting = test_key(3, Purpose::VOTING, 0xA2);

        let mut modern = bare_identity(0x08);
        let mut legacy = bare_identity(0x08);
        strand(&mut modern, &mut legacy, M, &owner, owner.clear());
        strand(
            &mut modern,
            &mut legacy,
            M,
            &payout,
            PrivateKeyData::AlwaysClear(payout.secret),
        );
        hold(&mut legacy, V, &voting, voting.clear());
        legacy.associated_voter_identity = Some(voter_identity(0xD0, &[&voting.public]));
        legacy.associated_owner_key_id = Some(1);

        let plan = compute_recovery_plan(&modern, &legacy);

        assert_eq!(
            plan_items(&plan),
            vec![
                key_item(M, 1),
                key_item(M, 2),
                RecoveryItem::OwnerKeyAssociation,
            ],
        );
        assert_eq!(
            excluded_items(&plan),
            vec![
                (key_item(V, 3), ExclusionReason::LinkedIdentityUnverified),
                (
                    RecoveryItem::VoterAssociation,
                    ExclusionReason::VoterLinkWithoutVotingKey,
                ),
            ],
        );
    }

    /// M9 — the structural additive-only proof, over every fixture shape: the
    /// merged key map is a superset of the modern one with every modern entry
    /// byte-identical, and the fields with no write path here — alias, status,
    /// the dpp identity, DPNS names, wallet index, identity type — are
    /// untouched no matter what the legacy record holds.
    #[test]
    fn m9_merge_is_additive_and_touches_no_other_field() {
        let held = test_key(1, Purpose::OWNER, 0x11);
        let rotated = test_key(1, Purpose::OWNER, 0x99);
        let stranded = test_key(2, Purpose::TRANSFER, 0x22);

        for legacy_alias in [None, Some("legacy-name")] {
            let mut modern = bare_identity(0x09);
            modern.alias = Some("modern-name".to_string());
            publish_on(&mut modern.identity, &held.public);
            publish_on(&mut modern.identity, &stranded.public);
            hold(&mut modern, M, &held, held.clear());
            let mut modern_no_alias = modern.clone();
            modern_no_alias.alias = None;

            for base in [modern, modern_no_alias] {
                let mut legacy = bare_identity(0x09);
                legacy.alias = legacy_alias.map(str::to_string);
                legacy.status = IdentityStatus::NotFound;
                legacy.identity_type = IdentityType::User;
                legacy.wallet_index = Some(42);
                legacy.dpns_names = vec![crate::model::qualified_identity::DPNSNameInfo {
                    name: "legacy".to_string(),
                    acquired_at: 1,
                }];
                hold(&mut legacy, M, &rotated, rotated.clear());
                hold(&mut legacy, M, &stranded, stranded.clear());

                let plan = compute_recovery_plan(&base, &legacy);
                let applied = apply_recovery_plan(&base, legacy, &plan.approved_items());
                let merged = &applied.merged;

                for (map_key, entry) in &base.private_keys.private_keys {
                    assert_eq!(
                        merged.private_keys.private_keys.get(map_key),
                        Some(entry),
                        "every modern key must survive byte-identical",
                    );
                }
                assert!(
                    merged.private_keys.private_keys.len() >= base.private_keys.private_keys.len(),
                    "the merged key map must be a superset of the modern one",
                );
                assert_eq!(merged.alias, base.alias, "alias has no write path here");
                assert_eq!(merged.status, base.status, "status has no write path here");
                assert_eq!(
                    merged.identity_type, base.identity_type,
                    "identity type has no write path here",
                );
                assert_eq!(
                    merged.wallet_index, base.wallet_index,
                    "the wallet link has no write path here",
                );
                assert_eq!(
                    merged.dpns_names, base.dpns_names,
                    "DPNS names have no write path here",
                );
                assert_eq!(
                    merged.identity, base.identity,
                    "the on-chain identity has no write path here",
                );
            }
        }
    }

    /// M10 — the allowlist-intersection rule: an approval that is no longer
    /// missing is reported stale and changes nothing, and a candidate the user
    /// did not approve is left alone (the key removed between preview and
    /// execution).
    #[test]
    fn m10_merge_is_the_intersection_of_approved_and_still_missing() {
        let held = test_key(1, Purpose::OWNER, 0x11);
        let rotated = test_key(1, Purpose::OWNER, 0x99);
        let unapproved = test_key(2, Purpose::TRANSFER, 0x22);

        let mut modern = bare_identity(0x0A);
        publish_on(&mut modern.identity, &held.public);
        publish_on(&mut modern.identity, &unapproved.public);
        hold(&mut modern, M, &held, held.clear());
        let mut legacy = bare_identity(0x0A);
        hold(&mut legacy, M, &rotated, rotated.clear());
        hold(&mut legacy, M, &unapproved, unapproved.clear());

        // Key 1 was re-added since the preview; key 2 is still missing but was
        // never approved.
        let applied = apply_recovery_plan(&modern, legacy, &[key_item(M, 1)]);

        assert_eq!(
            stale_items(&applied),
            vec![key_item(M, 1)],
            "an approval that is no longer missing is reported stale",
        );
        assert!(applied.applied.is_empty());
        assert_eq!(key_data(&applied.merged, M, 1), held.clear());
        assert!(
            !applied
                .merged
                .private_keys
                .private_keys
                .contains_key(&(M, 2)),
            "a candidate the user did not approve must never be merged",
        );
    }

    /// A saved key whose public half is not one of the identity's keys cannot
    /// sign for it. Offering it would flip the role to "present" and retire the
    /// remedy that would actually fix the node.
    #[test]
    fn a_key_the_identity_no_longer_uses_is_excluded() {
        let modern = bare_identity(0x21);
        let mut legacy = bare_identity(0x21);
        let orphan = test_key(1, Purpose::OWNER, 0xA0);
        hold(&mut legacy, M, &orphan, orphan.clear());

        let plan = compute_recovery_plan(&modern, &legacy);

        assert!(plan.items.is_empty());
        assert_eq!(
            excluded_items(&plan),
            vec![(key_item(M, 1), ExclusionReason::KeyNoLongerOnIdentity)],
        );
    }

    /// A saved private half that does not derive the public half it is stored
    /// with is not the identity's key, whatever the record claims.
    #[test]
    fn a_key_that_does_not_derive_its_public_half_is_excluded() {
        let published = test_key(1, Purpose::OWNER, 0xA0);
        let mut modern = bare_identity(0x24);
        publish_on(&mut modern.identity, &published.public);

        // The stored pair keeps the published public key but a different secret.
        let mut legacy = bare_identity(0x24);
        hold(
            &mut legacy,
            M,
            &published,
            PrivateKeyData::Clear([0x5C; 32]),
        );

        let plan = compute_recovery_plan(&modern, &legacy);

        assert_eq!(
            excluded_items(&plan),
            vec![(key_item(M, 1), ExclusionReason::KeyNoLongerOnIdentity)],
        );
    }

    /// The saved slot and the public key filed in it must name the same key id.
    /// Signing resolves private material by the on-chain key id it was asked
    /// for, so an entry filed under one id holding another id's public half
    /// would answer with a key that cannot sign for the slot it was found in.
    #[test]
    fn a_key_filed_under_a_slot_its_public_half_does_not_name_is_excluded() {
        let published = test_key(1, Purpose::OWNER, 0xA3);
        let mut modern = bare_identity(0x28);
        publish_on(&mut modern.identity, &published.public);

        // The pair is internally consistent; only the slot it sits in is wrong.
        let mut legacy = bare_identity(0x28);
        legacy.private_keys.private_keys.insert(
            (M, 7),
            (
                QualifiedIdentityPublicKey::from(published.public.clone()),
                published.clear(),
            ),
        );

        let plan = compute_recovery_plan(&modern, &legacy);

        assert_eq!(
            excluded_items(&plan),
            vec![(key_item(M, 7), ExclusionReason::KeyNoLongerOnIdentity)],
        );
    }

    /// The identity uses this key material — but at a different key id. The
    /// entry is stored under the legacy slot, and signing looks the private half
    /// up by the id it was asked for, so accepting it would report a role as
    /// held against a slot that resolves to nothing.
    #[test]
    fn a_key_live_only_under_a_different_key_id_is_excluded() {
        let live = test_key(9, Purpose::OWNER, 0xA4);
        let mut modern = bare_identity(0x29);
        publish_on(&mut modern.identity, &live.public);

        // The same material, saved as the identity's key 5. Nothing is live at 5.
        let IdentityPublicKey::V0(mut v0) = live.public.clone();
        v0.id = 5;
        let mut legacy = bare_identity(0x29);
        legacy.private_keys.private_keys.insert(
            (M, 5),
            (
                QualifiedIdentityPublicKey::from(IdentityPublicKey::V0(v0)),
                live.clear(),
            ),
        );

        let plan = compute_recovery_plan(&modern, &legacy);

        assert_eq!(
            excluded_items(&plan),
            vec![(key_item(M, 5), ExclusionReason::KeyNoLongerOnIdentity)],
            "the identity's key set vouches for id 9, never for id 5",
        );
    }

    /// A saved private half this build cannot turn into a public half is
    /// unusable, whatever the public metadata filed with it says. An all-zero
    /// ECDSA secret is the shape a truncated or wiped legacy blob leaves: it
    /// derives nothing, so it can never sign.
    #[test]
    fn a_private_half_that_derives_nothing_is_excluded() {
        let published = test_key(1, Purpose::OWNER, 0xA5);
        let malformed = [0u8; 32];
        assert!(
            published
                .public
                .key_type()
                .public_key_data_from_private_key_data(&malformed, Network::Testnet)
                .is_err(),
            "the premise: this secret genuinely has no public half to derive",
        );

        let mut modern = bare_identity(0x2A);
        publish_on(&mut modern.identity, &published.public);
        let mut legacy = bare_identity(0x2A);
        hold(&mut legacy, M, &published, PrivateKeyData::Clear(malformed));

        let plan = compute_recovery_plan(&modern, &legacy);

        assert_eq!(
            excluded_items(&plan),
            vec![(key_item(M, 1), ExclusionReason::KeyNoLongerOnIdentity)],
            "a key whose correspondence cannot be checked must not be restored",
        );
    }

    /// A key the chain has retired cannot sign either, so restoring it would
    /// report a role as held on a node that still cannot use it.
    #[test]
    fn a_retired_key_is_excluded() {
        let rotated_out = test_key(1, Purpose::TRANSFER, 0xA0).retired();
        let mut modern = bare_identity(0x25);
        publish_on(&mut modern.identity, &rotated_out.public);
        let mut legacy = bare_identity(0x25);
        hold(&mut legacy, M, &rotated_out, rotated_out.clear());

        let plan = compute_recovery_plan(&modern, &legacy);

        assert_eq!(
            excluded_items(&plan),
            vec![(key_item(M, 1), ExclusionReason::KeyNoLongerOnIdentity)],
        );
    }

    /// A key held on a voter identity is vouched for by the legacy file alone
    /// when the modern record links to no voter identity: the same
    /// unauthenticated blob supplies the key *and* the identity that publishes
    /// it, so a key the chain retired months ago still reads as live there.
    /// Unverifiable is not the same as valid — it is reported, not offered.
    ///
    /// Restoring it would flip the node's voting role to "present" and retire
    /// the manual remedy that would actually fix the node.
    #[test]
    fn a_voter_key_only_the_legacy_blob_vouches_for_is_excluded() {
        let voting = test_key(1, Purpose::VOTING, 0xB7);
        let voter = voter_identity(0xD1, &[&voting.public]);
        let modern = bare_identity(0x26);
        let mut legacy = bare_identity(0x26);
        hold(&mut legacy, V, &voting, voting.clear());
        legacy.associated_voter_identity = Some(voter.clone());

        let plan = compute_recovery_plan(&modern, &legacy);

        assert!(
            plan.items.is_empty(),
            "nothing outside the legacy file says this key is still the node's",
        );
        assert_eq!(
            excluded_items(&plan),
            vec![
                (key_item(V, 1), ExclusionReason::LinkedIdentityUnverified),
                (
                    RecoveryItem::VoterAssociation,
                    ExclusionReason::VoterLinkWithoutVotingKey,
                ),
            ],
        );

        // The same key against the same voter identity, once the *modern*
        // record is the one naming it: a witness this flow did not decode from
        // the file under test, so the key becomes a candidate.
        let mut linked = bare_identity(0x26);
        linked.associated_voter_identity = Some(voter);

        assert_eq!(
            plan_items(&compute_recovery_plan(&linked, &legacy)),
            vec![key_item(V, 1)],
        );
    }

    /// An operator-identity link the modern record does not carry rests on the
    /// legacy file alone, exactly like a key held on that identity. Restoring it
    /// would write the file's own claim into the modern record, where the next
    /// pass would read it as the outside witness that vouches for the same
    /// file's operator keys — self-vouching in two steps instead of one.
    #[test]
    fn an_operator_link_only_the_legacy_blob_vouches_for_is_excluded() {
        let operator_key = test_key(1, Purpose::AUTHENTICATION, 0xB9);
        let modern = bare_identity(0x27);
        let mut legacy = bare_identity(0x27);
        hold(&mut legacy, O, &operator_key, operator_key.clear());
        legacy.associated_operator_identity = Some(voter_identity(0xD2, &[&operator_key.public]));

        let plan = compute_recovery_plan(&modern, &legacy);

        assert!(
            plan.items.is_empty(),
            "nothing outside the legacy file names this operator identity",
        );
        assert_eq!(
            excluded_items(&plan),
            vec![
                (key_item(O, 1), ExclusionReason::LinkedIdentityUnverified),
                (
                    RecoveryItem::OperatorAssociation,
                    ExclusionReason::LinkedIdentityUnverified,
                ),
            ],
        );

        // The laundering step itself: approving the link must not put it in the
        // record, or the key above becomes a candidate on the next pass.
        let applied = apply_recovery_plan(&modern, legacy, &[RecoveryItem::OperatorAssociation]);
        assert!(
            applied.merged.associated_operator_identity.is_none(),
            "an unverifiable link must never reach the modern record",
        );
    }

    /// The voter-identity link alone reports the node as able to vote. Without
    /// a voting key behind it that is a false assurance, so it is not offered.
    #[test]
    fn a_voter_link_without_a_voting_key_is_excluded() {
        let modern = bare_identity(0x22);
        let mut legacy = bare_identity(0x22);
        legacy.associated_voter_identity = Some(voter_identity(0xD0, &[]));

        let plan = compute_recovery_plan(&modern, &legacy);

        assert!(plan.items.is_empty());
        assert_eq!(
            excluded_items(&plan),
            vec![(
                RecoveryItem::VoterAssociation,
                ExclusionReason::VoterLinkWithoutVotingKey,
            )],
        );
    }

    /// A wallet-derivation reference carries no plaintext, so it is a candidate
    /// and lands verbatim — but only while this install still holds the wallet
    /// it names, since the reference resolves through nothing else.
    #[test]
    fn a_wallet_derived_key_needs_the_wallet_this_install_holds() {
        let derived = test_key(1, Purpose::AUTHENTICATION, 0x0B);
        let wallet = open_wallet();
        let seed_hash = wallet.seed_hash();
        let reference = WalletDerivationPath {
            wallet_seed_hash: seed_hash,
            derivation_path: DerivationPath::from(vec![]),
        };

        let mut modern = bare_identity(0x0B);
        let mut legacy = bare_identity(0x0B);
        strand(
            &mut modern,
            &mut legacy,
            M,
            &derived,
            PrivateKeyData::AtWalletDerivationPath(reference.clone()),
        );

        let unknown_wallet = compute_recovery_plan(&modern, &legacy);
        assert_eq!(
            excluded_items(&unknown_wallet),
            vec![(key_item(M, 1), ExclusionReason::KeyNoLongerOnIdentity)],
            "a reference to a wallet this install does not hold resolves to nothing",
        );

        modern
            .associated_wallets
            .insert(seed_hash, Arc::new(RwLock::new(wallet)));
        let plan = compute_recovery_plan(&modern, &legacy);
        assert_eq!(plan_items(&plan), vec![key_item(M, 1)]);

        let applied = apply_recovery_plan(&modern, legacy, &plan.approved_items());
        assert_eq!(
            key_data(&applied.merged, M, 1),
            PrivateKeyData::AtWalletDerivationPath(reference),
        );
    }

    /// A vault placeholder in a legacy blob carries no bytes to restore, so it
    /// is excluded rather than merged as an empty reference.
    #[test]
    fn in_vault_legacy_key_is_excluded_as_having_no_material() {
        let placeholder = test_key(1, Purpose::OWNER, 0x0C);
        let mut modern = bare_identity(0x0C);
        let mut legacy = bare_identity(0x0C);
        strand(
            &mut modern,
            &mut legacy,
            M,
            &placeholder,
            PrivateKeyData::InVault,
        );

        let plan = compute_recovery_plan(&modern, &legacy);

        assert!(plan.items.is_empty());
        assert_eq!(
            excluded_items(&plan),
            vec![(key_item(M, 1), ExclusionReason::NoMaterial)],
        );
    }

    /// Approving something that was never restorable reports it as excluded,
    /// with its reason — never as a stale approval, which means the opposite:
    /// already back in place, nothing to do.
    #[test]
    fn an_excluded_approval_is_reported_excluded_and_never_stale() {
        let unreadable = test_key(1, Purpose::OWNER, 0x0D);
        let mut modern = bare_identity(0x0D);
        let mut legacy = bare_identity(0x0D);
        strand(
            &mut modern,
            &mut legacy,
            M,
            &unreadable,
            PrivateKeyData::Encrypted(vec![0x77; 48]),
        );

        let applied = apply_recovery_plan(&modern, legacy, &[key_item(M, 1)]);

        assert!(
            applied.skipped_stale.is_empty(),
            "an always-excluded item must not be reported as a stale approval",
        );
        assert_eq!(
            applied
                .excluded
                .iter()
                .map(|(d, reason)| (d.item.clone(), *reason))
                .collect::<Vec<_>>(),
            vec![(key_item(M, 1), ExclusionReason::LegacyEncryptedFormat)],
        );
    }
}
