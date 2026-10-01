//! An identity's own DPNS username requests: availability and request status.
//!
//! Pure decision logic over proved contest snapshots. The backend fetches the
//! snapshots; this module decides what they mean for one label or identity.

use std::time::Duration;

use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::platform::Identifier;
use serde::{Deserialize, Serialize};

use super::dpns::{ContestDurations, is_contested_label};
use super::qualified_identity::QualifiedIdentity;

/// How long a finished request stays listed on the identity.
pub const OUTCOME_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Re-read request status on hub arrival only when the last read is older than this.
pub const ARRIVAL_REFRESH_AFTER: Duration = Duration::from_secs(5 * 60);
/// Periodic refresh while a request is pending on mainnet.
const MAINNET_PENDING_REFRESH: Duration = Duration::from_secs(15 * 60);
/// Periodic refresh while a request is pending on testing networks (90-minute contests).
const TESTING_PENDING_REFRESH: Duration = Duration::from_secs(2 * 60);

/// How often to re-read status while at least one request is pending.
pub fn pending_refresh_interval(network: dash_sdk::dpp::dashcore::Network) -> Duration {
    if network == dash_sdk::dpp::dashcore::Network::Mainnet {
        MAINNET_PENDING_REFRESH
    } else {
        TESTING_PENDING_REFRESH
    }
}

/// Whether the hub should refresh username requests now.
///
/// `arriving` is true on the first frame after the hub is shown; `last` is the
/// time of the previous refresh, if any.
pub fn username_refresh_due(
    now: TimestampMillis,
    last: Option<TimestampMillis>,
    arriving: bool,
    any_pending: bool,
    network: dash_sdk::dpp::dashcore::Network,
) -> bool {
    let Some(last) = last else {
        return true;
    };
    let elapsed = now.saturating_sub(last);
    (arriving && elapsed > duration_ms(ARRIVAL_REFRESH_AFTER))
        || (any_pending && elapsed >= duration_ms(pending_refresh_interval(network)))
}

/// Whether `key` may sign documents that require `required` security: an enabled
/// authentication key at least as strong as required, and never a master key.
pub fn key_can_sign_documents(
    key: &dash_sdk::platform::IdentityPublicKey,
    required: dash_sdk::dpp::identity::SecurityLevel,
) -> bool {
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
    key.purpose() == Purpose::AUTHENTICATION
        && !key.is_disabled()
        && key.security_level() != SecurityLevel::MASTER
        && key.security_level() <= required
}

/// The weakest key security level that may sign both DPNS documents a registration
/// creates (preorder and domain); falls back to `HIGH` if the contract lacks them.
pub fn dpns_signing_requirement(
    contract: &dash_sdk::platform::DataContract,
) -> dash_sdk::dpp::identity::SecurityLevel {
    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
    use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
    ["preorder", "domain"]
        .into_iter()
        .filter_map(|name| contract.document_type_for_name(name).ok())
        .map(|document_type| document_type.security_level_requirement())
        .min()
        .unwrap_or(dash_sdk::dpp::identity::SecurityLevel::HIGH)
}

/// Whether this device holds a private key that can sign username registrations
/// (documents requiring `required` security) for `identity`.
pub fn can_register_usernames(
    identity: &QualifiedIdentity,
    required: dash_sdk::dpp::identity::SecurityLevel,
) -> bool {
    identity
        .private_keys
        .identity_public_keys()
        .iter()
        .any(|(_, key)| key_can_sign_documents(&key.identity_public_key, required))
}

/// The device-only name the user chose, ignoring a legacy automatic copy of a
/// username. Earlier versions wrote `{name}.dash` into the alias at registration,
/// including for requests later lost, so any alias of that exact form is treated
/// as automatic and never hides the chosen main username.
pub fn user_alias(identity: &QualifiedIdentity) -> Option<&str> {
    let alias = identity.alias.as_deref()?.trim();
    let automatic = alias.strip_suffix(".dash").is_some_and(|stem| {
        super::dpns::validate_dpns_name(stem) == super::dpns::DpnsNameValidationResult::Valid
    });
    (!alias.is_empty() && !automatic).then_some(alias)
}

