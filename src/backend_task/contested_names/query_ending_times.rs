use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::request_type::RequestType;
use chrono::{DateTime, Duration, Utc};
use dash_sdk::dpp::voting::vote_polls::VotePoll;
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::drive::query::VotePollsByEndDateDriveQuery;
use dash_sdk::platform::FetchMany;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::backend_task::TaskResult;
use dash_sdk::Sdk;
impl AppContext {
    pub(super) async fn query_dpns_ending_times(
        self: &Arc<Self>,
        sdk: Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<(), TaskError> {
        let now: DateTime<Utc> = Utc::now();
        let start_time_dt = now - Duration::weeks(2);
        let end_time_dt = now + Duration::weeks(2);
        let mut start_time = Some((start_time_dt.timestamp_millis() as u64, true));
        let end_time = Some((end_time_dt.timestamp_millis() as u64, true));

        let mut contests_end_times = BTreeMap::new();

        const MAX_RETRIES: usize = 3;
        let mut retries = 0;

        loop {
            let end_time_query = VotePollsByEndDateDriveQuery {
                start_time,
                end_time,
                limit: Some(100),
                offset: None,
                order_ascending: true,
            };

            let page = match VotePoll::fetch_many(&sdk, end_time_query.clone()).await {
                Ok(vote_polls) => dpns_end_times_page(vote_polls),
                Err(e) => {
                    tracing::error!("Error fetching vote polls: {}", e);
                    super::log_contested_proof_error(&e, RequestType::GetVotePollsByEndDate);
                    // TODO(#875): replace substring match once the SDK exposes a
                    // structural "contract not found" variant.
                    if matches!(e, dash_sdk::Error::StaleNode(_))
                        || e.to_string().contains(
                            "contract not found when querying from value with contract info",
                        )
                    {
                        retries += 1;
                        if retries > MAX_RETRIES {
                            tracing::error!("Max retries reached for query: {}", e);
                            return Err(TaskError::from(e));
                        } else {
                            continue;
                        }
                    } else {
                        return Err(TaskError::from(e));
                    }
                }
            };

            contests_end_times.extend(page.names);

            // Page on every poll returned, not only DPNS contests: a page holding
            // only other polls must not end the walk.
            let Some(last_end_time) = page.last_end_time else {
                break;
            };
            start_time = Some((last_end_time, false));
        }

        self.update_contested_name_ending_times(contests_end_times)?;
        Ok(())
    }
}

/// One page of the vote-poll end-date index.
#[derive(Debug, Default, PartialEq, Eq)]
struct EndTimesPage {
    /// DPNS names whose contests end in this page, with their end time.
    names: BTreeMap<String, u64>,
    /// The latest end time of any poll in the page; `None` for an empty page.
    last_end_time: Option<u64>,
}

/// Collect the DPNS contest end times from one page of the end-date index.
///
/// The index lists every vote poll kind, so the page can also hold polls that
/// are not DPNS contests (yes/no polls); they count only towards
/// `last_end_time`, which is where the next page starts.
fn dpns_end_times_page(page: impl IntoIterator<Item = (u64, Vec<VotePoll>)>) -> EndTimesPage {
    let mut result = EndTimesPage::default();
    for (timestamp, vote_polls) in page {
        result.last_end_time = result.last_end_time.max(Some(timestamp));
        for vote_poll in vote_polls {
            let VotePoll::ContestedDocumentResourceVotePoll(ContestedDocumentResourceVotePoll {
                index_values,
                ..
            }) = vote_poll
            else {
                continue;
            };
            if let Some(name) = index_values.get(1).and_then(|value| value.to_str().ok()) {
                result.names.insert(name.to_string(), timestamp);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::platform_value::Value;
    use dash_sdk::dpp::voting::vote_polls::yes_no_vote_poll::{
        YesNoMinimumVotingPower, YesNoVotePoll,
    };

    fn dpns_contest(name: &str) -> VotePoll {
        VotePoll::ContestedDocumentResourceVotePoll(ContestedDocumentResourceVotePoll {
            contract_id: Default::default(),
            document_type_name: "domain".to_string(),
            index_name: "parentNameAndLabel".to_string(),
            index_values: vec![
                Value::Text("dash".to_string()),
                Value::Text(name.to_string()),
            ],
        })
    }

    fn yes_no_poll() -> VotePoll {
        VotePoll::YesNoVotePoll(YesNoVotePoll {
            resource_path: vec![vec![1u8, 2, 3].into()],
            supermajority_numerator: 2,
            supermajority_denominator: 3,
            minimum_voting_power: YesNoMinimumVotingPower::Absolute { voting_power: 400 },
        })
    }

    #[test]
    fn end_times_page_keeps_dpns_contests_and_skips_yes_no_polls() {
        let page = vec![
            (100, vec![dpns_contest("alice"), yes_no_poll()]),
            (200, vec![dpns_contest("bob")]),
        ];

        let result = dpns_end_times_page(page);

        assert_eq!(
            result.names,
            BTreeMap::from([("alice".to_string(), 100), ("bob".to_string(), 200)])
        );
        assert_eq!(result.last_end_time, Some(200));
    }

    #[test]
    fn end_times_page_of_only_yes_no_polls_still_advances_the_walk() {
        let page = vec![
            (100, vec![dpns_contest("alice")]),
            (300, vec![yes_no_poll()]),
        ];

        let result = dpns_end_times_page(page);

        assert_eq!(result.names, BTreeMap::from([("alice".to_string(), 100)]));
        assert_eq!(
            result.last_end_time,
            Some(300),
            "the next page must start after the yes/no poll, not after alice"
        );

        let only_yes_no = dpns_end_times_page(vec![(400, vec![yes_no_poll()])]);
        assert!(only_yes_no.names.is_empty());
        assert_eq!(only_yes_no.last_end_time, Some(400));
    }

    #[test]
    fn empty_end_times_page_ends_the_walk() {
        assert_eq!(dpns_end_times_page(Vec::new()), EndTimesPage::default());
    }
}
