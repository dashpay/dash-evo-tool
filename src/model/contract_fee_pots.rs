//! Who may claim a contract fee pot (protocol version 14).

use std::collections::BTreeSet;

use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::config::v2::DataContractConfigGettersV2;
use dash_sdk::dpp::data_contract::document_type::action_fees::ContractFeePot;
use dash_sdk::platform::Identifier;

/// The seated moderation team of an elected contract, read alongside its fee
/// pots.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SeatedModerators {
    /// No charter is seated, so the interim team the contract declares claims
    /// the moderators pot.
    #[default]
    NotSeated,
    /// A charter is seated: its leader and active members claim the
    /// moderators pot, and the interim team no longer does.
    Seated(BTreeSet<Identifier>),
    /// The team could not be read, so who claims the moderators pot of an
    /// elected contract is not known on this device.
    Unknown,
}

/// The identities that may claim a fee pot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PotClaimants {
    /// Exactly these identities. Empty when nobody may claim.
    Known(BTreeSet<Identifier>),
    /// Not known on this device; Platform decides when the claim is sent.
    Unknown,
}

impl PotClaimants {
    /// Whether anyone may claim the pot. `true` when not known.
    pub fn any(&self) -> bool {
        match self {
            PotClaimants::Known(ids) => !ids.is_empty(),
            PotClaimants::Unknown => true,
        }
    }

    /// Whether `identity_id` may claim the pot. `true` when not known.
    pub fn includes(&self, identity_id: &Identifier) -> bool {
        match self {
            PotClaimants::Known(ids) => ids.contains(identity_id),
            PotClaimants::Unknown => true,
        }
    }
}

/// Whether `contract` declares elected moderation, whose moderators pot goes
/// to a seated team once a charter is seated.
pub fn has_elected_moderation(contract: &DataContract) -> bool {
    contract
        .config()
        .moderation()
        .is_some_and(|moderation| moderation.moderators.elected().is_some())
}

/// Who may claim `pot` of `contract`, given its seated team: the rule
/// Platform applies to a claim.
pub fn pot_claimants(
    pot: ContractFeePot,
    contract: &DataContract,
    seated: &SeatedModerators,
) -> PotClaimants {
    let declared = pot.recipients(contract);
    match pot {
        ContractFeePot::Owner => PotClaimants::Known(declared),
        ContractFeePot::Moderators => {
            moderators_pot_claimants(declared, has_elected_moderation(contract), seated)
        }
    }
}

/// Who may claim the moderators pot: the declared team, except that an
/// elected contract's seated team replaces its interim team.
fn moderators_pot_claimants(
    declared: BTreeSet<Identifier>,
    elected: bool,
    seated: &SeatedModerators,
) -> PotClaimants {
    if !elected {
        return PotClaimants::Known(declared);
    }
    match seated {
        SeatedModerators::NotSeated => PotClaimants::Known(declared),
        SeatedModerators::Seated(team) => PotClaimants::Known(team.clone()),
        SeatedModerators::Unknown => PotClaimants::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Identifier {
        Identifier::from([byte; 32])
    }

    /// A member of the seated team claims even when the interim team does not
    /// name it, and the interim team no longer claims.
    #[test]
    fn a_seated_team_replaces_the_interim_team() {
        let interim = BTreeSet::from([id(1)]);
        let seated = SeatedModerators::Seated(BTreeSet::from([id(2), id(3)]));

        let claimants = moderators_pot_claimants(interim, true, &seated);

        assert!(claimants.includes(&id(2)));
        assert!(claimants.includes(&id(3)));
        assert!(!claimants.includes(&id(1)));
    }

    /// Until a charter is seated, the interim team claims.
    #[test]
    fn the_interim_team_claims_until_a_charter_is_seated() {
        let interim = BTreeSet::from([id(1)]);

        let claimants =
            moderators_pot_claimants(interim.clone(), true, &SeatedModerators::NotSeated);

        assert_eq!(claimants, PotClaimants::Known(interim));
    }

    /// When the team could not be read, nobody is ruled out: Platform decides.
    #[test]
    fn an_unread_team_rules_nobody_out() {
        let claimants =
            moderators_pot_claimants(BTreeSet::from([id(1)]), true, &SeatedModerators::Unknown);

        assert_eq!(claimants, PotClaimants::Unknown);
        assert!(claimants.any());
        assert!(claimants.includes(&id(9)));
    }

    /// A contract without elected moderation has no seated team: its declared
    /// team claims whatever was read.
    #[test]
    fn a_seated_team_matters_only_for_elected_moderation() {
        let declared = BTreeSet::from([id(1)]);
        for seated in [
            SeatedModerators::Unknown,
            SeatedModerators::Seated(BTreeSet::from([id(2)])),
        ] {
            assert_eq!(
                moderators_pot_claimants(declared.clone(), false, &seated),
                PotClaimants::Known(declared.clone()),
                "{seated:?}"
            );
        }
    }

    /// A contract that declares no moderation: nobody claims its moderators
    /// pot, its owner claims the owner pot.
    #[test]
    fn a_contract_without_moderation_pays_only_its_owner() {
        let contract = dash_sdk::dpp::system_data_contracts::load_system_data_contract(
            dash_sdk::dpp::system_data_contracts::SystemDataContract::DPNS,
            dash_sdk::dpp::version::PlatformVersion::latest(),
        )
        .expect("DPNS contract");
        assert!(!has_elected_moderation(&contract));

        let moderators = pot_claimants(
            ContractFeePot::Moderators,
            &contract,
            &SeatedModerators::Unknown,
        );
        assert!(!moderators.any());

        let owner = pot_claimants(ContractFeePot::Owner, &contract, &SeatedModerators::Unknown);
        assert_eq!(
            owner,
            PotClaimants::Known(BTreeSet::from([contract.owner_id()]))
        );
    }
}
