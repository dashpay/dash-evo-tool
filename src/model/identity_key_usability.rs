//! Which identity keys can sign a given state transition right now.
//!
//! From protocol version 14 an identity key may carry usage limits (a lifetime
//! `total_budget` and an `expires_at` instant, V1 keys only) and an
//! AUTHENTICATION key may be bound to one contract, one document type or a
//! contract group. Picking a signing key by purpose, security level and key type
//! alone can therefore choose a key Platform refuses: an expired key, a key
//! bound elsewhere, or a contract-bound key on a transition outside a batch.
//!
//! These are pure rules — the caller passes the wall clock — mirroring upstream
//! `platform-wallet`'s `key_selection`: an unlimited key is preferred over a
//! limited one, a contract-group bound key is never chosen automatically (group
//! membership is Platform state, not known offline), and what is left of a
//! budget is not known offline either, so a spent key is refused by Platform.

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v1::IdentityPublicKeyGettersV1;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::identity::{
    Identity, IdentityPublicKey, KeyType, Purpose, SecurityLevel, TimestampMillis,
};
use dash_sdk::platform::Identifier;

/// What a signature authorizes, as far as contract bounds are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningScope<'a> {
    /// A state transition outside a batch: identity update, credit transfer or
    /// withdrawal, data contract create or update, masternode vote. Platform
    /// refuses a contract-bound key on any of them.
    NonBatch,
    /// A document transition on `document_type_name` of `contract_id`.
    Document {
        contract_id: Identifier,
        document_type_name: &'a str,
    },
    /// Transitions spanning `contract_id` as a whole: a token transition (token
    /// operations are contract-wide), or document transitions on several
    /// document types of one contract (DPNS preorder + domain). A document-type
    /// bound never covers them.
    ContractWide { contract_id: Identifier },
}

/// Why a key cannot sign in a [`SigningScope`], or what the user should know
/// before choosing it. Ordered by severity: the first applicable one is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCaveat {
    /// The key is disabled; Platform refuses it.
    Disabled,
    /// The key's expiry has passed; Platform refuses it.
    Expired { expired_at: TimestampMillis },
    /// The key is bound to a different contract or document type.
    OutOfBounds,
    /// The key is bound to a contract group; whether the group covers this
    /// contract is only known to Platform.
    ContractGroupBound { group_id: Identifier },
    /// The key has a lifetime budget; Platform refuses it once the budget is
    /// spent.
    Budgeted { total_budget: u64 },
    /// The key expires at `expires_at` (in the future).
    Expiring { expires_at: TimestampMillis },
}

impl KeyCaveat {
    /// Whether Platform is certain to refuse a signature from the key.
    pub fn blocks_signing(&self) -> bool {
        match self {
            KeyCaveat::Disabled | KeyCaveat::Expired { .. } | KeyCaveat::OutOfBounds => true,
            KeyCaveat::ContractGroupBound { .. }
            | KeyCaveat::Budgeted { .. }
            | KeyCaveat::Expiring { .. } => false,
        }
    }
}

/// Whether a key carrying `bounds` may sign in `scope`, when that can be known
/// offline. `None` means only Platform can answer (a contract-group bound on a
/// batch transition).
pub fn bounds_permit(bounds: Option<&ContractBounds>, scope: SigningScope<'_>) -> Option<bool> {
    match (bounds, scope) {
        (None, _) => Some(true),
        (Some(_), SigningScope::NonBatch) => Some(false),
        (
            Some(ContractBounds::SingleContract { id }),
            SigningScope::Document { contract_id, .. } | SigningScope::ContractWide { contract_id },
        ) => Some(*id == contract_id),
        (
            Some(ContractBounds::SingleContractDocumentType {
                id,
                document_type_name: bound_type,
            }),
            SigningScope::Document {
                contract_id,
                document_type_name,
            },
        ) => Some(*id == contract_id && bound_type == document_type_name),
        (
            Some(ContractBounds::SingleContractDocumentType { .. }),
            SigningScope::ContractWide { .. },
        ) => Some(false),
        (
            Some(ContractBounds::ContractGroup { .. }),
            SigningScope::Document { .. } | SigningScope::ContractWide { .. },
        ) => None,
    }
}