/// Whether a username can be requested right now, and on what terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsernameAvailability {
    /// No one owns or asked for it, and it needs no community vote.
    Available,
    /// No one owns or asked for it, but registering it opens a community vote.
    NeedsVote,
    /// Others already asked for it and new requests may still join the vote.
    Joinable {
        /// Number of requests already in the vote.
        contenders: u32,
        /// When new requests stop being accepted.
        join_end: TimestampMillis,
    },
    /// A vote is running and no longer accepts new requests.
    JoinClosed,
    /// Someone already owns the name.
    Taken,
    /// A community vote locked the name; no one can register it.
    Locked,
    /// This identity already asked for the name; the request is in the vote.
    AlreadyRequested,
}

impl UsernameAvailability {
    /// Whether registration may proceed with this result.
    pub fn allows_registration(self) -> bool {
        matches!(
            self,
            Self::Available | Self::NeedsVote | Self::Joinable { .. }
        )
    }

    /// Whether registration opens or joins a community vote.
    pub fn needs_vote(self) -> bool {
        matches!(self, Self::NeedsVote | Self::Joinable { .. })
    }
}

/// How a finished contest ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContestWinner {
    /// The name went to this identity.
    Identity(Identifier),
    /// More votes went to locking the name.
    Locked,
    /// The contest ended without a winner.
    NoWinner,
}

/// One request in a contest, with its weighted vote count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContenderTally {
    /// The requesting identity.
    pub identity_id: Identifier,
    /// The label as the requester typed it, when the document is available.
    pub label: Option<String>,
    /// Weighted votes for this request.
    pub votes: u32,
    /// When the request was made.
    pub created_at: Option<TimestampMillis>,
}

/// The proved state of one name contest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContestSnapshot {
    /// The result, once the contest has ended.
    pub winner: Option<ContestWinner>,
    /// When the result was decided.
    pub decided_at: Option<TimestampMillis>,
    /// Current requests and their votes.
    pub contenders: Vec<ContenderTally>,
    /// Weighted votes to lock the name.
    pub lock_votes: u32,
    /// Weighted abstain votes.
    pub abstain_votes: u32,
}

impl ContestSnapshot {
    /// When the contest started: the earliest request.
    pub fn started_at(&self) -> Option<TimestampMillis> {
        self.contenders.iter().filter_map(|c| c.created_at).min()
    }
}

/// Decide whether `requester` can ask for `label`.
///
/// `awarded` says whether a registered name already exists; `contest` is the
/// proved contest for the label, if one was fetched. A contested label with no
/// contest is never reported as `Available`, and a contest the requester is
/// already in is `AlreadyRequested` (Platform rejects a second request).
pub fn classify_availability(
    requester: Identifier,
    label: &str,
    awarded: bool,
    contest: Option<&ContestSnapshot>,
    now: TimestampMillis,
    durations: ContestDurations,
) -> UsernameAvailability {
    if awarded {
        return UsernameAvailability::Taken;
    }
    if let Some(contest) = contest {
        if contest.winner.is_none()
            && contest
                .contenders
                .iter()
                .any(|c| c.identity_id == requester)
        {
            return UsernameAvailability::AlreadyRequested;
        }
        match contest.winner {
            Some(ContestWinner::Locked) => return UsernameAvailability::Locked,
            Some(ContestWinner::Identity(_)) => return UsernameAvailability::Taken,
            Some(ContestWinner::NoWinner) => {}
            None if !contest.contenders.is_empty() => {
                return match contest.started_at() {
                    Some(start) => {
                        let join_end = start.saturating_add(duration_ms(durations.join));
                        if now < join_end {
                            UsernameAvailability::Joinable {
                                contenders: u32::try_from(contest.contenders.len())
                                    .unwrap_or(u32::MAX),
                                join_end,
                            }
                        } else {
                            UsernameAvailability::JoinClosed
                        }
                    }
                    // Without a start time the join window can't be proven open.
                    None => UsernameAvailability::JoinClosed,
                };
            }
            None => {}
        }
    }
    if is_contested_label(label) {
        UsernameAvailability::NeedsVote
    } else {
        UsernameAvailability::Available
    }
}

/// Where an identity's username request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestPhase {
    /// Others may still ask for the same name.
    Joinable,
    /// Only masternode votes decide now.
    Voting,
    /// The name went to this identity.
    Won,
    /// The name went to someone else.
    Lost,
    /// The vote locked the name for everyone.
    Locked,
    /// The vote ended and no one got the name.
    NoWinner,
}

