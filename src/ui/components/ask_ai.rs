//! Reusable prompt preview and explicit consent before handing off to an assistant.
use super::component_trait::{Component, ComponentResponse};
use super::modal_chrome::{ModalChromeConfig, modal_chrome};
use crate::model::error_report::AiAssistant;
use crate::ui::theme::{ComponentStyles, Spacing};
use egui::{InnerResponse, Ui};

/// The caller handles the confirmed assistant; this component never opens a URL.
#[derive(Clone, Default)]
pub struct AskAiResponse {
    /// Assistant approved for this frame only.
    pub send: Option<AiAssistant>,
    /// The caller may discard the dialog.
    pub closed: bool,
}
impl ComponentResponse for AskAiResponse {
    type DomainType = AiAssistant;
    fn has_changed(&self) -> bool {
        self.send.is_some()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn changed_value(&self) -> &Option<AiAssistant> {
        &self.send
    }
    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// Own this dialog until its response reports closure.
#[derive(Clone)]
pub struct AskAi {
    prompt: String,
    assistant: AiAssistant,
    confirming: bool,
    closed: bool,
}
impl AskAi {
    /// Preview caller-supplied text. Callers are responsible for redaction.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            assistant: AiAssistant::ChatGpt,
            confirming: false,
            closed: false,
        }
    }
    /// The exact text displayed for consent.
    pub fn prompt(&self) -> &str {
        &self.prompt
    }
    /// Choose the destination before displaying the prompt.
    pub fn with_assistant(mut self, assistant: AiAssistant) -> Self {
        self.assistant = assistant;
        self
    }
}
impl Component for AskAi {
    type DomainType = AiAssistant;
    type Response = AskAiResponse;
    fn current_value(&self) -> Option<AiAssistant> {
        None
    }
    fn show(&mut self, ui: &mut Ui) -> InnerResponse<AskAiResponse> {
        let mut result = AskAiResponse {
            closed: self.closed,
            ..Default::default()
        };
        if self.closed {
            return InnerResponse::new(
                result,
                ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
            );
        }
        let chrome = modal_chrome(
            ui.ctx(),
            ModalChromeConfig {
                title: "Ask an AI assistant".into(),
                overlay_id: ui.scope_id().with("ask_ai"),
                overlay_order: egui::Order::Middle,
                window_order: egui::Order::Foreground,
                resizable: true,
                show_close_button: true,
                blocks_input: true,
                inner_margin: Spacing::MD_I8,
            },
            |ui| {
                let screen = ui.ctx().content_rect();
                // Reserve the frame, title bar and action row outside the scrolling body.
                ui.set_max_width(560.0_f32.min((screen.width() - 64.0).max(0.0)));
                let body_height = (screen.height() - 160.0).max(0.0);
                let dark = ui.visuals().dark_mode;
                let warning = self.confirming.then(|| {
                    ui.painter().layout(
                        format!("This sends the question and its technical details to {name} ({operator}) as soon as it opens in your browser. Inspect the question for private or sensitive information before confirming. Do not share recovery phrases, keys or passwords.", name = self.assistant.name(), operator = self.assistant.operator()),
                        egui::TextStyle::Body.resolve(ui.style()),
                        ui.visuals().text_color(),
                        ui.available_width(),
                    )
                });
                let warning_height = warning
                    .as_ref()
                    .map_or(0.0, |text| text.size().y + ui.spacing().item_spacing.y);
                egui::ScrollArea::vertical()
                    .id_salt("ask_ai_body")
                    .max_height((body_height - warning_height).max(0.0))
                    .show(ui, |ui| {
                        ui.label(format!(
                            "Read the question before sending it to {name}.",
                            name = self.assistant.name()
                        ));
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(&self.prompt).selectable(true).wrap());
                            });
                    });
                if let Some(warning) = warning {
                    ui.label(warning);
                }
                if self.confirming {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ComponentStyles::add_primary_button(ui, "Confirm").clicked() {
                            result.send = Some(self.assistant);
                            result.closed = true;
                        }
                        if ComponentStyles::add_secondary_button(ui, "Cancel", dark).clicked() {
                            self.confirming = false;
                        }
                    });
                } else if ComponentStyles::add_primary_button(ui, "Send").clicked() {
                    self.confirming = true;
                }
            },
        );
        if chrome.closed_via_x || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            result.send = None;
            result.closed = true;
        }
        self.closed = result.closed;
        InnerResponse::new(
            result,
            chrome
                .window_response
                .unwrap_or_else(|| ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover())),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn ask_ai_escape_closes_without_sending_in_both_steps() {
        for confirming in [false, true] {
            let closed = Rc::new(Cell::new(false));
            let observed = closed.clone();
            let mut dialog =
                AskAi::new("Explain this help topic.").with_assistant(AiAssistant::ChatGpt);
            let mut harness = Harness::builder()
                .with_size(egui::vec2(900.0, 700.0))
                .build_ui(move |ui| {
                    let response = dialog.show(ui).inner;
                    assert!(response.send.is_none());
                    observed.set(response.closed);
                });
            harness.run();
            if confirming {
                harness.get_by_label("Send").click();
                harness.run();
            }
            harness.key_press(egui::Key::Escape);
            harness.run();
            assert!(closed.get());
        }
    }
}
