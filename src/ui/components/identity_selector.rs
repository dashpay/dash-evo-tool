use std::collections::BTreeMap;

use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::{
    identity::accessors::IdentityGettersV0, platform_value::string_encoding::Encoding,
};
use dash_sdk::platform::Identifier;
use egui::{ComboBox, Response, TextEdit, Ui, Widget, WidgetText};

/// A reusable identity selector widget that combines a ComboBox dropdown of available identities
/// with a text edit field for manual entry. Implements the egui `Widget` trait for idiomatic usage.
///
/// The widget includes an "Other" option in the dropdown that allows users to manually enter
/// identity addresses. When a known identity ID is entered in the text field, the corresponding
/// identity is automatically selected in the dropdown.
///
/// # Example
/// ```rust
/// use dash_evo_tool::ui::components::identity_selector::IdentitySelector;
/// use dash_sdk::query_types::IndexMap;
/// use dash_sdk::platform::Identifier;
/// use dash_evo_tool::model::qualified_identity::QualifiedIdentity;
/// use egui::{RichText, Color32};
///
/// // This example shows the API usage, but cannot be run in doctest
/// // due to complex dependencies
/// fn example_usage(ui: &mut egui::Ui, identities: &[QualifiedIdentity]) {
///     let mut identity_str = String::new();
///     let exclude_list = vec![/* some identifiers */];
///
///     // Basic usage with string label
///     let response1 = ui.add(IdentitySelector::new(
///         "my_selector1",
///         &mut identity_str,
///         identities
///     )
///     .width(250.0)
///     .label("Select Identity:")               // accepts &str
///     .exclude(&exclude_list));
///
///     // Advanced usage with styled RichText label
///     let response2 = ui.add(IdentitySelector::new(
///         "my_selector2",
///         &mut identity_str,
///         identities
///     )
///     .width(300.0)
///     .label(RichText::new("Styled Label").color(Color32::RED).strong()) // accepts RichText
///     .exclude(&exclude_list));
///
///     if response1.changed() || response2.changed() {
///         // Identity was changed via dropdown selection, "Other" selection, or text input
///     }
/// }
/// ```
pub struct IdentitySelector<'a> {
    /// A unique ID for this selector (used for egui's ID system)
    id: String,
    /// Width of the ComboBox
    width: f32,
    /// Mutable reference to the current identity string
    identity_str: &'a mut String,
    /// Selected identity, if any
    identity: Option<&'a mut Option<QualifiedIdentity>>,
    /// Map of available identities to choose from
    identities: BTreeMap<Identifier, &'a QualifiedIdentity>,
    /// Slice of identity strings to exclude from dropdown (can be empty)
    exclude_identities: &'a [Identifier],
    /// Optional label to display before the selector
    label: Option<WidgetText>,
    other_option: bool,
}

impl<'a> IdentitySelector<'a> {
    /// Create a new IdentitySelector with the given ID and required parameters
    pub fn new<I: AsRef<QualifiedIdentity>>(
        id: impl Into<String>,
        identity_str: &'a mut String,
        identities: &'a [I],
    ) -> Self {
        Self {
            id: id.into(),
            width: 200.0,
            identity_str,
            identity: None,
            identities: identities
                .iter()
                .map(|q| {
                    let id = q.as_ref();
                    (id.identity.id(), id)
                })
                .collect(),
            exclude_identities: &[],
            label: None,
            other_option: true, // Default to showing "Other" option
        }
    }

    /// This method creates a selector that can update a mutable reference to the selected identity
    /// based on user input. This is useful when you want to allow users to select from existing identities
    /// or enter a new one, while keeping track of the selected identity in a mutable reference.
    ///
    /// `selected_identity` will be set to:
    /// * `Some(qualified_identity)` if a known identity is selected from the dropdown
    /// * `None` if the "Other" option is selected or the text input is empty or invalid
    pub fn selected_identity(
        mut self,
        selected_identity: &'a mut Option<QualifiedIdentity>,
    ) -> Result<Self, String> {
        self.identity = Some(selected_identity);
        // trigger change handling to initialize the state

        Ok(self)
    }

    /// Enable or disable the "Other" option in the dropdown
    pub fn other_option(mut self, other_option: bool) -> Self {
        self.other_option = other_option;

        self
    }

