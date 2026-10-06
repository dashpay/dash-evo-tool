use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::util::strings::convert_to_homograph_safe_chars;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::platform::Identifier;
use serde::{Deserialize, Serialize};
use std::fmt;

pub mod composer;
pub mod operator;
pub mod progress;

/// Grace period for admitting a scheduled vote to automatic execution.
pub const SCHEDULED_VOTE_MAX_LATENESS_MS: u64 = 120_000;

/// Whether the normal automatic admission window has passed.
pub fn dpns_schedule_is_overdue(scheduled_at_ms: u64, now_ms: u64) -> bool {
    scheduled_at_ms.saturating_add(SCHEDULED_VOTE_MAX_LATENESS_MS) < now_ms
}

/// Proved current vote state for one node × poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpnsCurrentVoteState {
    Checking,
    Available(Option<ResourceVoteChoice>),
    Unavailable,
}

/// Whether a vote poll can still accept a submitted transition.
///
/// Reconciliation needs a terminal negative verdict, and a current choice that
/// merely differs from the requested one never supplies it — the submitted
/// transition may still be waiting to apply. A closed poll does supply it: no
/// transition can apply to a decided contest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpnsVotePollAvailability {
    /// The poll is open, or DET has no proof that it closed. Absence of
    /// evidence must never be read as closure, so unknown polls land here.
    MayAccept,
    /// The contest has an authoritative terminal outcome.
    ProvedClosed,
}

/// Classify a poll from its cached contest, if DET has one.
///
/// A local deadline cannot prove closure; Platform may still accept votes until resolution.
pub fn dpns_vote_poll_availability(
    contest: Option<&crate::model::contested_name::ContestedName>,
) -> DpnsVotePollAvailability {
    let Some(contest) = contest else {
        return DpnsVotePollAvailability::MayAccept;
    };
    if contest.state.is_decided() {
        DpnsVotePollAvailability::ProvedClosed
    } else {
        DpnsVotePollAvailability::MayAccept
    }
}

/// Durable identity of one submitted voting batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DpnsVoteOperationId([u8; 16]);

impl DpnsVoteOperationId {
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

/// Compare-and-set request for one scheduled target; the voter and poll cannot change.
#[derive(Debug, Clone, PartialEq)]
pub struct DpnsScheduledVoteEdit {
    pub operation_id: DpnsVoteOperationId,
    pub key: DpnsVoteTargetKey,
    pub expected_choice: ResourceVoteChoice,
    pub expected_timestamp: u64,
    pub choice: ResourceVoteChoice,
    pub unix_timestamp: u64,
}

/// Reason a schedule edit cannot be applied to the selected contest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpnsScheduleEditValidationError {
    Time,
    Choice,
    Contest,
}

/// Validate a future voting time against the contest deadline, when known.
pub fn validate_dpns_schedule_time(
    unix_timestamp: u64,
    now_ms: u64,
    end_time: Option<u64>,
) -> Result<(), DpnsScheduleEditValidationError> {
    if unix_timestamp <= now_ms || end_time.is_some_and(|end| unix_timestamp >= end) {
        return Err(DpnsScheduleEditValidationError::Time);
    }
    Ok(())
}

/// Validate the new schedule against the current time and the selected contest.
pub fn validate_dpns_schedule_edit(
    choice: ResourceVoteChoice,
    unix_timestamp: u64,
    now_ms: u64,
    contest: &crate::model::contested_name::ContestedName,
) -> Result<(), DpnsScheduleEditValidationError> {
    if !contest.is_votable() {
        return Err(DpnsScheduleEditValidationError::Contest);
    }
    validate_dpns_schedule_time(unix_timestamp, now_ms, contest.end_time)?;
    if let ResourceVoteChoice::TowardsIdentity(id) = choice
        && !contest
            .contestants
            .as_ref()
            .is_some_and(|contenders| contenders.iter().any(|contender| contender.id == id))
    {
        return Err(DpnsScheduleEditValidationError::Choice);
    }
    Ok(())
}

