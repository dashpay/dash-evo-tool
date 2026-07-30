//! The Masternodes root screen domain (Expert-Mode gated).
//!
//! Node operators (the Priya persona) load masternode/evonode identities to
//! vote on DPNS name contests and manage owner/voting/payout keys. The page is
//! a sibling root screen behind the Expert-Mode nav gate (FR-1); its identities
//! are page-scoped and never leak into the everyday-user surfaces (FR-6, B1).
//!
//! The gate covers the screens, not this module's shared key helpers
//! ([`role_label_and_tip`], [`manage_keys_labels`], [`identity_keys`],
//! [`key_filed_at`]): those name, enumerate and resolve the keys of any identity
//! and are used from ungated surfaces — the identity keys list and the
//! recovery-offer component — so that one key cannot be called two different
//! things, or reported as saved on one screen and missing on another.

pub mod card;
pub mod detail_screen;
pub mod list_screen;
pub mod load_form;
pub mod testnet_fixture;

pub use list_screen::MasternodesScreen;

use crate::model::qualified_identity::{
    IdentityType, MasternodeKeyPresence, PrivateKeyTarget, QualifiedIdentity,
};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;

/// Tooltip copy for the Dash Core DIP-3 ProRegTx key roles, shared by the detail
/// view's "Manage keys" list and the load form's key fields so both surfaces use
/// the same wording. These are the roles Dash Evo Tool manages on behalf of the
/// masternode's Platform identity; the operator BLS key and Platform node key are
/// held by the node operator and are not entered here.
pub const TIP_OWNER_KEY: &str = "The owner key authorizes changes to this masternode's registration on Dash Core, such as \
     updating its operator, voting, or payout details.";
pub const TIP_VOTING_KEY: &str = "The voting key signs this masternode's votes on Dash governance proposals and contested \
     DPNS usernames.";
pub const TIP_PAYOUT_KEY: &str = "The payout address key controls the address that receives this masternode's rewards. On \
     Dash Platform it also authorizes withdrawing this identity's credit balance.";

/// A single `V`/`O`/`P` role token: the letter shown when the key is loaded, the
/// tooltip explaining what that key does, and whether it is present.
pub struct KeyRoleToken {
    /// Single-letter glyph for the role (`V`, `O`, `P`).
    pub letter: &'static str,
    /// Role explanation, shared with the "Manage keys" list and the load form.
    pub tooltip: &'static str,
    /// Whether this key is loaded for the masternode identity.
    pub present: bool,
}

/// The three key-role tokens in display order, each paired with its tooltip and
/// presence. Single source of truth for what `V`, `O` and `P` mean, so the list
/// card and the detail screen cannot drift apart. Present roles render as their
/// letter, absent roles as `·`.
pub fn key_status_tokens(presence: MasternodeKeyPresence) -> [KeyRoleToken; 3] {
    [
        KeyRoleToken {
            letter: "V",
            tooltip: TIP_VOTING_KEY,
            present: presence.voting,
        },
        KeyRoleToken {
            letter: "O",
            tooltip: TIP_OWNER_KEY,
            present: presence.owner,
        },
        KeyRoleToken {
            letter: "P",
            tooltip: TIP_PAYOUT_KEY,
            present: presence.payout,
        },
    ]
}

/// Tooltip for an authentication key — Platform-only, so it has no DIP-3 role
/// counterpart, and identity-neutral wording already.
pub const TIP_AUTH_KEY: &str =
    "An authentication key signs this identity's actions on Dash Platform.";

/// Role tooltips for a plain user identity. The DIP-3 wording above describes
/// masternode registration duties — owning a node, updating its registration,
/// receiving its rewards — none of which a user identity has. Shown there it
/// does not merely read as jargon: it asserts the user owns a masternode.
pub const TIP_USER_TRANSFER_KEY: &str =
    "This key lets you send Dash out of this identity, to another identity or to a Dash address.";
pub const TIP_USER_OWNER_KEY: &str = "This key proves you own this identity.";
pub const TIP_USER_VOTING_KEY: &str = "This key signs votes on contested usernames.";

/// Which vocabulary a key's role is named in — the two differ for the same
/// [`Purpose`](dash_sdk::dpp::identity::Purpose), so it is the identity in front
/// of the user that decides, not the key.
///
/// A masternode's `TRANSFER` key is its payout address; a user's is how they
/// send funds. Both namings are correct for their own identity, which is why
/// this is a parameter rather than one of them being a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyVocabulary {
    /// DIP-3 ProRegTx role words, for masternode and evonode identities.
    Masternode,
    /// Plain-language role words, for user identities.
    User,
}

