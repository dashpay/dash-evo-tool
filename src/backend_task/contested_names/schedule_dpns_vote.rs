use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledDPNSVote {
    pub contested_name: String,
    pub voter_id: Identifier,
    pub choice: ResourceVoteChoice,
    pub time: u64,
}

impl AppContext {
    /// Inserts votes into the local db to be cast later
    pub(super) async fn schedule_dpns_vote(
        self: &Arc<Self>,
        scheduled_votes: &Vec<ScheduledDPNSVote>,
    ) -> Result<BackendTaskSuccessResult, String> {
        for vote in scheduled_votes {
            self.db
                .insert_scheduled_vote(
                    vote.voter_id.as_slice(),
                    vote.contested_name.clone(),
                    vote.choice,
                    vote.time,
                    self,
                )
                .map_err(|e| format!("Failed to insert scheduled vote: {}", e))?;
        }
        Ok(BackendTaskSuccessResult::Message(
            "Successfully scheduled votes".to_string(),
        ))
    }
}
