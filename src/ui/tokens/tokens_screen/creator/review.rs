use super::*;
use crate::backend_task::error::TaskError;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::model::token_creator::{
    TokenDraft,
    registration::{RegistrationStatus, review_json},
};
use crate::ui::{MessageType, components::MessageBanner};
use dash_sdk::dpp::{
    data_contract::accessors::{v0::DataContractV0Getters, v1::DataContractV1Getters},
    identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0,
    platform_value::string_encoding::Encoding,
    version::PlatformVersion,
};

impl TokensScreen {
    pub(super) fn render_workspace_review(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        if !self.creator_workspace.recovery_loaded {
            match self.app_context.token_registration_record() {
                Ok(record) => {
                    self.creator_workspace.recovery = record.filter(|record| {
                        record.blocks_registration()
                            || Some(record.attempt_id) != self.creator_workspace.dismissed_receipt
                    });
                    self.creator_workspace.recovery_loaded = true;
                }
                Err(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                    return action;
                }
            }
        }
        let pending = self.creator_workspace.pending.is_some();
        if pending {
            ui.spinner();
            ui.label(
                "The token creation request is running. Wait for its result before continuing.",
            );
        }
        if let Some(record) = self.creator_workspace.recovery.clone() {
            card(ui, |ui| {
                ui.heading(match record.status {
                    RegistrationStatus::Confirmed => "Token created",
                    RegistrationStatus::Conflict => "Creation conflict",
                    _ => "Creation not yet confirmed",
                });
                ui.label(format!("Network: {}", record.network));
                copy_id(ui, "Contract ID", record.contract_id);
                if record.status == RegistrationStatus::Confirmed {
                    copy_id(ui, "Token ID", record.token_id);
                }
                copy_id(ui, "Owner", record.owner);
                if record.status == RegistrationStatus::Confirmed {
                    ui.label("These identifiers come from the accepted contract. A draft or a broadcast alone is not proof of creation.");
                    if let Some(outcome) = &self.creator_workspace.outcome {
                        if !outcome.local_cache_saved || !outcome.journal_saved {
                            ui.label("The token was created, but a local save did not finish. Check status to save the receipt and token again; this does not create or pay for another token.");
                        }
                    }
                    if ui.button("Open contract").clicked() {
                        self.tokens_subscreen = super::super::TokensSubscreen::SearchTokens;
                        self.selected_contract_id = Some(record.contract_id);
                        self.contract_details_loading = true;
                        self.selected_contract_description = None;
                        self.selected_token_infos.clear();
                        action=AppAction::BackendTask(BackendTask::ContractTask(Box::new(crate::backend_task::contract::ContractTask::FetchContractsWithDescriptions(vec![record.contract_id]))));
                    }
                    if ui
                        .add_enabled(!pending, egui::Button::new("Create another token"))
                        .clicked()
                    {
                        self.creator_workspace = CreatorWorkspace {
                            recovery_loaded: true,
                            dismissed_receipt: Some(record.attempt_id),
                            ..Default::default()
                        };
                    }
                } else {
                    ui.label(if record.status==RegistrationStatus::Conflict {"A contract with this ID contains different settings. This app will not treat it as your reviewed token or make another paid attempt."}else{"The result may still arrive. Check status without paying again. Closing this screen or restarting the app preserves this record and the block on another creation."});
                }
                if ui
                    .add_enabled(!pending, egui::Button::new("Check status · no payment"))
                    .clicked()
                {
                    action = self.begin_workspace_task(TokenTask::CheckTokenCreation);
                }
                if let Ok(contract) = record.contract() {
                    use dash_sdk::dpp::data_contract::associated_token::{
                        token_configuration::accessors::v0::TokenConfigurationV0Getters,
                        token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters,
                        token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters,
                    };
                    if let Ok(token) = contract.expected_token_configuration(0) {
                        ui.label(format!(
                            "Initial allocation: {} tokens ({} base units)",
                            format_amount(token.base_supply(), token.conventions().decimals()),
                            token.base_supply()
                        ));
                        copy_id(
                            ui,
                            "Initial recipient",
                            token
                                .distribution_rules()
                                .new_tokens_destination_identity()
                                .copied()
                                .unwrap_or(record.owner),
                        );
                    }
                    if let Ok(version) = PlatformVersion::get(record.protocol_version) {
                        json_tools(ui, &contract, version);
                    }
                }
            });
            return action;
        }
        let mut unlocked = false;
        card(ui, |ui| {
            heading(
                ui,
                "Review your token",
                "review",
                &mut self.creator_workspace.help,
            );
            ui.label("Preparation checks the current owner, key and contract without spending credits. Creation requires a separate confirmation.");
            ui.add_enabled_ui(!pending, |ui| {
                action |= crate::ui::helpers::add_identity_key_chooser(
                    ui,
                    &self.app_context,
                    self.identities.values(),
                    &mut self.selected_identity,
                    &mut self.selected_key,
                    crate::ui::helpers::TransactionType::RegisterContract,
                );
            });
            if self.identities.is_empty() && ui.button("Load identity").clicked() {
                action = AppAction::AddScreen(
                    crate::ui::ScreenType::AddExistingIdentity.create_screen(&self.app_context),
                );
            }
            self.update_selected_wallet();
            unlocked = self.ensure_wallet_unlocked(ui);
            let binding = self
                .selected_identity
                .as_ref()
                .zip(self.selected_key.as_ref());
            if self
                .creator_workspace
                .prepared
                .as_ref()
                .is_some_and(|prepared| {
                    binding.is_none_or(|(owner, key)| {
                        owner.identity.id() != prepared.contract.owner_id() || *key != prepared.key
                    }) || prepared.network != self.app_context.network
                        || prepared.protocol_version
                            != self.app_context.platform_version().protocol_version
                })
            {
                self.creator_workspace.prepared = None;
                self.creator_workspace.acknowledged = false;
            }
            if let Some((owner, key)) = binding {
                ui.label(format!(
                    "Selected owner: {}",
                    owner.identity.id().to_string(Encoding::Base58)
                ));
                ui.label(format!("Signing key: {}", key.id()));
            } else {
                ui.label(
                    "Choose an identity and an available authentication key to prepare the review.",
                );
            }
            if ui
                .add_enabled(
                    !pending && unlocked && binding.is_some(),
                    egui::Button::new("Prepare immutable review"),
                )
                .clicked()
            {
                if let Some((owner, key)) = binding {
                    let task = TokenTask::PrepareTokenCreation {
                        draft: Box::new(self.creator_workspace.draft.clone()),
                        owner: owner.identity.id(),
                        signing_key: Box::new(key.clone()),
                    };
                    self.creator_workspace.pending_draft =
                        Some(self.creator_workspace.draft.clone());
                    action = self.begin_workspace_task(task);
                }
            }
        });
        if let (Some(prepared), Some(draft)) = (
            self.creator_workspace.prepared.clone(),
            self.creator_workspace.reviewed_draft.clone(),
        ) {
            card(ui, |ui| {
                ui.heading("Immutable review");
                ui.label(format!(
                    "Network: {} · protocol version {}",
                    prepared.network, prepared.protocol_version
                ));
                copy_id(ui, "Owner", prepared.contract.owner_id());
                copy_id(ui, "Contract ID to be signed", prepared.contract.id());
                if let Some(id) = prepared.contract.token_id(0) {
                    copy_id(ui, "Token ID to be created", id);
                }
                ui.label("These IDs are prepared, not registered. If another operation changes the creation details, a new review is required.");
                ui.label(format!(
                    "Registration charge: {} credits",
                    prepared.registration_cost
                ));
                ui.label(format!(
                    "Owner balance checked during preparation: {} credits",
                    prepared.owner_balance
                ));
                ui.label("Processing and storage charges are additional. This is not a settled total fee; the app does not yet receive an exact final fee for this operation.");
                if prepared.owner_balance < prepared.registration_cost {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Add credits to this identity, then prepare the review again.",
                    );
                    if let Some(identity) = self.selected_identity.clone() {
                        if ui.button("Add identity credits").clicked() {
                            action = AppAction::AddScreen(
                                crate::ui::ScreenType::TopUpIdentity(identity)
                                    .create_screen(&self.app_context),
                            );
                        }
                    }
                }
                human_review(ui, &draft);
                if let Ok(version) = PlatformVersion::get(prepared.protocol_version) {
                    json_tools(ui, &prepared.contract, version);
                }
                checkbox(
                    ui,
                    "I reviewed the amounts, recipients, permissions and irreversible settings.",
                    "review",
                    &mut self.creator_workspace.acknowledged,
                    &mut self.creator_workspace.help,
                );
                if ui
                    .add_enabled(
                        !pending
                            && self.creator_workspace.acknowledged
                            && unlocked
                            && prepared.owner_balance >= prepared.registration_cost,
                        egui::Button::new("Create token · pay registration charge"),
                    )
                    .clicked()
                {
                    action = self.begin_workspace_task(TokenTask::SubmitTokenCreation {
                        prepared: Box::new(prepared),
                    });
                }
            });
        }
        action
    }
    fn begin_workspace_task(&mut self, task: TokenTask) -> AppAction {
        if self.creator_workspace.pending.is_some() {
            return AppAction::None;
        }
        let task = BackendTask::TokenTask(Box::new(task));
        let context = BackendTaskContext::for_dispatch_on(&task, self.app_context.network);
        self.creator_workspace.pending = Some(context.clone());
        self.creator_workspace.error = None;
        self.creator_workspace.notice = None;
        self.creator_workspace.error_section = None;
        AppAction::BackendTaskWithContext { task, context }
    }
    pub(in crate::ui::tokens::tokens_screen) fn workspace_result(
        &mut self,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) {
        if self.creator_workspace.pending.as_ref() != Some(context) {
            return;
        }
        match result {
            BackendTaskSuccessResult::TokenCreationPrepared(prepared) => {
                let snapshot = self.creator_workspace.pending_draft.take();
                if snapshot.as_ref() == Some(&self.creator_workspace.draft) {
                    self.creator_workspace.reviewed_draft = snapshot;
                    self.creator_workspace.prepared = Some(*prepared.clone());
                } else {
                    self.creator_workspace.prepared = None;
                    self.creator_workspace.notice=Some("The draft changed during preparation. Prepare it again to review the current settings.".into());
                }
                self.creator_workspace.acknowledged = false;
            }
            BackendTaskSuccessResult::TokenCreationOutcome(outcome) => {
                self.creator_workspace.recovery = Some(outcome.record.clone());
                self.creator_workspace.outcome = Some(*outcome.clone());
                self.creator_workspace.prepared = None;
            }
            _ => return,
        }
        self.creator_workspace.pending = None;
    }
    pub(in crate::ui::tokens::tokens_screen) fn workspace_error(
        &mut self,
        context: &BackendTaskContext,
        error: &TaskError,
    ) {
        if self.creator_workspace.pending.as_ref() != Some(context) {
            return;
        }
        self.creator_workspace.pending = None;
        self.creator_workspace.prepared = None;
        self.creator_workspace.notice = Some(error.to_string());
        if let TaskError::InvalidTokenDraft { source } = error {
            self.creator_workspace.error_section = Some(source.section);
        }
        self.creator_workspace.recovery_loaded = false;
    }
}

