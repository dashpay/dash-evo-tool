//! Contested resource (DPNS) Tauri IPC commands.
//!
//! Maps all 7 `ContestedResourceTask` variants plus direct database methods
//! to Tauri commands. Covers DPNS name contests, voting, and scheduled votes.

use crate::commands::identity::parse_identifier;
use crate::dto::common::IdentifierDto;
use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::contested_names::{ContestedResourceTask, ScheduledDPNSVote};
use dash_evo_tool::backend_task::BackendTask;

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Input DTOs
// ---------------------------------------------------------------------------

/// A vote choice for DPNS contested names.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum VoteChoiceDto {
    /// Vote to lock the name (no one gets it).
    TowardsIdentity {
        #[serde(rename = "identityId")]
        identity_id: IdentifierDto,
    },
    /// Abstain from voting.
    Abstain,
    /// Vote to lock the name.
    Lock,
}

impl VoteChoiceDto {
    fn to_backend(
        &self,
    ) -> Result<dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice, String>
    {
        use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
        match self {
            VoteChoiceDto::TowardsIdentity { identity_id } => {
                let id = parse_identifier(identity_id)?;
                Ok(ResourceVoteChoice::TowardsIdentity(id))
            }
            VoteChoiceDto::Abstain => Ok(ResourceVoteChoice::Abstain),
            VoteChoiceDto::Lock => Ok(ResourceVoteChoice::Lock),
        }
    }
}

/// A single vote entry: (contested name, vote choice).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoteEntry {
    pub contested_name: String,
    pub choice: VoteChoiceDto,
}

/// Input for voting on DPNS names.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoteOnDpnsNamesInput {
    /// List of (name, choice) pairs to vote on.
    pub votes: Vec<VoteEntry>,
    /// List of voter identity IDs.
    pub voter_identity_ids: Vec<IdentifierDto>,
}

/// A scheduled DPNS vote DTO.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledVoteDto {
    pub contested_name: String,
    pub voter_id: IdentifierDto,
    pub choice: VoteChoiceDto,
    pub unix_timestamp: u64,
    pub executed_successfully: bool,
}

impl ScheduledVoteDto {
    fn to_backend(&self) -> Result<ScheduledDPNSVote, String> {
        let voter_id = parse_identifier(&self.voter_id)?;
        let choice = self.choice.to_backend()?;
        Ok(ScheduledDPNSVote {
            contested_name: self.contested_name.clone(),
            voter_id,
            choice,
            unix_timestamp: self.unix_timestamp,
            executed_successfully: self.executed_successfully,
        })
    }
}

/// Input for scheduling DPNS votes.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDpnsVotesInput {
    pub votes: Vec<ScheduledVoteDto>,
}

/// Input for casting a single scheduled vote.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CastScheduledVoteInput {
    pub vote: ScheduledVoteDto,
}

/// Input for deleting a single scheduled vote.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteScheduledVoteInput {
    pub voter_id: IdentifierDto,
    pub contested_name: String,
}

// ---------------------------------------------------------------------------
// Helper: look up QualifiedIdentity
// ---------------------------------------------------------------------------

fn lookup_identity(
    state: &AppState,
    identity_id: &str,
) -> Result<dash_evo_tool::model::qualified_identity::QualifiedIdentity, String> {
    let identifier = parse_identifier(identity_id)?;
    let ctx = state.current_context();
    ctx.get_identity_by_id(&identifier)
        .map_err(|e| format!("Database error loading identity: {e}"))?
        .ok_or_else(|| format!("Identity {} not found in local database", identity_id))
}

// ---------------------------------------------------------------------------
// Async dispatch commands
// ---------------------------------------------------------------------------

