use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyID, Purpose, SecurityLevel};
use egui::{ComboBox, Response, TextEdit, Ui, Widget, WidgetText};
use std::collections::BTreeMap;

/// A reusable key selector widget that can operate in two modes:
/// 1. Identity-based mode: When a QualifiedIdentity is provided, shows a dropdown of available keys
/// 2. Manual mode: When no QualifiedIdentity is provided, shows text inputs for public and private keys
///
/// The widget implements the egui `Widget` trait for idiomatic usage and follows similar patterns
/// to the IdentitySelector component.
///
/// # Example
/// ```rust
/// // Identity-based mode
/// let mut selected_key_id = None;
/// let response1 = ui.add(KeySelector::new("key_selector_1")
///     .width(250.0)
///     .label("Select Key:")
///     .identity(&qualified_identity)
///     .selected_key_id(&mut selected_key_id));
///
/// // Manual mode
/// let mut public_key_str = String::new();
/// let mut private_key_str = String::new();
/// let response2 = ui.add(KeySelector::new("key_selector_2")
///     .width(300.0)
///     .label("Enter Keys:")
///     .manual_keys(&mut public_key_str, &mut private_key_str));
/// ```
pub struct KeySelector<'a> {
    /// A unique ID for this selector (used for egui's ID system)
    id: String,
    /// Width of the ComboBox/TextEdit fields
    width: f32,
    /// Optional QualifiedIdentity to select keys from
    identity: Option<&'a QualifiedIdentity>,
    /// Selected key ID when in identity mode
    selected_key_id: Option<&'a mut Option<KeyID>>,
    /// Manual public key string when in manual mode
    public_key_str: Option<&'a mut String>,
    /// Manual private key string when in manual mode
    private_key_str: Option<&'a mut String>,
    /// Optional label to display before the selector
    label: Option<WidgetText>,
    /// Filter keys by purpose
    purpose_filter: Option<Purpose>,
    /// Filter keys by security level
    security_level_filter: Option<SecurityLevel>,
}

impl<'a> KeySelector<'a> {
    /// Create a new KeySelector with the given ID
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            width: 200.0,
            identity: None,
            selected_key_id: None,
            public_key_str: None,
            private_key_str: None,
            label: None,
            purpose_filter: None,
            security_level_filter: None,
        }
    }

    /// Set the width of the input fields
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Set an optional label to display before the selector
    pub fn label(mut self, label: impl Into<WidgetText>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Configure for identity-based mode with a QualifiedIdentity and selected key ID
    pub fn identity(
        mut self,
        identity: &'a QualifiedIdentity,
        selected_key_id: &'a mut Option<KeyID>,
    ) -> Self {
        self.identity = Some(identity);
        self.selected_key_id = Some(selected_key_id);
        self
    }

    /// Configure for manual mode with text inputs for public and private keys
    pub fn manual_keys(
        mut self,
        public_key_str: &'a mut String,
        private_key_str: &'a mut String,
    ) -> Self {
        self.public_key_str = Some(public_key_str);
        self.private_key_str = Some(private_key_str);
        self
    }

    /// Filter keys by purpose (only applies in identity mode)
    pub fn purpose_filter(mut self, purpose: Purpose) -> Self {
        self.purpose_filter = Some(purpose);
        self
    }

    /// Filter keys by security level (only applies in identity mode)
    pub fn security_level_filter(mut self, security_level: SecurityLevel) -> Self {
        self.security_level_filter = Some(security_level);
        self
    }

    /// Get available keys from the identity based on filters
    fn get_available_keys(&self) -> BTreeMap<KeyID, &'a QualifiedIdentityPublicKey> {
        let Some(identity) = self.identity else {
            return BTreeMap::new();
        };

        let mut keys = BTreeMap::new();

        // Get all private keys from the identity
        for (_, qualified_public_key) in identity.private_keys.identity_public_keys() {
            let public_key = &qualified_public_key.identity_public_key;

            // Apply purpose filter
            if let Some(purpose_filter) = self.purpose_filter {
                if public_key.purpose() != purpose_filter {
                    continue;
                }
            }

            // Apply security level filter
            if let Some(security_level_filter) = self.security_level_filter {
                if public_key.security_level() != security_level_filter {
                    continue;
                }
            }

            keys.insert(public_key.id(), qualified_public_key);
        }

        keys
    }

    /// Get display string for a key
    fn key_display_string(&self, qualified_key: &QualifiedIdentityPublicKey) -> String {
        let public_key = &qualified_key.identity_public_key;
        format!(
            "ID: {} | Type: {:?} | Purpose: {:?} | Level: {:?}",
            public_key.id(),
            public_key.key_type(),
            public_key.purpose(),
            public_key.security_level()
        )
    }

    /// Handle changes to the key selection
    fn on_key_selection_change(&mut self, new_key_id: Option<KeyID>) {
        if let Some(selected_key_id) = &mut self.selected_key_id {
            **selected_key_id = new_key_id;
        }
    }
}

