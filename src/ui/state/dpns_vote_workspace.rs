//! Non-rendering state for the shared DPNS Voting Center composer.

use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use std::collections::{BTreeMap, BTreeSet};

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
    Now,
    Scheduled { days: u32, hours: u32, minutes: u32 },
}

/// Shared quick/bulk draft state; renders nothing.
#[derive(Debug, Clone)]
pub struct DpnsVoteWorkspace {
    pub step: DpnsVoteComposerStep,
    selected_nodes: BTreeSet<Identifier>,
    pub node_timing: BTreeMap<Identifier, DraftVoteTiming>,
    pub contest_choices: BTreeMap<String, ResourceVoteChoice>,
    pub set_all_timing: DraftVoteTiming,
}

impl DpnsVoteWorkspace {
    pub fn new(node_ids: impl IntoIterator<Item = Identifier>) -> Self {
        Self {
            step: DpnsVoteComposerStep::Nodes,
            selected_nodes: BTreeSet::new(),
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
        self.selected_nodes.clear();
        if let Some(timing) = self.node_timing.get_mut(&selected) {
            *timing = DraftVoteTiming::Now;
            self.selected_nodes.insert(selected);
        }
    }

    pub fn selected_node_count(&self) -> usize {
        self.selected_nodes.len()
    }

    pub fn is_node_selected(&self, node_id: &Identifier) -> bool {
        self.selected_nodes.contains(node_id)
    }

    pub fn set_node_selected(&mut self, node_id: Identifier, selected: bool) {
        if selected {
            if self.node_timing.contains_key(&node_id) {
                self.selected_nodes.insert(node_id);
            }
        } else {
            self.selected_nodes.remove(&node_id);
        }
    }

    pub fn apply_timing_to_selected(&mut self) {
        for node_id in &self.selected_nodes {
            if let Some(timing) = self.node_timing.get_mut(node_id) {
                *timing = self.set_all_timing;
            }
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
        workspace.set_node_selected(first, true);
        workspace.set_node_selected(second, true);
        workspace.apply_timing_to_selected();
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
        assert!(!workspace.is_node_selected(&other));
    }

    /// VOTE-TC-020: an unfiltered bulk draft starts with no nodes selected.
    #[test]
    fn bulk_draft_starts_without_selected_nodes() {
        let first = Identifier::from([1; 32]);
        let second = Identifier::from([2; 32]);
        let workspace = DpnsVoteWorkspace::new([first, second]);

        assert_eq!(workspace.selected_node_count(), 0);
        assert!(!workspace.is_node_selected(&first));
        assert!(!workspace.is_node_selected(&second));
    }

    /// VOTE-TC-021: set-all changes timing only for explicitly selected nodes.
    #[test]
    fn set_all_timing_ignores_unselected_nodes() {
        let selected = Identifier::from([1; 32]);
        let unselected = Identifier::from([2; 32]);
        let mut workspace = DpnsVoteWorkspace::new([selected, unselected]);
        workspace.set_node_selected(selected, true);
        workspace.set_all_timing = DraftVoteTiming::Scheduled {
            days: 1,
            hours: 2,
            minutes: 3,
        };

        workspace.apply_timing_to_selected();

        assert!(matches!(
            workspace.node_timing[&selected],
            DraftVoteTiming::Scheduled { .. }
        ));
        assert_eq!(workspace.node_timing[&unselected], DraftVoteTiming::Now);
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