impl RequestPhase {
    /// Whether the vote is still running.
    pub fn is_pending(self) -> bool {
        matches!(self, Self::Joinable | Self::Voting)
    }
}

/// Weighted votes in a request's contest, seen from the requester.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestTally {
    /// Votes for this identity's request.
    pub you: u32,
    /// Votes for each other request.
    pub others: Vec<(Identifier, u32)>,
    /// Votes to lock the name.
    pub lock: u32,
    /// Abstain votes.
    pub abstain: u32,
}

/// What would happen to the requester if the vote ended now (PF §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TallyStanding {
    /// The requester would get the name: more votes than every other request,
    /// or no other request at all, and lock is not ahead.
    Leading,
    /// Level with the strongest other request; ties go to the most recent request.
    Tied,
    /// Another request has more votes.
    Trailing,
    /// More votes to lock than to any request: no one would get the name.
    LockAhead,
}

impl RequestTally {
    /// The requester's standing under Platform's resolution rules.
    pub fn standing(&self) -> TallyStanding {
        let best_other = self.others.iter().map(|(_, votes)| *votes).max();
        let top_request = best_other.map_or(self.you, |other| other.max(self.you));
        if self.lock > top_request {
            return TallyStanding::LockAhead;
        }
        match best_other {
            None => TallyStanding::Leading,
            Some(other) => match self.you.cmp(&other) {
                std::cmp::Ordering::Greater => TallyStanding::Leading,
                std::cmp::Ordering::Equal => TallyStanding::Tied,
                std::cmp::Ordering::Less => TallyStanding::Trailing,
            },
        }
    }
}

/// An identity's request for a username that needs a community vote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsernameRequest {
    /// The label as requested.
    pub label: String,
    /// The homograph-normalized label that identifies the contest.
    pub normalized_label: String,
    /// Where the request stands.
    pub phase: RequestPhase,
    /// When the request was made.
    pub requested_at: Option<TimestampMillis>,
    /// When others stop being able to join.
    pub join_end: Option<TimestampMillis>,
    /// When the vote ends.
    pub end: Option<TimestampMillis>,
    /// When the result was decided, for finished requests.
    pub decided_at: Option<TimestampMillis>,
    /// Current weighted votes.
    pub tally: RequestTally,
    /// When this status was last read from the network.
    pub last_updated: TimestampMillis,
}

impl UsernameRequest {
    /// A request just submitted at `now`, before the network reports its contest.
    ///
    /// `joined_until` is the join deadline of an existing contest the request
    /// joined; the contest's dates are derived from it instead of from `now`.
    pub fn submitted(
        label: &str,
        now: TimestampMillis,
        durations: ContestDurations,
        joined_until: Option<TimestampMillis>,
    ) -> Self {
        let start = joined_until.map_or(now, |join_end| {
            join_end.saturating_sub(duration_ms(durations.join))
        });
        Self {
            label: label.to_owned(),
            normalized_label: super::dpns::normalize_dpns_label(label),
            phase: RequestPhase::Joinable,
            requested_at: Some(now),
            join_end: Some(start.saturating_add(duration_ms(durations.join))),
            end: Some(start.saturating_add(duration_ms(durations.total))),
            decided_at: None,
            tally: RequestTally::default(),
            last_updated: now,
        }
    }
}

