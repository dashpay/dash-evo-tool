use egui::{Sense, Widget, WidgetInfo};

/// A wrapper widget that adds a label to any `egui::Widget` for testing purposes.
///
/// This widget allows you to attach a label to any widget, which can be used in tests
/// to identify and interact with the widget from egui_kittest.
/// Only use when there is no other way to identify the widget in tests.
///
/// ## Example usage:
///
/// ```rust
/// use egui::Widget;
/// use dash_evo_tool::ui::components::test_label::{TestLabel, TestableWidget};
/// fn my_widget(ui: &mut egui::Ui) {
///     let my_button = egui::Button::new("Click me");
///     ui.add(my_button.test_label("my_button"));
/// }
/// ```
pub struct TestLabel<T: Widget> {
    pub label: String,
    pub inner: T,
}

impl<T: Widget> TestLabel<T> {
    pub fn new(inner: T, label: &str) -> Self {
        Self {
            label: label.to_string(),
            inner,
        }
    }
}

impl<T: Widget> egui::Widget for TestLabel<T> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let scope = ui.scope(|ui| self.inner.ui(ui));
        let response = scope.response.interact(Sense::click());

        response.widget_info(move || {
            let label = self.label.clone();
            WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), label)
        });

        // Pass all interactions from the inner widget
        scope.inner.union(response)
    }
}
/// Trait to allow any widget to be tested with a label
///
/// This trait provides a method to attach a test label to any widget.
/// The label can be used to identify the widget in tests, making it easier to interact with
/// and assert conditions on the widget during testing.
pub trait TestableWidget<T: Widget> {
    fn test_label(self, label: &str) -> TestLabel<T>;
}

impl<T: Widget> TestableWidget<T> for T {
    fn test_label(self, label: &str) -> TestLabel<T> {
        TestLabel::new(self, label)
    }
}
