use crate::model::amount::Amount;
use crate::ui::components::{Component, ComponentResponse};
use dash_sdk::dpp::balances::credits::MAX_CREDITS;
use dash_sdk::dpp::fee::Credits;
use egui::{InnerResponse, Response, TextEdit, Ui, Vec2, WidgetText};

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

/// A reusable amount input widget that handles decimal parsing and validation.
/// This widget can be used for any type of amount input (tokens, Dash, etc.).
///
/// The widget validates the input in real-time and shows error messages when
/// the input is invalid. It follows the component design pattern with lazy
/// initialization and response-based communication.
///
/// # Usage
///
/// Store the component as `Option<AmountInput>` in your screen struct for lazy
/// initialization, then use the fluent builder API to configure it:
///
/// ```rust,ignore
/// let amount_input = self.amount_input.get_or_insert_with(|| {
///     AmountInput::new(Amount::new_dash(0.0))
///         .label("Amount:")
///         .hint_text("Enter amount")
///         .max_amount(Some(1000000))
///         .min_amount(Some(1000))
///         .max_button(true)
/// });
///
/// let response = amount_input.show(ui);
/// response.inner.update(&mut self.amount);
/// ```
///
/// See the tests for complete usage examples.
pub struct AmountInput {
    // Raw data, as entered by the user
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
    // When true, we enforce that the input was changed, even if text edit didn't change.
    changed: bool,
}

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
            max_amount: Some(MAX_CREDITS),
            min_amount: Some(1), // Default minimum is 1 (greater than zero)
            show_max_button: false,
            desired_width: None,
            show_validation_errors: true, // Default to showing validation errors
            changed: true,                // Start as changed to force initial validation
        }
    }

    /// Sets whether the input has changed.
    /// This is useful for cases where you want to force the component to treat the input as changed,
    /// even if the text edit widget itself did not register a change.
    pub fn set_changed(&mut self, changed: bool) -> &mut Self {
        self.changed = changed;
        self
    }

    /// Gets the number of decimal places this input is configured for.
    pub fn decimal_places(&self) -> u8 {
        self.decimal_places
    }

    /// Update decimal places used to render values.
    ///
    /// Value displayed in the input is not changed, but the actual [Amount]
    /// will be multiplied or divided by 10^(difference of decimal places).
    ///
    /// ## Example
    ///
    /// The input contains `12.34` and decimal places is set to 3.
    /// It will be interpreted as `12.340` when parsed (credits value `12_340`).
    ///
    ///
    /// If you change the decimal places from 3 to 5:
    ///
    /// * The input will still display `12.34` (unchanged)
    /// * The next time the input is parsed, it will generate `12.34000`
    ///   (credits value `1_234_000`).
    pub fn set_decimal_places(&mut self, decimal_places: u8) -> &mut Self {
        self.decimal_places = decimal_places;
        self.changed = true;

        self
    }

    /// Gets the unit name this input is configured for.
    pub fn unit_name(&self) -> Option<&str> {
        self.unit_name.as_deref()
    }

    /// Sets the label for the input field.
    pub fn with_label<T: Into<WidgetText>>(mut self, label: T) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the label for the input field (mutable reference version).
    /// Use this for dynamic configuration when the label needs to change after initialization.
    pub fn set_label<T: Into<WidgetText>>(&mut self, label: T) -> &mut Self {
        self.label = Some(label.into());
        self
    }

    /// Sets value of the input field.
    ///
    /// This will update the internal state and mark the component as changed.
    pub fn set_value(&mut self, value: Amount) -> &mut Self {
        self.amount_str = value.to_string_without_unit();
        self.decimal_places = value.decimal_places();
        self.unit_name = value.unit_name().map(|s| s.to_string());
        self.changed = true; // Mark as changed to trigger validation
        self
    }

    /// Sets the hint text for the input field.
    pub fn with_hint_text<T: Into<WidgetText>>(mut self, hint_text: T) -> Self {
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
    pub fn with_max_amount(mut self, max_amount: Option<Credits>) -> Self {
        self.max_amount = max_amount;
        self
    }

    /// Sets the maximum amount allowed (mutable reference version).
    /// Use this for dynamic configuration when the max amount changes at runtime (e.g., balance updates).
    ///
    /// Defaults to [`MAX_CREDITS`](dash_sdk::dpp::balances::credits::MAX_CREDITS).
    pub fn set_max_amount(&mut self, max_amount: Option<Credits>) -> &mut Self {
        self.max_amount = max_amount;
        self
    }

    /// Sets the minimum amount allowed. Defaults to 1 (must be greater than zero).
    /// Set to Some(0) to allow zero amounts, or None to disable minimum validation.
    pub fn with_min_amount(mut self, min_amount: Option<Credits>) -> Self {
        self.min_amount = min_amount;
        self
    }

    /// Sets the minimum amount allowed (mutable reference version).
    pub fn set_min_amount(&mut self, min_amount: Option<Credits>) -> &mut Self {
        self.min_amount = min_amount;
        self
    }

    /// Whether to show a "Max" button that sets the amount to the maximum.
    pub fn with_max_button(mut self, show: bool) -> Self {
        self.show_max_button = show;
        self
    }

    /// Whether to show a "Max" button (mutable reference version).
    pub fn set_show_max_button(&mut self, show: bool) -> &mut Self {
        self.show_max_button = show;
        self
    }

    /// Sets the desired width of the input field.
    pub fn with_desired_width(mut self, width: f32) -> Self {
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

    /// Validates the current amount string and returns validation results.
    ///
    /// Returns `Ok(Some(Amount))` for valid input, `Ok(None)` for empty input,
    /// or `Err(String)` with error message if validation fails.
    fn validate_amount(&self) -> Result<Option<Amount>, String> {
        if self.amount_str.trim().is_empty() {
            return Ok(None);
        }

        match Amount::parse(&self.amount_str, self.decimal_places) {
            Ok(mut amount) => {
                // Apply the unit name if we have one
                if let Some(ref unit_name) = self.unit_name {
                    amount = amount.with_unit_name(unit_name);
                }

                // Check if amount exceeds maximum
                if let Some(max_amount) = self.max_amount {
                    if amount.value() > max_amount {
                        return Err(format!(
                            "Amount {} exceeds allowed maximum {}",
                            amount,
                            Amount::new(max_amount, self.decimal_places)
                        ));
                    }
                }

                // Check if amount is below minimum
                if let Some(min_amount) = self.min_amount {
                    if amount.value() < min_amount {
                        return Err(format!(
                            "Amount must be at least {}",
                            Amount::new(min_amount, self.decimal_places)
                        ));
                    }
                }

                Ok(Some(amount))
            }
            Err(error) => Err(error),
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
                        self.amount_str = Amount::new(max_amount, self.decimal_places).to_string();
                        max_clicked = true;
                        changed = true;
                    }
                } else if ui.button("Max").clicked() {
                    // Max button clicked but no max amount set - still report the click
                    max_clicked = true;
                }
            }

            // Validate the amount
            let (error_message, parsed_amount) = match self.validate_amount() {
                Ok(amount) => (None, amount),
                Err(error) => (Some(error), None),
            };

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
                error_message,
                max_clicked,
                parsed_amount,
            }
        })
    }
}

