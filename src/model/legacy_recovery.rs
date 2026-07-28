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

use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyID, Purpose};

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

    /// The user-facing name of this item, in the role words a node operator
    /// uses. One translation unit per label — no fragments are concatenated.
    pub fn label(&self) -> &'static str {
        match &self.item {
            RecoveryItem::Key { target, .. } => {
                if *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity {
                    return "Voting key";
                }
                match self.purpose {
                    Some(Purpose::VOTING) => "Voting key",
                    Some(Purpose::OWNER) => "Owner key",
                    Some(Purpose::TRANSFER) => "Payout key",
                    Some(Purpose::AUTHENTICATION) => "Authentication key",
                    _ => "Identity key",
                }
            }
            RecoveryItem::VoterAssociation => "Voting identity link",
            RecoveryItem::OperatorAssociation => "Operator identity link",
            RecoveryItem::OwnerKeyAssociation => "Owner key link",
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
}

impl ExclusionReason {
    /// What the user can do instead, written as a complete sentence for the
    /// Everyday User. Both reasons share one remedy: re-load the identity with
    /// the key in hand.
    pub fn explanation(self) -> &'static str {
        match self {
            ExclusionReason::LegacyEncryptedFormat => {
                "This key is saved in an older format that cannot be restored automatically. \
                 Load this identity again and enter the key to bring it back."
            }
            ExclusionReason::NoMaterial => {
                "The previous version kept no copy of this key. \
                 Load this identity again and enter the key to bring it back."
            }
        }
    }
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

    /// Whether approving a candidate voting key also approves the candidate
    /// voter-identity link.
    pub fn voter_association_is_grouped(&self) -> bool {
        self.contains(&RecoveryItem::VoterAssociation) && self.has_voter_key()
    }

    fn contains(&self, item: &RecoveryItem) -> bool {
        self.items.iter().any(|d| &d.item == item)
    }

    fn has_voter_key(&self) -> bool {
        self.items.iter().any(|d| is_voter_key(&d.item))
    }
}

/// The outcome of applying an approved allowlist to a modern record.
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
}

