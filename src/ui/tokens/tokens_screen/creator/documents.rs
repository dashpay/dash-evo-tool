use super::*;

pub(super) fn history(ui: &mut Ui, state: &mut CreatorWorkspace) {
    card(ui, |ui| {
        heading(ui, "Token history", "history", &mut state.help);
        ui.label("Choose which token actions keep their detailed Platform history. This does not hide the contract or make transactions private.");
        let count = state
            .draft
            .history
            .iter()
            .filter(|enabled| **enabled)
            .count();
        let mut all = count == 6;
        if ui
            .add(
                egui::Checkbox::new(&mut all, "Keep all six histories")
                    .indeterminate(count > 0 && count < 6),
            )
            .changed()
        {
            state.draft.history.fill(all);
        }
        for (index, caption) in [
            "Transfers",
            "Freezing and unfreezing",
            "Minting",
            "Burning",
            "Direct-price changes",
            "Direct purchases",
        ]
        .into_iter()
        .enumerate()
        {
            ui.checkbox(&mut state.draft.history[index], caption);
        }
        ui.small(format!("{count} of 6 histories are selected. History settings are part of the reviewed contract."));
    });
}

pub(super) fn show(ui: &mut Ui, state: &mut CreatorWorkspace) {
    card(ui, |ui| {
        heading(
            ui,
            "Make your token discoverable",
            "keywords",
            &mut state.help,
        );
        ui.label("Names marked searchable in Basics are included automatically. Add other keywords separated by commas.");
        text(
            ui,
            "Additional keywords",
            "keywords",
            &mut state.draft.keywords,
            &mut state.help,
        );
        match state.draft.parse_keywords() {
            Ok(keywords) => {
                ui.label(format!("Search keywords: {}", keywords.join(", ")));
            }
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error.to_string());
            }
        }
        ui.small("Keywords affect registration cost. The actual contract estimate is calculated during preparation.");
    });
    card(ui, |ui| {
        heading(
            ui,
            "Document types (optional)",
            "documents",
            &mut state.help,
        );
        ui.label("Paste a JSON object mapping document type names to schemas. This is not a full-contract import. Clearing the field removes every document type from this draft.");
        let caption = ui.label("Document schema JSON");
        ui.add(
            egui::TextEdit::multiline(&mut state.draft.documents)
                .code_editor()
                .desired_rows(15)
                .desired_width(f32::INFINITY),
        )
        .labelled_by(caption.id);
        match state.draft.parse_documents() {
            Ok(Some(schemas)) => {
                ui.label(format!("JSON object parsed: {} document types. Document schema and token rule checks run during preparation.",schemas.len()));
            }
            Ok(None) => {
                ui.label("No document types are included.");
            }
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error.to_string());
            }
        }
        if ui.button("Clear document types").clicked() {
            state.draft.documents.clear();
        }
        ui.small("Only local fragment references are accepted. External HTTP and file references are never loaded.");
    });
}
