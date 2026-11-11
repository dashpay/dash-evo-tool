//! Retained draft, navigation, and reversible comparison state for Token Creator.

use crate::model::token_creator::rules::RuleKind;
use crate::model::token_creator::{DraftError, Section, TokenDraft};
use crate::ui::components::local_schedule_input::LocalScheduleInput;
use std::collections::BTreeMap;

/// Which authority rows are shown; hidden rules remain part of the draft.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum RuleFilter {
    Enabled,
    Changeable,
    #[default]
    All,
}

/// A proposed complete replacement, applied only after comparison.
pub struct Proposal {
    pub title: &'static str,
    pub draft: TokenDraft,
}

/// Screen-owned editor state, independent of identity readiness.
#[derive(Default)]
pub struct CreatorWorkspace {
    pub draft: TokenDraft,
    pub section: Section,
    pub help: Option<String>,
    pub proposal: Option<Proposal>,
    pub undo: Option<TokenDraft>,
    pub rule_editor: Option<RuleKind>,
    pub rule_filter: RuleFilter,
    pub error: Option<DraftError>,
    pub notice: Option<String>,
    pub schedule_inputs: BTreeMap<u64, LocalScheduleInput>,
    pub next_schedule_id: u64,
    pub preview_reference: String,
}

impl CreatorWorkspace {
    /// Moves within the approved sequence without changing any draft data.
    pub fn navigate(&mut self, forward: bool) {
        let index = Section::ALL
            .iter()
            .position(|section| *section == self.section)
            .unwrap_or(0);
        if forward && index + 1 < Section::ALL.len() {
            self.section = Section::ALL[index + 1];
        } else if !forward && index > 0 {
            self.section = Section::ALL[index - 1];
        }
    }

    /// Applies a reviewed proposal and retains an exact undo copy.
    pub fn apply_proposal(&mut self) {
        if let Some(proposal) = self.proposal.take() {
            self.undo = Some(std::mem::replace(&mut self.draft, proposal.draft));
            self.error = None;
        }
    }

    /// Restores every previous input, including inactive distributions.
    pub fn undo_proposal(&mut self) {
        if let Some(previous) = self.undo.take() {
            self.draft = previous;
            self.error = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_sections_are_sequential_and_navigation_preserves_draft() {
        let mut state = CreatorWorkspace {
            draft: TokenDraft::example(true),
            ..Default::default()
        };
        let original = state.draft.clone();
        for section in Section::ALL {
            assert_eq!(state.section, section);
            state.navigate(true);
        }
        assert_eq!(state.section, Section::Review);
        for section in Section::ALL.into_iter().rev() {
            assert_eq!(state.section, section);
            state.navigate(false);
        }
        assert_eq!(state.draft, original);
    }

    #[test]
    fn comparison_cancel_apply_and_undo_preserve_complete_draft() {
        let mut state = CreatorWorkspace::default();
        state.draft.names[0].singular = "Studio".into();
        state.draft.documents = "{\"note\":{}}".into();
        state.draft.recurring.enabled = true;
        let original = state.draft.clone();
        state.proposal = Some(Proposal {
            title: "Fixed supply",
            draft: state.draft.example_proposal(false),
        });
        assert_eq!(state.draft, original);
        state.proposal = None;
        assert_eq!(state.draft, original);
        state.proposal = Some(Proposal {
            title: "Fixed supply",
            draft: state.draft.example_proposal(false),
        });
        state.apply_proposal();
        assert_eq!(state.draft.documents, original.documents);
        assert_eq!(state.draft.names, original.names);
        assert!(!state.draft.recurring.enabled);
        state.undo_proposal();
        assert_eq!(state.draft, original);
    }
}
