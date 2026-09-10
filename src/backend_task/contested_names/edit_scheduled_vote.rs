//! Validated, in-place edits of unstarted DPNS schedules.

use crate::app::TaskResult;
use crate::backend_task::{BackendTaskSuccessResult, error::TaskError};
use crate::context::AppContext;
use crate::model::dpns_voting::{
    DpnsScheduleEditValidationError, DpnsScheduledVoteEdit, validate_dpns_schedule_edit,
};
use dash_sdk::Sdk;

impl AppContext {
    pub(super) async fn edit_scheduled_dpns_vote(
        &self,
        edit: DpnsScheduledVoteEdit,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let target = self.scheduled_dpns_vote_edit_target(&edit)?;
        if self.dpns_vote_poll_id(&target.contested_name)? != edit.key.vote_poll_id {
            return Err(TaskError::DpnsScheduledVoteNotEditable);
        }
        self.checked_dpns_vote_poll(&target.contested_name, sdk)
            .await?;
        let normalized_name =
            dash_sdk::dpp::util::strings::convert_to_homograph_safe_chars(&target.contested_name);
        self.query_dpns_vote_contenders(&normalized_name, sdk, sender)
            .await?;
        // Identity removal takes this guard before the journal guard too. No
        // network work or state-publication mutex is held while editing storage.
        let lock = self.identity_record_lock(edit.key.voter_id);
        let _identity_guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.is_identity_listed(&edit.key.voter_id)?
            || !self
                .get_local_qualified_identity(&edit.key.voter_id)?
                .is_some_and(|identity| identity.can_cast_masternode_vote())
        {
            return Err(TaskError::DpnsScheduledVoteVoterUnavailable);
        }
        let contest = self
            .all_contested_names()?
            .into_iter()
            .find(|contest| contest.normalized_contested_name == normalized_name)
            .ok_or_else(|| TaskError::VotePollNotFound {
                name: target.contested_name.clone(),
            })?;
        let now_ms = std::time::UNIX_EPOCH
            .elapsed()
            .unwrap_or_default()
            .as_millis() as u64;
        validate_dpns_schedule_edit(edit.choice, edit.unix_timestamp, now_ms, &contest).map_err(
            |error| match error {
                DpnsScheduleEditValidationError::Time => TaskError::DpnsScheduledVoteInvalidTime,
                DpnsScheduleEditValidationError::Choice => {
                    TaskError::DpnsScheduledVoteInvalidChoice
                }
                DpnsScheduleEditValidationError::Contest => TaskError::VotePollNotFound {
                    name: target.contested_name.clone(),
                },
            },
        )?;
        let (operation_id, mirror_error) = self.edit_scheduled_dpns_vote_target(&edit)?;
        if let Some(error) = mirror_error {
            tracing::warn!(?error, %operation_id, "DPNS schedule edit was journaled but its compatibility mirror could not be updated");
        }
        Ok(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: self.network,
            operation_id,
        })
    }
}
