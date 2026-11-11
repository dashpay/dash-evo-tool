use super::*;
use crate::model::token_creator::groups::{GroupDraft, MemberDraft};
use dash_sdk::dpp::version::PlatformVersion;

pub(super) fn show(ui: &mut Ui, state: &mut CreatorWorkspace, version: &PlatformVersion) {
    card(ui, |ui| {
        heading(ui, "Approval groups", "groups", &mut state.help);
        ui.label("Combine member approval power to authorize actions. Labels are local; Review shows the positions stored in the contract.");
        let mut remove = None;
        for index in 0..state.draft.groups.len() {
            let id = state.draft.groups[index].position;
            let references = state.draft.group_references(id);
            let group = &mut state.draft.groups[index];
            ui.push_id(id, |ui| {
                ui.separator();
                ui.strong(format!("Draft group {id} · contract position {index}"));
                text(ui,"Group label (local only)","groupName",&mut group.label,&mut state.help);
                text(ui,"Required approval power","threshold",&mut group.threshold,&mut state.help);
                ui.small("At least two members are required. Each power and the requirement must be from 1 to 65,535.");
                let mut member_remove = None;
                for (member_index,member) in group.members.iter_mut().enumerate() {
                    ui.push_id(member_index, |ui| {
                        text(ui,"Member identity ID","identity",&mut member.identity,&mut state.help);
                        text(ui,"Approval power","weight",&mut member.weight,&mut state.help);
                        if ui.button("Remove member").clicked() { member_remove = Some(member_index); }
                    });
                }
                if let Some(index) = member_remove { group.members.remove(index); }
                if ui.button("Add member").clicked() { group.members.push(MemberDraft::default()); }
                ui.strong(group.power_summary());
                if let Err(error) = group.parse(version) { ui.colored_label(ui.visuals().error_fg_color,error.to_string()); }
                if references.is_empty() {
                    if ui.button("Delete group").clicked() { remove = Some(id); }
                } else { ui.label(format!("Used by: {}. Change these references before deleting this group.",references.join(", "))); }
            });
        }
        if let Some(id) = remove {
            state.draft.remove_group(id);
        }
        if ui.button("Add group").clicked() {
            if let Some(id) = (0..=u16::MAX)
                .find(|id| state.draft.groups.iter().all(|group| group.position != *id))
            {
                state.draft.groups.push(GroupDraft::new(id));
            }
        }
    });
    card(ui, |ui| {
        heading(ui, "Main control group", "mainGroup", &mut state.help);
        egui::ComboBox::from_id_salt("main_group")
            .selected_text(
                state
                    .draft
                    .main_group
                    .map_or_else(|| "None".into(), |id| format!("Draft group {id}")),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.draft.main_group, None, "None");
                for group in &state.draft.groups {
                    ui.selectable_value(
                        &mut state.draft.main_group,
                        Some(group.position),
                        &group.label,
                    );
                }
            });
        supply::actor(
            ui,
            "Who may replace the main group",
            "mainChange",
            &mut state.draft.permissions.main_group_change,
            &state.draft.groups,
            state.draft.main_group,
            &mut state.help,
        );
        ui.label("Changing the main group also changes the members addressed by every Main group authority.");
    });
}