/// Identity of one scheduled vote as the Scheduled view addresses it: a node
/// and a contested name on one network.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DpnsScheduledVoteKey {
    pub network: Network,
    pub voter_id: Identifier,
    pub contested_name: String,
}

/// Result of clearing one scheduled target from the voting journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpnsScheduledVoteClearDisposition {
    Cleared,
    InFlight(DpnsVoteTargetStatus),
}

/// Typed Clear All result for one scheduled target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DpnsScheduledVoteClearOutcome {
    pub operation_id: Option<DpnsVoteOperationId>,
    pub key: DpnsScheduledVoteKey,
    pub disposition: DpnsScheduledVoteClearDisposition,
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

impl DpnsVoteTarget {
    /// Whether submitting this target would change nothing on Platform.
    ///
    /// The single definition of no-op suppression: the review step filters on
    /// it before the operator commits, and [`DpnsVoteOperation::new`] applies
    /// it again when the batch is built.
    pub fn is_no_op(&self) -> bool {
        self.current_choice == Some(self.requested_choice)
    }
}

/// Persistable, user-meaningful failure category without task diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DpnsVoteFailure {
    PlatformRejected,
    SubmissionFailed,
    CurrentVoteUnavailable,
    ResultUnconfirmed,
    /// The contest closed before the target was submitted. Appended last:
    /// journal records are positionally encoded.
    VotingEnded,
    /// The node's voting key was not loaded when the target was due.
    VotingKeyMissing,
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
    /// Reserved: nothing produces it, because reconciliation cannot prove a
    /// submitted vote was not applied (dashpay/platform#4137). Kept because
    /// journal records are positionally encoded.
    NotApplied,
    /// The user cancelled a scheduled vote before it was submitted.
    Cancelled,
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

    /// Whether the vote ended without taking effect, so the user may review
    /// and submit it again.
    pub fn is_reviewable_failure(self) -> bool {
        match self {
            Self::Rejected | Self::FailedBeforeSubmission | Self::NotApplied => true,
            Self::Scheduled
            | Self::Queued
            | Self::Submitting
            | Self::Confirming
            | Self::Confirmed
            | Self::Unconfirmed
            | Self::Cancelled => false,
        }
    }
}

