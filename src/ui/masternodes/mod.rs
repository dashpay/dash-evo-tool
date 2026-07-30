//! The Masternodes root screen domain (Expert-Mode gated).
//!
//! Node operators (the Priya persona) load masternode/evonode identities to
//! vote on DPNS name contests and manage owner/voting/payout keys. The page is
//! a sibling root screen behind the Expert-Mode nav gate (FR-1); its identities
//! are page-scoped and never leak into the everyday-user surfaces (FR-6, B1).
//!
//! The gate covers the screens, not this module's shared key helpers
//! ([`role_label_and_tip`], [`manage_keys_labels`], [`identity_keys`]): those
//! name and enumerate the keys of any identity and are used from ungated
//! surfaces — the identity keys list and the recovery-offer component — so that
//! one key cannot be called two different things on two screens. Whether a key
//! is *held* is resolved by
//! [`KeyStorage::candidates`](crate::model::qualified_identity::encrypted_key_storage::KeyStorage::candidates),
//! shared for the same reason.

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

/// Every caption a key's role can carry, as whole translation units.
///
/// Whole on purpose: a caller handed a bare role word inevitably bolts an
/// English noun onto it (`"{role} key"`), and a state onto that
/// (`"{role} key (disabled)"`), which is a sentence no translator can reorder,
/// re-inflect or re-punctuate. Each caption here is one string a translator
/// owns end to end. Private for the same reason — the only way to a caption is
/// [`role_label_and_tip`].
const LABEL_VOTING_KEY: &str = "Voting key";
const LABEL_VOTING_KEY_DISABLED: &str = "Voting key (disabled)";
const LABEL_OWNER_KEY: &str = "Owner key";
const LABEL_OWNER_KEY_DISABLED: &str = "Owner key (disabled)";
const LABEL_PAYOUT_KEY: &str = "Payout address key";
const LABEL_PAYOUT_KEY_DISABLED: &str = "Payout address key (disabled)";
const LABEL_TRANSFER_KEY: &str = "Transfer key";
const LABEL_TRANSFER_KEY_DISABLED: &str = "Transfer key (disabled)";
const LABEL_AUTH_KEY: &str = "Authentication key";
const LABEL_AUTH_KEY_DISABLED: &str = "Authentication key (disabled)";
const LABEL_ENCRYPTION_KEY: &str = "Encryption key";
const LABEL_ENCRYPTION_KEY_DISABLED: &str = "Encryption key (disabled)";
const LABEL_DECRYPTION_KEY: &str = "Decryption key";
const LABEL_DECRYPTION_KEY_DISABLED: &str = "Decryption key (disabled)";
const LABEL_SYSTEM_KEY: &str = "System key";
const LABEL_SYSTEM_KEY_DISABLED: &str = "System key (disabled)";

/// The complete caption for a key's role, and the tooltip explaining it.
///
/// The single source of truth for what a key is called anywhere in this app:
/// the detail view's "Manage keys" buttons, the identity keys list, the Key Info
/// page and the offer to restore keys from the previous version all name a key
/// through this function, so no two surfaces can name it differently.
///
/// Returns a whole caption, never a word to decorate — see the label constants
/// above for why, and note the `&'static str` return makes composing one at a
/// callsite impossible rather than merely discouraged. `disabled` selects the
/// caption for a key Platform has retired: a node that rotates its payout
/// address keeps the old key on chain beside the new one, so a role alone does
/// not name one key.
///
/// Voter-identity keys are always the voting key; on the main identity, the
/// Platform Owner and Transfer keys of a masternode identity mirror the ProTx
/// owner key and payout address. Every [`Purpose`](dash_sdk::dpp::identity::Purpose)
/// is matched by name and none falls through to a `Debug` rendering, so a raw
/// enum can never reach a user; a variant added upstream breaks this build
/// instead, which is the point.
pub fn role_label_and_tip(
    vocabulary: KeyVocabulary,
    is_on_voter_identity: bool,
    purpose: dash_sdk::dpp::identity::Purpose,
    disabled: bool,
) -> (&'static str, Option<&'static str>) {
    use dash_sdk::dpp::identity::Purpose;
    let user = vocabulary == KeyVocabulary::User;
    // (caption, caption once Platform has retired the key, tooltip)
    let (active, retired, tip) = if is_on_voter_identity {
        // A key on a voter identity is the voting key whatever its purpose says,
        // and only a masternode has one.
        (
            LABEL_VOTING_KEY,
            LABEL_VOTING_KEY_DISABLED,
            Some(TIP_VOTING_KEY),
        )
    } else {
        match purpose {
            Purpose::VOTING if user => (
                LABEL_VOTING_KEY,
                LABEL_VOTING_KEY_DISABLED,
                Some(TIP_USER_VOTING_KEY),
            ),
            Purpose::OWNER if user => (
                LABEL_OWNER_KEY,
                LABEL_OWNER_KEY_DISABLED,
                Some(TIP_USER_OWNER_KEY),
            ),
            Purpose::TRANSFER if user => (
                LABEL_TRANSFER_KEY,
                LABEL_TRANSFER_KEY_DISABLED,
                Some(TIP_USER_TRANSFER_KEY),
            ),
            Purpose::VOTING => (
                LABEL_VOTING_KEY,
                LABEL_VOTING_KEY_DISABLED,
                Some(TIP_VOTING_KEY),
            ),
            Purpose::OWNER => (
                LABEL_OWNER_KEY,
                LABEL_OWNER_KEY_DISABLED,
                Some(TIP_OWNER_KEY),
            ),
            Purpose::TRANSFER => (
                LABEL_PAYOUT_KEY,
                LABEL_PAYOUT_KEY_DISABLED,
                Some(TIP_PAYOUT_KEY),
            ),
            Purpose::AUTHENTICATION => {
                (LABEL_AUTH_KEY, LABEL_AUTH_KEY_DISABLED, Some(TIP_AUTH_KEY))
            }
            // No tooltip: these carry no role a user acts on here, but they
            // still get a name of their own rather than a raw enum.
            Purpose::ENCRYPTION => (LABEL_ENCRYPTION_KEY, LABEL_ENCRYPTION_KEY_DISABLED, None),
            Purpose::DECRYPTION => (LABEL_DECRYPTION_KEY, LABEL_DECRYPTION_KEY_DISABLED, None),
            Purpose::SYSTEM => (LABEL_SYSTEM_KEY, LABEL_SYSTEM_KEY_DISABLED, None),
        }
    };
    (if disabled { retired } else { active }, tip)
}

