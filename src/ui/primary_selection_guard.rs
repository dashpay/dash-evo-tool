//! Keeps selected text out of the X11/Wayland PRIMARY selection.
//!
//! egui reports every settled text selection (labels and non-password text
//! fields) as [`egui::OutputCommand::TextSelectionSettled`], and egui-winit
//! writes it to PRIMARY, where every other application and clipboard manager can
//! read it. The app shows recovery phrases, private keys and WIFs as text, so
//! merely selecting one would leak it. Copying stays explicit (Ctrl+C sends
//! [`egui::OutputCommand::CopyText`], which this guard leaves alone).

/// egui plugin that drops [`egui::OutputCommand::TextSelectionSettled`] from
/// every pass's output before the platform integration sees it.
///
/// It filters in [`egui::Plugin::output_hook`], which egui runs at the end of
/// each pass, after the app's ui and after every plugin's end-of-pass hook (label
/// selections are reported from one), for every viewport.
#[derive(Debug, Default)]
pub struct PrimarySelectionGuard;

impl PrimarySelectionGuard {
    /// Register the guard on `ctx`. Registering it again is a no-op.
    pub fn install(ctx: &egui::Context) {
        ctx.add_plugin(Self);
    }
}

impl egui::Plugin for PrimarySelectionGuard {
    fn debug_name(&self) -> &'static str {
        "PrimarySelectionGuard"
    }

    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut egui::FullOutput) {
        output
            .platform_output
            .commands
            .retain(|command| !matches!(command, egui::OutputCommand::TextSelectionSettled(_)));
    }
}

#[cfg(test)]
mod tests {
    use super::PrimarySelectionGuard;
    use egui::{Event, Key, Modifiers, OutputCommand, PointerButton};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    const LABEL_TEXT: &str = "Selectable recovery words";
    const FIELD_TEXT: &str = "Selectable private key";

    fn harness(guarded: bool, mut ui_fn: impl FnMut(&mut egui::Ui) + 'static) -> Harness<'static> {
        Harness::builder()
            .with_context_setup(move |ctx| {
                if guarded {
                    PrimarySelectionGuard::install(ctx);
                }
            })
            .build_ui(move |ui| ui_fn(ui))
    }

    /// Step `frames` times, collecting the output commands of every frame.
    fn collect_commands(harness: &mut Harness<'_>, frames: usize) -> Vec<OutputCommand> {
        let mut commands = Vec::new();
        for _ in 0..frames {
            harness.step();
            commands.extend(harness.output().platform_output.commands.iter().cloned());
        }
        commands
    }

    fn settled_selections(commands: &[OutputCommand]) -> Vec<&str> {
        commands
            .iter()
            .filter_map(|command| match command {
                OutputCommand::TextSelectionSettled(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Drag across the whole label to select it; returns every frame's commands.
    fn select_label(guarded: bool) -> Vec<OutputCommand> {
        let mut harness = harness(guarded, |ui| {
            ui.add(egui::Label::new(LABEL_TEXT).selectable(true));
        });
        harness.run();
        let rect = harness.get_by_label(LABEL_TEXT).rect();
        let (start, end) = (rect.left_center(), rect.right_center());
        let mut commands = Vec::new();
        for event in [
            Event::PointerMoved(start),
            Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            Event::PointerMoved(end),
            Event::PointerButton {
                pos: end,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ] {
            harness.event(event);
            commands.extend(collect_commands(&mut harness, 1));
        }
        commands.extend(collect_commands(&mut harness, 3));
        commands
    }

    /// Focus a non-password text field, select all of it, then press Copy.
    /// Returns the commands of the selection frames and of the copy frames.
    fn select_and_copy_field(guarded: bool) -> (Vec<OutputCommand>, Vec<OutputCommand>) {
        let mut text = FIELD_TEXT.to_owned();
        let mut harness = harness(guarded, move |ui| {
            ui.add(egui::TextEdit::singleline(&mut text))
                .accessible_name("Key");
        });
        harness.run();
        harness.get_by_label("Key").focus();
        harness.step();
        harness.event(Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        let selected = collect_commands(&mut harness, 3);
        harness.event(Event::Copy);
        let copied = collect_commands(&mut harness, 2);
        (selected, copied)
    }

    #[test]
    fn selecting_label_text_never_reaches_the_primary_selection() {
        let unguarded = select_label(false);
        assert_eq!(
            settled_selections(&unguarded),
            [LABEL_TEXT],
            "without the guard, egui reports the label selection (this test's control)"
        );

        let guarded = select_label(true);
        assert!(
            settled_selections(&guarded).is_empty(),
            "the guard must keep the selected label text out of PRIMARY: {guarded:?}"
        );
    }

    #[test]
    fn selecting_text_field_text_never_reaches_the_primary_selection_and_copy_still_works() {
        let (unguarded, _) = select_and_copy_field(false);
        assert_eq!(
            settled_selections(&unguarded),
            [FIELD_TEXT],
            "without the guard, egui reports the field selection (this test's control)"
        );

        let (selected, copied) = select_and_copy_field(true);
        assert!(
            settled_selections(&selected).is_empty() && settled_selections(&copied).is_empty(),
            "the guard must keep the selected field text out of PRIMARY: {selected:?} {copied:?}"
        );
        assert!(
            copied.iter().any(
                |command| matches!(command, OutputCommand::CopyText(text) if text == FIELD_TEXT)
            ),
            "an explicit copy must still reach the clipboard: {copied:?}"
        );
    }
}
