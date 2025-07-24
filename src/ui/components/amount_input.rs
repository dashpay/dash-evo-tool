use crate::model::amount::Amount;
use dash_sdk::dpp::fee::Credits;
use egui::{InnerResponse, Response, TextEdit, Ui, Widget, WidgetText};

/// Response from the amount input widget
#[derive(Clone)]
pub struct AmountInputResponse {
    /// The response from the text edit widget
    pub response: Response,
    /// Whether the input text has changed
    pub changed: bool,
    /// The error message if the input is invalid
    pub error_message: Option<String>,
    /// Whether the max button was clicked
    pub max_clicked: bool,
    /// The parsed amount if the input is valid (None for empty input or validation errors)
    pub parsed_amount: Option<Amount>,
}

type CallbackFn = Box<dyn FnMut(&ShowResponse)>;

/// A reusable amount input widget that handles decimal parsing and validation.
/// This widget can be used for any type of amount input (tokens, Dash, etc.).
///
/// The widget validates the input in real-time and shows error messages when
/// the input is invalid. It properly handles decimal places according to the
/// configured number of decimals. When an amount is successfully parsed, it can
/// include a unit name that will be displayed in the UI.
///
/// # Features
/// - Real-time validation with error messages
/// - Configurable minimum and maximum amounts
/// - Optional "Max" button for quick maximum input
/// - Automatic unit name display from Amount objects
/// - Proper layout alignment when buttons are present
/// - Self-contained state management
/// - Enable/disable functionality to prevent changes during operations
///
/// # Example
/// ```rust
/// use dash_evo_tool::ui::components::amount_input::AmountInput;
/// use dash_evo_tool::model::amount::Amount;
/// use egui::Ui;
///
/// // Store Option<AmountInput> for lazy initialization
/// struct MyScreen {
///     amount_input: Option<AmountInput>,
///     operation_in_progress: bool,
/// }
///
/// impl MyScreen {
///     fn new() -> Self {
///         Self {
///             amount_input: None, // Lazy initialization
///             operation_in_progress: false,
///         }
///     }
///
///     fn show(&mut self, ui: &mut Ui) {
///         // All configuration happens during initialization
///         let amount_input = self.amount_input.get_or_insert_with(|| {
///             AmountInput::new(Amount::dash(0))
///                 .label("Amount:")
///                 .hint_text("Enter amount")
///                 .max_amount(Some(1000000))
///                 .min_amount(Some(1000))
///                 .max_button(true)
///         });
///
///         // Disable input during operations to prevent changes
///         if self.operation_in_progress {
///             amount_input.enabled(false);
///         } else {
///             amount_input.enabled(true);
///         }
///
///         let response = amount_input.show(ui);
///
///         if let Some(amount) = response.inner.parsed_amount {
///             println!("Amount changed: {}", amount);
///         }
///     }
/// }
/// ```
pub struct AmountInput {
    amount_str: String,
    decimal_places: u8,
    label: Option<WidgetText>,
    hint_text: Option<WidgetText>,
    max_amount: Option<Credits>,
    min_amount: Option<Credits>,
    show_max_button: bool,
    desired_width: Option<f32>,
    /// Whether the input is enabled (editable) - when false, prevents changes during operations
    enabled: bool,
    /// Function to execute when correct amount is entered
    pub on_success_fn: Option<CallbackFn>,
    /// Function to execute when invalid amount is entered
    pub on_error_fn: Option<CallbackFn>,
}

pub type ShowResponse = InnerResponse<AmountInputResponse>;

impl AmountInput {
    /// Creates a new amount input widget from an Amount.
    ///
    /// # Arguments
    /// * `amount` - The initial amount to display (determines decimal places automatically)
    ///
    /// The decimal places are automatically set based on the Amount object.
    /// Amount entered by the user will be available through [`AmountInputResponse`].
    pub fn new(amount: Amount) -> Self {
        let amount_str = if amount.value() == 0 {
            String::new()
        } else {
            amount.to_string_without_unit()
        };
        Self {
            amount_str,
            decimal_places: amount.decimal_places(),
            label: None,
            hint_text: None,
            max_amount: None,
            min_amount: Some(1), // Default minimum is 1 (greater than zero)
            show_max_button: false,
            desired_width: None,
            enabled: true,
            on_success_fn: None,
            on_error_fn: None,
        }
    }

    /// Gets the currently parsed amount without showing the widget.
    /// Returns None if the current text is empty or invalid.
    pub fn get_current_amount(&self) -> Option<Amount> {
        if self.amount_str.trim().is_empty() {
            None
        } else {
            Amount::parse_with_decimals(&self.amount_str, self.decimal_places).ok()
        }
    }

