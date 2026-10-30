//! Which identity keys can sign a given state transition right now.
//!
//! From protocol version 14 an identity key may carry usage limits (a lifetime
//! `total_budget` and an `expires_at` instant, V1 keys only) and an
//! AUTHENTICATION key may be bound to one contract, one document type or a
//! contract group. Picking a signing key by purpose, security level and key type
//! alone can therefore choose a key Platform refuses: an expired key, a key
//! bound elsewhere, or a contract-bound key on a transition outside a batch.
//!
//! The rules are upstream's: platform-wallet's [`usable_signing_key`] chooses a
//! key automatically and [`check_signing_key`] checks one the user chose, with
//! dpp's contract-bounds verdicts. This module adds what the key chooser shows
//! the user about a key ([`KeyCaveat`]) and the argument bundle DET's callers
//! pass ([`KeyRequirements`]).

use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v1::IdentityPublicKeyGettersV1;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::BatchedTransitionBoundsCheck;
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::dpp::identity::{
    Identity, IdentityPublicKey, KeyType, Purpose, SecurityLevel, TimestampMillis,
};
use dash_sdk::dpp::platform_value::BinaryData;
use dash_sdk::dpp::{ProtocolError, address_funds::AddressWitness};
use dash_sdk::platform::Identifier;
pub use platform_wallet::{SigningScope, check_signing_key, usable_signing_key};

/// The key types Platform verifies an identity-signed state transition's
/// signature with (drive-abci's `SUPPORTED_KEY_TYPES`), all of which DET's
/// signer signs with. Platform refuses `EDDSA_25519_HASH160` and
/// `BIP13_SCRIPT_HASH` keys, whatever the transition.
// TODO(upstream): take the set from dpp once it exposes one; drive-abci's
// is private.
pub const PLATFORM_SIGNING_KEY_TYPES: &[KeyType] = &[
    KeyType::ECDSA_SECP256K1,
    KeyType::ECDSA_HASH160,
    KeyType::BLS12_381,
];

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
    match scope.bounds_check(key) {
        BatchedTransitionBoundsCheck::Allowed => {}
        BatchedTransitionBoundsCheck::Denied => caveats.push(KeyCaveat::OutOfBounds),
        BatchedTransitionBoundsCheck::RequiresContractGroupMembership {
            contract_group_id, ..
        } => caveats.push(KeyCaveat::ContractGroupBound {
            group_id: contract_group_id,
        }),
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

/// The signing key requirements of a transition, as DET's callers pass them to
/// upstream's [`usable_signing_key`] and [`check_signing_key`].
#[derive(Debug, Clone, Copy)]
pub struct KeyRequirements<'a> {
    pub purpose: Purpose,
    pub security_levels: &'a [SecurityLevel],
    /// `None` accepts every key type. [`Self::new`] sets
    /// [`PLATFORM_SIGNING_KEY_TYPES`].
    pub key_types: Option<&'a [KeyType]>,
    pub scope: SigningScope<'a>,
}

impl KeyRequirements<'_> {
    /// Whether `key` has the purpose, security level and key type asked for,
    /// whether or not it can sign now.
    pub(crate) fn matches(&self, key: &IdentityPublicKey) -> bool {
        key.purpose() == self.purpose
            && self.security_levels.contains(&key.security_level())
            && self
                .key_types
                .is_none_or(|types| types.contains(&key.key_type()))
    }
}

impl<'a> KeyRequirements<'a> {
    /// Requirements accepting the key types Platform verifies signatures
    /// with, [`PLATFORM_SIGNING_KEY_TYPES`].
    pub const fn new(
        purpose: Purpose,
        security_levels: &'a [SecurityLevel],
        scope: SigningScope<'a>,
    ) -> Self {
        Self {
            purpose,
            security_levels,
            key_types: Some(PLATFORM_SIGNING_KEY_TYPES),
            scope,
        }
    }

    /// Accepts only `key_types` instead of the default.
    pub const fn with_key_types(mut self, key_types: &'a [KeyType]) -> Self {
        self.key_types = Some(key_types);
        self
    }

    /// The key of `identity` upstream's [`usable_signing_key`] picks for these
    /// requirements among the keys `signer` can sign with: every qualifying
    /// unlimited key at any accepted level before any limited one, then key id
    /// order. `None` when no key qualifies or `signer` holds none of them.
    pub fn select<'i>(
        &self,
        identity: &'i Identity,
        signer: &impl Signer<IdentityPublicKey>,
    ) -> Option<&'i IdentityPublicKey> {
        let all_key_types = KeyType::all_key_types();
        usable_signing_key(
            identity,
            signer,
            self.purpose,
            self.security_levels,
            self.key_types.unwrap_or(&all_key_types),
            self.scope,
        )
        .ok()
        .flatten()
    }

    /// Upstream's [`check_signing_key`] for `key` under these requirements,
    /// with `signer` holding the private half ([`KeyLookupOnly`] to skip
    /// that part).
    pub fn check(
        &self,
        key: &IdentityPublicKey,
        signer: &impl Signer<IdentityPublicKey>,
    ) -> Result<(), Box<dash_sdk::Error>> {
        let all_key_types = KeyType::all_key_types();
        check_signing_key(
            key,
            signer,
            self.purpose,
            self.security_levels,
            self.key_types.unwrap_or(&all_key_types),
            self.scope,
        )
        .map_err(Box::new)
    }

    /// Whether `key` may be picked automatically under these requirements:
    /// [`Self::check`] passes and its bounds certainly cover the scope (a
    /// contract-group bound key is never picked automatically, as upstream's
    /// [`usable_signing_key`] skips it).
    pub fn auto_selectable(&self, key: &IdentityPublicKey) -> bool {
        self.check(key, &KeyLookupOnly).is_ok()
            && self.scope.bounds_check(key) == BatchedTransitionBoundsCheck::Allowed
    }
}

