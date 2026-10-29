use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::request_type::RequestType;
use chrono::{DateTime, Duration, Utc};
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::dpp::voting::vote_polls::VotePoll;
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::drive::query::VotePollsByEndDateDriveQuery;
use dash_sdk::platform::FetchMany;
use std::cmp::Ordering;
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
        let mut start_time: PageStart = (start_time_dt.timestamp_millis() as u64, true);
        let end_time = end_time_dt.timestamp_millis() as u64;

        let mut contests_end_times = BTreeMap::new();

        /// The platform's largest page of vote polls.
        const PAGE_LIMIT: u16 = 100;
        const MAX_RETRIES: usize = 3;
        let mut retries = 0;

        loop {
            let end_time_query = VotePollsByEndDateDriveQuery {
                start_time: Some(start_time),
                end_time: Some((end_time, true)),
                limit: Some(PAGE_LIMIT),
                offset: None,
                order_ascending: true,
            };

            let new_end_times = match VotePoll::fetch_many(&sdk, end_time_query.clone()).await {
                Ok(vote_polls) => {
                    let mut end_times = BTreeMap::new();
                    for (timestamp, vote_poll_list) in vote_polls {
                        let contests = vote_poll_list.into_iter().filter_map(|vote_poll| {
                            let VotePoll::ContestedDocumentResourceVotePoll(
                                ContestedDocumentResourceVotePoll {
                                    contract_id: _,
                                    document_type_name: _,
                                    index_name: _,
                                    index_values,
                                },
                            ) = vote_poll;

                            index_values
                                .get(1)
                                .and_then(|a| a.to_str().ok().map(|a| (a.to_string(), timestamp)))
                        });
                        end_times.extend(contests);
                    }
                    end_times
                }
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

            let page_contests = new_end_times.len();
            let last_found_ending_time = new_end_times.values().max().copied();
            // A contest re-read at a page boundary keeps its single entry.
            contests_end_times.extend(new_end_times);

            let Some(next_start) = next_page_start(start_time, last_found_ending_time, end_time)
            else {
                break;
            };
            // TODO(upstream): proved vote-poll queries reject a non-zero offset, so
            // polls past the page limit that share one end time cannot be fetched.
            if next_start == (start_time.0, false) && page_contests >= usize::from(PAGE_LIMIT) {
                tracing::warn!(
                    end_time = start_time.0,
                    "More contests end at one time than a page holds; some may lack an end time"
                );
            }
            start_time = next_start;
        }

        self.update_contested_name_ending_times(contests_end_times)?;
        Ok(())
    }
}

/// Where a page of the vote-poll end-date index starts: `(end time, included)`.
type PageStart = (TimestampMillis, bool);

