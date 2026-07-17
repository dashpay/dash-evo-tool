#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
    use dash_sdk::platform::Identifier;

    fn target(
        voter: u8,
        poll: u8,
        current: Option<ResourceVoteChoice>,
        requested: ResourceVoteChoice,
    ) -> DpnsVoteTarget {
        DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([voter; 32]),
                vote_poll_id: Identifier::from([poll; 32]),
            },
            voter_alias: Some(format!("node-{voter}")),
            contested_name: format!("contest-{poll}"),
            requested_choice: requested,
            current_choice: current,
            timing: VoteTiming::Now,
        }
    }

    /// VOTE-TC-003: choosing the proved current vote cannot create a target.
    #[test]
    fn operation_suppresses_exact_no_ops() {
        let operation = DpnsVoteOperation::new(vec![
            target(
                1,
                1,
                Some(ResourceVoteChoice::Lock),
                ResourceVoteChoice::Lock,
            ),
            target(1, 2, None, ResourceVoteChoice::Abstain),
        ]);

        assert_eq!(operation.targets.len(), 1);
        assert_eq!(operation.no_op_count, 1);
        assert_eq!(
            operation.targets[0].target.key.vote_poll_id,
            Identifier::from([2; 32])
        );
    }

    /// VOTE-TC-032: outcomes retain the exact voter × contest target.
    #[test]
    fn outcomes_retain_target_correlation() {
        let operation =
            DpnsVoteOperation::new(vec![target(7, 9, None, ResourceVoteChoice::Abstain)]);
        let outcome = &operation.targets[0];

        assert_eq!(outcome.operation_id, operation.id);
        assert_eq!(outcome.target.key.voter_id, Identifier::from([7; 32]));
        assert_eq!(outcome.target.contested_name, "contest-9");
    }

    /// VOTE-TC-040/044: unresolved states keep the target lock across restart.
    #[test]
    fn unresolved_statuses_hold_target_locks() {
        for status in [
            DpnsVoteTargetStatus::Scheduled,
            DpnsVoteTargetStatus::Queued,
            DpnsVoteTargetStatus::Submitting,
            DpnsVoteTargetStatus::Confirming,
            DpnsVoteTargetStatus::Unconfirmed,
        ] {
            assert!(status.holds_lock(), "{status:?} must hold its lock");
        }
        for status in [
            DpnsVoteTargetStatus::Confirmed,
            DpnsVoteTargetStatus::Rejected,
            DpnsVoteTargetStatus::FailedBeforeSubmission,
            DpnsVoteTargetStatus::NotApplied,
        ] {
            assert!(!status.holds_lock(), "{status:?} must release its lock");
        }
    }

    /// VOTE-TC-072: the network is part of target identity.
    #[test]
    fn target_keys_are_network_scoped() {
        let testnet = DpnsVoteTargetKey {
            network: Network::Testnet,
            voter_id: Identifier::from([1; 32]),
            vote_poll_id: Identifier::from([2; 32]),
        };
        let mainnet = DpnsVoteTargetKey {
            network: Network::Mainnet,
            ..testnet.clone()
        };

        assert_ne!(testnet, mainnet);
    }

    /// VOTE-TC-073: the durable operation shape contains no signing material.
    #[test]
    fn serialized_operation_contains_identifiers_and_choices_only() {
        let operation = DpnsVoteOperation::new(vec![target(4, 5, None, ResourceVoteChoice::Lock)]);
        let serialized = serde_json::to_string(&operation).expect("serialize operation");

        assert!(!serialized.contains("private"));
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("wif"));
    }
}
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// Proved current vote state for one node × poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpnsCurrentVoteState {
    Checking,
    Available(Option<ResourceVoteChoice>),
    Unavailable,
}

/// Durable identity of one submitted voting batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DpnsVoteOperationId([u8; 16]);

impl DpnsVoteOperationId {
    /// Generate a random operation identifier without adding a UUID dependency.
    pub fn random() -> Self {
        let mut bytes = [0; 16];
        rand::rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Return the stable persisted byte representation.
    pub fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    /// Restore an identifier from its persisted byte representation.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for DpnsVoteOperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Exact network + node + poll lock identity for one vote target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DpnsVoteTargetKey {
    pub network: Network,
    /// The masternode ProTxHash used by Platform's proved vote query.
    pub voter_id: Identifier,
    pub vote_poll_id: Identifier,
}

/// When a target should enter the shared executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoteTiming {
    Now,
    Scheduled(TimestampMillis),
}