/// Build `identity_id`'s request from a contest snapshot.
///
/// Returns `None` when a running contest does not include the identity.
/// `end_time` is the network-reported end, used in preference to the
/// computed one. `label` is the fallback when no contender document carries it.
pub fn username_request_from_contest(
    identity_id: Identifier,
    normalized_label: &str,
    contest: &ContestSnapshot,
    end_time: Option<TimestampMillis>,
    now: TimestampMillis,
    durations: ContestDurations,
) -> Option<UsernameRequest> {
    let mine = contest
        .contenders
        .iter()
        .find(|c| c.identity_id == identity_id);
    let phase = match contest.winner {
        Some(ContestWinner::Identity(winner)) if winner == identity_id => RequestPhase::Won,
        Some(ContestWinner::Locked) => RequestPhase::Locked,
        Some(ContestWinner::NoWinner) => RequestPhase::NoWinner,
        Some(ContestWinner::Identity(_)) => RequestPhase::Lost,
        None => {
            mine?;
            let start = contest.started_at();
            let join_end = start.map(|s| s.saturating_add(duration_ms(durations.join)));
            if join_end.is_some_and(|end| now < end) {
                RequestPhase::Joinable
            } else {
                RequestPhase::Voting
            }
        }
    };
    let start = contest.started_at();
    let tally = RequestTally {
        you: mine.map_or(0, |c| c.votes),
        others: contest
            .contenders
            .iter()
            .filter(|c| c.identity_id != identity_id)
            .map(|c| (c.identity_id, c.votes))
            .collect(),
        lock: contest.lock_votes,
        abstain: contest.abstain_votes,
    };
    Some(UsernameRequest {
        label: mine
            .and_then(|c| c.label.clone())
            .unwrap_or_else(|| normalized_label.to_owned()),
        normalized_label: normalized_label.to_owned(),
        phase,
        requested_at: mine.and_then(|c| c.created_at),
        join_end: start.map(|s| s.saturating_add(duration_ms(durations.join))),
        end: end_time.or_else(|| start.map(|s| s.saturating_add(duration_ms(durations.total)))),
        decided_at: contest.decided_at,
        tally,
        last_updated: now,
    })
}

/// Combine a fresh read with what was stored before.
///
/// Finished outcomes keep the dates and tally they were last seen with when the
/// fresh snapshot no longer carries them, and drop out after
/// [`OUTCOME_RETENTION`]. Pending requests not in `fresh` stay as stored until
/// their result can be read.
pub fn merge_requests(
    previous: &[UsernameRequest],
    fresh: Vec<UsernameRequest>,
    now: TimestampMillis,
) -> Vec<UsernameRequest> {
    let mut merged: Vec<UsernameRequest> = fresh
        .into_iter()
        .map(|mut request| {
            if let Some(old) = previous
                .iter()
                .find(|old| old.normalized_label == request.normalized_label)
            {
                if !request.phase.is_pending() {
                    if request.tally == RequestTally::default() {
                        request.tally = old.tally.clone();
                    }
                    request.requested_at = request.requested_at.or(old.requested_at);
                    request.join_end = request.join_end.or(old.join_end);
                    request.end = request.end.or(old.end);
                    request.decided_at = request.decided_at.or(old.end).or(Some(now));
                }
                if request.label == request.normalized_label && old.label != old.normalized_label {
                    request.label = old.label.clone();
                }
            } else if !request.phase.is_pending() && request.decided_at.is_none() {
                request.decided_at = Some(now);
            }
            request
        })
        .collect();
    for old in previous {
        if !merged
            .iter()
            .any(|r| r.normalized_label == old.normalized_label)
        {
            merged.push(old.clone());
        }
    }
    let retention = duration_ms(OUTCOME_RETENTION);
    merged.retain(|r| {
        r.phase.is_pending()
            || r.decided_at
                .is_none_or(|decided| now.saturating_sub(decided) < retention)
    });
    merged.sort_by(|a, b| {
        (!a.phase.is_pending(), a.end, &a.normalized_label).cmp(&(
            !b.phase.is_pending(),
            b.end,
            &b.normalized_label,
        ))
    });
    merged
}