/// A [`Signer`] that reports every key as available and signs nothing: it
/// lets upstream's key rules answer which key qualifies, whatever this device
/// holds. Never hand it to anything that signs.
#[derive(Debug, Clone, Copy)]
pub struct KeyLookupOnly;

#[dash_sdk::dpp::async_trait::async_trait]
impl Signer<IdentityPublicKey> for KeyLookupOnly {
    async fn sign(&self, _: &IdentityPublicKey, _: &[u8]) -> Result<BinaryData, ProtocolError> {
        Err(ProtocolError::NotSupported(
            "KeyLookupOnly never signs".to_string(),
        ))
    }

    async fn sign_create_witness(
        &self,
        _: &IdentityPublicKey,
        _: &[u8],
    ) -> Result<AddressWitness, ProtocolError> {
        Err(ProtocolError::NotSupported(
            "KeyLookupOnly never signs".to_string(),
        ))
    }

    fn can_sign_with(&self, _: &IdentityPublicKey) -> bool {
        true
    }
}

/// The current wall-clock time in milliseconds since the Unix epoch, the clock
/// key expiry is compared against (as upstream's selector does). Platform
/// compares against block time, which trails the wall clock by seconds at most.
pub fn now_ms() -> TimestampMillis {
    chrono::Utc::now().timestamp_millis().max(0) as TimestampMillis
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
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
        SigningScope::DocumentType {
            contract_id: CONTRACT,
            document_type_name: "note",
        }
    }

    /// A key bound to a contract group may sign inside a batch only if the
    /// group covers the contract, which only Platform knows; outside a batch
    /// it never signs.
    #[test]
    fn a_contract_group_bound_key_is_left_to_platform_inside_a_batch() {
        let k = key(0, Some(ContractBounds::ContractGroup { id: OTHER }));
        assert_eq!(
            key_caveats(&k, doc_scope(), NOW),
            vec![KeyCaveat::ContractGroupBound { group_id: OTHER }]
        );
        assert_eq!(
            key_caveats(&k, SigningScope::NonBatch, NOW),
            vec![KeyCaveat::OutOfBounds]
        );
    }

    /// Regression: requirements accepted every key type unless a caller
    /// narrowed them, so a caller that did not picked an EdDSA or BIP13 key,
    /// which Platform refuses for every identity-signed transition.
    #[test]
    fn requirements_take_only_keys_platform_verifies_by_default() {
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
        let typed = |key_type: KeyType| {
            let mut k = key(0, None);
            k.set_key_type(key_type);
            k
        };
        let defaults =
            KeyRequirements::new(Purpose::AUTHENTICATION, &[SecurityLevel::HIGH], doc_scope());
        for refused in [KeyType::EDDSA_25519_HASH160, KeyType::BIP13_SCRIPT_HASH] {
            assert!(
                defaults.check(&typed(refused), &KeyLookupOnly).is_err(),
                "{refused:?}"
            );
        }
        assert!(
            defaults
                .check(&typed(KeyType::BLS12_381), &KeyLookupOnly)
                .is_ok()
        );
        assert!(
            defaults
                .with_key_types(&[KeyType::EDDSA_25519_HASH160])
                .check(&typed(KeyType::EDDSA_25519_HASH160), &KeyLookupOnly)
                .is_ok(),
            "an explicit key type list overrides the default"
        );
    }

    /// The automatic rule skips a key the explicit check accepts only when
    /// its bounds are a contract group, as upstream's selector does.
    #[test]
    fn a_contract_group_bound_key_is_never_auto_selectable() {
        let requirements =
            KeyRequirements::new(Purpose::AUTHENTICATION, &[SecurityLevel::HIGH], doc_scope());
        let grouped = key(0, Some(ContractBounds::ContractGroup { id: OTHER }));
        assert!(requirements.check(&grouped, &KeyLookupOnly).is_ok());
        assert!(!requirements.auto_selectable(&grouped));
        assert!(requirements.auto_selectable(&key(1, None)));
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

    /// Contract bounds limit only AUTHENTICATION keys, as in consensus: a
    /// bound TRANSFER key is not out of bounds outside a batch.
    #[test]
    fn bounds_limit_only_authentication_keys() {
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
        let mut transfer = key(0, Some(ContractBounds::SingleContract { id: OTHER }));
        transfer.set_purpose(Purpose::TRANSFER);
        assert!(key_caveats(&transfer, SigningScope::NonBatch, NOW).is_empty());
    }

    #[test]
    fn a_plain_key_has_no_caveats() {
        assert!(key_caveats(&key(0, None), doc_scope(), NOW).is_empty());
    }
}
