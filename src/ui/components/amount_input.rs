use crate::model::amount::Amount;
use crate::ui::components::{
    Component, ComponentResponse, ComponentWithCallbacks, UpdatableComponentResponse,
};
use dash_sdk::dpp::fee::Credits;
use egui::{InnerResponse, Response, TextEdit, Ui, Vec2, Widget, WidgetText};

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

impl AmountInputResponse {
    /// Returns whether the input is valid (no error message)
    pub fn is_valid(&self) -> bool {
        self.error_message.is_none()
    }

    /// Returns whether the input has changed
    pub fn has_changed(&self) -> bool {
        self.changed
    }
}

impl ComponentResponse for AmountInputResponse {
    type DomainType = Amount;
    fn has_changed(&self) -> bool {
        self.changed
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.parsed_amount
    }

    fn is_valid(&self) -> bool {
        self.error_message.is_none()
    }

    fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }
}

impl UpdatableComponentResponse<Amount> for AmountInputResponse {
    // we rely on default implementation
}

type CallbackFn = Box<dyn FnMut(&ShowResponse)>;

/// A reusable amount input widget that handles decimal parsing and validation.
/// This widget can be used for any type of amount input (tokens, Dash, etc.).
///
/// The widget validates the input in real-time and shows error messages when
/// the input is invalid. It properly handles decimal places according to the
/// configured number of decimals. Unit names from the initial Amount are
/// automatically preserved and applied to all parsed amounts.
///
/// # Features
/// - Real-time validation with error messages
/// - Configurable minimum and maximum amounts
/// - Optional "Max" button for quick maximum input
/// - Automatic unit name preservation from initial Amount objects
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
///     current_amount: Option<Amount>, // Track the current amount with unit name
///     operation_in_progress: bool,
/// }
///
/// impl MyScreen {
///     fn new() -> Self {
///         Self {
///             amount_input: None, // Lazy initialization
///             current_amount: None, // Will be set by [AmountInputResponse::update()]
///             operation_in_progress: false,
///         }
///     }
///
///     fn show(&mut self, ui: &mut Ui) {
///         // Initialize AmountInput with the current amount (preserves unit name)
///         let amount_input = self.amount_input.get_or_insert_with(|| {
///             AmountInput::new(Amount::dash(0.0)) // Start with zero DASH
///                 .label("Amount:")
///                 .hint_text("Enter amount")
///                 .max_amount(Some(1000000))
///                 .min_amount(Some(1000))
///                 .max_button(true)
///         });
///
///         // Use egui's enabled state to disable input during operations
///         let response = ui.add_enabled_ui(!self.operation_in_progress, |ui| {
///             amount_input.show(ui)
///         }).inner;
///
///         // Simple, correct handling using the helper method
///         if response.inner.update(&mut self.current_amount) {
///             // Put logic to handle the amount change, if any
///             println!("Amount updated: {:?}", self.current_amount);
///         }
///         // Note: error handling is done inside AmountInput, so we don't need to display errors here
///     }
/// }
/// ```
pub struct AmountInput {
    amount_str: String,
    decimal_places: u8,
    unit_name: Option<String>,
    label: Option<WidgetText>,
    hint_text: Option<WidgetText>,
    max_amount: Option<Credits>,
    min_amount: Option<Credits>,
    show_max_button: bool,
    desired_width: Option<f32>,
    show_validation_errors: bool,
    /// Function to execute when correct amount is entered
    pub on_success_fn: Option<CallbackFn>,
    /// Function to execute when invalid amount is entered
    pub on_error_fn: Option<CallbackFn>,

    // When true, we enforce that the input was changed, even if text edit didn't change.
    changed: bool,
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
    pub fn new<T: AsRef<Amount>>(amount: T) -> Self {
        let amount = amount.as_ref();
        let amount_str = if amount.value() == 0 {
            String::new()
        } else {
            amount.to_string_without_unit()
        };
        Self {
            amount_str,
            decimal_places: amount.decimal_places(),
            unit_name: amount.unit_name().map(|s| s.to_string()),
            label: None,
            hint_text: None,
            max_amount: None,
            min_amount: Some(1), // Default minimum is 1 (greater than zero)
            show_max_button: false,
            desired_width: None,
            show_validation_errors: true, // Default to showing validation errors
            on_success_fn: None,
            on_error_fn: None,
            changed: false,
        }
    }