impl From<IdentityType> for KeyVocabulary {
    fn from(identity_type: IdentityType) -> Self {
        match identity_type {
            IdentityType::User => KeyVocabulary::User,
            IdentityType::Masternode | IdentityType::Evonode => KeyVocabulary::Masternode,
        }
    }
}

/// The role word for a key and its tooltip, aligned with the Dash Core DIP-3
/// ProRegTx roles.
///
/// The single source of truth for what a key is called anywhere in this app:
/// the detail view's "Manage keys" buttons and the offer to restore keys from
/// the previous version both label the same key through this function, so the
/// two surfaces cannot name it differently. Voter-identity keys are always the
/// voting key; on the main identity, the Platform Owner and Transfer keys of a
/// masternode identity mirror the ProTx owner key and payout address. Unknown
/// purposes fall back to their name with no tooltip.
pub fn role_label_and_tip(
    vocabulary: KeyVocabulary,
    is_on_voter_identity: bool,
    purpose: dash_sdk::dpp::identity::Purpose,
) -> (String, Option<&'static str>) {
    use dash_sdk::dpp::identity::Purpose;
    // A key on a voter identity is the voting key whatever its purpose says, and
    // only a masternode has one.
    if is_on_voter_identity {
        return ("Voting".to_string(), Some(TIP_VOTING_KEY));
    }
    let user = vocabulary == KeyVocabulary::User;
    match purpose {
        Purpose::VOTING if user => ("Voting".to_string(), Some(TIP_USER_VOTING_KEY)),
        Purpose::OWNER if user => ("Owner".to_string(), Some(TIP_USER_OWNER_KEY)),
        Purpose::TRANSFER if user => ("Transfer".to_string(), Some(TIP_USER_TRANSFER_KEY)),
        Purpose::VOTING => ("Voting".to_string(), Some(TIP_VOTING_KEY)),
        Purpose::OWNER => ("Owner".to_string(), Some(TIP_OWNER_KEY)),
        Purpose::TRANSFER => ("Payout address".to_string(), Some(TIP_PAYOUT_KEY)),
        Purpose::AUTHENTICATION => ("Authentication".to_string(), Some(TIP_AUTH_KEY)),
        other => (format!("{other:?}"), None),
    }
}

/// A short role name for one key and its tooltip, from the shared
/// [`role_label_and_tip`] vocabulary.
pub(crate) fn key_role_label(
    vocabulary: KeyVocabulary,
    target: &PrivateKeyTarget,
    key: &dash_sdk::platform::IdentityPublicKey,
) -> (String, Option<&'static str>) {
    role_label_and_tip(
        vocabulary,
        *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity,
        key.purpose(),
    )
}

/// Every key of `identity` a "Manage keys" list shows: main-identity keys
/// first, then voter-identity keys, each paired with the [`PrivateKeyTarget`]
/// that scopes it.
///
/// Shared so the masternode detail view and the identity keys list enumerate
/// keys identically. The target paired here is the *structural* one — which
/// identity's key map the key came from — which is only half of pairing a public
/// key with the private material the device may hold for it. Resolving that is
/// [`key_filed_at`], shared for the same reason: enumerating alike while
/// resolving differently is how the two surfaces came to disagree about whether
/// one key was saved on this device.
pub fn identity_keys(
    identity: &QualifiedIdentity,
) -> Vec<(PrivateKeyTarget, dash_sdk::platform::IdentityPublicKey)> {
    let mut keys: Vec<_> = identity
        .identity
        .public_keys()
        .values()
        .map(|key| (PrivateKeyTarget::PrivateKeyOnMainIdentity, key.clone()))
        .collect();
    if let Some((voter, _)) = identity.associated_voter_identity.as_ref() {
        keys.extend(
            voter
                .public_keys()
                .values()
                .map(|key| (PrivateKeyTarget::PrivateKeyOnVoterIdentity, key.clone())),
        );
    }
    keys
}

