//! The Masternodes root screen domain (Expert-Mode gated).
//!
//! Node operators (the Priya persona) load masternode/evonode identities to
//! vote on DPNS name contests and manage owner/voting/payout keys. The page is
//! a sibling root screen behind the Expert-Mode nav gate (FR-1); its identities
//! are page-scoped and never leak into the everyday-user surfaces (FR-6, B1).

pub mod card;
pub mod detail_screen;
pub mod list_screen;
pub mod load_form;
pub mod testnet_fixture;

pub use list_screen::MasternodesScreen;

use crate::model::qualified_identity::{
    MasternodeKeyPresence, PrivateKeyTarget, QualifiedIdentity,
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
/// counterpart.
pub const TIP_AUTH_KEY: &str =
    "An authentication key signs this identity's actions on Dash Platform.";

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
    is_on_voter_identity: bool,
    purpose: dash_sdk::dpp::identity::Purpose,
) -> (String, Option<&'static str>) {
    use dash_sdk::dpp::identity::Purpose;
    if is_on_voter_identity {
        return ("Voting".to_string(), Some(TIP_VOTING_KEY));
    }
    match purpose {
        Purpose::VOTING => ("Voting".to_string(), Some(TIP_VOTING_KEY)),
        Purpose::OWNER => ("Owner".to_string(), Some(TIP_OWNER_KEY)),
        Purpose::TRANSFER => ("Payout address".to_string(), Some(TIP_PAYOUT_KEY)),
        Purpose::AUTHENTICATION => ("Authentication".to_string(), Some(TIP_AUTH_KEY)),
        other => (format!("{other:?}"), None),
    }
}

/// A short role name for one key and its tooltip, from the shared
/// [`role_label_and_tip`] vocabulary.
pub fn key_role_label(
    target: &PrivateKeyTarget,
    key: &dash_sdk::platform::IdentityPublicKey,
) -> (String, Option<&'static str>) {
    role_label_and_tip(
        *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity,
        key.purpose(),
    )
}

/// Every key of `identity` a "Manage keys" list shows: main-identity keys
/// first, then voter-identity keys, each paired with the [`PrivateKeyTarget`]
/// that scopes it.
///
/// Shared so the masternode detail view and the identity keys list enumerate
/// keys identically. The target is what pairs a public key with the private
/// material the device may hold for it, so deriving it in one place is what
/// keeps the two surfaces agreeing on which keys are held.
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

/// Button labels (and DIP-3-aligned tooltips) for a "Manage keys" list, one
/// per entry of `keys`, in order.
///
/// Each label is the key's role word (`Owner`/`Payout address`/`Voting`/…)
/// plus a `(disabled)` marker for keys platform has retired: a node that
/// rotates its payout address keeps the old, disabled Payout key on-chain
/// next to the new active one, so a role word alone is not unique. Keys that
/// would still collide are told apart by their key id.
pub fn manage_keys_labels(
    keys: &[(PrivateKeyTarget, dash_sdk::platform::IdentityPublicKey)],
) -> Vec<(String, Option<&'static str>)> {
    let mut labels: Vec<(String, Option<&'static str>)> = keys
        .iter()
        .map(|(target, key)| {
            let (role, tip) = key_role_label(target, key);
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

        let labels: Vec<String> = manage_keys_labels(&keys)
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

        let labels: Vec<String> = manage_keys_labels(&keys)
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
        // A voter-identity key is always the voting key, regardless of purpose.
        assert_eq!(
            role_label_and_tip(true, Purpose::AUTHENTICATION),
            ("Voting".to_string(), Some(TIP_VOTING_KEY))
        );
        // A voting-purpose key on the main identity is the voting key too.
        assert_eq!(
            role_label_and_tip(false, Purpose::VOTING),
            ("Voting".to_string(), Some(TIP_VOTING_KEY))
        );
        // Main-identity roles mirror the DIP-3 ProRegTx owner key and payout
        // address; the Platform Transfer key surfaces as "Payout address".
        assert_eq!(
            role_label_and_tip(false, Purpose::OWNER),
            ("Owner".to_string(), Some(TIP_OWNER_KEY))
        );
        assert_eq!(
            role_label_and_tip(false, Purpose::TRANSFER),
            ("Payout address".to_string(), Some(TIP_PAYOUT_KEY))
        );
        assert_eq!(
            role_label_and_tip(false, Purpose::AUTHENTICATION),
            ("Authentication".to_string(), Some(TIP_AUTH_KEY))
        );
        // An unmapped purpose keeps its name and carries no tooltip.
        assert_eq!(
            role_label_and_tip(false, Purpose::ENCRYPTION),
            (format!("{purpose:?}", purpose = Purpose::ENCRYPTION), None,)
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
}