/// Decide where the next page of the end-date index starts, or `None` when the walk is done.
///
/// The page limit counts vote polls (and empty end-time trees), not end times, so a full
/// page can stop partway through the polls ending at its last end time. The next page
/// therefore re-reads that end time, and callers must tolerate polls fetched twice. A
/// page that holds nothing past its own included start moves past that end time; with
/// no offset available on proved queries, polls beyond the limit at one end time are
/// out of reach. Every returned start is strictly past `page_start`, so the walk ends.
///
/// `page_last_end_time` is the latest end time in the page, `None` for an empty page.
/// `range_end` is the walk's included upper bound: the platform rejects a start past it,
/// or equal to it unless included.
fn next_page_start(
    page_start: PageStart,
    page_last_end_time: Option<TimestampMillis>,
    range_end: TimestampMillis,
) -> Option<PageStart> {
    // TODO(upstream): an empty page does not prove
    // the rest of the range empty: the limit also counts empty end-time trees, which a
    // proved page never returns, so a page of `limit` legacy empty trees comes back
    // empty and ends the walk before the polls behind them. Proved queries refuse an
    // offset, so no request can page past them.
    let last = page_last_end_time?;
    let next = match last.cmp(&page_start.0) {
        Ordering::Greater => (last, true),
        Ordering::Equal if page_start.1 => (last, false),
        // A page not past its own start cannot come from this query; stop, never loop.
        _ => return None,
    };
    let past_range_end = next.0 > range_end || (next.0 == range_end && !next.1);
    (!past_range_end).then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// A fake end-date index: `(end time, poll id)` in index order, with empty end-time
    /// trees marked by `None`. Pages follow the platform's rules: the limit counts
    /// subquery slots (polls and empty trees alike), and empty trees return nothing.
    fn fake_page(
        index: &[(TimestampMillis, Option<u32>)],
        start: PageStart,
        range_end: TimestampMillis,
        limit: usize,
    ) -> BTreeMap<TimestampMillis, Vec<u32>> {
        let mut page: BTreeMap<TimestampMillis, Vec<u32>> = BTreeMap::new();
        index
            .iter()
            .filter(|(time, _)| {
                (*time > start.0 || (start.1 && *time == start.0)) && *time <= range_end
            })
            .take(limit)
            .for_each(|(time, poll)| {
                if let Some(poll) = poll {
                    page.entry(*time).or_default().push(*poll);
                }
            });
        page
    }

    /// Walk the fake index with [`next_page_start`] and return every poll fetched, with
    /// the number of pages read. Panics instead of looping forever.
    fn walk(
        index: &[(TimestampMillis, Option<u32>)],
        range: (TimestampMillis, TimestampMillis),
        limit: usize,
    ) -> (BTreeSet<u32>, usize) {
        let mut start = (range.0, true);
        let mut fetched = BTreeSet::new();
        for pages in 1..=2 * index.len() + 1 {
            let page = fake_page(index, start, range.1, limit);
            let last = page.keys().max().copied();
            fetched.extend(page.into_values().flatten());
            match next_page_start(start, last, range.1) {
                Some(next) => start = next,
                None => return (fetched, pages),
            }
        }
        panic!("the walk did not end");
    }

    fn polls(entries: &[(TimestampMillis, u32)]) -> Vec<(TimestampMillis, Option<u32>)> {
        entries
            .iter()
            .map(|(time, poll)| (*time, Some(*poll)))
            .collect()
    }

    #[test]
    fn a_page_cut_inside_an_end_time_rereads_that_end_time() {
        // A limit of 3 cuts the first page between polls 3 and 4, which both end at 20.
        let index = polls(&[(10, 1), (20, 2), (20, 3), (20, 4), (30, 5)]);

        let (fetched, _) = walk(&index, (0, 100), 3);

        assert_eq!(fetched, BTreeSet::from([1, 2, 3, 4, 5]));
    }

    #[test]
    fn a_cut_page_starts_the_next_one_at_its_last_end_time_included() {
        assert_eq!(next_page_start((5, true), Some(20), 100), Some((20, true)));
        assert_eq!(next_page_start((5, false), Some(20), 100), Some((20, true)));
    }

    #[test]
    fn more_polls_at_one_end_time_than_the_limit_still_end_the_walk() {
        // Five polls end at 20 but a page holds three: without an offset, the
        // last two are out of reach, yet the walk must move on and find poll 7.
        let index = polls(&[
            (10, 1),
            (20, 2),
            (20, 3),
            (20, 4),
            (20, 5),
            (20, 6),
            (30, 7),
        ]);

        let (fetched, pages) = walk(&index, (0, 100), 3);

        assert!(fetched.is_superset(&BTreeSet::from([1, 2, 3, 4, 7])));
        assert!(pages <= 5, "read {pages} pages");
    }

    #[test]
    fn a_page_holding_only_its_included_start_moves_past_it() {
        assert_eq!(
            next_page_start((20, true), Some(20), 100),
            Some((20, false))
        );
    }

    #[test]
    fn an_empty_page_ends_the_walk() {
        // Pins the current behaviour, which can miss polls behind a page of empty
        // end-time trees; see the TODO(upstream) in `next_page_start`.
        assert_eq!(next_page_start((20, true), None, 100), None);
        assert_eq!(next_page_start((20, false), None, 100), None);
        assert_eq!(walk(&[], (0, 100), 3), (BTreeSet::new(), 1));
    }

    #[test]
    fn a_stale_page_not_after_its_start_ends_the_walk() {
        assert_eq!(next_page_start((20, false), Some(20), 100), None);
        assert_eq!(next_page_start((20, true), Some(10), 100), None);
    }

    #[test]
    fn a_poll_at_the_range_end_ends_the_walk_without_an_invalid_query() {
        // The platform rejects a start equal to the end unless both are included.
        assert_eq!(next_page_start((100, true), Some(100), 100), None);
        assert_eq!(
            next_page_start((50, true), Some(100), 100),
            Some((100, true))
        );

        let index = polls(&[(50, 1), (100, 2), (100, 3)]);
        let (fetched, _) = walk(&index, (0, 100), 2);
        assert_eq!(fetched, BTreeSet::from([1, 2, 3]));
    }

    #[test]
    fn empty_end_time_trees_taking_page_slots_lose_no_poll() {
        // Empty end-time trees fill slots, so a cut page holds fewer polls than the limit.
        let index = vec![
            (10, None),
            (20, Some(1)),
            (30, Some(2)),
            (30, Some(3)),
            (40, None),
            (50, Some(4)),
        ];

        let (fetched, _) = walk(&index, (0, 100), 3);

        assert_eq!(fetched, BTreeSet::from([1, 2, 3, 4]));
    }
}