/// Query all contested DPNS names and vote contenders.
#[tauri::command]
#[specta::specta]
pub fn contested_query_dpns_contests(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::ContestedResourceTask(ContestedResourceTask::QueryDPNSContests);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Vote on one or more contested DPNS names.
#[tauri::command]
#[specta::specta]
pub fn contested_vote_on_dpns_names(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: VoteOnDpnsNamesInput,
) -> Result<DispatchTaskResponse, String> {
    let votes: Vec<(
        String,
        dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice,
    )> = input
        .votes
        .iter()
        .map(|v| {
            let choice = v.choice.to_backend()?;
            Ok((v.contested_name.clone(), choice))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let voters: Vec<dash_evo_tool::model::qualified_identity::QualifiedIdentity> = input
        .voter_identity_ids
        .iter()
        .map(|id| lookup_identity(&state, id))
        .collect::<Result<Vec<_>, String>>()?;

    let task =
        BackendTask::ContestedResourceTask(ContestedResourceTask::VoteOnDPNSNames(votes, voters));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Schedule DPNS votes for future execution.
#[tauri::command]
#[specta::specta]
pub fn contested_schedule_dpns_votes(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: ScheduleDpnsVotesInput,
) -> Result<DispatchTaskResponse, String> {
    let votes: Vec<ScheduledDPNSVote> = input
        .votes
        .iter()
        .map(|v| v.to_backend())
        .collect::<Result<Vec<_>, String>>()?;

    let task = BackendTask::ContestedResourceTask(ContestedResourceTask::ScheduleDPNSVotes(votes));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Cast a single scheduled vote immediately.
#[tauri::command]
#[specta::specta]
pub fn contested_cast_scheduled_vote(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: CastScheduledVoteInput,
) -> Result<DispatchTaskResponse, String> {
    let vote = input.vote.to_backend()?;
    let voter = lookup_identity(&state, &hex::encode(vote.voter_id.as_slice()))?;
    let task = BackendTask::ContestedResourceTask(ContestedResourceTask::CastScheduledVote(
        vote,
        Box::new(voter),
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Clear all scheduled votes.
#[tauri::command]
#[specta::specta]
pub fn contested_clear_all_scheduled_votes(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::ContestedResourceTask(ContestedResourceTask::ClearAllScheduledVotes);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Clear only executed scheduled votes.
#[tauri::command]
#[specta::specta]
pub fn contested_clear_executed_scheduled_votes(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task =
        BackendTask::ContestedResourceTask(ContestedResourceTask::ClearExecutedScheduledVotes);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Delete a single scheduled vote.
#[tauri::command]
#[specta::specta]
pub fn contested_delete_scheduled_vote(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: DeleteScheduledVoteInput,
) -> Result<DispatchTaskResponse, String> {
    let voter_id = parse_identifier(&input.voter_id)?;
    let task = BackendTask::ContestedResourceTask(ContestedResourceTask::DeleteScheduledVote(
        voter_id,
        input.contested_name,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// Direct database reads
// ---------------------------------------------------------------------------

/// Get all scheduled votes from local database.
#[tauri::command]
#[specta::specta]
pub fn contested_get_scheduled_votes(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<ScheduledVoteDto>, String> {
    let ctx = state.current_context();
    let votes = ctx
        .get_scheduled_votes()
        .map_err(|e| format!("Failed to load scheduled votes: {e}"))?;
    Ok(votes
        .into_iter()
        .map(|v| {
            use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
            let choice = match v.choice {
                ResourceVoteChoice::TowardsIdentity(id) => VoteChoiceDto::TowardsIdentity {
                    identity_id: hex::encode(id.as_slice()),
                },
                ResourceVoteChoice::Abstain => VoteChoiceDto::Abstain,
                ResourceVoteChoice::Lock => VoteChoiceDto::Lock,
            };
            ScheduledVoteDto {
                contested_name: v.contested_name,
                voter_id: hex::encode(v.voter_id.as_slice()),
                choice,
                unix_timestamp: v.unix_timestamp,
                executed_successfully: v.executed_successfully,
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vote_choice_dto_serializes() {
        let choice = VoteChoiceDto::TowardsIdentity {
            identity_id: "abc".into(),
        };
        let json = serde_json::to_string(&choice).unwrap();
        assert!(json.contains("\"towardsIdentity\""));
        assert!(json.contains("\"identityId\":\"abc\""));

        let abstain = VoteChoiceDto::Abstain;
        let json = serde_json::to_string(&abstain).unwrap();
        assert!(json.contains("\"abstain\""));

        let lock = VoteChoiceDto::Lock;
        let json = serde_json::to_string(&lock).unwrap();
        assert!(json.contains("\"lock\""));
    }

    #[test]
    fn vote_entry_serializes() {
        let entry = VoteEntry {
            contested_name: "alice".into(),
            choice: VoteChoiceDto::Abstain,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"contestedName\":\"alice\""));
    }

    #[test]
    fn vote_on_dpns_names_input_serializes() {
        let input = VoteOnDpnsNamesInput {
            votes: vec![VoteEntry {
                contested_name: "alice".into(),
                choice: VoteChoiceDto::Abstain,
            }],
            voter_identity_ids: vec!["abc".into()],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"voterIdentityIds\""));
    }

    #[test]
    fn scheduled_vote_dto_serializes() {
        let vote = ScheduledVoteDto {
            contested_name: "bob".into(),
            voter_id: "abc".into(),
            choice: VoteChoiceDto::Lock,
            unix_timestamp: 1234567890,
            executed_successfully: false,
        };
        let json = serde_json::to_string(&vote).unwrap();
        assert!(json.contains("\"contestedName\":\"bob\""));
        assert!(json.contains("\"unixTimestamp\":1234567890"));
        assert!(json.contains("\"executedSuccessfully\":false"));
    }

    #[test]
    fn delete_scheduled_vote_input_serializes() {
        let input = DeleteScheduledVoteInput {
            voter_id: "abc".into(),
            contested_name: "alice".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"voterId\":\"abc\""));
        assert!(json.contains("\"contestedName\":\"alice\""));
    }
}