pub(crate) fn unavailable_preflight_outcome(
    timing: VoteTiming,
) -> (DpnsVoteTargetStatus, Option<DpnsVoteFailure>) {
    let status = match timing {
        VoteTiming::Scheduled(_) => DpnsVoteTargetStatus::Scheduled,
        VoteTiming::Now => DpnsVoteTargetStatus::FailedBeforeSubmission,
    };
    (status, Some(DpnsVoteFailure::CurrentVoteUnavailable))
}

pub(crate) fn failed_before_broadcast_outcome(
    timing: VoteTiming,
) -> (DpnsVoteTargetStatus, Option<DpnsVoteFailure>) {
    let status = match timing {
        VoteTiming::Scheduled(_) => DpnsVoteTargetStatus::Scheduled,
        VoteTiming::Now => DpnsVoteTargetStatus::FailedBeforeSubmission,
    };
    (status, Some(DpnsVoteFailure::SubmissionFailed))
}

/// One reviewed node × contest action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpnsVoteTarget {
    pub key: DpnsVoteTargetKey,
    pub voter_alias: Option<String>,
    pub contested_name: String,
    pub requested_choice: ResourceVoteChoice,
    pub current_choice: Option<ResourceVoteChoice>,
    pub timing: VoteTiming,
}

/// Persistable, user-meaningful failure category without task diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DpnsVoteFailure {
    PlatformRejected,
    SubmissionFailed,
    CurrentVoteUnavailable,
    ResultUnconfirmed,
}

/// Lifecycle of one target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DpnsVoteTargetStatus {
    Scheduled,
    Queued,
    Submitting,
    Confirming,
    Confirmed,
    Unconfirmed,
    Rejected,
    FailedBeforeSubmission,
    NotApplied,
}

impl DpnsVoteTargetStatus {
    /// Whether another operation must remain blocked for the same target.
    pub fn holds_lock(self) -> bool {
        matches!(
            self,
            Self::Scheduled
                | Self::Queued
                | Self::Submitting
                | Self::Confirming
                | Self::Unconfirmed
        )
    }
}

/// Durable result and progress for one operation target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpnsVoteOutcome {
    pub operation_id: DpnsVoteOperationId,
    pub target: DpnsVoteTarget,
    pub status: DpnsVoteTargetStatus,
    pub transition_hash: Option<[u8; 32]>,
    pub failure: Option<DpnsVoteFailure>,
}

/// Reviewed voting batch stored before its first broadcast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpnsVoteOperation {
    pub id: DpnsVoteOperationId,
    pub created_at: TimestampMillis,
    pub targets: Vec<DpnsVoteOutcome>,
    pub no_op_count: usize,
}

impl DpnsVoteOperation {
    /// Build an operation while removing targets that match proved current state.
    pub fn new(targets: Vec<DpnsVoteTarget>) -> Self {
        let id = DpnsVoteOperationId::random();
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as TimestampMillis;
        let original_len = targets.len();
        let targets = targets
            .into_iter()
            .filter(|target| target.current_choice != Some(target.requested_choice))
            .map(|target| {
                let status = match target.timing {
                    VoteTiming::Now => DpnsVoteTargetStatus::Queued,
                    VoteTiming::Scheduled(_) => DpnsVoteTargetStatus::Scheduled,
                };
                DpnsVoteOutcome {
                    operation_id: id,
                    target,
                    status,
                    transition_hash: None,
                    failure: None,
                }
            })
            .collect::<Vec<_>>();
        let no_op_count = original_len.saturating_sub(targets.len());
        Self {
            id,
            created_at,
            targets,
            no_op_count,
        }
    }

    /// Find one outcome by its exact target key.
    pub fn outcome(&self, key: &DpnsVoteTargetKey) -> Option<&DpnsVoteOutcome> {
        self.targets
            .iter()
            .find(|outcome| &outcome.target.key == key)
    }

    /// Whether all targets have reached a lock-releasing state.
    pub fn is_complete(&self) -> bool {
        self.targets
            .iter()
            .all(|outcome| !outcome.status.holds_lock())
    }
}