impl<'a> Widget for KeySelector<'a> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        ui.vertical(|ui| {
            // Display label if present
            if let Some(label) = &self.label {
                ui.add(egui::Label::new(label.clone()));
                ui.add_space(5.0);
            }

            // Determine mode and render accordingly
            if let Some(_identity) = self.identity {
                // Identity-based mode: show dropdown of available keys
                let available_keys = self.get_available_keys();

                if available_keys.is_empty() {
                    return ui.label("No keys available for the selected filters");
                }

                let current_key_id = self.selected_key_id.as_ref().and_then(|k| **k);

                let current_key_display = if let Some(key_id) = current_key_id {
                    available_keys
                        .get(&key_id)
                        .map(|k| self.key_display_string(k))
                        .unwrap_or_else(|| "Invalid key selected".to_string())
                } else {
                    "Select a key...".to_string()
                };

                let combo_response = ComboBox::from_id_salt(&self.id)
                    .width(self.width)
                    .selected_text(current_key_display)
                    .show_ui(ui, |ui| {
                        let mut combo_changed = false;

                        for (key_id, qualified_key) in available_keys.iter() {
                            let checked = current_key_id == Some(*key_id);
                            let display_string = self.key_display_string(qualified_key);

                            if ui.selectable_label(checked, display_string).clicked() {
                                combo_changed = true;
                                self.on_key_selection_change(Some(*key_id));
                            }
                        }

                        // Add "None" option to clear selection
                        if ui
                            .selectable_label(current_key_id.is_none(), "None")
                            .clicked()
                        {
                            combo_changed = true;
                            self.on_key_selection_change(None);
                        }

                        combo_changed
                    });

                let combo_changed = combo_response.inner.unwrap_or(false);
                let mut response = combo_response.response;
                if combo_changed {
                    response.mark_changed();
                }
                response
            } else {
                // Manual mode: show text inputs for public and private keys
                match (self.public_key_str, self.private_key_str) {
                    (Some(public_key_str), Some(private_key_str)) => {
                        ui.horizontal(|ui| {
                            ui.label("Public Key:");
                            let public_response = ui.add(
                                TextEdit::singleline(public_key_str)
                                    .desired_width(self.width)
                                    .hint_text("Enter public key..."),
                            );

                            ui.label("Private Key:");
                            let private_response = ui.add(
                                TextEdit::singleline(private_key_str)
                                    .desired_width(self.width)
                                    .hint_text("Enter private key...")
                                    .password(true),
                            );

                            // Return combined response
                            let mut response = public_response;
                            if private_response.changed() {
                                response.mark_changed();
                            }
                            response
                        })
                        .inner
                    }
                    _ => ui.label(
                        "KeySelector: Manual mode requires both public and private key strings",
                    ),
                }
            }
        })
        .inner
    }
}
