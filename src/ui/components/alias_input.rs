//! Text input for wallet and imported-key names (aliases).
//!
//! The character counter shows the length the model validator measures
//! ([`alias_char_count`]: control/format characters removed, surrounding
//! whitespace trimmed), so the on-screen count never disagrees with what is
//! saved. Typed or pasted text beyond [`MAX_WALLET_ALIAS_CHARS`] is truncated
//! at a grapheme-cluster boundary, so an emoji sequence or a letter with
//! combining marks is never split. Cleaning, default naming, and uniqueness
//! are resolved by the backend when the alias is saved — this widget only
//! collects the raw text.
//!
//! # Usage
//!
//! ```rust,ignore
//! let input = self.alias_input.get_or_insert_with(|| {
//!     AliasInput::new().with_label("Wallet name")
//! });
//! input.show(ui);
//! let raw_alias = input.text().to_owned();
//! ```

use egui::{InnerResponse, Response, RichText, TextEdit, Ui};
use unicode_segmentation::UnicodeSegmentation;

use crate::model::wallet::alias::{MAX_WALLET_ALIAS_CHARS, alias_char_count};
use crate::ui::components::{Component, ComponentResponse};
use crate::ui::theme::DashColors;

const DEFAULT_HELPER_TEXT: &str = "Leave the name blank to use a default name.";

/// Response from [`AliasInput::show`].
#[derive(Clone)]
pub struct AliasInputResponse {
    /// The underlying `TextEdit` response.
    pub response: Response,
    /// Whether the text changed this frame.
    pub changed: bool,
    changed_value: Option<String>,
}

impl ComponentResponse for AliasInputResponse {
    type DomainType = String;

    fn has_changed(&self) -> bool {
        self.changed
    }

    /// Always valid: over-long input is truncated while typing, and a blank
    /// alias is a request for the default name.
    fn is_valid(&self) -> bool {
        true
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.changed_value
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// Single-line alias input with a cleaned-length counter and helper text.
pub struct AliasInput {
    text: String,
    label: Option<String>,
    hint_text: String,
    helper_text: String,
    desired_width: Option<f32>,
}

impl Default for AliasInput {
    fn default() -> Self {
        Self::new()
    }
}

impl AliasInput {
    /// Create an empty alias input with the default helper text.
    pub fn new() -> Self {
        Self {
            text: String::new(),
            label: None,
            hint_text: String::new(),
            helper_text: DEFAULT_HELPER_TEXT.to_owned(),
            desired_width: None,
        }
    }

    /// Render `label` above the field and use it as the field's accessible name.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Placeholder shown while the field is empty.
    pub fn with_hint_text(mut self, hint_text: impl Into<String>) -> Self {
        self.hint_text = hint_text.into();
        self
    }

    /// Replace the helper sentence shown under the counter.
    pub fn with_helper_text(mut self, helper_text: impl Into<String>) -> Self {
        self.helper_text = helper_text.into();
        self
    }

    /// Preferred width of the text field.
    pub fn with_desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    /// The raw text as entered (after live truncation).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replace the text, truncating it the same way typed input is.
    pub fn set_text(&mut self, text: impl Into<String>) {
        let mut text = text.into();
        text.truncate(alias_limit_boundary(&text));
        self.text = text;
    }

    /// Empty the field.
    pub fn clear(&mut self) {
        self.text.clear();
    }
}

impl Component for AliasInput {
    type DomainType = String;
    type Response = AliasInputResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        ui.vertical(|ui| {
            let label_id = self.label.as_deref().map(|label| ui.label(label).id);
            let mut text_edit =
                TextEdit::singleline(&mut self.text).hint_text(self.hint_text.clone());
            if let Some(width) = self.desired_width {
                text_edit = text_edit.desired_width(width);
            }
            let mut response = ui.add(text_edit);
            if let Some(label_id) = label_id {
                response = response.labelled_by(label_id);
            }

            let changed = response.changed();
            if changed {
                let end = alias_limit_boundary(&self.text);
                self.text.truncate(end);
            }

            let secondary = DashColors::text_secondary(ui.style().visuals.dark_mode);
            ui.label(
                RichText::new(format!(
                    "{count} of {max} characters used.",
                    count = alias_char_count(&self.text),
                    max = MAX_WALLET_ALIAS_CHARS
                ))
                .small()
                .color(secondary),
            );
            ui.label(
                RichText::new(self.helper_text.as_str())
                    .small()
                    .color(secondary),
            );

            AliasInputResponse {
                changed_value: changed.then(|| self.text.clone()),
                response,
                changed,
            }
        })
    }

    fn current_value(&self) -> Option<Self::DomainType> {
        Some(self.text.clone())
    }
}

/// Byte offset of the longest grapheme-cluster prefix of `raw` whose cleaned
/// length fits [`MAX_WALLET_ALIAS_CHARS`]. Returns `raw.len()` when it all fits.
fn alias_limit_boundary(raw: &str) -> usize {
    if alias_char_count(raw) <= MAX_WALLET_ALIAS_CHARS {
        return raw.len();
    }
    let boundaries: Vec<usize> = raw
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(raw.len()))
        .collect();
    // The cleaned length never shrinks as the prefix grows (removed characters
    // add nothing, trailing whitespace is trimmed until followed by text), so
    // the fitting boundaries form a prefix of `boundaries`. The empty prefix
    // at offset 0 always fits, so at least one boundary qualifies.
    let fitting =
        boundaries.partition_point(|&end| alias_char_count(&raw[..end]) <= MAX_WALLET_ALIAS_CHARS);
    boundaries[fitting.saturating_sub(1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fitted(raw: &str) -> &str {
        &raw[..alias_limit_boundary(raw)]
    }

    #[test]
    fn text_within_the_limit_is_untouched() {
        let raw = "w".repeat(MAX_WALLET_ALIAS_CHARS);
        assert_eq!(fitted(&raw), raw);
    }

    #[test]
    fn ascii_overflow_is_cut_at_the_limit() {
        let raw = "w".repeat(70);
        assert_eq!(fitted(&raw), "w".repeat(MAX_WALLET_ALIAS_CHARS));
    }

    #[test]
    fn zwj_emoji_sequence_is_never_split() {
        let raw = format!(
            "{}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            "a".repeat(63)
        );
        assert_eq!(fitted(&raw), "a".repeat(63));
    }

    #[test]
    fn combining_mark_cluster_is_never_split() {
        let raw = format!("{}e\u{0301}", "a".repeat(63));
        assert_eq!(fitted(&raw), "a".repeat(63));
    }

    #[test]
    fn invisible_characters_do_not_consume_the_budget() {
        let raw = format!("{}{}", "\u{200B}".repeat(10), "a".repeat(64));
        assert_eq!(fitted(&raw), raw);
    }

    #[test]
    fn set_text_truncates_like_typing() {
        let mut input = AliasInput::new();
        input.set_text("w".repeat(80));
        assert_eq!(input.text().chars().count(), MAX_WALLET_ALIAS_CHARS);
    }

    #[test]
    fn counter_matches_the_validator_count() {
        let mut input = AliasInput::new();
        input.set_text("  \u{202E}abc\u{200B}  ");
        assert_eq!(alias_char_count(input.text()), 3);
    }
}