fn duration_ms(duration: Duration) -> TimestampMillis {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60_000;

    fn durations() -> ContestDurations {
        ContestDurations {
            total: Duration::from_secs(90 * 60),
            join: Duration::from_secs(45 * 60),
        }
    }

    fn id(byte: u8) -> Identifier {
        Identifier::from([byte; 32])
    }

    fn contender(byte: u8, votes: u32, created_at: u64) -> ContenderTally {
        ContenderTally {
            identity_id: id(byte),
            label: Some(format!("Name{byte}")),
            votes,
            created_at: Some(created_at),
        }
    }

    fn running(contenders: Vec<ContenderTally>) -> ContestSnapshot {
        ContestSnapshot {
            contenders,
            ..Default::default()
        }
    }

    #[test]
    fn refresh_cadence_follows_arrival_and_pending_rules() {
        // USR-TC-028
        use dash_sdk::dpp::dashcore::Network;
        let min = |m: u64| m * MINUTE;
        assert!(username_refresh_due(
            min(1),
            None,
            true,
            false,
            Network::Mainnet
        ));
        assert!(!username_refresh_due(
            min(2),
            Some(0),
            true,
            false,
            Network::Mainnet
        ));
        assert!(username_refresh_due(
            min(6),
            Some(0),
            true,
            false,
            Network::Mainnet
        ));
        assert!(!username_refresh_due(
            min(14),
            Some(0),
            false,
            true,
            Network::Mainnet
        ));
        assert!(username_refresh_due(
            min(15),
            Some(0),
            false,
            true,
            Network::Mainnet
        ));
        assert!(username_refresh_due(
            min(2),
            Some(0),
            false,
            true,
            Network::Testnet
        ));
        assert!(!username_refresh_due(
            min(60),
            Some(0),
            false,
            false,
            Network::Testnet
        ));
    }

    #[test]
    fn available_when_nothing_exists_and_label_is_uncontested() {
        // USR-TC-010
        assert_eq!(
            classify_availability(id(9), "alice2", false, None, 0, durations()),
            UsernameAvailability::Available
        );
    }

    #[test]
    fn contested_label_without_contest_needs_vote() {
        // USR-TC-011
        assert_eq!(
            classify_availability(id(9), "alice", false, None, 0, durations()),
            UsernameAvailability::NeedsVote
        );
        assert_eq!(
            classify_availability(
                id(9),
                "alice",
                false,
                Some(&ContestSnapshot::default()),
                0,
                durations()
            ),
            UsernameAvailability::NeedsVote
        );
    }

    #[test]
    fn running_contest_inside_join_window_is_joinable() {
        // USR-TC-012
        let contest = running(vec![contender(1, 0, 1_000), contender(2, 0, 5_000)]);
        assert_eq!(
            classify_availability(
                id(9),
                "alice",
                false,
                Some(&contest),
                1_000 + 10 * MINUTE,
                durations()
            ),
            UsernameAvailability::Joinable {
                contenders: 2,
                join_end: 1_000 + 45 * MINUTE
            }
        );
    }

    #[test]
    fn running_contest_after_join_window_is_closed() {
        // USR-TC-013
        let contest = running(vec![contender(1, 0, 1_000)]);
        assert_eq!(
            classify_availability(
                id(9),
                "alice",
                false,
                Some(&contest),
                1_000 + 45 * MINUTE,
                durations()
            ),
            UsernameAvailability::JoinClosed
        );
    }

    #[test]
    fn awarded_name_is_taken() {
        // USR-TC-014
        assert_eq!(
            classify_availability(id(9), "alice", true, None, 0, durations()),
            UsernameAvailability::Taken
        );
        assert_eq!(
            classify_availability(id(9), "alice2", true, None, 0, durations()),
            UsernameAvailability::Taken
        );
    }

    #[test]
    fn own_running_request_is_already_requested() {
        // QA regression: the requester must not be told to join its own contest,
        // because Platform rejects a second request after the preorder is paid.
        let contest = running(vec![contender(9, 0, 1_000)]);
        let availability =
            classify_availability(id(9), "alice", false, Some(&contest), 1_000, durations());
        assert_eq!(availability, UsernameAvailability::AlreadyRequested);
        assert!(!availability.allows_registration());
        assert_eq!(
            classify_availability(id(8), "alice", false, Some(&contest), 1_000, durations()),
            UsernameAvailability::Joinable {
                contenders: 1,
                join_end: 1_000 + 45 * MINUTE
            }
        );
    }

    #[test]
    fn one_identity_gets_a_request_per_contest_it_is_in() {
        // USR-TC-016: contender in two of three running contests → two requests.
        let snapshots = [
            (
                "b",
                running(vec![contender(9, 0, 1_000), contender(8, 0, 1_100)]),
            ),
            ("c", running(vec![contender(9, 4, 2_000)])),
            ("d", running(vec![contender(7, 1, 3_000)])),
        ];
        let requests: Vec<_> = snapshots
            .iter()
            .filter_map(|(name, snapshot)| {
                username_request_from_contest(
                    id(9),
                    name,
                    snapshot,
                    Some(9_000),
                    5_000,
                    durations(),
                )
            })
            .collect();
        assert_eq!(
            requests
                .iter()
                .map(|r| r.normalized_label.as_str())
                .collect::<Vec<_>>(),
            ["b", "c"]
        );
        assert_eq!(requests[1].tally.you, 4);
    }

    #[test]
    fn joined_request_uses_the_contest_dates() {
        // QA regression: joining a running contest must not push its deadlines out.
        let join_end = 1_000 + 45 * MINUTE;
        let request = UsernameRequest::submitted("alice", 30 * MINUTE, durations(), Some(join_end));
        assert_eq!(request.join_end, Some(join_end));
        assert_eq!(request.end, Some(1_000 + 90 * MINUTE));
        let opened = UsernameRequest::submitted("alice", 5_000, durations(), None);
        assert_eq!(opened.join_end, Some(5_000 + 45 * MINUTE));
    }

    #[test]
    fn legacy_automatic_alias_never_labels_the_identity() {
        // QA regression: earlier versions wrote `{name}.dash` at registration,
        // also for requests later lost; such an alias must not hide the main name.
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{
            DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
        };
        let mut identity = QualifiedIdentity {
            identity: dash_sdk::dpp::identity::Identity::create_basic_identity(
                id(1),
                dash_sdk::dpp::version::PlatformVersion::latest(),
            )
            .expect("identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: Some("alice.dash".to_owned()),
            private_keys: KeyStorage::default(),
            dpns_names: vec![DPNSNameInfo {
                name: "bob".to_owned(),
                acquired_at: 0,
            }],
            associated_wallets: Default::default(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        };
        assert_eq!(user_alias(&identity), None);
        assert_eq!(identity.to_string(), "bob");
        identity.alias = Some("Alice Novak".to_owned());
        assert_eq!(user_alias(&identity), Some("Alice Novak"));
        identity.alias = Some("  ".to_owned());
        assert_eq!(user_alias(&identity), None);
    }

    #[test]
    fn signing_gate_matches_platform_key_rules() {
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
        use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
        let key = |purpose, level| {
            let mut key = dash_sdk::platform::IdentityPublicKey::random_key(
                1,
                Some(1),
                dash_sdk::dpp::version::PlatformVersion::latest(),
            );
            key.set_purpose(purpose);
            key.set_security_level(level);
            key
        };
        let high = SecurityLevel::HIGH;
        assert!(key_can_sign_documents(
            &key(Purpose::AUTHENTICATION, SecurityLevel::CRITICAL),
            high
        ));
        assert!(key_can_sign_documents(
            &key(Purpose::AUTHENTICATION, high),
            high
        ));
        assert!(!key_can_sign_documents(
            &key(Purpose::AUTHENTICATION, SecurityLevel::MEDIUM),
            high
        ));
        assert!(!key_can_sign_documents(
            &key(Purpose::AUTHENTICATION, SecurityLevel::MASTER),
            high
        ));
        assert!(!key_can_sign_documents(
            &key(Purpose::TRANSFER, SecurityLevel::CRITICAL),
            high
        ));
    }

    #[test]
    fn locked_contest_is_locked_even_without_awarded_domain() {
        // USR-TC-015: the SDK availability helper would report this name as free.
        let contest = ContestSnapshot {
            winner: Some(ContestWinner::Locked),
            ..Default::default()
        };
        assert_eq!(
            classify_availability(id(9), "alice", false, Some(&contest), 0, durations()),
            UsernameAvailability::Locked
        );
    }

    #[test]
    fn only_available_needs_vote_and_joinable_allow_registration() {
        assert!(UsernameAvailability::Available.allows_registration());
        assert!(UsernameAvailability::NeedsVote.allows_registration());
        assert!(
            UsernameAvailability::Joinable {
                contenders: 1,
                join_end: 0
            }
            .allows_registration()
        );
        for blocked in [
            UsernameAvailability::JoinClosed,
            UsernameAvailability::Taken,
            UsernameAvailability::Locked,
        ] {
            assert!(!blocked.allows_registration(), "{blocked:?}");
        }
    }

    #[test]
    fn request_tracks_phase_dates_and_tally() {
        // USR-TC-016
        let contest = ContestSnapshot {
            contenders: vec![contender(1, 31, 1_000), contender(2, 18, 2_000)],
            lock_votes: 8,
            abstain_votes: 3,
            ..Default::default()
        };
        let request = username_request_from_contest(
            id(1),
            "a11ce",
            &contest,
            Some(1_000 + 90 * MINUTE),
            1_000 + 50 * MINUTE,
            durations(),
        )
        .expect("identity is a contender");
        assert_eq!(request.label, "Name1");
        assert_eq!(request.phase, RequestPhase::Voting);
        assert_eq!(request.requested_at, Some(1_000));
        assert_eq!(request.join_end, Some(1_000 + 45 * MINUTE));
        assert_eq!(request.end, Some(1_000 + 90 * MINUTE));
        assert_eq!(
            request.tally,
            RequestTally {
                you: 31,
                others: vec![(id(2), 18)],
                lock: 8,
                abstain: 3
            }
        );
        assert_eq!(request.tally.standing(), TallyStanding::Leading);
        assert!(
            username_request_from_contest(id(9), "a11ce", &contest, None, 0, durations()).is_none()
        );
    }

    #[test]
    fn finished_contests_yield_outcomes() {
        // USR-TC-017
        let finished = |winner| ContestSnapshot {
            winner: Some(winner),
            decided_at: Some(7_000),
            ..Default::default()
        };
        let phase = |winner| {
            username_request_from_contest(id(1), "x", &finished(winner), None, 9_000, durations())
                .map(|r| (r.phase, r.decided_at))
        };
        assert_eq!(
            phase(ContestWinner::Identity(id(1))),
            Some((RequestPhase::Won, Some(7_000)))
        );
        assert_eq!(
            phase(ContestWinner::Identity(id(2))),
            Some((RequestPhase::Lost, Some(7_000)))
        );
        assert_eq!(
            phase(ContestWinner::Locked),
            Some((RequestPhase::Locked, Some(7_000)))
        );
        assert_eq!(
            phase(ContestWinner::NoWinner),
            Some((RequestPhase::NoWinner, Some(7_000)))
        );
    }

    #[test]
    fn standing_reports_ties_and_lock() {
        let tally = |you, others: Vec<u32>, lock| RequestTally {
            you,
            others: others.into_iter().map(|v| (id(2), v)).collect(),
            lock,
            abstain: 0,
        };
        // A lone request wins at the end even with no votes.
        let lone = RequestTally::default();
        assert_eq!(lone.standing(), TallyStanding::Leading);
        assert_eq!(tally(5, vec![5], 0).standing(), TallyStanding::Tied);
        assert_eq!(tally(5, vec![9], 0).standing(), TallyStanding::Trailing);
        // Lock ahead of every request means no one gets the name; a lock tie does not.
        assert_eq!(tally(5, vec![1], 6).standing(), TallyStanding::LockAhead);
        assert_eq!(
            RequestTally {
                you: 0,
                lock: 3,
                ..Default::default()
            }
            .standing(),
            TallyStanding::LockAhead
        );
        assert_eq!(tally(6, vec![1], 6).standing(), TallyStanding::Leading);
    }

    fn stored(label: &str, phase: RequestPhase, decided_at: Option<u64>) -> UsernameRequest {
        UsernameRequest {
            label: label.to_owned(),
            normalized_label: label.to_owned(),
            phase,
            requested_at: Some(1),
            join_end: Some(2),
            end: Some(3),
            decided_at,
            tally: RequestTally {
                you: 4,
                ..Default::default()
            },
            last_updated: 0,
        }
    }

    #[test]
    fn merge_keeps_unseen_pending_and_prunes_old_outcomes() {
        let now = duration_ms(OUTCOME_RETENTION) + 10;
        let previous = vec![
            stored("b", RequestPhase::Voting, None),
            stored("old", RequestPhase::Lost, Some(5)),
            stored("recent", RequestPhase::Locked, Some(now - 1)),
        ];
        let fresh = vec![stored("c", RequestPhase::Joinable, None)];
        let labels: Vec<_> = merge_requests(&previous, fresh, now)
            .into_iter()
            .map(|r| r.normalized_label)
            .collect();
        assert_eq!(labels, ["b", "c", "recent"]);
    }

    #[test]
    fn merge_fills_outcome_from_previous_pending() {
        let previous = vec![stored("b", RequestPhase::Voting, None)];
        let mut won = stored("b", RequestPhase::Won, None);
        won.tally = RequestTally::default();
        won.end = None;
        let merged = merge_requests(&previous, vec![won], 100);
        assert_eq!(merged[0].phase, RequestPhase::Won);
        assert_eq!(merged[0].tally.you, 4);
        assert_eq!(merged[0].end, Some(3));
        assert_eq!(merged[0].decided_at, Some(3));
    }
}