/// Durable result and progress for one operation target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpnsVoteOutcome {
    pub operation_id: DpnsVoteOperationId,
    pub target: DpnsVoteTarget,
    pub status: DpnsVoteTargetStatus,
    pub failure: Option<DpnsVoteFailure>,
    /// Display-only relative preset, hydrated from journal-owned metadata.
    /// Excluded from the established binary vote record format.
    #[serde(skip)]
    pub relative_schedule_preset_ms: Option<u64>,
    /// When restart recovery stopped this immediate vote before it was sent,
    /// in Unix ms. Display-only and hydrated like the preset: it lets the
    /// session that stopped the vote report it, although the vote is older.
    #[serde(skip)]
    pub stopped_by_recovery_at_ms: Option<u64>,
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
    pub fn new(
        id: DpnsVoteOperationId,
        created_at: TimestampMillis,
        targets: Vec<DpnsVoteTarget>,
    ) -> Self {
        let original_len = targets.len();
        let targets = targets
            .into_iter()
            .filter(|target| !target.is_no_op())
            .map(|target| {
                let status = match target.timing {
                    VoteTiming::Now => DpnsVoteTargetStatus::Queued,
                    VoteTiming::Scheduled(_) => DpnsVoteTargetStatus::Scheduled,
                };
                DpnsVoteOutcome {
                    operation_id: id,
                    target,
                    status,
                    failure: None,
                    relative_schedule_preset_ms: None,
                    stopped_by_recovery_at_ms: None,
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

/// Build `[Value::from("dash"), Value::Text(normalized_label.to_owned())]` for a DPNS vote poll.
///
/// Caller must pre-normalize the label via `convert_to_homograph_safe_chars`
/// (`alice` → `a11ce`); Platform indexes polls under the normalized form.
fn dpns_vote_poll_index_values(normalized_label: &str) -> Vec<Value> {
    vec![
        Value::from("dash"),
        Value::Text(normalized_label.to_owned()),
    ]
}

/// A DPNS contract schema cannot describe the requested vote poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DpnsVotePollError {
    #[error("The username contract is missing its domain definition. Refresh and try again.")]
    MissingDomain,
    #[error("The username contract does not support voting. Refresh and try again.")]
    MissingContestedIndex,
}

/// The exact Platform vote poll for one DPNS label under `dpns_contract`.
///
/// Normalizes `name` itself, so callers pass the label as the user typed it.
/// This is the only construction of the poll: its `unique_id()` keys every
/// target lock and journal record, while the poll itself is what gets voted on,
/// so a second spelling would let DET lock one poll and submit another.
///
/// # Errors
///
/// Returns a schema error if the domain document or its contested index is missing.
pub fn dpns_vote_poll(
    dpns_contract: &DataContract,
    name: &str,
) -> Result<ContestedDocumentResourceVotePoll, DpnsVotePollError> {
    let document_type = dpns_contract
        .document_type_for_name("domain")
        .map_err(|_| DpnsVotePollError::MissingDomain)?;
    let Some(contested_index) = document_type.find_contested_index() else {
        return Err(DpnsVotePollError::MissingContestedIndex);
    };
    Ok(ContestedDocumentResourceVotePoll {
        index_name: contested_index.name.clone(),
        index_values: dpns_vote_poll_index_values(&convert_to_homograph_safe_chars(name)),
        document_type_name: document_type.name().to_owned(),
        contract_id: dpns_contract.id(),
    })
}

/// Ordering that decides which operation currently speaks for one target.
pub type DpnsVoteAuthorityRank = (bool, TimestampMillis, DpnsVoteOperationId);

/// Rank one operation's outcome for a target: a lock holder outranks history,
/// then the newest `(created_at, operation id)` wins.
///
/// The operation id is part of the rank, not a fallback: `created_at` comes from
/// a millisecond clock, so a bulk review commits several operations under one
/// timestamp and ties are routinely reachable. Every site that asks "which
/// operation speaks for this target" must order by this rank, or the Scheduled
/// Votes screen, the node cards and the cast/cancel paths can each name a
/// different operation for the same journal.
pub fn dpns_vote_authority_rank(
    created_at: TimestampMillis,
    operation_id: DpnsVoteOperationId,
    status: DpnsVoteTargetStatus,
) -> DpnsVoteAuthorityRank {
    (status.holds_lock(), created_at, operation_id)
}

/// Select one outcome per exact target, optionally restricting the eligible history.
/// A lock holder wins, then the newest operation; equal ranks select the last record.
pub fn authoritative_dpns_vote_outcomes(
    operations: &[DpnsVoteOperation],
    include: impl Fn(&DpnsVoteOutcome) -> bool,
) -> std::collections::BTreeMap<&DpnsVoteTargetKey, (&DpnsVoteOperation, &DpnsVoteOutcome)> {
    let mut selected = std::collections::BTreeMap::new();
    for operation in operations {
        for outcome in operation.targets.iter().filter(|outcome| include(outcome)) {
            let rank = dpns_vote_authority_rank(operation.created_at, operation.id, outcome.status);
            let entry = selected
                .entry(&outcome.target.key)
                .or_insert((operation, outcome));
            if rank >= dpns_vote_authority_rank(entry.0.created_at, entry.0.id, entry.1.status) {
                *entry = (operation, outcome);
            }
        }
    }
    selected
}

/// The outcome that currently speaks for `key`, or `None` when it is absent.
pub fn authoritative_dpns_vote_outcome<'a>(
    operations: &'a [DpnsVoteOperation],
    key: &DpnsVoteTargetKey,
) -> Option<&'a DpnsVoteOutcome> {
    authoritative_dpns_vote_outcomes(operations, |outcome| &outcome.target.key == key)
        .into_values()
        .next()
        .map(|(_, outcome)| outcome)
}

/// Operations still holding the target lock for `key`; more than one is a conflict.
pub fn dpns_vote_lock_holders<'a>(
    operations: &'a [DpnsVoteOperation],
    key: &'a DpnsVoteTargetKey,
) -> impl Iterator<Item = &'a DpnsVoteOperation> {
    operations.iter().filter(move |operation| {
        operation
            .outcome(key)
            .is_some_and(|outcome| outcome.status.holds_lock())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
    use dash_sdk::platform::Identifier;

    #[test]
    fn authoritative_outcomes_preserve_exact_keys_and_share_tie_breaking() {
        let key_target = target(1, 1, None, ResourceVoteChoice::Lock);
        let mut locked = DpnsVoteOperation::new(
            DpnsVoteOperationId::from_bytes([1; 16]),
            1,
            vec![key_target.clone()],
        );
        locked.targets[0].status = DpnsVoteTargetStatus::Unconfirmed;
        let mut newer = DpnsVoteOperation::new(
            DpnsVoteOperationId::from_bytes([2; 16]),
            2,
            vec![key_target.clone()],
        );
        newer.targets[0].status = DpnsVoteTargetStatus::Confirmed;
        let mut another_network = newer.clone();
        another_network.targets[0].target.key.network = Network::Mainnet;
        let mut another_poll = newer.clone();
        another_poll.targets[0].target.key.vote_poll_id = Identifier::from([2; 32]);
        let operations = vec![newer, locked, another_network, another_poll];
        let selected = authoritative_dpns_vote_outcomes(&operations, |_| true);
        assert_eq!(
            selected.len(),
            3,
            "same name must not collapse networks or polls"
        );
        assert_eq!(selected[&key_target.key].0.id, operations[1].id);
        for (key, (_, outcome)) in selected {
            assert_eq!(
                authoritative_dpns_vote_outcome(&operations, key),
                Some(outcome)
            );
        }
        let mut tied = operations[1].clone();
        tied.id = DpnsVoteOperationId::from_bytes([3; 16]);
        tied.targets[0].operation_id = tied.id;
        for history in [
            vec![operations[1].clone(), tied.clone()],
            vec![tied.clone(), operations[1].clone()],
        ] {
            assert_eq!(
                authoritative_dpns_vote_outcomes(&history, |_| true)[&key_target.key]
                    .0
                    .id,
                tied.id
            );
        }
    }

    #[test]
    fn automatic_window_boundary_and_overflow() {
        assert!(!dpns_schedule_is_overdue(1_000, 121_000));
        assert!(dpns_schedule_is_overdue(1_000, 121_001));
        assert!(!dpns_schedule_is_overdue(1_000, 999));
        assert!(!dpns_schedule_is_overdue(u64::MAX, u64::MAX));
    }

    #[test]
    fn operation_preserves_supplied_id_and_creation_time() {
        let id = DpnsVoteOperationId::from_bytes([7; 16]);
        let operation =
            DpnsVoteOperation::new(id, 123, vec![target(1, 1, None, ResourceVoteChoice::Lock)]);
        assert_eq!(operation.id, id);
        assert_eq!(operation.created_at, 123);
        assert_eq!(operation.targets[0].operation_id, id);
    }

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

    #[test]
    fn index_values_uses_the_given_normalized_label() {
        // Given: a pre-normalized DPNS label (homographs already substituted).
        let normalized = "a11ce";

        // When: constructing the vote poll index values.
        let values = dpns_vote_poll_index_values(normalized);

        // Then: first element is the `"dash"` parent, second is the label as-given.
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], Value::from("dash"));
        assert_eq!(values[1], Value::Text("a11ce".to_owned()));
    }

    #[test]
    fn index_values_do_not_renormalize_the_label() {
        // Given: a label that still contains homograph characters.
        let not_yet_normalized = "alice";

        // When: passing it directly to the helper (violating the contract).
        let values = dpns_vote_poll_index_values(not_yet_normalized);

        // Then: the helper does NOT renormalize — the raw label is returned as-is.
        // (Caller is responsible for normalizing before calling.)
        assert_eq!(values[1], Value::Text("alice".to_owned()));
    }

    /// VOTE-TC-003: choosing the proved current vote cannot create a target.
    #[test]
    fn operation_suppresses_exact_no_ops() {
        let operation = DpnsVoteOperation::new(
            DpnsVoteOperationId::from_bytes([1; 16]),
            1,
            vec![
                target(
                    1,
                    1,
                    Some(ResourceVoteChoice::Lock),
                    ResourceVoteChoice::Lock,
                ),
                target(1, 2, None, ResourceVoteChoice::Abstain),
            ],
        );

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
        let operation = DpnsVoteOperation::new(
            DpnsVoteOperationId::from_bytes([1; 16]),
            1,
            vec![target(7, 9, None, ResourceVoteChoice::Abstain)],
        );
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
            DpnsVoteTargetStatus::Cancelled,
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
    fn only_terminal_unapplied_statuses_are_reviewable_failures() {
        use DpnsVoteTargetStatus as S;
        for (status, expected) in [
            (S::Scheduled, false),
            (S::Queued, false),
            (S::Submitting, false),
            (S::Confirming, false),
            (S::Confirmed, false),
            (S::Unconfirmed, false),
            (S::Rejected, true),
            (S::FailedBeforeSubmission, true),
            (S::NotApplied, true),
            (S::Cancelled, false),
        ] {
            assert_eq!(status.is_reviewable_failure(), expected, "{status:?}");
            assert!(
                !(status.is_reviewable_failure() && status.holds_lock()),
                "{status:?}: a reviewable failure never holds the target lock"
            );
        }
    }

    #[test]
    fn serialized_operation_contains_identifiers_and_choices_only() {
        fn collect_keys(value: &serde_json::Value, keys: &mut std::collections::BTreeSet<String>) {
            match value {
                serde_json::Value::Object(object) => {
                    for (key, nested) in object {
                        keys.insert(key.clone());
                        collect_keys(nested, keys);
                    }
                }
                serde_json::Value::Array(items) => {
                    items.iter().for_each(|item| collect_keys(item, keys));
                }
                _ => {}
            }
        }

        let operation = DpnsVoteOperation::new(
            DpnsVoteOperationId::from_bytes([1; 16]),
            1,
            vec![target(4, 5, None, ResourceVoteChoice::Lock)],
        );
        let serialized = serde_json::to_value(&operation).expect("serialize operation");
        let mut keys = std::collections::BTreeSet::new();
        collect_keys(&serialized, &mut keys);

        // Any new persisted field — signing material included — must be
        // reviewed here, whatever its name.
        let expected: std::collections::BTreeSet<String> = [
            // Serde tag of the upstream `ResourceVoteChoice`.
            "$type",
            "contested_name",
            "created_at",
            "current_choice",
            "failure",
            "id",
            "key",
            "network",
            "no_op_count",
            "operation_id",
            "requested_choice",
            "status",
            "target",
            "targets",
            "timing",
            "vote_poll_id",
            "voter_alias",
            "voter_id",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(keys, expected);
    }

    #[test]
    fn scheduled_edit_validation_rejects_past_deadline_and_unknown_contender() {
        let mut contest = crate::model::contested_name::ContestedName {
            normalized_contested_name: "name".to_owned(),
            contestants: Some(vec![]),
            locked_votes: None,
            abstain_votes: None,
            awarded_to: None,
            end_time: Some(200),
            state: crate::model::contested_name::ContestState::Ongoing,
            last_updated: None,
            my_votes: Default::default(),
        };
        assert_eq!(
            validate_dpns_schedule_edit(ResourceVoteChoice::Lock, 101, 100, &contest),
            Ok(())
        );
        for time in [0, 100, 200, u64::MAX] {
            assert_eq!(
                validate_dpns_schedule_edit(ResourceVoteChoice::Lock, time, 100, &contest),
                Err(DpnsScheduleEditValidationError::Time)
            );
        }
        assert_eq!(
            validate_dpns_schedule_edit(
                ResourceVoteChoice::TowardsIdentity(Identifier::from([1; 32])),
                101,
                100,
                &contest
            ),
            Err(DpnsScheduleEditValidationError::Choice)
        );
        contest.state = crate::model::contested_name::ContestState::Locked;
        assert_eq!(
            validate_dpns_schedule_edit(ResourceVoteChoice::Abstain, 101, 100, &contest),
            Err(DpnsScheduleEditValidationError::Contest)
        );
    }
}