/// Everything the user should know about signing with `key` in `scope` at
/// `now_ms`, most severe first. Empty for an unlimited, unbound, live key.
pub fn key_caveats(
    key: &IdentityPublicKey,
    scope: SigningScope<'_>,
    now_ms: TimestampMillis,
) -> Vec<KeyCaveat> {
    let mut caveats = Vec::new();
    if key.is_disabled() {
        caveats.push(KeyCaveat::Disabled);
    }
    if let Some(expires_at) = key.expires_at()
        && key.is_expired_at(now_ms)
    {
        caveats.push(KeyCaveat::Expired {
            expired_at: expires_at,
        });
    }
    match bounds_permit(key.contract_bounds(), scope) {
        Some(true) => {}
        Some(false) => caveats.push(KeyCaveat::OutOfBounds),
        None => {
            if let Some(group_id) = key.contract_bounds().and_then(|b| b.contract_group_id()) {
                caveats.push(KeyCaveat::ContractGroupBound {
                    group_id: *group_id,
                });
            }
        }
    }
    if let Some(total_budget) = key.total_budget() {
        caveats.push(KeyCaveat::Budgeted { total_budget });
    }
    if let Some(expires_at) = key.expires_at()
        && !key.is_expired_at(now_ms)
    {
        caveats.push(KeyCaveat::Expiring { expires_at });
    }
    caveats
}

/// Whether `key` may be picked automatically for `scope` at `now_ms`: not
/// disabled, not expired, and bounds known to permit the scope.
pub fn is_auto_selectable(
    key: &IdentityPublicKey,
    scope: SigningScope<'_>,
    now_ms: TimestampMillis,
) -> bool {
    !key.is_disabled()
        && !key.is_expired_at(now_ms)
        && bounds_permit(key.contract_bounds(), scope) == Some(true)
}

/// The signing key requirements of a transition.
#[derive(Debug, Clone, Copy)]
pub struct KeyRequirements<'a> {
    pub purpose: Purpose,
    pub security_levels: &'a [SecurityLevel],
    /// `None` accepts every key type.
    pub key_types: Option<&'a [KeyType]>,
    pub scope: SigningScope<'a>,
}

impl KeyRequirements<'_> {
    fn matches(&self, key: &IdentityPublicKey) -> bool {
        key.purpose() == self.purpose
            && self.security_levels.contains(&key.security_level())
            && self
                .key_types
                .is_none_or(|types| types.contains(&key.key_type()))
    }
}

/// The key of `keys` to pick automatically for `requirements` at `now_ms`
/// among those `can_sign` accepts, or `None` when none qualifies.
///
/// Mirrors upstream platform-wallet's signer-aware `usable_authentication_key`:
/// a key qualifies when it matches purpose, security level and key type and
/// [`is_auto_selectable`]; qualifying unlimited keys come first, then limited
/// ones (a budgeted or expiring key is an application key), each in `keys`
/// order (key id order for an identity's key map); the first of that sequence
/// this device can sign with wins. So a limited key held here beats an
/// unlimited one whose private half lives elsewhere — the delegated-key case
/// protocol version 14 limits exist for.
pub fn select_signing_key<'k>(
    keys: impl IntoIterator<Item = &'k IdentityPublicKey>,
    requirements: KeyRequirements<'_>,
    now_ms: TimestampMillis,
    can_sign: impl Fn(&IdentityPublicKey) -> bool,
) -> Option<&'k IdentityPublicKey> {
    let qualifying: Vec<&IdentityPublicKey> = keys
        .into_iter()
        .filter(|key| {
            requirements.matches(key) && is_auto_selectable(key, requirements.scope, now_ms)
        })
        .collect();
    let unlimited = qualifying.iter().filter(|key| !key.has_limits());
    let limited = qualifying.iter().filter(|key| key.has_limits());
    unlimited.chain(limited).find(|key| can_sign(key)).copied()
}

impl<'a> KeyRequirements<'a> {
    /// Requirements accepting every key type.
    pub const fn new(
        purpose: Purpose,
        security_levels: &'a [SecurityLevel],
        scope: SigningScope<'a>,
    ) -> Self {
        Self {
            purpose,
            security_levels,
            key_types: None,
            scope,
        }
    }

    /// Restricts the accepted key types.
    pub const fn with_key_types(mut self, key_types: &'a [KeyType]) -> Self {
        self.key_types = Some(key_types);
        self
    }
}

/// [`select_signing_key`] over `identity`'s keys.
pub fn select_identity_signing_key<'i>(
    identity: &'i Identity,
    requirements: KeyRequirements<'_>,
    now_ms: TimestampMillis,
    can_sign: impl Fn(&IdentityPublicKey) -> bool,
) -> Option<&'i IdentityPublicKey> {
    select_signing_key(
        identity.public_keys().values(),
        requirements,
        now_ms,
        can_sign,
    )
}