    /// Set the width of the ComboBox
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Set the identities to exclude from the dropdown
    pub fn exclude(mut self, exclude_identities: &'a [Identifier]) -> Self {
        self.exclude_identities = exclude_identities;
        self
    }

    /// Set an optional label to display before the selector
    pub fn label(mut self, label: impl Into<WidgetText>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Validate the given identity string and return the corresponding identity if valid.
    fn get_identity(&self, identity_str: &str) -> Option<&'a QualifiedIdentity> {
        let identifier = Identifier::from_string_unknown_encoding(identity_str).ok()?;

        if self.exclude_identities.contains(&identifier) {
            return None;
        }

        self.identities.get(&identifier).copied()
    }

    /// Handle changes to the identity selector
    fn on_change(&mut self) {
        let selected_identity = self.get_identity(self.identity_str);
        if let Some(self_identity) = &mut self.identity {
            if let Some(new_identity) = selected_identity {
                self_identity.replace(new_identity.clone());
                tracing::trace!(
                    "updating selected identity: {:?} {:?}",
                    new_identity,
                    self.identity,
                );
            } else {
                self_identity.take(); // Clear the existing identity reference if it was None
            };
        }
    }
}

impl<'a> Widget for IdentitySelector<'a> {
    /// Render the identity selector widget
    ///
    /// ## Panics
    ///
    /// This method will panic if there are no identities available to select from
    /// and no "Other" option is enabled. It requires at least one identity to function
    /// correctly, as it needs to provide a default selection.
    fn ui(mut self, ui: &mut Ui) -> Response {
        ui.horizontal(|ui| {
            // Display label if present, with centered vertical alignment
            if let Some(label) = &self.label {
                ui.vertical(|ui| {
                    // FIXME we add space because vertical alignment is not working as expected
                    ui.add_space(15.0);
                    ui.add(egui::Label::new(label.clone()));
                });
            }

            // If the "Other" option is disabled, we automatically select first identity
            if !self.other_option && self.identity_str.is_empty() {
                if let Some(first_identity) = self
                    .identities
                    .keys()
                    .find(|id| !self.exclude_identities.contains(id))
                {
                    *self.identity_str = first_identity.to_string(Encoding::Base58);
                    // trigger change handling to update the selected identity
                    self.on_change();
                }
            }

            // Check if current identity_str matches any existing identity; current_identity = None means
            // no identity is selected or the input is empty.
            let current_identity = self.get_identity(self.identity_str);

            let has_matching_identity = current_identity.is_some();

            let current_identity_combo_label = current_identity
                .map(|q| q.display_string())
                .unwrap_or_else(|| {
                    if self.other_option {
                        "Other".to_string()
                    } else {
                        "No identities found".to_string()
                    }
                });

            // ComboBox for selecting existing identities
            let combo_response = ComboBox::from_id_salt(&self.id)
                .width(self.width)
                .selected_text(current_identity_combo_label)
                .show_ui(ui, |ui| {
                    let mut combo_changed = false;

                    // Add existing identities to the dropdown
                    for (identifier, qualified_identity) in self.identities.iter() {
                        // Filter out excluded identities
                        if self.exclude_identities.contains(identifier) {
                            continue;
                        }
                        let id_str = identifier.to_string(Encoding::Base58);
                        let checked = current_identity.is_some_and(|x| qualified_identity.eq(&x));

                        if ui
                            .selectable_label(checked, qualified_identity.display_string())
                            .clicked()
                        {
                            combo_changed = true;
                            *self.identity_str = id_str;
                        }
                    }

                    // Add "Other" option
                    if self.other_option
                        && ui
                            .selectable_label(!has_matching_identity, "Other")
                            .clicked()
                    {
                        self.identity_str.clear();
                        combo_changed = true;
                    }

                    combo_changed
                });

            // Text edit field for manual entry
            let text_response = TextEdit::singleline(self.identity_str)
                .interactive(self.other_option)
                .ui(ui);

            // Handle identity selection updates after combo box and text input
            let combo_changed = combo_response.inner.unwrap_or(false);
            if combo_changed || text_response.changed() {
                self.on_change();
            }

            // Return a response that indicates if anything changed
            let mut response = text_response;
            if combo_changed {
                // note: response inherits the changed state from text_response
                response.mark_changed();
            }

            response
        })
        .inner
    }
}