/// The complete caption for one key and its tooltip, from the shared
/// [`role_label_and_tip`] vocabulary. Reads the retired state off the key, so
/// no caller has to remember to mark it.
pub(crate) fn key_role_label(
    vocabulary: KeyVocabulary,
    target: &PrivateKeyTarget,
    key: &dash_sdk::platform::IdentityPublicKey,
) -> (&'static str, Option<&'static str>) {
    role_label_and_tip(
        vocabulary,
        *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity,
        key.purpose(),
        key.is_disabled(),
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
/// [`KeyStorage::candidates`](crate::model::qualified_identity::encrypted_key_storage::KeyStorage::candidates),
/// which every surface shares: enumerating alike while resolving differently is
/// how the two surfaces came to disagree about whether one key was saved on this
/// device.
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
/// Each label is the key's complete role caption from [`key_role_label`], which
/// already distinguishes a key Platform has retired. Keys that would still
/// collide are told apart by their key id.
pub fn manage_keys_labels(
    vocabulary: KeyVocabulary,
    keys: &[(PrivateKeyTarget, dash_sdk::platform::IdentityPublicKey)],
) -> Vec<(String, Option<&'static str>)> {
    let mut labels: Vec<(String, Option<&'static str>)> = keys
        .iter()
        .map(|(target, key)| {
            let (label, tip) = key_role_label(vocabulary, target, key);
            (label.to_string(), tip)
        })
        .collect();

    let key_ids: Vec<_> = keys.iter().map(|(_, key)| Some(key.id())).collect();
    disambiguate_role_labels(&mut labels, &key_ids);
    labels
}

/// The one form in which a role caption is qualified by the key's on-chain id.
///
/// A single translation unit with named placeholders, so a translation may
/// reorder or re-punctuate both parts. `label` is always a complete caption
/// from [`role_label_and_tip`] — never a fragment this then completes.
fn label_with_key_id(label: &str, key_id: dash_sdk::dpp::identity::KeyID) -> String {
    format!("{label} #{key_id}")
}

/// Make every label in `labelled` unique by qualifying the ones that would
/// otherwise appear more than once with their key id.
///
/// A role caption alone is not unique: an evonode that rotates its payout
/// address holds two Transfer keys. Every list of keys the user is asked to
/// read — and especially one they are asked to approve — needs each row to name
/// exactly one key. Rows with no key id (a role link) are left as they are.
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
            let qualified = label_with_key_id(label, *key_id);
            *label = qualified;
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
            role_label_and_tip(node, true, Purpose::AUTHENTICATION, false),
            (LABEL_VOTING_KEY, Some(TIP_VOTING_KEY))
        );
        // A voting-purpose key on the main identity is the voting key too.
        assert_eq!(
            role_label_and_tip(node, false, Purpose::VOTING, false),
            (LABEL_VOTING_KEY, Some(TIP_VOTING_KEY))
        );
        // Main-identity roles mirror the DIP-3 ProRegTx owner key and payout
        // address; the Platform Transfer key surfaces as "Payout address".
        assert_eq!(
            role_label_and_tip(node, false, Purpose::OWNER, false),
            (LABEL_OWNER_KEY, Some(TIP_OWNER_KEY))
        );
        assert_eq!(
            role_label_and_tip(node, false, Purpose::TRANSFER, false),
            (LABEL_PAYOUT_KEY, Some(TIP_PAYOUT_KEY))
        );
        assert_eq!(
            role_label_and_tip(node, false, Purpose::AUTHENTICATION, false),
            (LABEL_AUTH_KEY, Some(TIP_AUTH_KEY))
        );
        // A purpose with no role the user acts on here still gets a name of its
        // own — never the raw enum.
        assert_eq!(
            role_label_and_tip(node, false, Purpose::ENCRYPTION, false),
            (LABEL_ENCRYPTION_KEY, None)
        );
    }

    /// Every caption the app can show a user is a whole, self-contained phrase:
    /// one translation unit ending in the noun it names, never a role word the
    /// caller finishes, and never a raw `Purpose` rendered through `Debug`.
    ///
    /// Exhaustive over the whole vocabulary because that is the only way this
    /// holds for the combinations no screen happens to render today — an
    /// encryption key on a user identity, a retired system key — which is
    /// exactly where a `{:?}` fallback used to hide.
    #[test]
    fn every_role_caption_is_a_complete_translation_unit() {
        use dash_sdk::dpp::identity::Purpose;

        let purposes = [
            Purpose::AUTHENTICATION,
            Purpose::ENCRYPTION,
            Purpose::DECRYPTION,
            Purpose::TRANSFER,
            Purpose::SYSTEM,
            Purpose::VOTING,
            Purpose::OWNER,
        ];
        for vocabulary in [KeyVocabulary::Masternode, KeyVocabulary::User] {
            for purpose in purposes {
                for voter in [false, true] {
                    for disabled in [false, true] {
                        let (label, _) = role_label_and_tip(vocabulary, voter, purpose, disabled);
                        assert!(
                            !label.contains(&format!("{purpose:?}")),
                            "a raw Purpose must never reach a caption: {label}"
                        );
                        assert!(
                            label.starts_with(char::is_uppercase),
                            "a caption is a phrase of its own, so it opens like one: {label}"
                        );
                        let expected_tail = if disabled { "key (disabled)" } else { "key" };
                        assert!(
                            label.ends_with(expected_tail),
                            "a caption names the thing it is, complete: {label}"
                        );
                    }
                }
            }
        }
    }

    /// A retired key's caption is one unit too, not the active caption with an
    /// English state bolted on — the two are separately translatable strings.
    #[test]
    fn a_retired_key_gets_its_own_whole_caption() {
        use dash_sdk::dpp::identity::Purpose;
        let node = KeyVocabulary::Masternode;

        assert_eq!(
            role_label_and_tip(node, false, Purpose::TRANSFER, true),
            (LABEL_PAYOUT_KEY_DISABLED, Some(TIP_PAYOUT_KEY)),
            "retiring a key changes its caption, not its role or its tooltip"
        );
        // The user vocabulary retires its own wording, not the node's.
        assert_eq!(
            role_label_and_tip(KeyVocabulary::User, false, Purpose::TRANSFER, true),
            (LABEL_TRANSFER_KEY_DISABLED, Some(TIP_USER_TRANSFER_KEY))
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
            role_label_and_tip(user, false, Purpose::TRANSFER, false),
            (LABEL_TRANSFER_KEY, Some(TIP_USER_TRANSFER_KEY)),
            "a user's transfer key is not a masternode payout address"
        );
        assert_eq!(
            role_label_and_tip(user, false, Purpose::OWNER, false),
            (LABEL_OWNER_KEY, Some(TIP_USER_OWNER_KEY))
        );
        assert_eq!(
            role_label_and_tip(user, false, Purpose::VOTING, false),
            (LABEL_VOTING_KEY, Some(TIP_USER_VOTING_KEY))
        );
        // Already identity-neutral, so it is shared verbatim.
        assert_eq!(
            role_label_and_tip(user, false, Purpose::AUTHENTICATION, false),
            role_label_and_tip(
                KeyVocabulary::Masternode,
                false,
                Purpose::AUTHENTICATION,
                false
            )
        );

        // None of the node tooltips may reach a user identity.
        for purpose in [Purpose::TRANSFER, Purpose::OWNER, Purpose::VOTING] {
            let (_, tip) = role_label_and_tip(user, false, purpose, false);
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
}
