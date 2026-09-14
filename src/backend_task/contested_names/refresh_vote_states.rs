//! Proved current-vote queries against Platform, one pass per loaded masternode.

use crate::backend_task::error::TaskError;
use crate::context::{AppContext, DpnsVoteRefreshResults, MAX_CONCURRENT_DPNS_VOTERS};
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::votes::resource_vote::ResourceVote;
use dash_sdk::dpp::voting::votes::resource_vote::accessors::v0::ResourceVoteGettersV0;
use dash_sdk::drive::query::contested_resource_votes_given_by_identity_query::ContestedResourceVotesGivenByIdentityQuery;
use dash_sdk::platform::{FetchMany, Identifier};
use futures::{StreamExt, stream};
use std::collections::BTreeMap;
use std::sync::Arc;

const VOTE_QUERY_PAGE_SIZE: u16 = 100;

impl AppContext {
    /// Refresh proved vote state once per loaded masternode, paging only as needed.
    pub(crate) async fn refresh_dpns_vote_states(
        &self,
        sdk: &Sdk,
    ) -> Result<DpnsVoteRefreshResults, TaskError> {
        let voters = self.load_local_masternode_identities()?;
        let kv = self.det_kv()?;

        Ok(stream::iter(voters)
            .map(|voter| {
                let sdk = sdk.clone();
                let kv = kv.clone();
                async move {
                    let voter_id = voter.identity.id();
                    let result = self.publish_dpns_vote_state(
                        &kv, voter_id, fetch_votes_for_voter(&sdk, voter_id),
                    ).await.map_err(Arc::new);
                    if let Err(error) = &result {
                        tracing::warn!(?error, %voter_id, "Could not refresh proved DPNS vote state");
                    }
                    (voter_id, result)
                }
            })
            .buffer_unordered(MAX_CONCURRENT_DPNS_VOTERS)
            .collect::<BTreeMap<_, _>>()
            .await)
    }
}

async fn fetch_votes_for_voter(
    sdk: &Sdk,
    voter_id: Identifier,
) -> Result<BTreeMap<[u8; 32], ResourceVoteChoice>, Box<dash_sdk::Error>> {
    let mut votes = BTreeMap::new();
    let mut start_at = None;
    loop {
        let page = ResourceVote::fetch_many(
            sdk,
            ContestedResourceVotesGivenByIdentityQuery {
                identity_id: voter_id,
                offset: None,
                limit: Some(VOTE_QUERY_PAGE_SIZE),
                start_at,
                order_ascending: true,
            },
        )
        .await?;
        let page_len = page.len();
        let last_key = page.last().map(|(id, _)| id.to_buffer());
        for (poll_id, vote) in page {
            if let Some(vote) = vote {
                votes.insert(poll_id.to_buffer(), vote.resource_vote_choice());
            }
        }
        if page_len < usize::from(VOTE_QUERY_PAGE_SIZE) {
            break;
        }
        let Some(last_key) = last_key else {
            break;
        };
        start_at = Some((last_key, false));
    }
    Ok(votes)
}
