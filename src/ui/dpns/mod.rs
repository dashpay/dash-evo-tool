//! Masternode voting on contested DPNS names (Masternodes ▸ Votes).

pub mod attention_chip;
pub mod copy;
pub mod dpns_contested_names_screen;

/// Sub-view of the Masternodes ▸ Votes segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VotesView {
    /// Open contests at least one node-set node still has to decide on.
    #[default]
    ToDecide,
    /// Open contests every node-set node has already voted on.
    Voted,
    /// Scheduled and missed automatic votes.
    Scheduled,
    /// Finished contests and how the operator's nodes voted.
    History,
}

impl VotesView {
    /// Every sub-view, in chip order.
    pub const ALL: [Self; 4] = [Self::ToDecide, Self::Voted, Self::Scheduled, Self::History];

    /// The chip label.
    pub fn label(self) -> &'static str {
        match self {
            Self::ToDecide => "To decide",
            Self::Voted => "Voted",
            Self::Scheduled => "Scheduled",
            Self::History => "History",
        }
    }
}
