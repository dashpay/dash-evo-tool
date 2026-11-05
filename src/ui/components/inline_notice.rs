//! An inline notice: a message of one [`MessageType`] shown in place, next to
//! the content it is about, with optional action links ("Try again",
//! "Dismiss"). App-wide messages stay with [`MessageBanner`]; this is for a
//! notice a screen keeps on its own surface, such as a failed refresh above
//! the list it could not update.
//!
//! [`MessageBanner`]: crate::ui::components::message_banner::MessageBanner

use crate::ui::MessageType;
use crate::ui::theme::{DashColors, ResponseExt, Shape, Spacing};
use egui::{Frame, Margin, RichText, Ui};

/// An inline notice with its action links.
pub struct InlineNotice<'a> {
    kind: MessageType,
    text: &'a str,
    links: Vec<NoticeLink<'a>>,
}

struct NoticeLink<'a> {
    label: &'a str,
    /// Why the link cannot be used right now, when it cannot.
    disabled_reason: Option<&'a str>,
}

/// What the user did with the notice this frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InlineNoticeResponse {
    /// The index of the link that was clicked, in the order they were added.
    pub clicked: Option<usize>,
}

impl<'a> InlineNotice<'a> {
    /// A notice of `kind` saying `text`.
    pub fn new(kind: MessageType, text: &'a str) -> Self {
        Self {
            kind,
            text,
            links: Vec::new(),
        }
    }

    /// Add an action link after the text.
    pub fn with_link(mut self, label: &'a str) -> Self {
        self.links.push(NoticeLink {
            label,
            disabled_reason: None,
        });
        self
    }

    /// Add an action link that is disabled, with `reason` on hover, while
    /// `disabled_reason` is `Some`.
    pub fn with_link_unless(mut self, label: &'a str, disabled_reason: Option<&'a str>) -> Self {
        self.links.push(NoticeLink {
            label,
            disabled_reason,
        });
        self
    }

    /// Paint the notice across the available width.
    pub fn show(self, ui: &mut Ui) -> InlineNoticeResponse {
        let dark = ui.visuals().dark_mode;
        let mut response = InlineNoticeResponse::default();
        Frame::new()
            .fill(DashColors::message_background_color(self.kind, dark))
            .corner_radius(Shape::RADIUS_MD)
            .inner_margin(Margin::same(Spacing::SM_I8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(glyph(self.kind))
                            .color(DashColors::message_color(self.kind, dark)),
                    );
                    ui.label(self.text);
                    for (index, link) in self.links.iter().enumerate() {
                        let clicked = ui
                            .add_enabled(
                                link.disabled_reason.is_none(),
                                egui::Link::new(link.label),
                            )
                            .disabled_tooltip(link.disabled_reason.unwrap_or_default())
                            .clicked();
                        if clicked {
                            response.clicked = Some(index);
                        }
                    }
                });
            });
        response
    }
}

/// The leading mark for each kind; text, so it reads without colour.
fn glyph(kind: MessageType) -> &'static str {
    match kind {
        MessageType::Error | MessageType::Warning => "▲",
        MessageType::Success => "✓",
        MessageType::Info => "ℹ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    #[test]
    fn links_report_their_index_and_a_disabled_link_does_not_click() {
        let mut harness = Harness::builder().build_ui_state(
            |ui, last: &mut InlineNoticeResponse| {
                let response = InlineNotice::new(MessageType::Error, "It failed.")
                    .with_link_unless("Try again", Some("Not yet."))
                    .with_link("Dismiss")
                    .show(ui);
                if response.clicked.is_some() {
                    *last = response;
                }
            },
            InlineNoticeResponse::default(),
        );
        harness.run();
        assert!(harness.query_by_label("It failed.").is_some());
        assert!(
            harness
                .get_by_label("Try again")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("Dismiss").click();
        harness.run_steps(2);
        assert_eq!(harness.state().clicked, Some(1));
    }
}
