use crate::model::wallet::alias::MAX_CHARS;
use egui::{RichText, TextEdit, Ui};

pub(crate) const ALIAS_COUNTER_SHOW_THRESHOLD: usize = 50;
pub(crate) const OPTIONAL_ALIAS_HELPER_TEXT: &str = "Leave blank to use a default name.";

pub(crate) fn render_optional_alias_input(ui: &mut Ui, label: &str, alias_input: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(TextEdit::singleline(alias_input).char_limit(MAX_CHARS));
    });

    ui.horizontal(|ui| {
        ui.label(RichText::new(OPTIONAL_ALIAS_HELPER_TEXT).weak().size(12.0));

        let raw_char_count = alias_input.chars().count();
        if raw_char_count > ALIAS_COUNTER_SHOW_THRESHOLD {
            ui.label(
                RichText::new(format!("{raw_char_count}/{MAX_CHARS}"))
                    .weak()
                    .size(12.0),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        ALIAS_COUNTER_SHOW_THRESHOLD, OPTIONAL_ALIAS_HELPER_TEXT, render_optional_alias_input,
    };
    use crate::model::wallet::alias::MAX_CHARS;
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn optional_alias_input_enforces_char_limit() {
        let mut harness = Harness::new_ui_state(
            |ui, alias_input: &mut String| {
                render_optional_alias_input(ui, "Name:", alias_input);
            },
            String::new(),
        );

        let text_input = harness.get_by_role(Role::TextInput);
        text_input.focus();
        text_input.type_text(&"a".repeat(MAX_CHARS + 5));
        harness.run();

        let expected = "a".repeat(MAX_CHARS);
        assert_eq!(harness.state().chars().count(), MAX_CHARS);
        assert_eq!(
            harness.get_by_role(Role::TextInput).value().as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn optional_alias_input_shows_helper_and_counter_near_limit() {
        let alias_input = "a".repeat(ALIAS_COUNTER_SHOW_THRESHOLD + 1);
        let mut harness = Harness::new_ui_state(
            |ui, alias_input: &mut String| {
                render_optional_alias_input(ui, "Name:", alias_input);
            },
            alias_input,
        );

        harness.run();

        assert!(harness.query_by_label(OPTIONAL_ALIAS_HELPER_TEXT).is_some());
        assert!(
            harness
                .query_by_label(&format!(
                    "{}/{}",
                    ALIAS_COUNTER_SHOW_THRESHOLD + 1,
                    MAX_CHARS
                ))
                .is_some()
        );
    }

    #[test]
    fn optional_alias_input_does_not_render_inline_error() {
        let mut harness = Harness::new_ui_state(
            |ui, alias_input: &mut String| {
                render_optional_alias_input(ui, "Name:", alias_input);
            },
            " ".repeat(MAX_CHARS),
        );

        harness.run();

        assert!(harness.query_by_label(OPTIONAL_ALIAS_HELPER_TEXT).is_some());
        assert!(
            harness
                .query_by_label("Enter a name with at least one non-whitespace character.")
                .is_none()
        );
        assert!(harness.query_by_label_contains("Name is ").is_none());
    }
}