    /// Gets the number of decimal places this input is configured for.
    pub fn decimal_places(&self) -> u8 {
        self.decimal_places
    }

    /// Sets the label for the input field.
    pub fn label<T: Into<WidgetText>>(mut self, label: T) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the label for the input field (mutable reference version).
    /// Use this for dynamic configuration when the label needs to change after initialization.
    pub fn set_label<T: Into<WidgetText>>(&mut self, label: T) -> &mut Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the hint text for the input field.
    pub fn hint_text<T: Into<WidgetText>>(mut self, hint_text: T) -> Self {
        self.hint_text = Some(hint_text.into());
        self
    }

    /// Sets the hint text for the input field (mutable reference version).
    pub fn set_hint_text<T: Into<WidgetText>>(&mut self, hint_text: T) -> &mut Self {
        self.hint_text = Some(hint_text.into());
        self
    }

    /// Sets the maximum amount allowed. If provided, a "Max" button will be shown
    /// when `show_max_button` is true.
    pub fn max_amount(mut self, max_amount: Option<Credits>) -> Self {
        self.max_amount = max_amount;
        self
    }

    /// Sets the maximum amount allowed (mutable reference version).
    /// Use this for dynamic configuration when the max amount changes at runtime (e.g., balance updates).
    pub fn set_max_amount(&mut self, max_amount: Option<Credits>) -> &mut Self {
        self.max_amount = max_amount;
        self
    }

    /// Sets the minimum amount allowed. Defaults to 1 (must be greater than zero).
    /// Set to Some(0) to allow zero amounts, or None to disable minimum validation.
    pub fn min_amount(mut self, min_amount: Option<Credits>) -> Self {
        self.min_amount = min_amount;
        self
    }

    /// Sets the minimum amount allowed (mutable reference version).
    pub fn set_min_amount(&mut self, min_amount: Option<Credits>) -> &mut Self {
        self.min_amount = min_amount;
        self
    }

    /// Whether to show a "Max" button that sets the amount to the maximum.
    pub fn max_button(mut self, show: bool) -> Self {
        self.show_max_button = show;
        self
    }

    /// Whether to show a "Max" button (mutable reference version).
    pub fn set_show_max_button(&mut self, show: bool) -> &mut Self {
        self.show_max_button = show;
        self
    }

    /// Sets the desired width of the input field.
    pub fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    /// Sets the desired width of the input field (mutable reference version).
    pub fn set_desired_width(&mut self, width: f32) -> &mut Self {
        self.desired_width = Some(width);
        self
    }

    /// Sets a function to call when a valid amount is entered.
    pub fn on_success(mut self, on_success_fn: impl FnMut(&ShowResponse) + 'static) -> Self {
        self.on_success_fn = Some(Box::new(on_success_fn));
        self
    }

    /// Sets a function to call when invalid amount is entered
    pub fn on_error(mut self, on_error_fn: impl FnMut(&ShowResponse) + 'static) -> Self {
        self.on_error_fn = Some(Box::new(on_error_fn));
        self
    }

    /// Sets whether the input is enabled (editable). When disabled, the input becomes read-only.
    pub fn set_enabled(&mut self, enabled: bool) -> &mut Self {
        self.enabled = enabled;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.set_enabled(enabled);
        self
    }

    /// Returns whether the input is currently enabled (editable).
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Standard show method for backwards compatibility
    pub fn show(&mut self, ui: &mut Ui) -> InnerResponse<AmountInputResponse> {
        let result = self.show_internal(ui);
        if result.inner.changed {
            if result.inner.parsed_amount.is_some()
                && let Some(on_success_fn) = &mut self.on_success_fn
            {
                on_success_fn(&result);
            }

            // Call the error function if provided
            if let Some(on_error_fn) = &mut self.on_error_fn
                && result.inner.error_message.is_some()
            {
                on_error_fn(&result);
            }
        }

        result
    }

    /// Validates the current amount string and returns validation results.
    fn validate_amount(&self) -> (Option<String>, Option<Amount>) {
        if self.amount_str.trim().is_empty() {
            return (None, None);
        }

        match Amount::parse_with_decimals(&self.amount_str, self.decimal_places) {
            Ok(amount) => {
                // Check if amount exceeds maximum
                if let Some(max_amount) = self.max_amount {
                    if amount.value() > max_amount {
                        return (Some("Amount exceeds maximum allowed".to_string()), None);
                    }
                }

                // Check if amount is below minimum
                if let Some(min_amount) = self.min_amount {
                    if amount.value() < min_amount {
                        return (
                            Some(format!(
                                "Amount must be at least {}",
                                Amount::format_amount(min_amount, self.decimal_places)
                            )),
                            None,
                        );
                    }
                }

                (None, Some(amount))
            }
            Err(error) => (Some(error), None),
        }
    }

