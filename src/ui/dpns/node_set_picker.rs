//! `Vote with:` node-set chip and popover (VOTE-FR-075, frame V3).

use crate::model::dpns_voting::operator::{
    ListMembership, NodeExclusion, NodeSet, ResolvedNodeSet, VotingNode, VotingNodeKind,
    node_exclusion,
};
use crate::ui::dpns::contest_card::node_label;
use crate::ui::dpns::copy::node_set_chip_label;
use eframe::egui::{self, RichText, Ui};
use std::collections::{BTreeMap, BTreeSet};

/// Display name of a node set.
pub fn node_set_name(set: &NodeSet) -> &'static str {
    match set {
        NodeSet::All => "All my nodes",
        NodeSet::EvonodesOnly => "Evonodes only",
        NodeSet::MasternodesOnly => "Masternodes only",
        NodeSet::Custom(_) => "Custom",
    }
}

/// Why a node row is disabled, or a membership note for a usable node.
pub fn node_row_note(node: &VotingNode) -> Option<&'static str> {
    match node_exclusion(node) {
        Some(NodeExclusion::NoVotingKey) => Some("No voting key is loaded for this node."),
        Some(NodeExclusion::NotInMasternodeList) => {
            Some("Not in the masternode list. Its votes don't count.")
        }
        None if node.membership == ListMembership::Unknown => {
            Some("Masternode list membership unknown.")
        }
        None => None,
    }
}

/// What the operator changed in the picker this frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NodeSetPickerResponse {
    /// The new set for this session, when it changed.
    pub changed: Option<NodeSet>,
    /// `Save as my default` was clicked for the current set.
    pub save_default: bool,
}

/// Render the chip and, when open, its popover.
pub fn show(
    ui: &mut Ui,
    set: &NodeSet,
    resolved: &ResolvedNodeSet,
    nodes: &[VotingNode],
) -> NodeSetPickerResponse {
    let mut response = NodeSetPickerResponse::default();
    let chip = ui.button(node_set_chip_label(
        node_set_name(set),
        resolved.included.len(),
        resolved.weight,
    ));
    egui::Popup::from_toggle_button_response(&chip)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(320.0);
            ui.label(RichText::new("Vote with").strong());
            for preset in [
                NodeSet::All,
                NodeSet::EvonodesOnly,
                NodeSet::MasternodesOnly,
            ] {
                if ui.radio(*set == preset, node_set_name(&preset)).clicked() {
                    response.changed = Some(preset);
                }
            }
            let custom = matches!(set, NodeSet::Custom(_));
            if ui.radio(custom, "Custom").clicked() && !custom {
                response.changed =
                    Some(NodeSet::Custom(resolved.included.iter().copied().collect()));
            }
            ui.separator();
            let labels: BTreeMap<_, _> = nodes
                .iter()
                .filter_map(|node| node.alias.clone().map(|alias| (node.id, alias)))
                .collect();
            for node in nodes {
                let usable = node_exclusion(node).is_none();
                let mut checked = set.selects(node) && usable;
                let kind = match node.kind {
                    VotingNodeKind::Evonode => "Evonode",
                    VotingNodeKind::Masternode => "Masternode",
                };
                ui.horizontal(|ui| {
                    let toggled = ui
                        .add_enabled(
                            usable,
                            egui::Checkbox::new(
                                &mut checked,
                                format!("{label} · {kind}", label = node_label(node.id, &labels)),
                            ),
                        )
                        .changed();
                    if toggled {
                        let mut ids: BTreeSet<_> = nodes
                            .iter()
                            .filter(|other| set.selects(other) && node_exclusion(other).is_none())
                            .map(|other| other.id)
                            .collect();
                        if checked {
                            ids.insert(node.id);
                        } else {
                            ids.remove(&node.id);
                        }
                        response.changed = Some(NodeSet::Custom(ids));
                    }
                    if let Some(note) = node_row_note(node) {
                        ui.label(RichText::new(note).small().weak());
                    }
                });
            }
            ui.separator();
            if ui.button("Save as my default").clicked() {
                response.save_default = true;
            }
        });
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::platform::Identifier;

    fn node(key: bool, membership: ListMembership) -> VotingNode {
        VotingNode {
            id: Identifier::from([1; 32]),
            kind: VotingNodeKind::Evonode,
            has_voting_key: key,
            membership,
            alias: None,
        }
    }

    /// VOTE-TC-096/097: disabled rows carry their reason; unknown membership
    /// is a note, never an exclusion (fail open).
    #[test]
    fn rows_explain_exclusions_and_unknown_membership() {
        assert_eq!(
            node_row_note(&node(false, ListMembership::Listed)),
            Some("No voting key is loaded for this node.")
        );
        assert_eq!(
            node_row_note(&node(true, ListMembership::NotListed)),
            Some("Not in the masternode list. Its votes don't count.")
        );
        let unknown = node(true, ListMembership::Unknown);
        assert_eq!(
            node_row_note(&unknown),
            Some("Masternode list membership unknown.")
        );
        assert_eq!(node_exclusion(&unknown), None);
        assert_eq!(node_row_note(&node(true, ListMembership::Listed)), None);
    }
}
