use eframe::egui::{self, Response, Ui, Widget};

/// A reusable collapsing header widget that prevents hover effects on the arrow
/// and shows a pointer cursor when hovering over the header.
///
/// This component provides a consistent, clickable collapsing header experience
/// without the default egui hover effects (blue color, scaling) on the arrow.
///
/// # Features
/// - Prevents arrow hover effects (no blue color, no scaling)
/// - Shows pointer cursor when hovering over header
/// - Fully clickable header area
/// - Customizable ID for multiple instances
/// - Force open/closed states support
///
/// # Example
/// ```no_run
/// # use eframe::egui;
/// # let mut ui: &mut egui::Ui = todo!();
/// # use dash_evo_tool::ui::components::clickable_collapsing_header::ClickableCollapsingHeader;
///
/// ClickableCollapsingHeader::new("My Settings")
///     .id_salt("my_settings_header")
///     .default_open(false)
///     .show(ui, |ui| {
///         ui.label("Content goes here");
///         // ... more UI content
///     });
/// ```
///
/// # Force States Example  
/// ```no_run
/// # use eframe::egui;
/// # let mut ui: &mut egui::Ui = todo!();
/// # use dash_evo_tool::ui::components::clickable_collapsing_header::ClickableCollapsingHeader;
///
/// ClickableCollapsingHeader::new("Always Closed")
///     .force_closed()
///     .show(ui, |ui| {
///         ui.label("This will be collapsed");
///     });
/// ```
pub struct ClickableCollapsingHeader {
    text: String,
    id_salt: String,
    default_open: bool,
    open_override: Option<bool>,
}

impl ClickableCollapsingHeader {
    /// Create a new clickable collapsing header with the given text
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            id_salt: text.clone(),
            text,
            default_open: false,
            open_override: None,
        }
    }

    /// Set a unique ID salt for this header (useful when you have multiple headers with the same text)
    pub fn id_salt(mut self, id_salt: impl Into<String>) -> Self {
        self.id_salt = id_salt.into();
        self
    }

    /// Set whether the header should be open by default
    pub fn default_open(mut self, default_open: bool) -> Self {
        self.default_open = default_open;
        self
    }

    /// Force the header to be in a specific open/closed state
    pub fn open(mut self, open: Option<bool>) -> Self {
        self.open_override = open;
        self
    }

    /// Convenience method to force close the header
    pub fn force_closed(self) -> Self {
        self.open(Some(false))
    }

    /// Convenience method to force open the header
    pub fn force_open(self) -> Self {
        self.open(Some(true))
    }

    /// Show the collapsing header and return the response from the body content
    pub fn show<R>(
        self,
        ui: &mut Ui,
        add_body: impl FnOnce(&mut Ui) -> R,
    ) -> egui::collapsing_header::CollapsingResponse<R> {
        // Create the collapsing header
        let mut collapsing_header = egui::CollapsingHeader::new(&self.text)
            .id_salt(&self.id_salt)
            .default_open(self.default_open);

        // Apply open override if specified
        if let Some(open) = self.open_override {
            collapsing_header = collapsing_header.open(Some(open));
        }

        // Customize the styling to prevent hover effects on arrow
        ui.scope(|ui| {
            // Override the widget colors and expansion to prevent hover effects on collapsing header arrow
            ui.style_mut().visuals.widgets.hovered.bg_fill =
                ui.style().visuals.widgets.inactive.bg_fill;
            ui.style_mut().visuals.widgets.hovered.weak_bg_fill =
                ui.style().visuals.widgets.inactive.weak_bg_fill;
            ui.style_mut().visuals.widgets.hovered.fg_stroke =
                ui.style().visuals.widgets.inactive.fg_stroke;
            ui.style_mut().visuals.widgets.hovered.expansion =
                ui.style().visuals.widgets.inactive.expansion;

            let response = collapsing_header.show(ui, add_body);

            // Change cursor to pointer when hovering over the header
            if response.header_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            response
        })
        .inner
    }
}

impl Widget for ClickableCollapsingHeader {
    fn ui(self, ui: &mut Ui) -> Response {
        // For the Widget trait, we just show the header without body content
        self.show(ui, |_ui| {}).header_response
    }
}
