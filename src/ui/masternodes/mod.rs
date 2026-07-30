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

use crate::model::qualified_identity::MasternodeKeyPresence;

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
