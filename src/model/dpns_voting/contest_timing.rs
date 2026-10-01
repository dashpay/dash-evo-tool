//! Contest durations and the urgency window per network.
//!
//! TODO(stream-u): stand-in for `model::dpns::{contest_durations, urgency_window}`,
//! which the usernames stream owns. Signatures match the agreed API; delete this
//! module and switch the imports once that lands.

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::version::PlatformVersion;
use std::time::Duration;

/// Contest timing for a network at a protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContestDurations {
    pub total: Duration,
    pub join: Duration,
}

/// Mainnet 14 d / 7 d; any other network 90 min / 45 min, read from the
/// protocol version.
pub fn contest_durations(network: Network, platform_version: &PlatformVersion) -> ContestDurations {
    let voting = &platform_version.dpp.voting_versions;
    let joining = &platform_version.dpp.validation.voting;
    let (total, join) = match network {
        Network::Mainnet => (
            voting.default_vote_poll_time_duration_mainnet_ms,
            joining.allow_other_contenders_time_mainnet_ms,
        ),
        _ => (
            voting.default_vote_poll_time_duration_test_network_ms,
            joining.allow_other_contenders_time_testing_ms,
        ),
    };
    ContestDurations {
        total: Duration::from_millis(total),
        join: Duration::from_millis(join),
    }
}

/// How close to its deadline a contest counts as urgent: 24 h on mainnet,
/// 30 min elsewhere.
pub fn urgency_window(network: Network) -> Duration {
    match network {
        Network::Mainnet => Duration::from_secs(24 * 60 * 60),
        _ => Duration::from_secs(30 * 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VOTE-FR-085: durations come from the protocol version, never literals.
    #[test]
    fn durations_follow_the_network() {
        let pv = PlatformVersion::latest();
        assert_eq!(
            contest_durations(Network::Mainnet, pv),
            ContestDurations {
                total: Duration::from_secs(14 * 24 * 3600),
                join: Duration::from_secs(7 * 24 * 3600),
            }
        );
        assert_eq!(
            contest_durations(Network::Testnet, pv),
            ContestDurations {
                total: Duration::from_secs(90 * 60),
                join: Duration::from_secs(45 * 60),
            }
        );
        assert_eq!(
            urgency_window(Network::Mainnet),
            Duration::from_secs(86_400)
        );
        assert_eq!(urgency_window(Network::Devnet), Duration::from_secs(1_800));
    }
}
