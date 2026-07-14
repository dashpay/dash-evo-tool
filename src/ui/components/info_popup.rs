use crate::ui::components::modal_chrome::{ModalChromeConfig, modal_chrome};
use crate::ui::helpers::clicked_outside_window;
use crate::ui::theme::{ComponentStyles, DashColors};
use egui::{InnerResponse, Ui, WidgetText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

/// A simple info popup that displays information with a close button
/// Similar to ConfirmationDialog but for showing informational content only
/// Supports both plain text and markdown rendering
pub struct InfoPopup {
    title: WidgetText,
    message: String,
    close_text: WidgetText,
    is_open: bool,
    markdown: bool,
}

impl InfoPopup {
    /// Create a new info popup with the given title and message
    pub fn new(title: impl Into<WidgetText>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            close_text: "Close".into(),
            is_open: true,
            markdown: false,
        }
    }

    /// Set the text for the close button
    pub fn close_text(mut self, text: impl Into<WidgetText>) -> Self {
        self.close_text = text.into();
        self
    }

    /// Set whether the popup is open
    pub fn open(mut self, open: bool) -> Self {
        self.is_open = open;
        self
    }

    /// Enable markdown rendering for the message content
    pub fn markdown(mut self, enable: bool) -> Self {
        self.markdown = enable;
        self
    }

    /// Show the popup and return whether it was closed
    /// Returns true if the popup was closed (user clicked Close, X button, or Escape)
    pub fn show(&mut self, ui: &mut Ui) -> InnerResponse<bool> {
        if !self.is_open {
            return InnerResponse::new(
                false,
                ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
            );
        }

        let mut was_closed = false;
        let is_markdown = self.markdown;
        let message = self.message.clone();
        let close_text = self.close_text.clone();

        let chrome = modal_chrome(
            ui.ctx(),
            ModalChromeConfig {
                title: self.title.clone(),
                overlay_id: egui::Id::new("info_popup_overlay"),
                overlay_order: egui::Order::Background,
                window_order: egui::Order::Middle,
                resizable: is_markdown, // Allow resizing for markdown content
                inner_margin: 16,
            },
            |ui| {
                // Set minimum and maximum width for the popup
                ui.set_min_width(300.0);
                if is_markdown {
                    ui.set_max_width(600.0);
                } else {
                    ui.set_max_width(500.0);
                }

                let dark_mode = ui.style().visuals.dark_mode;

                // Message content
                ui.add_space(10.0);

                if is_markdown {
                    // Render markdown content with scroll area
                    egui::ScrollArea::vertical()
                        .max_height(400.0)
                        .show(ui, |ui| {
                            let mut cache = CommonMarkCache::default();
                            CommonMarkViewer::new().show(ui, &mut cache, &message);
                        });
                } else {
                    // Render plain text with tight spacing
                    ui.spacing_mut().item_spacing.y = 2.0;

                    // Split on double newlines (paragraphs) and render with controlled spacing
                    let paragraphs: Vec<&str> = message.split("\n\n").collect();
                    for (i, paragraph) in paragraphs.iter().enumerate() {
                        // Replace single newlines with spaces for proper wrapping within paragraphs
                        let text = paragraph.replace('\n', " ");
                        ui.label(
                            egui::RichText::new(text).color(DashColors::text_primary(dark_mode)),
                        );
                        if i < paragraphs.len() - 1 {
                            ui.add_space(4.0);
                        }
                    }
                }

                ui.add_space(20.0);

                // Close button
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ComponentStyles::add_primary_button(ui, close_text).clicked() {
                            was_closed = true;
                        }
                    });
                });
            },
        );

        if chrome.closed_via_x {
            was_closed = true;
        }

        // Handle Escape key press
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            was_closed = true;
        }

        // Handle Enter key press — no focus guard needed since InfoPopup has no input fields
        if !was_closed && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            was_closed = true;
        }

        // Handle click outside window
        if let Some(ref wr) = chrome.window_response
            && !was_closed
            && clicked_outside_window(ui.ctx(), wr.rect)
        {
            was_closed = true;
        }

        self.is_open = !was_closed;

        if let Some(window_response) = chrome.window_response {
            InnerResponse::new(was_closed, window_response)
        } else {
            InnerResponse::new(
                was_closed,
                ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
            )
        }
    }

    /// Check if the popup is currently open
    pub fn is_open(&self) -> bool {
        self.is_open
    }
}