/// Whether a stored public key and a live one are the same key.
///
/// Compares every field except `disabled_at`. A Platform identity public key is
/// immutable once added, with that single exception: disabling one rewrites that
/// field. The stored copy is a snapshot taken when the private half was saved, so
/// plain `==` stops matching as soon as a key is disabled or rotated, and a key
/// this device demonstrably holds is reported as missing.
///
/// Comparing only the id and the key material would fix that and reopen a worse
/// hole in the other direction. Both are shared by construction where it matters:
/// `id` is already the lookup key, and a main identity's voting key and a linked
/// voter identity's key can carry identical `data`, leaving `purpose` as the only
/// thing telling them apart. Conflating those hands over private material the
/// clicked key does not own — so this excludes the one field that legitimately
/// moves, and nothing else.
fn same_key(
    stored: &dash_sdk::platform::IdentityPublicKey,
    live: &dash_sdk::platform::IdentityPublicKey,
) -> bool {
    stored.id() == live.id()
        && stored.key_type() == live.key_type()
        && stored.purpose() == live.purpose()
        && stored.security_level() == live.security_level()
        && stored.read_only() == live.read_only()
        && stored.contract_bounds() == live.contract_bounds()
        && stored.data() == live.data()
}

/// Which store `key`'s private half is actually in, or `None` if this device
/// holds no material for it.
///
/// `structural` is the target [`identity_keys`] paired the key with — which
/// identity's key map it was found in. Two conventions for that target are in use
/// in this codebase: the structural one, and `impl From<Purpose> for
/// PrivateKeyTarget`, which files by purpose alone. Real installs hold material
/// written under each, so both are tried.
///
/// Shared by every "Manage keys" surface on purpose. [`identity_keys`] enumerates
/// the same keys for all of them and this resolves held-ness the same way for all
/// of them; splitting either one is how the identity keys list and the masternode
/// detail view came to disagree about whether one key was saved on this device.
///
/// This is a read. Looking in the second place can only find material that is
/// already there, so it corrects a false "not saved on this device" without
/// moving anything; reconciling the two conventions on the write path is a
/// migration, tracked separately.
///
/// Each candidate has to hold *this* public key, not merely have its slot filled:
/// a voter identity's own key can share a key id with a main-identity key, so an
/// occupied slot proves nothing about whose material is in it.
///
/// Reads the stored public half only, never the private one: fetching the entry
/// would clone the raw private key out of the vault unscrubbed, and this runs
/// every frame for every key.
pub fn key_filed_at(
    identity: &QualifiedIdentity,
    structural: &PrivateKeyTarget,
    key: &dash_sdk::platform::IdentityPublicKey,
) -> Option<PrivateKeyTarget> {
    let derived: PrivateKeyTarget = key.purpose().into();
    [structural.clone(), derived].into_iter().find(|candidate| {
        identity
            .private_keys
            .public_key_for(&(candidate.clone(), key.id()))
            .is_some_and(|stored| same_key(&stored.identity_public_key, key))
    })
}

