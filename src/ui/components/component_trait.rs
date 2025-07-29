use egui::{InnerResponse, Ui};

/// Generic response trait for all UI components following the design pattern.
///
/// All component responses should implement this trait to provide consistent
/// access to basic response properties.
pub trait ComponentResponse: Clone {
    /// The domain object type that this response represents.
    /// This type represents the data this component is designed to handle,
    /// such as Amount, Identity, etc.
    ///
    /// It must be equal to the `DomainType` of the component that produced this response.
    type DomainType;

    /// Returns whether the component input/state has changed
    fn has_changed(&self) -> bool;

    /// Returns whether the component is in a valid state (no error)
    fn is_valid(&self) -> bool;

    /// Returns the changed value of the component, if any; otherwise, `None`.
    /// It is Some() only if `has_changed()` is true.
    ///
    /// Note that only valid values should be returned here.
    /// If the component value is invalid, this should return `None`.
    fn changed_value(&self) -> &Option<Self::DomainType>;

    /// Returns any error message from the component
    fn error_message(&self) -> Option<&str>;
}

/// Core trait that all UI components following the design pattern should implement.
///
/// This trait provides a standardized interface for components that follow the
/// established patterns of lazy initialization, dual configuration APIs, and
/// response-based communication.
///
/// # Type Parameters
///
/// * `DomainType` - The domain object type that this component is designed to handle.
///   This represents the conceptual data type the component works with (e.g., Amount, Identity).
/// * `Response` - The specific response type returned by the component's `show()` method
///
/// # Design Pattern Implementation
///
/// Components implementing this trait should follow these patterns:
///
/// 1. **Self-Contained State Management**: Manage internal state privately
/// 2. **Lazy Initialization**: Created only when first needed via `Option<Component>`
/// 3. **Builder API**: Provide fluent configuration methods like `new().with_config().with_label()`
/// 4. **Response-Based Communication**: Return structured response objects from `show()`
/// 5. **Dual Configuration API**: Provide both owned (`config()`) and mutable (`set_config()`) methods
///
/// # Example Implementation
///
/// ```ignore
/// use egui::{InnerResponse, Ui, WidgetText};
///
/// // Define domain type
/// struct ValidationRules {
///     min_length: usize,
///     max_length: usize,
/// }
///
/// // Component struct
/// pub struct MyInputComponent {
///     internal_state: String,
///     domain_data: ValidationRules,
///     label: Option<WidgetText>,
/// }
///
/// // Response struct
/// #[derive(Clone)]
/// pub struct MyInputResponse {
///     pub response: egui::Response,
///     pub changed: bool,
///     pub error_message: Option<String>,
///     pub parsed_data: Option<String>,
/// }
///
/// impl ComponentResponse for MyInputResponse {
///     type DomainType = String;
///     
///     fn has_changed(&self) -> bool { self.changed }
///     fn is_valid(&self) -> bool { self.error_message.is_none() }
///     fn changed(&self) -> Option<Self::DomainType> {
///         self.parsed_data.clone()
///     }
///     fn error_message(&self) -> Option<&str> { self.error_message.as_deref() }
/// }
///
/// impl Component for MyInputComponent {
///     type DomainType = ValidationRules;
///     type Response = MyInputResponse;
///     
///     fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
///         ui.horizontal(|ui| {
///             // Render component...
///             MyInputResponse {
///                 response: ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
///                 changed: false,
///                 error_message: None,
///                 parsed_data: None,
///             }
///         })
///     }
/// }
///
/// // Constructor implemented as regular method
/// impl MyInputComponent {
///     pub fn new(domain_object: ValidationRules) -> Self {
///         Self {
///             internal_state: String::new(),
///             domain_data: domain_object,
///             label: None,
///         }
///     }
/// }
///
/// // Owned configuration methods (for builder pattern during lazy initialization)
/// impl MyInputComponent {
///     pub fn with_label<T: Into<WidgetText>>(mut self, label: T) -> Self {
///         self.label = Some(label.into());
///         self
///     }
/// }
///
/// // Mutable reference configuration methods (for dynamic updates)
/// impl MyInputComponent {
///     pub fn set_label<T: Into<WidgetText>>(&mut self, label: T) -> &mut Self {
///         self.label = Some(label.into());
///         self
///     }
/// }
/// ```
///
/// # Usage in Screens
///
/// ```ignore
/// use egui::Ui;
///
/// struct MyScreen {
///     my_component: Option<MyInputComponent>,
///     operation_in_progress: bool,
/// }
///
/// impl MyScreen {
///     fn render_component(&mut self, ui: &mut Ui) {
///         // Static configuration during lazy initialization
///         let component = self.my_component.get_or_insert_with(|| {
///             MyInputComponent::new(ValidationRules {
///                 min_length: 1,
///                 max_length: 100
///             })
///             .with_label("My Label:")
///         });
///         
///         // Use egui's built-in enabled state for dynamic control
///         let response = ui.add_enabled_ui(!self.operation_in_progress, |ui| {
///             component.show(ui)
///         }).inner;
///         
///         // Handle response using the changed() method
///         if let Some(new_data) = response.inner.changed() {
///             self.handle_valid_input(new_data);
///         } else if response.inner.has_changed() {
///             // Input changed but no valid data - clear stale data
///             self.clear_data();
///         }
///         
///         // Show errors
///         if let Some(error) = response.inner.error_message() {
///             ui.colored_label(egui::Color32::RED, error);
///         }
///     }
///     
///     fn handle_valid_input(&mut self, _data: String) {
///         // Handle the valid input
///     }
///     
///     fn clear_data(&mut self) {
///         // Clear stale data
///     }
/// }
/// ```
pub trait Component {
    /// The domain object type that this component is designed to handle.
    /// This type represents the data this component is designed to handle,
    /// such as Amount, Identity, etc.
    type DomainType;

