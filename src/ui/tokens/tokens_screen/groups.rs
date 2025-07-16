use crate::ui::components::styled::ClickableCollapsingHeader;
use crate::ui::tokens::tokens_screen::TokensScreen;
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::data_contract::group::v0::GroupV0;
use dash_sdk::dpp::data_contract::group::{Group, GroupMemberPower, GroupRequiredPower};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::epaint::Color32;
use egui::ComboBox;
use std::collections::BTreeMap;

#[derive(Default, Clone)]
pub struct GroupMemberUI {
    /// The base58 identity for this member
    pub identity_str: String,
    /// The power (u32) as a string for user input
    pub power_str: String,
}

#[derive(Default, Clone)]
pub struct GroupConfigUI {
    /// Required power for the group (u32), user enters as string
    pub required_power_str: String,
    /// The members for this group
    pub members: Vec<GroupMemberUI>,
}

impl GroupConfigUI {
    /// Try converting this UI struct into a real `Group` (specifically `Group::V0`).
    /// We also return the `u16` key that this group should be inserted under in the contract’s `groups` map.
    fn parse_into_group(&self, pos: GroupContractPosition) -> Result<(u16, Group), String> {
        // 1) Parse required power
        let required_power = self.required_power_str.parse::<u32>().map_err(|_| {
            format!(
                "Invalid required power: '{}'. Must be an unsigned integer.",
                self.required_power_str
            )
        })? as GroupRequiredPower;

        // 2) Build a BTreeMap<Identifier, u32> for members
        let mut members_map = BTreeMap::new();
        for (i, member) in self.members.iter().enumerate() {
            // A) Parse member identity from base58
            let id =
                Identifier::from_string(&member.identity_str, Encoding::Base58).map_err(|_| {
                    format!(
                        "Member #{}: invalid base58 identity '{}'",
                        i + 1,
                        member.identity_str
                    )
                })?;

            // B) Parse power
            let power =
                member.power_str.parse::<u32>().map_err(|_| {
                    format!("Member #{}: invalid power '{}'", i + 1, member.power_str)
                })? as GroupMemberPower;

            // Insert into the map
            members_map.insert(id, power);
        }

        // 3) Construct Group::V0
        let group_v0 = GroupV0 {
            members: members_map,
            required_power,
        };

        // 5) Return as (group_position, Group::V0 wrapped in Group::V0())
        Ok((pos, Group::V0(group_v0)))
    }
}

impl TokensScreen {
    /// Attempt to parse all group UI data into a BTreeMap<u16, Group>.
    /// Returns an error if any row fails or duplicates a position, etc.
    pub fn parse_groups(&self) -> Result<BTreeMap<u16, Group>, String> {
        let mut map = BTreeMap::new();
        for (i, g) in self.groups_ui.iter().enumerate() {
            let (pos, group) = g
                .parse_into_group(i as GroupContractPosition)
                .map_err(|e| format!("Error in Group #{}: {e}", i + 1))?;

            // Check for duplicates
            if map.contains_key(&pos) {
                return Err(format!(
                    "Duplicate group position {pos} in Group #{}",
                    i + 1
                ));
            }

            map.insert(pos, group);
        }
        Ok(map)
    }