/// The current wall-clock time in milliseconds since the Unix epoch, the clock
/// key expiry is compared against. Platform compares against block time, which
/// trails the wall clock by seconds at most.
pub fn now_ms() -> TimestampMillis {
    chrono::Utc::now().timestamp_millis().max(0) as TimestampMillis
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::platform_value::BinaryData;

    const CONTRACT: Identifier = Identifier::new([0xDA; 32]);
    const OTHER: Identifier = Identifier::new([0x0E; 32]);
    const NOW: TimestampMillis = 1_800_000_000_000;

    fn key(id: u32, bounds: Option<ContractBounds>) -> IdentityPublicKey {
        IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::HIGH,
            contract_bounds: bounds,
            key_type: KeyType::ECDSA_SECP256K1,
            read_only: false,
            data: BinaryData::new(vec![2; 33]),
            disabled_at: None,
        })
    }

    fn doc_scope() -> SigningScope<'static> {
        SigningScope::Document {
            contract_id: CONTRACT,
            document_type_name: "note",
        }
    }

    fn requirements(scope: SigningScope<'_>) -> KeyRequirements<'_> {
        KeyRequirements {
            purpose: Purpose::AUTHENTICATION,
            security_levels: &[SecurityLevel::HIGH, SecurityLevel::CRITICAL],
            key_types: None,
            scope,
        }
    }

    #[test]
    fn an_unbound_key_covers_every_scope() {
        for scope in [
            SigningScope::NonBatch,
            doc_scope(),
            SigningScope::ContractWide {
                contract_id: CONTRACT,
            },
        ] {
            assert_eq!(bounds_permit(None, scope), Some(true), "{scope:?}");
        }
    }

    #[test]
    fn a_contract_bound_key_never_signs_outside_a_batch() {
        let bounds = ContractBounds::SingleContract { id: CONTRACT };
        assert_eq!(
            bounds_permit(Some(&bounds), SigningScope::NonBatch),
            Some(false)
        );
    }

    #[test]
    fn a_single_contract_bound_covers_its_documents_and_tokens_only() {
        let bounds = ContractBounds::SingleContract { id: CONTRACT };
        assert_eq!(bounds_permit(Some(&bounds), doc_scope()), Some(true));
        assert_eq!(
            bounds_permit(
                Some(&bounds),
                SigningScope::ContractWide {
                    contract_id: CONTRACT
                }
            ),
            Some(true)
        );
        assert_eq!(
            bounds_permit(
                Some(&bounds),
                SigningScope::Document {
                    contract_id: OTHER,
                    document_type_name: "note"
                }
            ),
            Some(false)
        );
    }

    #[test]
    fn a_document_type_bound_covers_that_type_and_no_token() {
        let bounds = ContractBounds::SingleContractDocumentType {
            id: CONTRACT,
            document_type_name: "note".to_string(),
        };
        assert_eq!(bounds_permit(Some(&bounds), doc_scope()), Some(true));
        assert_eq!(
            bounds_permit(
                Some(&bounds),
                SigningScope::Document {
                    contract_id: CONTRACT,
                    document_type_name: "other"
                }
            ),
            Some(false)
        );
        assert_eq!(
            bounds_permit(
                Some(&bounds),
                SigningScope::ContractWide {
                    contract_id: CONTRACT
                }
            ),
            Some(false)
        );
    }

    #[test]
    fn a_contract_group_bound_is_left_to_platform_inside_a_batch() {
        let bounds = ContractBounds::ContractGroup { id: OTHER };
        assert_eq!(bounds_permit(Some(&bounds), doc_scope()), None);
        assert_eq!(
            bounds_permit(Some(&bounds), SigningScope::NonBatch),
            Some(false)
        );
    }

    #[test]
    fn expiry_is_reached_at_the_instant_itself() {
        let k = key(1, None).with_limits(None, Some(NOW));
        assert!(!k.is_expired_at(NOW - 1));
        assert!(k.is_expired_at(NOW));
        assert!(!is_auto_selectable(&k, doc_scope(), NOW));
        assert!(is_auto_selectable(&k, doc_scope(), NOW - 1));
    }

    #[test]
    fn selection_prefers_an_unlimited_key_over_a_limited_one() {
        let limited = key(0, None).with_limits(Some(1_000), None);
        let unlimited = key(1, None);
        let picked = select_signing_key(
            [&limited, &unlimited],
            requirements(doc_scope()),
            NOW,
            |_| true,
        );
        assert_eq!(picked.map(|k| k.id()), Some(1));
    }

    #[test]
    fn selection_falls_back_to_a_limited_key() {
        let limited = key(0, None).with_limits(Some(1_000), Some(NOW + 1));
        let picked = select_signing_key([&limited], requirements(doc_scope()), NOW, |_| true);
        assert_eq!(picked.map(|k| k.id()), Some(0));
    }

    #[test]
    fn selection_skips_expired_disabled_out_of_bounds_and_group_keys() {
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;

        let expired = key(0, None).with_limits(None, Some(NOW));
        let mut disabled = key(1, None);
        disabled.set_disabled_at(1);
        let elsewhere = key(2, Some(ContractBounds::SingleContract { id: OTHER }));
        let grouped = key(3, Some(ContractBounds::ContractGroup { id: OTHER }));
        let keys = [&expired, &disabled, &elsewhere, &grouped];

        assert_eq!(
            select_signing_key(keys, requirements(doc_scope()), NOW, |_| true),
            None
        );
    }

    #[test]
    fn selection_takes_a_key_bound_to_the_target_contract() {
        let bound = key(0, Some(ContractBounds::SingleContract { id: CONTRACT }));
        assert_eq!(
            select_signing_key([&bound], requirements(doc_scope()), NOW, |_| true).map(|k| k.id()),
            Some(0)
        );
        assert_eq!(
            select_signing_key([&bound], requirements(SigningScope::NonBatch), NOW, |_| {
                true
            }),
            None,
            "but not for a transition outside a batch"
        );
    }

    #[test]
    fn selection_honours_purpose_security_level_and_key_type() {
        let k = key(0, None);
        let wrong_level = KeyRequirements {
            security_levels: &[SecurityLevel::CRITICAL],
            ..requirements(doc_scope())
        };
        let wrong_type = KeyRequirements {
            key_types: Some(&[KeyType::BLS12_381]),
            ..requirements(doc_scope())
        };
        let wrong_purpose = KeyRequirements {
            purpose: Purpose::TRANSFER,
            ..requirements(doc_scope())
        };
        for r in [wrong_level, wrong_type, wrong_purpose] {
            assert_eq!(select_signing_key([&k], r, NOW, |_| true), None);
        }
    }

    #[test]
    fn caveats_list_blocking_ones_first() {
        let k = key(0, Some(ContractBounds::SingleContract { id: OTHER }))
            .with_limits(Some(500), Some(NOW - 1));
        let caveats = key_caveats(&k, doc_scope(), NOW);
        assert_eq!(
            caveats,
            vec![
                KeyCaveat::Expired {
                    expired_at: NOW - 1
                },
                KeyCaveat::OutOfBounds,
                KeyCaveat::Budgeted { total_budget: 500 },
            ]
        );
        assert!(caveats[0].blocks_signing());
    }

    #[test]
    fn a_live_limited_key_has_only_informational_caveats() {
        let k = key(0, None).with_limits(Some(500), Some(NOW + 10));
        let caveats = key_caveats(&k, doc_scope(), NOW);
        assert_eq!(
            caveats,
            vec![
                KeyCaveat::Budgeted { total_budget: 500 },
                KeyCaveat::Expiring {
                    expires_at: NOW + 10
                },
            ]
        );
        assert!(caveats.iter().all(|c| !c.blocks_signing()));
    }

    #[test]
    fn a_plain_key_has_no_caveats() {
        assert!(key_caveats(&key(0, None), doc_scope(), NOW).is_empty());
    }

    /// A limited key this device can sign with beats an unlimited key whose
    /// private half lives elsewhere (upstream `usable_authentication_key`).
    #[test]
    fn a_held_limited_key_beats_an_unlimited_key_held_elsewhere() {
        let limited = key(4, None).with_limits(Some(100_000), None);
        let unlimited = key(1, None);
        let picked = select_signing_key(
            [&unlimited, &limited],
            requirements(doc_scope()),
            NOW,
            |k| k.id() == 4,
        );
        assert_eq!(picked.map(|k| k.id()), Some(4));
    }

    /// An unlimited key still wins when both can sign.
    #[test]
    fn an_unlimited_key_wins_when_both_can_sign() {
        let limited = key(4, None).with_limits(Some(100_000), None);
        let unlimited = key(1, None);
        let picked = select_signing_key(
            [&limited, &unlimited],
            requirements(doc_scope()),
            NOW,
            |_| true,
        );
        assert_eq!(picked.map(|k| k.id()), Some(1));
    }

    /// Availability never widens eligibility: a signable but ineligible key
    /// is not picked when the eligible one cannot sign.
    #[test]
    fn availability_does_not_relax_eligibility() {
        let bound_elsewhere = key(2, Some(ContractBounds::SingleContract { id: OTHER }));
        let eligible = key(1, None);
        let picked = select_signing_key(
            [&eligible, &bound_elsewhere],
            requirements(doc_scope()),
            NOW,
            |k| k.id() == 2,
        );
        assert_eq!(picked, None);
    }
}
