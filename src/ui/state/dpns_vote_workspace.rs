//! Non-rendering state for the shared DPNS Voting Center composer.

use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use std::collections::BTreeMap;

/// Current step of the full-page voting composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpnsVoteComposerStep {
    Nodes,
    Votes,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerKeyAction {
    None,
    Advance,
    Submit,
    CloseDraft,
}

/// Per-node timing override in the draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftVoteTiming {
    Excluded,
    Now,
    Scheduled { days: u32, hours: u32, minutes: u32 },
}

/// Shared quick/bulk draft state; renders nothing.
#[derive(Debug, Clone)]
pub struct DpnsVoteWorkspace {
    pub step: DpnsVoteComposerStep,
    pub node_timing: BTreeMap<Identifier, DraftVoteTiming>,
    pub contest_choices: BTreeMap<String, ResourceVoteChoice>,
    pub set_all_timing: DraftVoteTiming,
}

impl DpnsVoteWorkspace {
    pub fn new(node_ids: impl IntoIterator<Item = Identifier>) -> Self {
        Self {
            step: DpnsVoteComposerStep::Nodes,
            node_timing: node_ids
                .into_iter()
                .map(|node_id| (node_id, DraftVoteTiming::Now))
                .collect(),
            contest_choices: BTreeMap::new(),
            set_all_timing: DraftVoteTiming::Now,
        }
    }

    /// Restrict the initial draft to one node from a detail-page deep link.
    pub fn prefilter_node(&mut self, selected: Identifier) {
        for (node_id, timing) in &mut self.node_timing {
            *timing = if *node_id == selected {
                DraftVoteTiming::Now
            } else {
                DraftVoteTiming::Excluded
            };
        }
    }

    pub fn selected_node_count(&self) -> usize {
        self.node_timing
            .values()
            .filter(|timing| **timing != DraftVoteTiming::Excluded)
            .count()
    }

    pub fn apply_timing_to_all(&mut self) {
        for timing in self.node_timing.values_mut() {
            *timing = self.set_all_timing;
        }
    }

    /// Resolve keyboard intent without letting Enter submit before Review.
    pub fn keyboard_action(
        &self,
        enter_pressed: bool,
        escape_pressed: bool,
        can_continue: bool,
    ) -> ComposerKeyAction {
        if escape_pressed {
            return ComposerKeyAction::CloseDraft;
        }
        if !enter_pressed || !can_continue {
            return ComposerKeyAction::None;
        }
        match self.step {
            DpnsVoteComposerStep::Nodes | DpnsVoteComposerStep::Votes => ComposerKeyAction::Advance,
            DpnsVoteComposerStep::Review => ComposerKeyAction::Submit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VOTE-TC-021/022: set-all applies timing and an individual override survives.
    #[test]
    fn set_all_timing_allows_per_node_override() {
        let first = Identifier::from([1; 32]);
        let second = Identifier::from([2; 32]);
        let mut workspace = DpnsVoteWorkspace::new([first, second]);
        workspace.set_all_timing = DraftVoteTiming::Scheduled {
            days: 1,
            hours: 2,
            minutes: 3,
        };
        workspace.apply_timing_to_all();
        workspace.node_timing.insert(first, DraftVoteTiming::Now);

        assert_eq!(workspace.node_timing[&first], DraftVoteTiming::Now);
        assert!(matches!(
            workspace.node_timing[&second],
            DraftVoteTiming::Scheduled { .. }
        ));
    }

    /// VOTE-TC-024: a node-detail route selects only that node.
    #[test]
    fn node_prefilter_excludes_every_other_node() {
        let selected = Identifier::from([1; 32]);
        let other = Identifier::from([2; 32]);
        let mut workspace = DpnsVoteWorkspace::new([selected, other]);
        workspace.prefilter_node(selected);

        assert_eq!(workspace.selected_node_count(), 1);
        assert_eq!(workspace.node_timing[&selected], DraftVoteTiming::Now);
        assert_eq!(workspace.node_timing[&other], DraftVoteTiming::Excluded);
    }

    /// VOTE-TC-071: Enter advances drafts but submits only from Review; Escape closes drafts.
    #[test]
    fn keyboard_actions_respect_composer_step() {
        let mut workspace = DpnsVoteWorkspace::new([Identifier::from([1; 32])]);
        assert_eq!(
            workspace.keyboard_action(true, false, true),
            ComposerKeyAction::Advance
        );
        workspace.step = DpnsVoteComposerStep::Votes;
        assert_eq!(
            workspace.keyboard_action(true, false, true),
            ComposerKeyAction::Advance
        );
        workspace.step = DpnsVoteComposerStep::Review;
        assert_eq!(
            workspace.keyboard_action(true, false, true),
            ComposerKeyAction::Submit
        );
        assert_eq!(
            workspace.keyboard_action(false, true, true),
            ComposerKeyAction::CloseDraft
        );
    }
}