    /// The response type returned by the component's `show()` method.
    /// This type should implement `ComponentResponse` and contain all
    /// information about the component's current state and any changes.
    type Response: ComponentResponse<DomainType = Self::DomainType>;

    /// Renders the component and returns a response with interaction results.
    ///
    /// This method should handle both rendering the component and processing
    /// any user interactions. The returned response should contain information
    /// about whether the component state changed, validation results, and
    /// any parsed data.
    ///
    /// # Arguments
    ///
    /// * `ui` - The egui UI context for rendering
    ///
    /// # Returns
    ///
    /// An `InnerResponse` containing the component's response data
    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response>;
}

/// Optional trait for components that support callback functions.
///
/// Components may implement this trait to provide callback support
/// for scenarios requiring immediate response to state changes.
/// This is an optional enhancement to the primary response-based pattern.
///
/// # Design Guidelines
///
/// - Always make callbacks optional (`Option<Box<dyn FnMut>>`)
/// - Provide the full response object to callbacks for maximum flexibility
/// - Only trigger callbacks when the relevant change occurs
/// - Maintain response-based communication as the primary pattern
/// - Keep callbacks simple and focused
pub trait ComponentWithCallbacks<Response>: Component {
    /// Sets a callback function to be called when the component transitions to a valid state.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function to call when valid input is received
    fn on_success(self, callback: impl FnMut(&InnerResponse<Response>) + 'static) -> Self;

    /// Sets a callback function to be called when the component transitions to an invalid state.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function to call when invalid input is received  
    fn on_error(self, callback: impl FnMut(&InnerResponse<Response>) + 'static) -> Self;
}

/// Utility trait for components that work with optional values.
///
/// This trait provides helper methods for components that manage optional
/// data and need to handle state transitions between valid and invalid inputs.
///
/// # Type Parameters
///
/// * `T` - The type of data being managed (e.g., Amount, String, etc.)
pub trait UpdatableComponentResponse<T: Clone>: ComponentResponse<DomainType = T> {
    /// Binds the response to a mutable value, updating it if the component state has changed.
    /// This is a convenience method for the common pattern of updating
    /// an `Option<T>` field based on component state changes.
    ///
    /// ## Arguments
    ///
    /// * `value` - The optional value to update; it will be set to `None` if the component value is invalid
    ///
    /// # Returns
    ///
    /// * `true` if the value was updated (including change to `None`),
    /// * `false` if it was not changed (eg. `self.has_changed() == false`).
    ///
    /// This method is useful for components that manage optional data and need to handle state transitions between valid and invalid inputs.
    fn update(&self, value: &mut Option<Self::DomainType>) -> bool {
        if self.has_changed() {
            if let Some(inner) = self.changed_value() {
                value.replace(inner.clone());
                true
            } else {
                value.take();
                true
            }
        } else {
            false
        }
    }
}
