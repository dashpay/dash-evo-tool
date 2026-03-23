use egui::{Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use crate::model::secret::Secret;
use crate::ui::theme::{DashColors, ResponseExt};

/// Response from [`PasswordInput::show`].
///
/// Intentionally does NOT implement `ComponentResponse` — exposing the `Secret`
/// through a generic trait would undermine the security model.
// INTENTIONAL(PROJ-001): Custom response type instead of ComponentResponse.
pub struct PasswordInputResponse {
    /// The underlying `TextEdit` response.
    pub response: Response,
    /// Whether the text changed this frame.
    pub changed: bool,
}

/// A masked text input with a hold-to-reveal eye icon.
///
/// The backing buffer is a [`Secret`] that is zeroized on drop.
/// The eye icon uses a press-and-hold gesture: the plaintext is visible
/// only while the pointer is held down on the icon, and re-masked
/// immediately on release.
///
/// # Usage
///
/// Store as `Option<PasswordInput>` for lazy initialization:
///
/// ```rust,ignore
/// let pw = self.password.get_or_insert_with(|| {
///     PasswordInput::new()
///         .with_hint_text("Wallet password")
///         .with_desired_width(300.0)
/// });
/// let resp = pw.show(ui);
/// if resp.changed { /* validate */ }
/// ```
pub struct PasswordInput {
    secret: Secret,
    hint_text: String,
    desired_width: Option<f32>,
    error_message: Option<String>,
    monospace: bool,
    /// Carry-over from the previous frame (see module-level note on timing).
    revealing: bool,
}

impl PasswordInput {
    /// Create a new empty password input with default hint text.
    pub fn new() -> Self {
        Self {
            secret: Secret::default(),
            hint_text: "Enter password".to_string(),
            desired_width: None,
            error_message: None,
            monospace: false,
            revealing: false,
        }
    }

    // -- Builder methods (consuming self) ------------------------------------

    /// Override the hint / placeholder text.
    pub fn with_hint_text(mut self, hint: impl Into<String>) -> Self {
        self.hint_text = hint.into();
        self
    }

    /// Set the desired width of the text field (builder pattern).
    pub fn with_desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    /// Update the desired width of the text field on an existing instance.
    pub fn set_desired_width(&mut self, width: f32) {
        self.desired_width = Some(width);
    }

    /// Render the text in a monospace font (useful for WIF keys).
    pub fn with_monospace(mut self) -> Self {
        self.monospace = true;
        self
    }

    // -- Access methods ------------------------------------------------------

    /// Borrow the plaintext.
    pub fn text(&self) -> &str {
        self.secret.expose_secret()
    }

    /// Replace the secret with new content.
    pub fn set_text(&mut self, s: impl Into<String>) {
        self.secret = Secret::new(s);
    }

    /// Zeroize and reset the secret to empty.
    pub fn clear(&mut self) {
        self.secret = Secret::default();
    }

    /// Whether the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.secret.is_empty()
    }

    /// Set or clear an external error message shown below the input.
    pub fn set_error(&mut self, err: Option<String>) {
        self.error_message = err;
    }

    /// Borrow the underlying [`Secret`].
    pub fn secret(&self) -> &Secret {
        &self.secret
    }

    /// Take the secret, replacing it with an empty one.
    pub fn take_secret(&mut self) -> Secret {
        std::mem::take(&mut self.secret)
    }

    // -- Rendering -----------------------------------------------------------

    /// Render the password input. Returns a [`PasswordInputResponse`].
    pub fn show(&mut self, ui: &mut Ui) -> PasswordInputResponse {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // -- TextEdit --------------------------------------------------------
        // INTENTIONAL(SEC-005): Egui TextEdit may cache plaintext in layout galleys and
        // accessibility state. Accepted as inherent framework limitation for desktop GUI threat model.
        let mut text_edit = egui::TextEdit::singleline(&mut self.secret)
            .password(!self.revealing)
            .hint_text(&self.hint_text)
            .margin(egui::Margin {
                right: 28,
                ..egui::Margin::same(4)
            });

        if self.monospace {
            text_edit = text_edit.font(egui::TextStyle::Monospace);
        }

        if let Some(width) = self.desired_width {
            text_edit = text_edit.desired_width(width);
        } else {
            text_edit = text_edit.desired_width(ui.available_width());
        }

        let text_response = ui.add(text_edit);

        // SEC-001: Disable egui's Undoer to prevent plaintext undo history.
        if let Some(mut state) =
            egui::widgets::text_edit::TextEditState::load(ui.ctx(), text_response.id)
        {
            state.set_undoer(egui::util::undoer::Undoer::with_settings(
                egui::util::undoer::Settings {
                    max_undos: 0,
                    ..Default::default()
                },
            ));
            state.store(ui.ctx(), text_response.id);
        }

        let changed = text_response.changed();

        // -- Eye icon --------------------------------------------------------
        let eye_size = vec2(24.0, 24.0);
        let eye_center = pos2(
            text_response.rect.right() - 16.0,
            text_response.rect.center().y,
        );
        let eye_rect = Rect::from_center_size(eye_center, eye_size);

        let eye_id = text_response.id.with("eye");
        let eye_response = ui.interact(eye_rect, eye_id, Sense::click_and_drag());

        // Update for NEXT frame.
        let next_revealing = eye_response.is_pointer_button_down_on() && eye_response.hovered();
        let reveal_changed = next_revealing != self.revealing;
        self.revealing = next_revealing;

        let icon_color = if eye_response.hovered() || self.revealing {
            DashColors::DASH_BLUE
        } else {
            DashColors::text_secondary(dark_mode)
        };

        let painter = ui.painter();
        let cx = eye_center.x;
        let cy = eye_center.y;

        if self.revealing {
            // Open eye: circle outline + filled pupil.
            painter.circle_stroke(eye_center, 6.0, Stroke::new(1.5, icon_color));
            painter.circle_filled(eye_center, 2.5, icon_color);
        } else {
            // Closed eye: circle outline + diagonal slash.
            painter.circle_stroke(eye_center, 6.0, Stroke::new(1.5, icon_color));
            painter.line_segment(
                [pos2(cx - 5.0, cy - 5.0), pos2(cx + 5.0, cy + 5.0)],
                Stroke::new(1.5, icon_color),
            );
        }

        if eye_response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        eye_response.clickable_tooltip("Hold to reveal");

        if self.revealing || reveal_changed {
            ui.ctx().request_repaint();
        }

        // -- Error label -----------------------------------------------------
        if let Some(err) = &self.error_message {
            ui.colored_label(DashColors::VALIDATION_WARNING, err);
        }

        PasswordInputResponse {
            response: text_response,
            changed,
        }
    }
}

impl Default for PasswordInput {
    fn default() -> Self {
        Self::new()
    }
}
