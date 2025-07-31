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

    /// Binds the response to a mutable value, updating it if the component state has changed.
    ///
    /// Provided `value` will be updated whenever the user changes the component state.
    /// It will be set to `None` if the component state is invalid (eg. user entered value that didn't pass the validation).
    ///
    /// # Returns
    ///
    /// * `true` if the value was updated (including change to `None`),
    /// * `false` if it was not changed (eg. `self.has_changed() == false`).
    fn update(&self, value: &mut Option<Self::DomainType>) -> bool
    where
        Self::DomainType: Clone,
    {
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
/// # See also
///
/// See `doc/COMPONENT_DESIGN_PATTERN.md` for detailed design pattern documentation.
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
    /// any user interactions, including validation, error display, hints,
    /// and formatting.
    ///
    /// # Returns
    ///
    /// An [`InnerResponse`]  containing the component's response data in [`InnerResponse::inner`] field.
    /// [`InnerResponse::inner`] should implement [`ComponentResponse`] trait.
    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response>;

    /// Returns the current value of the component.
    ///
    /// This method is an equivalent of binding some variable using [`ComponentResponse::update()`].
    ///
    /// ## See also
    ///
    /// See [`ComponentResponse::update`].
    fn current_value(&self) -> Option<Self::DomainType>;
}