    /// Renders the amount input widget and returns an `InnerResponse` for use with `show()`.
    fn show_internal(&mut self, ui: &mut Ui) -> InnerResponse<AmountInputResponse> {
        ui.horizontal(|ui| {
            let has_max_button = self.show_max_button && self.max_amount.is_some();

            // Show label if provided
            if let Some(label) = &self.label {
                if has_max_button {
                    ui.add_space(15.0);
                }
                ui.label(label.clone());
            }

            // Create the text edit widget
            let mut text_edit = TextEdit::singleline(&mut self.amount_str);

            if let Some(hint) = &self.hint_text {
                text_edit = text_edit.hint_text(hint.clone());
            }

            if let Some(width) = self.desired_width {
                text_edit = text_edit.desired_width(width);
            }

            // Disable text edit if not enabled
            if !self.enabled {
                text_edit = text_edit.interactive(false);
            }

            let text_response = ui.add(text_edit);
            let changed = text_response.changed() && self.enabled;

            // Validate the amount
            let (error_message, parsed_amount) = self.validate_amount();

            // Show max button if enabled and max amount is available
            let mut max_clicked = false;
            if self.show_max_button && self.enabled {
                if let Some(max_amount) = self.max_amount {
                    if ui.button("Max").clicked() {
                        self.amount_str = Amount::format_amount(max_amount, self.decimal_places);
                        max_clicked = true;
                    }
                } else if ui.button("Max").clicked() {
                    // Max button clicked but no max amount set - still report the click
                    max_clicked = true;
                }
            } else if self.show_max_button && !self.enabled {
                // Show disabled Max button when not enabled
                ui.add_enabled(false, egui::Button::new("Max"));
            }

            AmountInputResponse {
                response: text_response,
                changed,
                error_message,
                max_clicked,
                parsed_amount,
            }
        })
    }
}

impl Widget for AmountInput {
    fn ui(mut self, ui: &mut Ui) -> Response {
        self.show(ui).response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialization_with_non_zero_amount_and_unit() {
        // Test that AmountInput correctly initializes from an existing amount
        let amount = Amount::dash(150_000_000_000); // 1.5 DASH with 11 decimal places

        assert_eq!(amount.unit_name(), Some("DASH"));
        assert_eq!(format!("{}", amount), "1.5 DASH");

        let amount_input = AmountInput::new(amount);

        // The amount_str should be initialized with the numeric part, not the unit
        assert_eq!(amount_input.amount_str, "1.5");
        assert_eq!(amount_input.decimal_places, 11);
    }

    #[test]
    fn test_initialization_with_zero_amount() {
        // Test that zero amounts initialize with empty string
        let amount = Amount::dash(0);
        let amount_input = AmountInput::new(amount);
        assert_eq!(amount_input.amount_str, "");
        assert_eq!(amount_input.decimal_places, 11);
    }

    #[test]
    fn test_minimum_amount_settings() {
        let amount = Amount::new(0, 8); // Generic amount with 8 decimal places

        // Default minimum should be 1
        let input = AmountInput::new(amount);
        assert_eq!(input.min_amount, Some(1));

        // Custom minimum
        let input = AmountInput::new(Amount::new(0, 8)).min_amount(Some(1000));
        assert_eq!(input.min_amount, Some(1000));

        // Allow zero
        let input = AmountInput::new(Amount::new(0, 8)).min_amount(Some(0));
        assert_eq!(input.min_amount, Some(0));

        // No minimum
        let input = AmountInput::new(Amount::new(0, 8)).min_amount(None);
        assert_eq!(input.min_amount, None);
    }

    #[test]
    fn test_enable_disable_functionality() {
        let amount = Amount::new(0, 8);
        let mut input = AmountInput::new(amount);

        // Initially enabled
        assert!(input.is_enabled());

        // Test disable
        input.set_enabled(false);
        assert!(!input.is_enabled());

        // Test enable
        input.set_enabled(true);
        assert!(input.is_enabled());
    }

    #[test]
    fn test_enabled_builder_method() {
        let amount = Amount::new(0, 8);

        // Test creating disabled input
        let input = AmountInput::new(amount).enabled(false);
        assert!(!input.is_enabled());

        // Test creating enabled input
        let input = AmountInput::new(Amount::new(0, 8)).enabled(true);
        assert!(input.is_enabled());
    }
}