    pub fn render_groups(&mut self, ui: &mut egui::Ui) {
        ui.add_space(5.0);

        ClickableCollapsingHeader::new("Groups")
            .id_salt("token_creator_groups")
            .open(if self.should_reset_collapsing_states {
                Some(false)
            } else {
                None
            })
            .show(ui, |ui| {
            ui.add_space(3.0);
            ui.label("Define one or more groups for multi-party control of the contract.");
            ui.add_space(2.0);

            // Add main group selection input
            ui.horizontal(|ui| {
                ui.label("Main Control Group Position:");
                ui.text_edit_singleline(&mut self.main_control_group_input);
            });

            ui.add_space(2.0);

            let mut group_to_remove = None;

            let last_group_position = self.groups_ui.len().saturating_sub(1);

            for (group_position, group_ui) in self.groups_ui.iter_mut().enumerate() {
                ClickableCollapsingHeader::new(format!("Group {}", group_position))
                    .id_salt(format!("group_header_{}", group_position))
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(3.0);

                        ui.horizontal(|ui| {
                            ui.label("Required Power:");
                            ui.text_edit_singleline(&mut group_ui.required_power_str);
                        });

                        ui.label("Members:");
                        ui.add_space(3.0);

                        let mut j = 0;
                        while j < group_ui.members.len() {
                            let mut member = group_ui.members[j].clone();

                            ui.horizontal(|ui| {
                                ui.label(format!("Member {}:", j + 1));

                                ComboBox::from_id_salt(format!("member_identity_selector_{}", j))
                                    .width(200.0)
                                    .selected_text(
                                        self.identities
                                            .get(&Identifier::from_string(&member.identity_str, Encoding::Base58).unwrap_or_default())
                                            .map(|q| q.display_string())
                                            .unwrap_or_else(|| member.identity_str.clone()),
                                    )
                                    .show_ui(ui, |ui| {
                                        for (identifier, qualified_identity) in self.identities.iter() {
                                            let id_str = identifier.to_string(Encoding::Base58);

                                            // Prevent duplicates unless in developer mode
                                            if !self.app_context.is_developer_mode()
                                                && group_ui
                                                .members
                                                .iter()
                                                .enumerate().any(|(i, m)| i != j && m.identity_str == id_str)
                                            {
                                                continue;
                                            }

                                            if ui
                                                .selectable_label(false, qualified_identity.display_string())
                                                .clicked()
                                            {
                                                member.identity_str = id_str.clone();
                                            }
                                        }
                                    });

                                ui.text_edit_singleline(&mut member.identity_str);

                                ui.label("Power (u32):");
                                ui.text_edit_singleline(&mut member.power_str);

                                // Show red warning if someone else already used this identity
                                if self.app_context.is_developer_mode()
                                    && !group_ui.members[j].identity_str.is_empty()
                                    && group_ui.members.iter().enumerate().any(|(i, m)| {
                                    i > j && m.identity_str == group_ui.members[j].identity_str
                                })
                                {
                                    ui.colored_label(
                                        Color32::RED,
                                        "This member is set later as part of this group (this entry will be ignored)",
                                    );
                                }


                                if ui.button("Remove Member").clicked() {
                                    group_ui.members.remove(j);// return so we skip the assignment at the end
                                } else {
                                    // Only assign back if we didn’t remove
                                    group_ui.members[j] = member;
                                }
                            });

                            j += 1;
                        }

                        ui.add_space(3.0);
                        if ui.button("Add Member").clicked() {
                            group_ui.members.push(GroupMemberUI {
                                identity_str: "".to_owned(),
                                power_str: "1".to_owned(),
                            });
                        }

                        ui.add_space(3.0);

                        if group_position == last_group_position {
                            // A remove button for the entire group, only for last group
                            if ui.button("Remove This Group").clicked() {
                                group_to_remove = Some(group_position);
                            }
                        }
                    });
            }

            if let Some(group_to_remove) = group_to_remove{
                self.groups_ui.remove(group_to_remove);
            }

            ui.add_space(5.0);
            if ui.button("Add New Group").clicked() {
                self.groups_ui.push(GroupConfigUI {
                    required_power_str: "2".to_owned(),
                    members: vec![GroupMemberUI {
                        identity_str: self.selected_identity.as_ref().map(|q| q.identity.id().to_string(Encoding::Base58)).unwrap_or_default(),
                        power_str: "1".to_string(),
                    }, GroupMemberUI {
                        identity_str: "".to_string(),
                        power_str: "1".to_string(),
                    }],
                });
            }
        });
    }
}