/// Everything `legacy` holds that `modern` does not, with no judgement about
/// whether it should come back.
///
/// A key is a candidate when the modern key map has no entry at its
/// `(target, key_id)` **and** the legacy entry carries material this flow can
/// carry forward — plaintext (`Clear`/`AlwaysClear`) or a wallet derivation
/// reference. A legacy per-key password envelope and a bare vault placeholder
/// are listed as [`RecoveryPlan::excluded`] instead. An association is a
/// candidate when modern is `None` and legacy is `Some`.
///
/// Anything present in the modern record wins unconditionally and never
/// appears. `alias`, `status`, the dpp identity, DPNS names, and the wallet
/// link are never candidates: the modern copy of each is the newer one, and
/// refilling them from a stale legacy blob would resurrect a user's edit.
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
            target,
            key_id,
            Some(public_key.identity_public_key.purpose()),
        );
        match data {
            PrivateKeyData::Clear(_)
            | PrivateKeyData::AlwaysClear(_)
            | PrivateKeyData::AtWalletDerivationPath(_) => plan.items.push(descriptor),
            PrivateKeyData::Encrypted(_) => plan
                .excluded
                .push((descriptor, ExclusionReason::LegacyEncryptedFormat)),
            PrivateKeyData::InVault => plan
                .excluded
                .push((descriptor, ExclusionReason::NoMaterial)),
        }
    }

    if modern.associated_voter_identity.is_none() && legacy.associated_voter_identity.is_some() {
        plan.items.push(RecoveryItemDescriptor::association(
            RecoveryItem::VoterAssociation,
        ));
    }
    if modern.associated_operator_identity.is_none()
        && legacy.associated_operator_identity.is_some()
    {
        plan.items.push(RecoveryItemDescriptor::association(
            RecoveryItem::OperatorAssociation,
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
/// the user did not approve is left alone.
///
/// Approving a voting key also applies the candidate voter-identity link — a
/// voting key without it cannot vote.
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
    if to_apply.iter().any(|d| is_voter_key(&d.item))
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

    let mut skipped_stale = Vec::new();
    for item in approved {
        let still_a_candidate = plan.items.iter().any(|d| &d.item == item);
        let already_reported = skipped_stale
            .iter()
            .any(|d: &RecoveryItemDescriptor| &d.item == item);
        if !still_a_candidate && !already_reported {
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

    AppliedRecovery {
        merged,
        applied,
        skipped_stale,
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

/// Whether `item` is a key on the voter identity.
fn is_voter_key(item: &RecoveryItem) -> bool {
    matches!(
        item,
        RecoveryItem::Key { target, .. } if *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_identity::encrypted_key_storage::{
        KeyStorage, WalletDerivationPath,
    };
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType};
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{Identity, KeyType, SecurityLevel};
    use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
    use dash_sdk::dpp::platform_value::BinaryData;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, IdentityPublicKey};
    use std::collections::BTreeMap;

    const M: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
    const V: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;

    /// A deterministic public key with a chosen id and purpose, so a test can
    /// assert on the label a purpose produces.
    fn public_key(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
        IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id,
            purpose,
            security_level: SecurityLevel::HIGH,
            contract_bounds: None,
            key_type: KeyType::ECDSA_HASH160,
            read_only: false,
            data: BinaryData::new(vec![id as u8; 20]),
            disabled_at: None,
        })
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

    /// An `(Identity, IdentityPublicKey)` pair usable as an association value.
    fn association_pair(id: u8) -> (Identity, IdentityPublicKey) {
        let pv = PlatformVersion::latest();
        (
            Identity::create_basic_identity(Identifier::from([id; 32]), pv)
                .expect("association identity"),
            public_key(0, Purpose::VOTING),
        )
    }

    /// Put a key at `(target, key_id)` carrying `data`.
    fn put_key(
        qi: &mut QualifiedIdentity,
        target: PrivateKeyTarget,
        key_id: KeyID,
        purpose: Purpose,
        data: PrivateKeyData,
    ) {
        qi.private_keys.private_keys.insert(
            (target, key_id),
            (
                QualifiedIdentityPublicKey::from(public_key(key_id, purpose)),
                data,
            ),
        );
    }

    fn key_item(target: PrivateKeyTarget, key_id: KeyID) -> RecoveryItem {
        RecoveryItem::Key { target, key_id }
    }

    fn plan_items(plan: &RecoveryPlan) -> Vec<RecoveryItem> {
        plan.items.iter().map(|d| d.item.clone()).collect()
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
        let modern = bare_identity(0x01);
        let mut legacy = bare_identity(0x01);
        put_key(
            &mut legacy,
            M,
            1,
            Purpose::OWNER,
            PrivateKeyData::Clear([0xA0; 32]),
        );

        let plan = compute_recovery_plan(&modern, &legacy);

        assert_eq!(plan_items(&plan), vec![key_item(M, 1)]);
        assert!(plan.excluded.is_empty());
    }

    /// M2 — a key both records hold is never a candidate, and the modern bytes
    /// survive even when a stale allowlist names it. Legacy must never win a
    /// collision.
    #[test]
    fn m2_key_present_in_both_keeps_the_modern_bytes() {
        let mut modern = bare_identity(0x02);
        put_key(
            &mut modern,
            M,
            1,
            Purpose::OWNER,
            PrivateKeyData::Clear([0x11; 32]),
        );
        let mut legacy = bare_identity(0x02);
        put_key(
            &mut legacy,
            M,
            1,
            Purpose::OWNER,
            PrivateKeyData::Clear([0x99; 32]),
        );

        let plan = compute_recovery_plan(&modern, &legacy);
        assert!(
            plan.items.is_empty(),
            "a key the modern record already holds is not a candidate",
        );

        let applied = apply_recovery_plan(&modern, legacy, &[key_item(M, 1)]);
        assert_eq!(
            key_data(&applied.merged, M, 1),
            PrivateKeyData::Clear([0x11; 32]),
            "the modern key bytes must survive an approval that named the same slot",
        );
        assert!(applied.applied.is_empty(), "nothing was restorable");
    }

    /// M3 — a key only the modern record holds is never in the plan and comes
    /// through the merge byte-identical.
    #[test]
    fn m3_modern_only_key_is_untouched() {
        let mut modern = bare_identity(0x03);
        put_key(
            &mut modern,
            M,
            7,
            Purpose::TRANSFER,
            PrivateKeyData::Clear([0x33; 32]),
        );
        let mut legacy = bare_identity(0x03);
        put_key(
            &mut legacy,
            M,
            8,
            Purpose::OWNER,
            PrivateKeyData::Clear([0x44; 32]),
        );

        let plan = compute_recovery_plan(&modern, &legacy);
        assert_eq!(plan_items(&plan), vec![key_item(M, 8)]);

        let applied = apply_recovery_plan(&modern, legacy, &plan.approved_items());
        assert_eq!(
            key_data(&applied.merged, M, 7),
            PrivateKeyData::Clear([0x33; 32]),
            "the modern-only key must survive the merge byte-identical",
        );
        assert_eq!(
            key_data(&applied.merged, M, 8),
            PrivateKeyData::Clear([0x44; 32])
        );
    }

    /// M4 — each association is a candidate exactly when the modern record
    /// lacks it and the legacy record has it, across all four combinations.
    #[test]
    fn m4_associations_are_candidates_only_when_modern_is_none() {
        for (modern_has, legacy_has, expected) in [
            (false, true, true),
            (false, false, false),
            (true, true, false),
            (true, false, false),
        ] {
            let mut modern = bare_identity(0x04);
            let mut legacy = bare_identity(0x04);
            if modern_has {
                modern.associated_voter_identity = Some(association_pair(0xC0));
                modern.associated_operator_identity = Some(association_pair(0xC1));
                modern.associated_owner_key_id = Some(5);
            }
            if legacy_has {
                legacy.associated_voter_identity = Some(association_pair(0xD0));
                legacy.associated_operator_identity = Some(association_pair(0xD1));
                legacy.associated_owner_key_id = Some(9);
            }

            let plan = compute_recovery_plan(&modern, &legacy);
            let items = plan_items(&plan);
            for item in [
                RecoveryItem::VoterAssociation,
                RecoveryItem::OperatorAssociation,
                RecoveryItem::OwnerKeyAssociation,
            ] {
                assert_eq!(
                    items.contains(&item),
                    expected,
                    "{item:?}: modern_has={modern_has}, legacy_has={legacy_has}",
                );
            }
        }
    }

    /// M5 — a legacy per-key password envelope is excluded with its own reason
    /// and never merged, even when an allowlist names it.
    #[test]
    fn m5_legacy_encrypted_key_is_excluded_and_never_applied() {
        let modern = bare_identity(0x05);
        let mut legacy = bare_identity(0x05);
        put_key(
            &mut legacy,
            M,
            1,
            Purpose::OWNER,
            PrivateKeyData::Encrypted(vec![0x77; 48]),
        );

        let plan = compute_recovery_plan(&modern, &legacy);
        assert!(
            plan.items.is_empty(),
            "an unreadable key is not a candidate"
        );
        assert_eq!(plan.excluded.len(), 1);
        assert_eq!(plan.excluded[0].0.item, key_item(M, 1));
        assert_eq!(plan.excluded[0].1, ExclusionReason::LegacyEncryptedFormat);

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
        let modern = bare_identity(0x06);
        let mut legacy = bare_identity(0x06);
        put_key(
            &mut legacy,
            V,
            2,
            Purpose::VOTING,
            PrivateKeyData::Clear([0xB0; 32]),
        );
        legacy.associated_voter_identity = Some(association_pair(0xD0));

        let plan = compute_recovery_plan(&modern, &legacy);
        assert!(plan.voter_association_is_grouped());
        assert_eq!(
            plan.preview_items()
                .into_iter()
                .map(|d| d.item.clone())
                .collect::<Vec<_>>(),
            vec![key_item(V, 2)],
            "the voter link is folded into its voting key, not offered separately",
        );

        let applied = apply_recovery_plan(&modern, legacy, &[key_item(V, 2)]);
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
        let mut modern = bare_identity(0x07);
        put_key(
            &mut modern,
            M,
            1,
            Purpose::OWNER,
            PrivateKeyData::Clear([0x55; 32]),
        );
        modern.associated_voter_identity = Some(association_pair(0xE0));
        let legacy = modern.clone();

        let plan = compute_recovery_plan(&modern, &legacy);

        assert!(plan.is_empty(), "identical records have nothing to restore");
    }

    /// M8 — the case this flow exists for: a bare modern record (a ProTxHash-only
    /// load) offers the legacy record's whole key set and its associations.
    #[test]
    fn m8_bare_modern_record_offers_every_legacy_key() {
        let modern = bare_identity(0x08);
        let mut legacy = bare_identity(0x08);
        put_key(
            &mut legacy,
            M,
            1,
            Purpose::OWNER,
            PrivateKeyData::Clear([0xA0; 32]),
        );
        put_key(
            &mut legacy,
            M,
            2,
            Purpose::TRANSFER,
            PrivateKeyData::AlwaysClear([0xA1; 32]),
        );
        put_key(
            &mut legacy,
            V,
            3,
            Purpose::VOTING,
            PrivateKeyData::Clear([0xA2; 32]),
        );
        legacy.associated_voter_identity = Some(association_pair(0xD0));
        legacy.associated_owner_key_id = Some(1);

        let plan = compute_recovery_plan(&modern, &legacy);

        assert_eq!(
            plan_items(&plan),
            vec![
                key_item(M, 1),
                key_item(M, 2),
                key_item(V, 3),
                RecoveryItem::VoterAssociation,
                RecoveryItem::OwnerKeyAssociation,
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
        for legacy_alias in [None, Some("legacy-name")] {
            let mut modern = bare_identity(0x09);
            modern.alias = Some("modern-name".to_string());
            put_key(
                &mut modern,
                M,
                1,
                Purpose::OWNER,
                PrivateKeyData::Clear([0x11; 32]),
            );
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
                put_key(
                    &mut legacy,
                    M,
                    1,
                    Purpose::OWNER,
                    PrivateKeyData::Clear([0x99; 32]),
                );
                put_key(
                    &mut legacy,
                    M,
                    2,
                    Purpose::TRANSFER,
                    PrivateKeyData::Clear([0x22; 32]),
                );

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
        let mut modern = bare_identity(0x0A);
        put_key(
            &mut modern,
            M,
            1,
            Purpose::OWNER,
            PrivateKeyData::Clear([0x11; 32]),
        );
        let mut legacy = bare_identity(0x0A);
        put_key(
            &mut legacy,
            M,
            1,
            Purpose::OWNER,
            PrivateKeyData::Clear([0x99; 32]),
        );
        put_key(
            &mut legacy,
            M,
            2,
            Purpose::TRANSFER,
            PrivateKeyData::Clear([0x22; 32]),
        );

        // Key 1 was re-added since the preview; key 2 is still missing but was
        // never approved.
        let applied = apply_recovery_plan(&modern, legacy, &[key_item(M, 1)]);

        assert_eq!(
            stale_items(&applied),
            vec![key_item(M, 1)],
            "an approval that is no longer missing is reported stale",
        );
        assert!(applied.applied.is_empty());
        assert_eq!(
            key_data(&applied.merged, M, 1),
            PrivateKeyData::Clear([0x11; 32]),
        );
        assert!(
            !applied
                .merged
                .private_keys
                .private_keys
                .contains_key(&(M, 2)),
            "a candidate the user did not approve must never be merged",
        );
    }

    /// A wallet-derivation reference carries no plaintext, so it is a candidate
    /// and lands verbatim.
    #[test]
    fn wallet_derived_key_is_a_candidate_and_lands_verbatim() {
        let modern = bare_identity(0x0B);
        let mut legacy = bare_identity(0x0B);
        let reference = WalletDerivationPath {
            wallet_seed_hash: [0x07; 32],
            derivation_path: DerivationPath::from(vec![]),
        };
        put_key(
            &mut legacy,
            M,
            1,
            Purpose::AUTHENTICATION,
            PrivateKeyData::AtWalletDerivationPath(reference.clone()),
        );

        let plan = compute_recovery_plan(&modern, &legacy);
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
        let modern = bare_identity(0x0C);
        let mut legacy = bare_identity(0x0C);
        put_key(&mut legacy, M, 1, Purpose::OWNER, PrivateKeyData::InVault);

        let plan = compute_recovery_plan(&modern, &legacy);

        assert!(plan.items.is_empty());
        assert_eq!(plan.excluded[0].1, ExclusionReason::NoMaterial);
    }

    /// Every label a plan can produce is a distinct role word the operator
    /// recognises, with the voter target overriding the key's own purpose.
    #[test]
    fn labels_name_each_role_in_operator_terms() {
        assert_eq!(
            RecoveryItemDescriptor::key(M, 1, Some(Purpose::OWNER)).label(),
            "Owner key",
        );
        assert_eq!(
            RecoveryItemDescriptor::key(M, 1, Some(Purpose::TRANSFER)).label(),
            "Payout key",
        );
        assert_eq!(
            RecoveryItemDescriptor::key(V, 1, Some(Purpose::AUTHENTICATION)).label(),
            "Voting key",
            "a key on the voter identity is always the voting key",
        );
        assert_eq!(
            RecoveryItemDescriptor::association(RecoveryItem::VoterAssociation).label(),
            "Voting identity link",
        );
    }
}