fn copy_id(ui: &mut Ui, caption: &str, id: dash_sdk::platform::Identifier) {
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{caption}: {}", id.to_string(Encoding::Base58)));
        if ui.button(format!("Copy {caption}")).clicked() {
            ui.ctx().copy_text(id.to_string(Encoding::Base58));
        }
    });
}
fn json_tools(ui: &mut Ui, contract: &dash_sdk::platform::DataContract, version: &PlatformVersion) {
    match review_json(contract, version) {
        Ok(json) => {
            ui.horizontal(|ui| {
                if ui.button("Copy contract JSON").clicked(){ui.ctx().copy_text(json.clone());}
                if ui.button("Save contract JSON").clicked() {
                    if let Some(path)=rfd::FileDialog::new().set_file_name("token-contract.json").add_filter("JSON",&["json"]).save_file() {
                        if let Err(error)=std::fs::write(path,&json) {MessageBanner::set_global(ui.ctx(),"The contract file could not be saved. Choose a writable folder and try again.",MessageType::Error).with_details(error);}
                    }
                }
            });
            egui::CollapsingHeader::new("Inspect complete contract JSON · amounts are base units")
                .show(ui, |ui| {
                    egui::ScrollArea::both().max_height(300.0).show(ui, |ui| {
                        ui.monospace(&json);
                    });
                });
        }
        Err(error) => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "The contract could not be shown. Prepare the review again.",
            );
            tracing::warn!(?error, "Cannot serialize reviewed token contract");
        }
    }
}
fn human_review(ui: &mut Ui, draft: &TokenDraft) {
    for section in Section::ALL
        .into_iter()
        .filter(|section| *section != Section::Review)
    {
        egui::CollapsingHeader::new(format!("Reviewed {}",section.label())).default_open(section==Section::Supply).show(ui,|ui| {
            match section {
                Section::Basics=>{for name in &draft.names {ui.label(format!("{}: {} / {} · searchable: {}",name.language,name.singular,name.plural,supply::yes(name.searchable)));}ui.label(format!("Description: {}",draft.description));ui.label(format!("Precision: {} · capitalization: {}",draft.decimals,supply::yes(draft.capitalize)));},
                Section::Supply=>{
                    for (name,value) in [("Initial supply",&draft.initial),("Maximum supply",&draft.maximum)] {
                        if name=="Maximum supply"&&draft.no_maximum {ui.label("Maximum supply: no configured limit");}else if let Ok(raw)=value.parse(draft.decimals){ui.label(format!("{name}: {} tokens ({raw} base units)",format_amount(raw,draft.decimals)));}
                    }
                    ui.label(format!("Initial and default mint recipient: {}",recipient_label(&draft.mint_recipient)));
                    ui.label(format!("Choose a recipient per mint: {} · ordinary transfers: {} · starts paused: {} · receive while frozen: {}",supply::yes(draft.permissions.allow_mint_recipient_choice),supply::yes(draft.transferable),supply::yes(draft.start_paused),supply::yes(draft.allow_receive_frozen)));
                    for kind in RuleKind::ALL {let rule=draft.permissions.get(kind);ui.separator();ui.strong(kind.label());ui.label(format!("Performer: {} · administrator: {}",rule.performer.label(),rule.administrator.label()));ui.label(format!("Allow disabling action: {} · allow removing administrator: {} · allow replacing administrator: {}",supply::yes(rule.allow_disable_action),supply::yes(rule.allow_remove_administrator),supply::yes(rule.allow_replace_administrator)));}
                },
                Section::Distribution=>{
                    ui.label(format!("Recurring: {}",supply::yes(draft.recurring.enabled)));
                    if draft.recurring.enabled {ui.label(format!("Interval: {} {:?} · time unit: {} · recipient: {}",draft.recurring.interval,draft.recurring.interval_kind,draft.recurring.time_unit.label(),recipient_label(&draft.recurring.recipient)));let function=&draft.recurring.functions[draft.recurring.function as usize];ui.strong(function.kind.label());for (parameter,value) in &function.values {ui.label(format!("{}: {value}",parameter.label()));}for (parameter,value) in &function.amounts {ui.label(format!("{}: {} {}",parameter.label(),value.text,if value.base_units {"base units"}else{"tokens"}));}for (index,value) in &function.steps {ui.label(format!("Period {index}: {} {}",value.text,if value.base_units {"base units"}else{"tokens"}));}}
                    ui.label(format!("Scheduled allocations: {}",supply::yes(draft.scheduled_enabled)));
                    if draft.scheduled_enabled {for row in &draft.scheduled {if let (Some(time),Ok(raw))=(row.timestamp,row.amount.parse(draft.decimals)){ui.label(format!("{} · {} · {} tokens ({raw} base units)",crate::model::datetime::local_date_time_from_unix_millis(time),recipient_label(&row.recipient),format_amount(raw,draft.decimals)));}}}
                    ui.label(format!("Once per identity: {}",supply::yes(draft.once_enabled)));if draft.once_enabled {ui.label(format!("Amount: {} {}",draft.once_amount.text,if draft.once_amount.base_units {"base units"}else{"tokens"}));}
                },
                Section::Groups=>{for (position,group) in draft.groups.iter().enumerate(){ui.strong(format!("{} · local group {} → contract position {position}",group.label,group.position));ui.label(format!("Required power: {}",group.threshold));for member in &group.members {ui.label(format!("{} · voting power {}",member.identity,member.weight));}}ui.label(format!("Main group: {:?} · replacement authority: {}",draft.main_group,draft.permissions.main_group_change.label()));},
                Section::History=>{for (label,on) in ["Transfers","Freezing","Minting","Burning","Direct pricing","Direct purchases"].into_iter().zip(draft.history){ui.label(format!("{label}: {}",supply::yes(on)));}},
                Section::Documents=>{ui.label(format!("Additional keywords: {}",draft.keywords));ui.label("Searchable names are also included as keywords. The complete document schema is in the contract JSON.");if let Ok(Some(documents))=draft.parse_documents(){for name in documents.keys(){ui.label(format!("Document type: {name}"));}}else{ui.label("No document types");}},
                Section::Review=>{},
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared(
        context: &crate::context::AppContext,
    ) -> crate::model::token_creator::registration::PreparedRegistration {
        let version = PlatformVersion::latest();
        let owner = dash_sdk::platform::Identifier::from([17; 32]);
        let draft = TokenDraft::example(false);
        let contract = crate::model::token_creator::contract::canonical_contract(
            owner,
            3,
            draft.parse(owner, context.network, version, 0).unwrap(),
            version,
        )
        .unwrap();
        crate::model::token_creator::registration::PreparedRegistration {
            contract,
            nonce: 3,
            key: dash_sdk::platform::IdentityPublicKey::default_versioned(version).unwrap(),
            network: context.network,
            protocol_version: version.protocol_version,
            alias: "Fixture".into(),
            registration_cost: 1,
            owner_balance: 2,
        }
    }
    #[test]
    fn token_registration_preparation_result_never_approves_a_draft_edited_while_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen =
            TokensScreen::new(&context, super::super::super::TokensSubscreen::TokenCreator);
        screen.creator_workspace.draft = TokenDraft::example(false);
        screen.creator_workspace.pending_draft = Some(screen.creator_workspace.draft.clone());
        let request = BackendTaskContext::for_dispatch_on(&BackendTask::None, context.network);
        screen.creator_workspace.pending = Some(request.clone());
        screen.creator_workspace.draft.names[0].singular = "Edited while waiting".into();
        screen.workspace_result(
            &request,
            &BackendTaskSuccessResult::TokenCreationPrepared(Box::new(prepared(&context))),
        );
        assert!(screen.creator_workspace.prepared.is_none());
        assert_eq!(
            screen.creator_workspace.draft.names[0].singular,
            "Edited while waiting"
        );
        assert!(screen.creator_workspace.pending.is_none());
    }
    #[test]
    fn token_registration_unrelated_error_does_not_finish_the_current_request() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen =
            TokensScreen::new(&context, super::super::super::TokensSubscreen::TokenCreator);
        let request = BackendTaskContext::for_dispatch_on(&BackendTask::None, context.network);
        let other = BackendTaskContext::for_dispatch_on(&BackendTask::None, context.network);
        screen.creator_workspace.pending = Some(request.clone());
        screen.workspace_error(&other, &TaskError::TokenCreatorChanged);
        assert_eq!(screen.creator_workspace.pending, Some(request.clone()));
        screen.workspace_error(&request, &TaskError::TokenCreatorChanged);
        assert!(screen.creator_workspace.pending.is_none());
    }
}
