use crate::model::amount::Amount;
use egui::{InnerResponse, Response, TextEdit, Ui, Widget, WidgetText};

/// A reusable amount input widget that handles decimal parsing and validation.
/// This widget can be used for any type of amount input (tokens, Dash, etc.).
///
/// The widget validates the input in real-time and shows error messages when
/// the input is invalid. It properly handles decimal places according to the
/// configured number of decimals. When an amount is successfully parsed, it can
/// include a unit name that will be displayed in the UI.
///
/// # Example
/// ```rust
/// use dash_evo_tool::ui::components::amount_input::AmountInput;
/// use dash_evo_tool::model::amount::Amount;
///
/// let mut amount = Amount::new(0, 8);
/// let response = ui.add(
///     AmountInput::new(&mut amount)
///         .label("Amount:")
///         .hint_text("Enter amount")
///         .max_amount(Some(1000000))
///         .show_max_button(true)
/// );
///
/// if response.inner.changed {
///     println!("Amount changed: {}", amount);
/// }
/// ```
pub struct AmountInput<'a> {
    amount: &'a mut Amount,
    amount_str: String,
    label: Option<WidgetText>,
    hint_text: Option<WidgetText>,
    max_amount: Option<u64>,
    show_max_button: bool,
    desired_width: Option<f32>,
}

/// Response from the amount input widget
pub struct AmountInputResponse {
    /// The response from the text edit widget
    pub response: Response,
    /// Whether the input text has changed
    pub changed: bool,
    /// The error message if the input is invalid
    pub error_message: Option<String>,
    /// Whether the max button was clicked
    pub max_clicked: bool,
}

impl<'a> AmountInput<'a> {
    /// Creates a new amount input widget.
    ///
    /// # Arguments
    /// * `amount` - Mutable reference to the Amount object to edit
    pub fn new(amount: &'a mut Amount) -> Self {
        let amount_str = if amount.value() == 0 {
            String::new()
        } else {
            amount.to_string()
        };

        Self {
            amount,
            amount_str,
            label: None,
            hint_text: None,
            max_amount: None,
            show_max_button: false,
            desired_width: None,
        }
    }

    /// Sets the label for the input field.
    pub fn label<T: Into<WidgetText>>(mut self, label: T) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the hint text for the input field.
    pub fn hint_text<T: Into<WidgetText>>(mut self, hint_text: T) -> Self {
        self.hint_text = Some(hint_text.into());
        self
    }

    /// Sets the maximum amount allowed. If provided, a "Max" button will be shown
    /// when `show_max_button` is true.
    pub fn max_amount(mut self, max_amount: Option<u64>) -> Self {
        self.max_amount = max_amount;
        self
    }

    /// Whether to show a "Max" button that sets the amount to the maximum.
    pub fn show_max_button(mut self, show: bool) -> Self {
        self.show_max_button = show;
        self
    }

    /// Sets the desired width of the input field.
    pub fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    /// Renders the amount input widget and returns an `InnerResponse` for use with `show()`.
    pub fn show(mut self, ui: &mut Ui) -> InnerResponse<AmountInputResponse> {
        ui.horizontal(|ui| {
            // Show label if provided
            if let Some(label) = self.label {
                ui.label(label);
            }

            // Create the text edit widget
            let mut text_edit = TextEdit::singleline(&mut self.amount_str);

            if let Some(hint) = self.hint_text {
                text_edit = text_edit.hint_text(hint);
            }

            if let Some(width) = self.desired_width {
                text_edit = text_edit.desired_width(width);
            }

            let text_response = ui.add(text_edit);
            let changed = text_response.changed();

            // Parse the amount and validate if text changed
            let error_message = if self.amount_str.trim().is_empty() {
                if changed {
                    // Reset to zero amount when empty
                    *self.amount = Amount::new(0, self.amount.decimal_places());
                }
                None
            } else {
                match Amount::parse_with_decimals(&self.amount_str, self.amount.decimal_places()) {
                    Ok(mut parsed_amount) => {
                        // Preserve the unit name from the original amount
                        if let Some(unit) = self.amount.unit_name() {
                            parsed_amount = parsed_amount.with_unit_name(unit.to_string());
                        }

                        // Check if amount exceeds maximum
                        if let Some(max_amount) = self.max_amount {
                            if parsed_amount.value() > max_amount {
                                Some("Amount exceeds maximum allowed".to_string())
                            } else if parsed_amount.value() == 0 {
                                Some("Amount must be greater than zero".to_string())
                            } else {
                                // Update the amount if validation passes
                                if changed {
                                    *self.amount = parsed_amount;
                                }
                                None
                            }
                        } else if parsed_amount.value() == 0 {
                            Some("Amount must be greater than zero".to_string())
                        } else {
                            // Update the amount if validation passes
                            if changed {
                                *self.amount = parsed_amount;
                            }
                            None
                        }
                    }
                    Err(error) => Some(error),
                }
            };

            // Show unit name if available
            if let Some(unit) = self.amount.unit_name() {
                ui.label(unit);
            }

            // Show max button if enabled and max amount is available
            let mut max_clicked = false;
            if self.show_max_button {
                if let Some(max_amount) = self.max_amount {
                    if ui.button("Max").clicked() {
                        self.amount_str =
                            Amount::format_amount(max_amount, self.amount.decimal_places());
                        // Update the amount immediately
                        let mut new_amount = Amount::new(max_amount, self.amount.decimal_places());
                        if let Some(unit) = self.amount.unit_name() {
                            new_amount = new_amount.with_unit_name(unit.to_string());
                        }
                        *self.amount = new_amount;
                        max_clicked = true;
                    }
                } else if ui.button("Max").clicked() {
                    // Max button clicked but no max amount set - still report the click
                    max_clicked = true;
                }
            }

            AmountInputResponse {
                response: text_response,
                changed,
                error_message,
                max_clicked,
            }
        })
    }
}

impl<'a> Widget for AmountInput<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui).response
    }
}
