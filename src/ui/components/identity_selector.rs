use std::collections::BTreeMap;

use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::{
    identity::accessors::IdentityGettersV0, platform_value::string_encoding::Encoding,
};
use dash_sdk::platform::Identifier;
use egui::{ComboBox, Response, Ui, Widget};

/// A reusable identity selector widget that combines a ComboBox dropdown of available identities
/// with a text edit field for manual entry. Implements the egui `Widget` trait for idiomatic usage.
///
/// The widget includes an "Other" option in the dropdown that allows users to manually enter
/// identity addresses. When a known identity ID is entered in the text field, the corresponding
/// identity is automatically selected in the dropdown.
///
/// # Example
/// ```rust,no_run
/// use dash_evo_tool::ui::components::identity_selector::IdentitySelector;
/// use dash_sdk::query_types::IndexMap;
/// use dash_sdk::platform::Identifier;
/// use dash_evo_tool::model::qualified_identity::QualifiedIdentity;
///
/// // This example shows the API usage, but cannot be run in doctest
/// // due to complex dependencies
/// fn example_usage(ui: &mut egui::Ui, identities: &IndexMap<Identifier, QualifiedIdentity>) {
///     let mut identity_str = String::new();
///     let exclude_list = vec!["already_used_identity".to_string()];
///
///     let response = ui.add(IdentitySelector::new(
///         "my_selector",
///         &mut identity_str,
///         identities
///     )
///     .width(250.0)
///     .allow_duplicates(false)
///     .exclude(&exclude_list));
///
///     if response.changed() {
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
    /// Map of available identities to choose from
    identities: BTreeMap<Identifier, &'a QualifiedIdentity>,
    /// Slice of identity strings to exclude from dropdown (can be empty)
    exclude_identities: &'a [Identifier],
}

impl<'a> IdentitySelector<'a> {
    /// Create a new IdentitySelector with the given ID and required parameters
    pub fn new(
        id: impl Into<String>,
        identity_str: &'a mut String,
        identities: &'a [QualifiedIdentity],
    ) -> Self {
        Self {
            id: id.into(),
            width: 200.0,
            identity_str,
            identities: identities.iter().map(|q| (q.identity.id(), q)).collect(),
            exclude_identities: &[],
        }
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
}

impl<'a> Widget for IdentitySelector<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        ui.horizontal(|ui| {
            // Check if current identity_str matches any existing identity
            let current_identity = if !self.identity_str.is_empty() {
                Identifier::from_string(self.identity_str, Encoding::Base58).ok()
            } else {
                None
            };

            let has_matching_identity = current_identity
                .and_then(|id| self.identities.get(&id))
                .is_some();

            // ComboBox for selecting existing identities
            let combo_response = ComboBox::from_id_salt(&self.id)
                .width(self.width)
                .selected_text(if has_matching_identity {
                    // Show the display name of the matching identity
                    current_identity
                        .and_then(|id| self.identities.get(&id))
                        .map(|q| q.display_string())
                        .unwrap_or_else(|| "Other".to_string())
                } else if self.identity_str.is_empty() {
                    // Show "Other" when input is empty
                    "Other".to_string()
                } else {
                    // Show "Other" when input doesn't match any known identity
                    "Other".to_string()
                })
                .show_ui(ui, |ui| {
                    let mut combo_changed = false;

                    // Add existing identities to the dropdown
                    for (identifier, qualified_identity) in self.identities.iter() {
                        // Filter out excluded identities
                        if self.exclude_identities.contains(identifier) {
                            continue;
                        }
                        let id_str = identifier.to_string(Encoding::Base58);

                        if ui
                            .selectable_label(
                                current_identity == Some(*identifier),
                                qualified_identity.display_string(),
                            )
                            .clicked()
                        {
                            *self.identity_str = id_str;
                            combo_changed = true;
                        }
                    }

                    // Add "Other" option
                    if ui
                        .selectable_label(!has_matching_identity, "Other")
                        .clicked()
                    {
                        self.identity_str.clear();
                        combo_changed = true;
                    }

                    combo_changed
                });

            // Text edit field for manual entry
            let text_response = ui.text_edit_singleline(self.identity_str);

            // Return a response that indicates if anything changed
            let combo_changed = combo_response.inner.unwrap_or(false);

            // Combine the responses, preferring the text edit response for interactions
            let mut response = text_response;
            if combo_changed {
                response.mark_changed();
            }
            response
        })
        .inner
    }
}