    /// Sets whether the input has changed.
    /// This is useful for cases where you want to force the component to treat the input as changed,
    /// even if the text edit widget itself did not register a change.
    pub fn set_changed(&mut self, changed: bool) -> &mut Self {
        self.changed = changed;
        self
    }
    /// Gets the currently parsed amount without showing the widget.
    /// Returns None if the current text is empty or invalid.
    pub fn get_current_amount(&self) -> Option<Amount> {
        if self.amount_str.trim().is_empty() {
            None
        } else {
            match Amount::parse_with_decimals(&self.amount_str, self.decimal_places) {
                Ok(mut amount) => {
                    // Apply the unit name if we have one
                    if let Some(ref unit_name) = self.unit_name {
                        amount = amount.with_unit_name(unit_name.clone());
                    }
                    Some(amount)
                }
                Err(_) => None,
            }
        }
    }

    /// Gets the number of decimal places this input is configured for.
    pub fn decimal_places(&self) -> u8 {
        self.decimal_places
    }

    /// Gets the unit name this input is configured for.
    pub fn unit_name(&self) -> Option<&str> {
        self.unit_name.as_deref()
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

    /// Controls whether validation errors are displayed as a label within the component.
    pub fn show_validation_errors(mut self, show: bool) -> Self {
        self.show_validation_errors = show;
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
            Ok(mut amount) => {
                // Apply the unit name if we have one
                if let Some(ref unit_name) = self.unit_name {
                    amount = amount.with_unit_name(unit_name.clone());
                }

                // Check if amount exceeds maximum
                if let Some(max_amount) = self.max_amount {
                    if amount.value() > max_amount {
                        return (
                            Some(format!(
                                "Amount {} exceeds allowed maximum {}",
                                amount,
                                Amount::new(max_amount, self.decimal_places)
                            )),
                            None,
                        );
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
            if self.show_max_button {
                // ensure we have height predefined to correctly vertically align the input field;
                // see StyledButton::show() to see how y is calculated
                ui.allocate_space(Vec2::new(0.0, 30.0));
            }
            // Show label if provided
            if let Some(label) = &self.label {
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

            let text_response = ui.add(text_edit);

            let mut changed = text_response.changed() && ui.is_enabled();

            // Show max button if max amount is available
            let mut max_clicked = false;
            if self.show_max_button {
                if let Some(max_amount) = self.max_amount {
                    if ui.button("Max").clicked() {
                        self.amount_str = Amount::format_amount(max_amount, self.decimal_places);
                        max_clicked = true;
                        changed = true;
                    }
                } else if ui.button("Max").clicked() {
                    // Max button clicked but no max amount set - still report the click
                    max_clicked = true;
                }
            }

            // Validate the amount
            let (error_message, parsed_amount) = self.validate_amount();

            // Show validation error if enabled and error exists
            if self.show_validation_errors
                && let Some(error_msg) = &error_message
            {
                ui.colored_label(ui.visuals().error_fg_color, error_msg);
            }

            if self.changed {
                changed = true; // Force changed if set
                self.changed = false; // Reset after use
            }

            AmountInputResponse {
                response: text_response,
                changed,
                error_message: error_message.clone(),
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

impl Component for AmountInput {
    type DomainType = Amount;
    type Response = AmountInputResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        AmountInput::show(self, ui)
    }
}

impl ComponentWithCallbacks<AmountInputResponse> for AmountInput {
    fn on_success(
        self,
        callback: impl FnMut(&InnerResponse<AmountInputResponse>) + 'static,
    ) -> Self {
        AmountInput::on_success(self, callback)
    }

    fn on_error(self, callback: impl FnMut(&InnerResponse<AmountInputResponse>) + 'static) -> Self {
        AmountInput::on_error(self, callback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialization_with_non_zero_amount_and_unit() {
        // Test that AmountInput correctly initializes from an existing amount
        let amount = Amount::dash(1.5); // 1.5 DASH 

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
        let amount = Amount::dash(0.0);
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
    fn test_unit_name_preservation() {
        let amount = Amount::new_with_unit(150_000_000_000, 11, "DASH".to_string()); // 1.5 DASH
        let mut input = AmountInput::new(amount);

        // Check that unit name is preserved
        assert_eq!(input.unit_name(), Some("DASH"));

        // Test that get_current_amount preserves unit name
        input.amount_str = "2.5".to_string();
        let current = input.get_current_amount().unwrap();
        assert_eq!(current.unit_name(), Some("DASH"));
        assert_eq!(format!("{}", current), "2.5 DASH");

        // Test validation also preserves unit name
        let (error, parsed) = input.validate_amount();
        assert!(error.is_none());
        let parsed = parsed.unwrap();
        assert_eq!(parsed.unit_name(), Some("DASH"));
        assert_eq!(format!("{}", parsed), "2.5 DASH");
    }

    #[test]
    fn test_token_unit_name_preservation() {
        let amount = Amount::new_with_unit(1000000, 6, "MYTOKEN".to_string()); // 1.0 MYTOKEN
        let mut input = AmountInput::new(amount);

        // Check that token unit name is preserved
        assert_eq!(input.unit_name(), Some("MYTOKEN"));

        // Test with different amount
        input.amount_str = "5.5".to_string();
        let current = input.get_current_amount().unwrap();
        assert_eq!(current.unit_name(), Some("MYTOKEN"));
        assert_eq!(format!("{}", current), "5.5 MYTOKEN");
    }

    #[test]
    fn test_validation_states() {
        let amount = Amount::new(0, 2); // 2 decimal places for simple testing
        let mut input = AmountInput::new(amount);

        // Test empty input (valid)
        input.amount_str = "".to_string();
        let (error, parsed) = input.validate_amount();
        assert!(error.is_none(), "Empty input should be valid");
        assert!(parsed.is_none(), "Empty input should have no parsed amount");

        // Test valid input
        input.amount_str = "10.50".to_string();
        let (error, parsed) = input.validate_amount();
        assert!(error.is_none(), "Valid input should have no error");
        assert!(parsed.is_some(), "Valid input should have parsed amount");

        // Test invalid input (too many decimals)
        input.amount_str = "10.555".to_string();
        let (error, parsed) = input.validate_amount();
        assert!(error.is_some(), "Invalid input should have error");
        assert!(
            parsed.is_none(),
            "Invalid input should have no parsed amount"
        );

        // Test invalid input (non-numeric)
        input.amount_str = "abc".to_string();
        let (error, parsed) = input.validate_amount();
        assert!(error.is_some(), "Non-numeric input should have error");
        assert!(
            parsed.is_none(),
            "Non-numeric input should have no parsed amount"
        );
    }

    #[test]
    fn test_min_max_validation() {
        let amount = Amount::new(0, 2);
        let mut input = AmountInput::new(amount)
            .min_amount(Some(100)) // Minimum 1.00
            .max_amount(Some(10000)); // Maximum 100.00

        // Test amount below minimum
        input.amount_str = "0.50".to_string(); // 50 (below min of 100)
        let (error, parsed) = input.validate_amount();
        assert!(error.is_some(), "Amount below minimum should have error");
        assert!(
            parsed.is_none(),
            "Amount below minimum should have no parsed amount"
        );

        // Test amount above maximum
        input.amount_str = "150.00".to_string(); // 15000 (above max of 10000)
        let (error, parsed) = input.validate_amount();
        assert!(error.is_some(), "Amount above maximum should have error");
        assert!(
            parsed.is_none(),
            "Amount above maximum should have no parsed amount"
        );

        // Test valid amount within range
        input.amount_str = "50.00".to_string(); // 5000 (within range)
        let (error, parsed) = input.validate_amount();
        assert!(error.is_none(), "Amount within range should have no error");
        assert!(
            parsed.is_some(),
            "Amount within range should have parsed amount"
        );
    }
}
