use super::*;
use crate::ui::components::styled::styled_text_edit_multiline;

pub(super) fn history(ui: &mut Ui, state: &mut CreatorWorkspace) {
    card(ui, |ui| {
        heading(ui, "History", "history", &mut state.help);
        muted(
            ui,
            "Choose which token events are retained. These choices are independent from the powers that perform the actions. They configure the contract's event histories, not local privacy settings.",
        );
        hint(
            ui,
            "This does not hide the contract or make transactions private.",
        );
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
            "Keep transfers history",
            "Keep freezes and unfreezes history",
            "Keep mints history",
            "Keep burns history",
            "Keep direct-pricing changes history",
            "Keep direct purchases history",
        ]
        .into_iter()
        .enumerate()
        {
            checkbox(
                ui,
                caption,
                "history",
                &mut state.draft.history[index],
                &mut state.help,
            );
        }
        inline_notice(ui, MessageType::Info, |ui| {
            ui.label(format!("{count} of 6 histories are selected. History settings are part of the reviewed contract."));
        });
    });
}

pub(super) fn show(ui: &mut Ui, state: &mut CreatorWorkspace) {
    card(ui, |ui| {
        heading(ui, "Search & documents", "documents", &mut state.help);
        muted(
            ui,
            "Help people recognize the token, and optionally define related document types.",
        );
        text(
            ui,
            "Description (optional)",
            "description",
            &mut state.draft.description,
            &mut state.help,
        );
        hint(
            ui,
            "Explain the token's purpose in plain language. Use 3 to 100 UTF-8 bytes when provided.",
        );
        muted(
            ui,
            "Names marked searchable in Basics are included automatically. Add other keywords separated by commas.",
        );
        text(
            ui,
            "Search keywords",
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
        hint(
            ui,
            "Keywords affect registration cost. The actual contract estimate is calculated during preparation.",
        );
    });
    card(ui, |ui| {
        subheading(
            ui,
            "Document types (optional)",
            "documents",
            &mut state.help,
        );
        muted(
            ui,
            "Paste a JSON object mapping document type names to schemas. This is not a full-contract import. Clearing the field removes every document type from this draft.",
        );
        let caption = ui.label("Document types · JSON");
        ui.add(
            styled_text_edit_multiline(&mut state.draft.documents)
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
        hint(
            ui,
            "Only local fragment references are accepted. External HTTP and file references are never loaded.",
        );
    });
}
