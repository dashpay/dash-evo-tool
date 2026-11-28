//! Who may claim a contract fee pot (protocol version 14).

use std::collections::BTreeSet;

use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::config::v2::DataContractConfigGettersV2;
use dash_sdk::dpp::data_contract::document_type::action_fees::ContractFeePot;
use dash_sdk::platform::Identifier;
use dash_sdk::platform::moderation_charters::ModerationTeam;

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
    Seated(Box<ModerationTeam>),
    /// The team could not be read, so who claims the moderators pot of an
    /// elected contract is not known on this device.
    Unknown,
}

/// The identities that may claim a fee pot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PotClaimants {
    /// Exactly these identities. Empty when nobody may claim.
    Known(BTreeSet<Identifier>),
    /// Not known on this device; retry the read before claiming.
    Unknown,
}

impl PotClaimants {
    /// Whether anyone may claim the pot. `false` when not known.
    pub fn any(&self) -> bool {
        match self {
            PotClaimants::Known(ids) => !ids.is_empty(),
            PotClaimants::Unknown => false,
        }
    }

    /// Whether `identity_id` may claim the pot. `false` when not known.
    pub fn includes(&self, identity_id: &Identifier) -> bool {
        match self {
            PotClaimants::Known(ids) => ids.contains(identity_id),
            PotClaimants::Unknown => false,
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
    if pot == ContractFeePot::Moderators
        && has_elected_moderation(contract)
        && matches!(seated, SeatedModerators::Unknown)
    {
        return PotClaimants::Unknown;
    }
    let team = match seated {
        SeatedModerators::Seated(team) => Some(team.as_ref()),
        _ => None,
    };
    let mut candidates = pot.recipients(contract);
    if let Some(team) = team {
        candidates.extend(team.members.iter().copied());
        candidates.insert(team.leader_id);
    }
    let eligibility = platform_wallet::contract_fee_claim_eligibility(contract, team, candidates);
    PotClaimants::Known(match pot {
        ContractFeePot::Owner => eligibility.owner_pot_claimants,
        ContractFeePot::Moderators => eligibility.moderators_pot_claimants,
    })
}

/// Minimum credits required by the resolved payout split.
pub fn minimum_claim_credits(claimants: &PotClaimants, seated: &SeatedModerators) -> u64 {
    match (claimants, seated) {
        (_, SeatedModerators::Seated(_)) | (PotClaimants::Unknown, _) => 1,
        (PotClaimants::Known(ids), _) => (ids.len() as u64).max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seated_team(ids: &[Identifier]) -> SeatedModerators {
        SeatedModerators::Seated(Box::new(ModerationTeam {
            elected_charter_id: Identifier::random(),
            submitted_charter_id: Identifier::random(),
            leader_id: ids[0],
            members: ids.iter().copied().collect(),
            elected_members: ids.to_vec(),
        }))
    }

    fn moderators_pot_claimants(
        declared: BTreeSet<Identifier>,
        elected: bool,
        seated: &SeatedModerators,
    ) -> PotClaimants {
        use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Setters;
        use dash_sdk::dpp::data_contract::config::{
            DataContractConfig,
            moderation::{
                ContractModerationConfig, ContractModerators,
                elected::{ElectedModerators, InterimModerators},
            },
        };
        let mut contract = dash_sdk::dpp::system_data_contracts::load_system_data_contract(
            dash_sdk::dpp::system_data_contracts::SystemDataContract::DPNS,
            dash_sdk::dpp::version::PlatformVersion::latest(),
        )
        .unwrap();
        let moderators = if elected {
            ContractModerators::Elected(Box::new(ElectedModerators {
                join_window: 0,
                vote_window: 0,
                challenge_cool_down: None,
                election_delay: None,
                max_added_moderators: 2,
                moderated_document_types: Default::default(),
                interim: InterimModerators::AppointedModerators(declared),
                owner_protected: false,
            }))
        } else {
            ContractModerators::AppointedModerators(declared)
        };
        let config: DataContractConfig = contract.config().clone();
        contract.set_config(config.with_moderation(Some(ContractModerationConfig {
            banlist: true,
            suspensions: false,
            warnings: false,
            moderators,
        })));
        pot_claimants(ContractFeePot::Moderators, &contract, seated)
    }

    #[test]
    fn equal_split_remainder_waits_for_one_credit_per_recipient() {
        let claimants = PotClaimants::Known(BTreeSet::from([id(1), id(2), id(3)]));
        assert_eq!(
            minimum_claim_credits(&claimants, &SeatedModerators::NotSeated),
            3
        );
        assert_eq!(
            minimum_claim_credits(&claimants, &seated_team(&[id(1), id(2), id(3)])),
            1
        );
    }

    fn id(byte: u8) -> Identifier {
        Identifier::from([byte; 32])
    }

    /// A member of the seated team claims even when the interim team does not
    /// name it, and the interim team no longer claims.
    #[test]
    fn a_seated_team_replaces_the_interim_team() {
        let interim = BTreeSet::from([id(1)]);
        let seated = seated_team(&[id(2), id(3)]);

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

    /// A failed team lookup cannot authorize a claim.
    #[test]
    fn an_unread_team_does_not_authorize_claims() {
        let claimants =
            moderators_pot_claimants(BTreeSet::from([id(1)]), true, &SeatedModerators::Unknown);

        assert_eq!(claimants, PotClaimants::Unknown);
        assert!(!claimants.any());
        assert!(!claimants.includes(&id(9)));
    }

    /// A contract without elected moderation has no seated team: its declared
    /// team claims whatever was read.
    #[test]
    fn a_seated_team_matters_only_for_elected_moderation() {
        let declared = BTreeSet::from([id(1)]);
        for seated in [SeatedModerators::Unknown, seated_team(&[id(2)])] {
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