impl Component for AmountInput {
    type DomainType = Amount;
    type Response = AmountInputResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        AmountInput::show_internal(self, ui)
    }

    fn current_value(&self) -> Option<Self::DomainType> {
        // Validate the current amount string and return the parsed amount
        match self.validate_amount() {
            Ok(Some(amount)) => Some(amount),
            Ok(None) => None, // Empty input
            Err(_) => None,   // Invalid input returns None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialization_with_non_zero_amount_and_unit() {
        // Test that AmountInput correctly initializes from an existing amount
        let amount = Amount::new_dash(1.5); // 1.5 DASH 

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
        let amount = Amount::new_dash(0.0);
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
        let input = AmountInput::new(Amount::new(0, 8)).with_min_amount(Some(1000));
        assert_eq!(input.min_amount, Some(1000));

        // Allow zero
        let input = AmountInput::new(Amount::new(0, 8)).with_min_amount(Some(0));
        assert_eq!(input.min_amount, Some(0));

        // No minimum
        let input = AmountInput::new(Amount::new(0, 8)).with_min_amount(None);
        assert_eq!(input.min_amount, None);
    }

    #[test]
    fn test_unit_name_preservation() {
        let amount = Amount::new(150_000_000_000, 11).with_unit_name("DASH"); // 1.5 DASH
        let mut input = AmountInput::new(amount);

        // Check that unit name is preserved
        assert_eq!(input.unit_name(), Some("DASH"));

        // Test that get_current_amount preserves unit name
        input.amount_str = "2.5".to_string();
        let current = input.validate_amount().unwrap().unwrap();
        assert_eq!(current.unit_name(), Some("DASH"));
        assert_eq!(format!("{}", current), "2.5 DASH");

        // Test validation also preserves unit name
        let validation_result = input.validate_amount();
        assert!(validation_result.is_ok());
        let parsed = validation_result.unwrap().unwrap();
        assert_eq!(parsed.unit_name(), Some("DASH"));
        assert_eq!(format!("{}", parsed), "2.5 DASH");
    }

    #[test]
    fn test_token_unit_name_preservation() {
        let amount = Amount::new(1000000, 6).with_unit_name("MYTOKEN"); // 1.0 MYTOKEN
        let mut input = AmountInput::new(amount);

        // Check that token unit name is preserved
        assert_eq!(input.unit_name(), Some("MYTOKEN"));

        // Test with different amount
        input.amount_str = "5.5".to_string();
        let current = input.validate_amount().unwrap().unwrap();
        assert_eq!(current.unit_name(), Some("MYTOKEN"));
        assert_eq!(format!("{}", current), "5.5 MYTOKEN");
    }

    #[test]
    fn test_validation_states() {
        let amount = Amount::new(0, 2); // 2 decimal places for simple testing
        let mut input = AmountInput::new(amount);

        // Test empty input (valid)
        input.amount_str = "".to_string();
        let validation_result = input.validate_amount();
        assert!(validation_result.is_ok(), "Empty input should be valid");
        assert!(
            validation_result.unwrap().is_none(),
            "Empty input should have no parsed amount"
        );

        // Test valid input
        input.amount_str = "10.50".to_string();
        let validation_result = input.validate_amount();
        assert!(
            validation_result.is_ok(),
            "Valid input should have no error"
        );
        assert!(
            validation_result.unwrap().is_some(),
            "Valid input should have parsed amount"
        );

        // Test invalid input (too many decimals)
        input.amount_str = "10.555".to_string();
        let validation_result = input.validate_amount();
        assert!(
            validation_result.is_err(),
            "Invalid input should have error"
        );

        // Test invalid input (non-numeric)
        input.amount_str = "abc".to_string();
        let validation_result = input.validate_amount();
        assert!(
            validation_result.is_err(),
            "Non-numeric input should have error"
        );
    }

    #[test]
    fn test_min_max_validation() {
        let amount = Amount::new(0, 2);
        let mut input = AmountInput::new(amount)
            .with_min_amount(Some(100)) // Minimum 1.00
            .with_max_amount(Some(10000)); // Maximum 100.00

        // Test amount below minimum
        input.amount_str = "0.50".to_string(); // 50 (below min of 100)
        let validation_result = input.validate_amount();
        assert!(
            validation_result.is_err(),
            "Amount below minimum should have error"
        );

        // Test amount above maximum
        input.amount_str = "150.00".to_string(); // 15000 (above max of 10000)
        let validation_result = input.validate_amount();
        assert!(
            validation_result.is_err(),
            "Amount above maximum should have error"
        );

        // Test valid amount within range
        input.amount_str = "50.00".to_string(); // 5000 (within range)
        let validation_result = input.validate_amount();
        assert!(
            validation_result.is_ok(),
            "Amount within range should have no error"
        );
        assert!(
            validation_result.unwrap().is_some(),
            "Amount within range should have parsed amount"
        );
    }
}