/// Button labels (and DIP-3-aligned tooltips) for a "Manage keys" list, one
/// per entry of `keys`, in order.
///
/// Each label is the key's role word (`Owner`/`Payout address`/`Voting`/…)
/// plus a `(disabled)` marker for keys platform has retired: a node that
/// rotates its payout address keeps the old, disabled Payout key on-chain
/// next to the new active one, so a role word alone is not unique. Keys that
/// would still collide are told apart by their key id.
pub fn manage_keys_labels(
    vocabulary: KeyVocabulary,
    keys: &[(PrivateKeyTarget, dash_sdk::platform::IdentityPublicKey)],
) -> Vec<(String, Option<&'static str>)> {
    let mut labels: Vec<(String, Option<&'static str>)> = keys
        .iter()
        .map(|(target, key)| {
            let (role, tip) = key_role_label(vocabulary, target, key);
            let label = if key.is_disabled() {
                format!("{role} key (disabled)")
            } else {
                format!("{role} key")
            };
            (label, tip)
        })
        .collect();

    let key_ids: Vec<_> = keys.iter().map(|(_, key)| Some(key.id())).collect();
    disambiguate_role_labels(&mut labels, &key_ids);
    labels
}

/// Make every label in `labelled` unique by appending `#{key id}` to the ones
/// that would otherwise appear more than once.
///
/// A role word alone is not unique: an evonode that rotates its payout address
/// holds two Transfer keys. Every list of keys the user is asked to read — and
/// especially one they are asked to approve — needs each row to name exactly
/// one key. Rows with no key id (a role link) are left as they are.
pub fn disambiguate_role_labels(
    labelled: &mut [(String, Option<&'static str>)],
    key_ids: &[Option<dash_sdk::dpp::identity::KeyID>],
) {
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (label, _) in labelled.iter() {
        *counts.entry(label.clone()).or_default() += 1;
    }
    for ((label, _), key_id) in labelled.iter_mut().zip(key_ids) {
        if let Some(key_id) = key_id
            && counts.get(label.as_str()).copied().unwrap_or(0) > 1
        {
            *label = format!("{label} #{key_id}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a masternode key with a chosen id / purpose / disabled state.
    fn mn_key(
        id: dash_sdk::dpp::identity::KeyID,
        purpose: dash_sdk::dpp::identity::Purpose,
        disabled: bool,
    ) -> dash_sdk::platform::IdentityPublicKey {
        use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
        use dash_sdk::dpp::identity::{KeyType, SecurityLevel};
        use dash_sdk::dpp::platform_value::BinaryData;
        IdentityPublicKeyV0 {
            id,
            key_type: KeyType::ECDSA_HASH160,
            purpose,
            security_level: SecurityLevel::CRITICAL,
            read_only: true,
            data: BinaryData::new(vec![id as u8; 20]),
            disabled_at: disabled.then_some(1),
            contract_bounds: None,
        }
        .into()
    }

    /// An evonode that has rotated its payout address holds two `TRANSFER`
    /// (Payout) keys on its main identity — the active new one and the disabled
    /// old one — plus the owner key and a voter-identity voting key. Every
    /// "Manage keys" button must get a distinct, correct label: the disabled
    /// payout key is marked `(disabled)` instead of colliding with the active
    /// one under a bare "Payout key".
    #[test]
    fn manage_keys_labels_disambiguate_rotated_evonode_payout_keys() {
        use dash_sdk::dpp::identity::Purpose;
        let keys = vec![
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(0, Purpose::TRANSFER, false),
            ),
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(1, Purpose::OWNER, false),
            ),
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(2, Purpose::TRANSFER, true),
            ),
            (
                PrivateKeyTarget::PrivateKeyOnVoterIdentity,
                mn_key(0, Purpose::VOTING, false),
            ),
        ];

        let labels: Vec<String> = manage_keys_labels(KeyVocabulary::Masternode, &keys)
            .into_iter()
            .map(|(label, _tip)| label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "Payout address key".to_string(),
                "Owner key".to_string(),
                "Payout address key (disabled)".to_string(),
                "Voting key".to_string(),
            ]
        );
        // No two buttons ever share a label.
        let unique: std::collections::BTreeSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "labels must be unique");
    }

    /// When even the role + `(disabled)` marker still collides — a payout
    /// address rotated twice leaves two disabled Payout keys — the key id
    /// breaks the tie so every button stays unique.
    #[test]
    fn manage_keys_labels_fall_back_to_key_id_on_residual_collision() {
        use dash_sdk::dpp::identity::Purpose;
        let keys = vec![
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(0, Purpose::TRANSFER, false),
            ),
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(2, Purpose::TRANSFER, true),
            ),
            (
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                mn_key(3, Purpose::TRANSFER, true),
            ),
        ];

        let labels: Vec<String> = manage_keys_labels(KeyVocabulary::Masternode, &keys)
            .into_iter()
            .map(|(label, _tip)| label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "Payout address key".to_string(),
                "Payout address key (disabled) #2".to_string(),
                "Payout address key (disabled) #3".to_string(),
            ]
        );
        let unique: std::collections::BTreeSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "labels must be unique");
    }

    #[test]
    fn role_labels_follow_dip3_protx_terms() {
        use dash_sdk::dpp::identity::Purpose;
        let node = KeyVocabulary::Masternode;
        // A voter-identity key is always the voting key, regardless of purpose.
        assert_eq!(
            role_label_and_tip(node, true, Purpose::AUTHENTICATION),
            ("Voting".to_string(), Some(TIP_VOTING_KEY))
        );
        // A voting-purpose key on the main identity is the voting key too.
        assert_eq!(
            role_label_and_tip(node, false, Purpose::VOTING),
            ("Voting".to_string(), Some(TIP_VOTING_KEY))
        );
        // Main-identity roles mirror the DIP-3 ProRegTx owner key and payout
        // address; the Platform Transfer key surfaces as "Payout address".
        assert_eq!(
            role_label_and_tip(node, false, Purpose::OWNER),
            ("Owner".to_string(), Some(TIP_OWNER_KEY))
        );
        assert_eq!(
            role_label_and_tip(node, false, Purpose::TRANSFER),
            ("Payout address".to_string(), Some(TIP_PAYOUT_KEY))
        );
        assert_eq!(
            role_label_and_tip(node, false, Purpose::AUTHENTICATION),
            ("Authentication".to_string(), Some(TIP_AUTH_KEY))
        );
        // An unmapped purpose keeps its name and carries no tooltip.
        assert_eq!(
            role_label_and_tip(node, false, Purpose::ENCRYPTION),
            (format!("{purpose:?}", purpose = Purpose::ENCRYPTION), None,)
        );
    }

    /// A user identity never owns a masternode, so the DIP-3 duties the node
    /// wording describes — updating a registration, receiving node rewards — are
    /// not merely jargon there: they assert something untrue about the identity
    /// in front of the user. Same key, same purpose, different identity, and the
    /// words have to follow the identity.
    #[test]
    fn a_user_identity_gets_plain_language_role_words() {
        use dash_sdk::dpp::identity::Purpose;
        let user = KeyVocabulary::User;

        assert_eq!(
            role_label_and_tip(user, false, Purpose::TRANSFER),
            ("Transfer".to_string(), Some(TIP_USER_TRANSFER_KEY)),
            "a user's transfer key is not a masternode payout address"
        );
        assert_eq!(
            role_label_and_tip(user, false, Purpose::OWNER),
            ("Owner".to_string(), Some(TIP_USER_OWNER_KEY))
        );
        assert_eq!(
            role_label_and_tip(user, false, Purpose::VOTING),
            ("Voting".to_string(), Some(TIP_USER_VOTING_KEY))
        );
        // Already identity-neutral, so it is shared verbatim.
        assert_eq!(
            role_label_and_tip(user, false, Purpose::AUTHENTICATION),
            role_label_and_tip(KeyVocabulary::Masternode, false, Purpose::AUTHENTICATION)
        );

        // None of the node tooltips may reach a user identity.
        for purpose in [Purpose::TRANSFER, Purpose::OWNER, Purpose::VOTING] {
            let (_, tip) = role_label_and_tip(user, false, purpose);
            let tip = tip.expect("every mapped role carries a tooltip");
            assert!(
                ![TIP_OWNER_KEY, TIP_VOTING_KEY, TIP_PAYOUT_KEY].contains(&tip),
                "a user identity must not be told about masternode duties: {tip}"
            );
        }
    }

    /// The identity type decides the vocabulary, and evonodes read as nodes.
    #[test]
    fn vocabulary_follows_the_identity_type() {
        assert_eq!(KeyVocabulary::from(IdentityType::User), KeyVocabulary::User);
        assert_eq!(
            KeyVocabulary::from(IdentityType::Masternode),
            KeyVocabulary::Masternode
        );
        assert_eq!(
            KeyVocabulary::from(IdentityType::Evonode),
            KeyVocabulary::Masternode
        );
    }

    /// Two rows that would read identically are told apart by their key id;
    /// rows that are already unique, and rows with no key id, are left alone.
    #[test]
    fn colliding_role_labels_are_told_apart_by_key_id() {
        let mut labels = vec![
            ("Payout address key".to_string(), None),
            ("Payout address key".to_string(), None),
            ("Owner key".to_string(), None),
            ("Voting identity link".to_string(), None),
        ];
        disambiguate_role_labels(&mut labels, &[Some(2), Some(3), Some(1), None]);

        assert_eq!(
            labels.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(),
            vec![
                "Payout address key #2",
                "Payout address key #3",
                "Owner key",
                "Voting identity link",
            ],
        );
    }

    #[test]
    fn tc_fr3_08_key_tokens_reflect_presence() {
        let tokens = key_status_tokens(MasternodeKeyPresence {
            voting: true,
            owner: false,
            payout: true,
        });
        let presence: Vec<_> = tokens.iter().map(|t| (t.letter, t.present)).collect();
        assert_eq!(presence, vec![("V", true), ("O", false), ("P", true)]);
    }

    /// Each letter must carry the DIP-3 ProTx role wording used by the "Manage
    /// keys" buttons right below it — hovering `V` must not explain the owner key.
    #[test]
    fn key_tokens_carry_matching_role_tooltips() {
        let tokens = key_status_tokens(MasternodeKeyPresence::default());
        let tips: Vec<_> = tokens.iter().map(|t| (t.letter, t.tooltip)).collect();
        assert_eq!(
            tips,
            vec![
                ("V", TIP_VOTING_KEY),
                ("O", TIP_OWNER_KEY),
                ("P", TIP_PAYOUT_KEY),
            ]
        );
    }

    /// Build a `QualifiedIdentity` holding the private half of `stored` filed at
    /// `at`, whose on-chain key set is `live`.
    fn identity_holding(
        at: PrivateKeyTarget,
        stored: dash_sdk::platform::IdentityPublicKey,
        live: &[dash_sdk::platform::IdentityPublicKey],
    ) -> QualifiedIdentity {
        use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
        use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType};
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;
        use dash_sdk::platform::Identifier;
        use std::collections::BTreeMap;

        let identity = Identity::new_with_id_and_keys(
            Identifier::from([0x60u8; 32]),
            live.iter().map(|k| (k.id(), k.clone())).collect(),
            PlatformVersion::latest(),
        )
        .expect("identity with keys");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: Some("filed-at".to_string()),
            private_keys: KeyStorage {
                private_keys: BTreeMap::from([(
                    (at, stored.id()),
                    (
                        QualifiedIdentityPublicKey::from(stored),
                        PrivateKeyData::Clear([0x60; 32]),
                    ),
                )]),
            },
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        }
    }

    /// The resolution rule every "Manage keys" surface shares, in one place.
    ///
    /// Two target conventions are in use, so both are tried; a key must be
    /// matched by its own public half rather than by an occupied slot; and
    /// `disabled_at` must not break the match, because it is the one field
    /// Platform lets move after a key is added.
    #[test]
    fn held_material_is_found_under_either_convention_and_only_for_the_right_key() {
        use dash_sdk::dpp::identity::Purpose;

        // Filed structurally, where `identity_keys` says it is.
        let key = mn_key(0, Purpose::AUTHENTICATION, false);
        let identity = identity_holding(
            PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key.clone(),
            std::slice::from_ref(&key),
        );
        assert_eq!(
            key_filed_at(&identity, &PrivateKeyTarget::PrivateKeyOnMainIdentity, &key),
            Some(PrivateKeyTarget::PrivateKeyOnMainIdentity),
            "material filed where the key structurally sits must be found"
        );

        // Filed under the purpose-derived convention instead: a voting key on
        // the main identity, entered by hand. Reported as unheld before the
        // fallback existed.
        let voting = mn_key(0, Purpose::VOTING, false);
        let identity = identity_holding(
            PrivateKeyTarget::PrivateKeyOnVoterIdentity,
            voting.clone(),
            std::slice::from_ref(&voting),
        );
        assert_eq!(
            key_filed_at(
                &identity,
                &PrivateKeyTarget::PrivateKeyOnMainIdentity,
                &voting
            ),
            Some(PrivateKeyTarget::PrivateKeyOnVoterIdentity),
            "material filed by purpose derivation must still be found"
        );

        // A *different* key at the same id must not match. These two share
        // `id` and `data`, so purpose is the only thing telling them apart —
        // matching here would report one key as held on the strength of
        // another's private half.
        assert_eq!(
            key_filed_at(
                &identity,
                &PrivateKeyTarget::PrivateKeyOnMainIdentity,
                &mn_key(0, Purpose::AUTHENTICATION, false)
            ),
            None,
            "an occupied slot proves nothing about whose material is in it"
        );

        // Disabling a key on chain does not remove its private half from this
        // device, so the stored snapshot must still match the live key.
        let disabled = mn_key(0, Purpose::AUTHENTICATION, true);
        let identity = identity_holding(
            PrivateKeyTarget::PrivateKeyOnMainIdentity,
            mn_key(0, Purpose::AUTHENTICATION, false),
            std::slice::from_ref(&disabled),
        );
        assert_eq!(
            key_filed_at(
                &identity,
                &PrivateKeyTarget::PrivateKeyOnMainIdentity,
                &disabled
            ),
            Some(PrivateKeyTarget::PrivateKeyOnMainIdentity),
            "`disabled_at` is the one field that legitimately moves, so it must \
             not break the match"
        );
    }
}
