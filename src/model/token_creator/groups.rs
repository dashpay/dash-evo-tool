//! Stable local group references and checked weighted membership.

use super::rules::parse_identity;
use super::{DraftError, DraftIssue, Section};
use dash_sdk::dpp::data_contract::group::{Group, v0::GroupV0};
use dash_sdk::dpp::version::PlatformVersion;
use std::collections::BTreeMap;

/// One weighted group member, including incomplete text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberDraft {
    pub identity: String,
    pub weight: String,
}

impl Default for MemberDraft {
    fn default() -> Self {
        Self {
            identity: String::new(),
            weight: "1".into(),
        }
    }
}

/// A stable local ID is mapped to a contiguous protocol position during approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDraft {
    pub position: u16,
    pub label: String,
    pub threshold: String,
    pub members: Vec<MemberDraft>,
}

impl GroupDraft {
    /// Starts an explicit two-member draft with a two-point requirement.
    pub fn new(position: u16) -> Self {
        Self {
            position,
            label: format!("Group {position}"),
            threshold: "2".into(),
            members: vec![MemberDraft::default(), MemberDraft::default()],
        }
    }

    /// Checks duplicates before map insertion and delegates protocol limits to DPP.
    pub fn parse(&self, version: &PlatformVersion) -> Result<Group, DraftError> {
        let error =
            |issue| DraftError::new(Section::Groups, "Group members and approval power", issue);
        let required_power = self
            .threshold
            .parse::<u32>()
            .map_err(|_| error(DraftIssue::GroupPower))?;
        let mut members = BTreeMap::new();
        let mut total = 0u64;
        for member in &self.members {
            let identity = parse_identity(&member.identity).map_err(error)?;
            let power = member
                .weight
                .parse::<u32>()
                .map_err(|_| error(DraftIssue::GroupPower))?;
            if power == 0 || power > u32::from(u16::MAX) {
                return Err(error(DraftIssue::GroupPower));
            }
            if members.insert(identity, power).is_some() {
                return Err(error(DraftIssue::DuplicateMember));
            }
            total += u64::from(power);
        }
        if required_power == 0 || required_power > u32::from(u16::MAX) {
            return Err(error(DraftIssue::GroupPower));
        }
        if total < u64::from(required_power) {
            return Err(error(DraftIssue::UnreachableGroup {
                required: required_power,
                available: total,
            }));
        }
        if members.len() < 2 {
            return Err(error(DraftIssue::GroupTooSmall));
        }
        let group = Group::V0(GroupV0 {
            members,
            required_power,
        });
        let validation = dash_sdk::platform::DataContract::validate_groups(
            &BTreeMap::from([(0, group.clone())]),
            version,
        )
        .map_err(|source| DraftError::protocol(Section::Groups, "Group", source))?;
        if !validation.is_valid() {
            return Err(DraftError::consensus(
                Section::Groups,
                "Group",
                validation.errors,
            ));
        }
        Ok(group)
    }

    /// Says N-of-M only when equal nonzero weights make that statement correct.
    pub fn power_summary(&self) -> String {
        let Ok(required) = self.threshold.parse::<u64>() else {
            return "Enter the required approval power.".into();
        };
        let Some(weights) = self
            .members
            .iter()
            .map(|member| member.weight.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
        else {
            return "Enter each member's approval power.".into();
        };
        let Some(available) = weights
            .iter()
            .try_fold(0u64, |total, weight| total.checked_add(*weight))
        else {
            return "Enter approval power from 1 to 65,535.".into();
        };
        if let Some(weight) = weights
            .first()
            .copied()
            .filter(|weight| *weight > 0 && weights.iter().all(|other| other == weight))
        {
            let count = required.div_ceil(weight);
            format!(
                "{count} of {members} members must approve ({required} of {available} approval points).",
                members = weights.len()
            )
        } else {
            format!("{required} approval points are required; {available} are available.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    use dash_sdk::platform::Identifier;

    fn member(value: u8, weight: &str) -> MemberDraft {
        MemberDraft {
            identity: Identifier::from([value; 32]).to_string(Encoding::Base58),
            weight: weight.into(),
        }
    }

    #[test]
    fn duplicate_members_and_unreachable_power_are_refused() {
        let mut group = GroupDraft::new(7);
        group.members = vec![member(1, "1"), member(1, "2")];
        assert!(matches!(
            group.parse(PlatformVersion::latest()).unwrap_err().issue,
            DraftIssue::DuplicateMember
        ));
        group.members = vec![member(1, "1"), member(2, "1")];
        group.threshold = "3".into();
        assert!(matches!(
            group.parse(PlatformVersion::latest()).unwrap_err().issue,
            DraftIssue::UnreachableGroup { .. }
        ));
    }

    #[test]
    fn weighted_summary_does_not_invent_a_member_count() {
        let mut group = GroupDraft::new(9);
        group.members = vec![member(1, "1"), member(2, "2"), member(3, "2")];
        group.threshold = "3".into();
        assert_eq!(
            group.power_summary(),
            "3 approval points are required; 5 are available."
        );
        assert!(group.parse(PlatformVersion::latest()).is_ok());
    }
}
