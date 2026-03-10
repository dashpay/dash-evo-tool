# PasswordInput Component -- UX Design Specification

**Date**: 2026-03-09
**Component**: `PasswordInput`
**Location**: `src/ui/components/password_input.rs`

---

## 1. Persona Walkthrough

### Alex Torres (Everyday User)
Alex unlocks wallets to send Dash. He types a password, sees dots, and needs a quick way to verify he typed it right before hitting "Unlock." The hold-to-peek eye icon is intuitive -- he has seen it in banking apps. He holds the eye, confirms his password visually, releases, and submits. He never thinks about it twice.

### Priya Nakamura (Power User)
Priya manages multiple wallets and pastes long private keys. She needs to verify the pasted key is correct before submitting. The hold-to-reveal works identically for private keys as for passwords. She also uses the RPC password field when configuring nodes -- same component, same muscle memory.

### Jordan Kim (Platform Developer)
Jordan enters private keys in hex and WIF format on the Add Existing Identity screen. The dynamic list of key inputs (add/remove) must each have their own eye icon. Jordan works fast and appreciates that the component handles masking, reveal, and styling consistently without boilerplate.

**Validation**: All three personas understand the eye icon immediately. The hold-to-reveal pattern is familiar from mobile banking and password managers. No persona needs instructions.

---

## 2. Visual Layout

```
[Label]                                    (external, above input)
+--------------------------------------------------+
|  * * * * * * * * * * * * * *          [eye_icon]  |
+--------------------------------------------------+
[Error message]                            (below, inline)
```

### Eye Icon Placement
- **Inside the text field**, right-aligned, vertically centered.
- The icon occupies a 24x24px logical area within the input's right padding.
- The text field has `right_margin` of 32px (24px icon + 8px padding) to prevent text from overlapping the icon.
- The icon is rendered as an overlay on top of the text field background using `ui.painter()`, not as a separate widget beside the field.

### Dimensions
- Input height: matches existing `TextEdit::singleline` height (typically ~28-30px with default egui style).
- Eye icon clickable area: 24x24px (meets WCAG AA 32x32 minimum when combined with vertical input padding that extends the hit area to at least 32px tall).
- Gap between text content and eye icon: 8px (SM spacing).

---

## 3. Eye Icon Geometry (painter primitives)

The icon is drawn with `ui.painter()` calls, following the same pattern as `info_icon_button`. Two states:

### Closed Eye (default -- password masked)

Drawn at 16x16px logical size within the 24x24px hit area, centered.

```
Geometry:
1. Eye outline: A single quadratic bezier arc (upper lid).
   - Left point:  (cx - 7, cy)
   - Control:     (cx, cy - 5)
   - Right point: (cx + 7, cy)
   Stroke: 1.5px, color from state.

2. Lower lid: Mirror arc below.
   - Left point:  (cx - 7, cy)
   - Control:     (cx, cy + 5)
   - Right point: (cx + 7, cy)
   Stroke: 1.5px, same color.

3. Slash line (indicates "hidden"):
   - From (cx - 5, cy - 5) to (cx + 5, cy + 5)
   Stroke: 1.5px, same color.
```

Implementation note: egui's `PathShape` with `QuadraticBezierShape` or approximate with small line segments forming the arc. Alternatively, use `painter.add(Shape::QuadraticBezier(...))`.

### Open Eye (pressed -- password revealed)

Same as closed eye but WITHOUT the slash line, and WITH a filled pupil circle:

```
Geometry:
1. Upper lid arc (same as closed eye).
2. Lower lid arc (same as closed eye).
3. Pupil: filled circle at (cx, cy), radius 2.5px.
   Fill: same color as stroke.
```

### Simplified Alternative (if bezier is too complex)

If quadratic beziers prove awkward in egui's painter API, use this circle-based approach:

```
Closed eye:
1. Circle outline: center (cx, cy), radius 6px, stroke 1.5px.
   (Represents the eye)
2. Slash: line from (cx-5, cy-5) to (cx+5, cy+5), stroke 1.5px.

Open eye:
1. Circle outline: same as above.
2. Pupil: filled circle at (cx, cy), radius 2.5px.
```

This is less visually refined but unambiguous and trivial to implement.

---

## 4. Hold-to-Reveal Interaction

### Trigger: Mouse down on eye icon area
- When the user presses the primary mouse button while the pointer is over the eye icon's 24x24px rect, the password is revealed (`.password(false)` on the TextEdit).
- The eye icon switches to the "open" variant.

### Hide: Mouse up OR pointer leaves the eye icon rect
- When the primary mouse button is released (anywhere), the password is re-masked.
- When the pointer moves outside the eye icon rect while the button is held, the password is re-masked immediately. This prevents "drag-to-keep-revealed" accidents and matches the user's stated requirement: "when mouse is moved or button released, password hides again."
- The eye icon switches back to the "closed" variant.

### Implementation in egui
```rust
// Allocate the eye icon rect inside the text field's right area
let eye_sense = egui::Sense::click_and_drag(); // need drag to detect held state
let (eye_rect, eye_response) = ui.allocate_exact_size(vec2(24.0, 24.0), eye_sense);

// Reveal only while pointer is over the icon AND primary button is held
let revealing = eye_response.is_pointer_button_down_on() && eye_response.hovered();
```

The key egui API: `Response::is_pointer_button_down_on()` returns true if the pointer initially pressed on this widget and the button is still held. Combined with `hovered()` (pointer is currently over the rect), this gives exact hold-to-reveal-while-over behavior.

### Keyboard Accessibility
- No keyboard toggle. egui has limited keyboard/focus support for custom-painted widgets. The eye icon is a mouse-only affordance.
- Rationale: Hold-to-reveal is inherently a pointer interaction. A keyboard equivalent would need to be a toggle (press to reveal, press again to hide), which contradicts the security model of hold-to-reveal. Since egui lacks ARIA/screen-reader support anyway, this is an acceptable limitation.
- If keyboard reveal is ever needed in the future, a separate `with_keyboard_reveal(Key)` builder method can be added.

### Cursor
- Hovering over the eye icon: `CursorIcon::PointingHand`.
- While held/revealing: `CursorIcon::PointingHand` (no change).

---

## 5. Component States

### 5.1 Default (masked, eye closed)
- Text field shows dots/bullets (egui's `.password(true)` rendering).
- Eye icon drawn in "closed" variant.
- Eye icon color: `DashColors::text_secondary(dark_mode)` (muted, not distracting).
- Border: 1px, `DashColors::border(dark_mode)`.

### 5.2 Hover over Eye Icon
- Eye icon color changes to `DashColors::DASH_BLUE` (#008de4).
- Cursor: `PointingHand`.
- Tooltip: "Hold to reveal" (via `.on_hover_text()`).
- Text field appearance unchanged.

### 5.3 Pressed/Held (revealing)
- Text shown in plain text (`.password(false)`).
- Eye icon drawn in "open" variant.
- Eye icon color: `DashColors::DASH_BLUE`.
- Border: unchanged from current focus/unfocus state.

### 5.4 Focused (input has keyboard focus)
- Border: 2px, `DashColors::DASH_BLUE`.
- Eye icon appearance unchanged from default (unless also hovered).
- Standard egui focus ring behavior.

### 5.5 Error (validation failed)
- Border: 2px, `DashColors::ERROR` (#eb5757).
- Error text shown below the input in `DashColors::VALIDATION_WARNING` color, `Typography::SCALE_SM` (14px).
- Eye icon remains functional and unchanged in color.

### 5.6 Disabled
- Text field: `ui.add_enabled(false, ...)`.
- Eye icon: drawn in `DashColors::text_disabled(dark_mode)`, no hover effect, no interaction.
- Cursor: default (no PointingHand).

### 5.7 Empty with Hint Text
- Hint text shown in `DashColors::text_secondary(dark_mode)`.
- Eye icon shown but muted (same as default state).
- Hint text examples: "Enter password", "Enter private key (WIF or hex)".

---

## 6. Component API

### Struct

```rust
pub struct PasswordInput {
    // Internal state
    text: String,          // The actual password/key text
    revealing: bool,       // Transient: true only while eye is held

    // Configuration (set via builder)
    label: Option<String>,
    hint_text: String,
    desired_width: Option<f32>,
    error_message: Option<String>,
    monospace: bool,       // For private keys (hex/WIF)
}
```

### Constructor

```rust
impl PasswordInput {
    pub fn new() -> Self { ... }
}
```

### Builder Methods

```rust
impl PasswordInput {
    /// Label rendered above the input. If None, no label is rendered.
    /// For cases where the caller renders the label externally (e.g., Grid layout),
    /// omit this.
    pub fn with_label(mut self, label: impl Into<String>) -> Self;

    /// Hint text shown when the input is empty.
    pub fn with_hint_text(mut self, hint: impl Into<String>) -> Self;

    /// Fixed width for the input. If None, uses available width.
    pub fn with_desired_width(mut self, width: f32) -> Self;

    /// Use monospace font (for private keys, hex strings).
    pub fn with_monospace(mut self) -> Self;
}
```

### Show Method

```rust
impl Component for PasswordInput {
    type Response = PasswordInputResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<PasswordInputResponse>;
}
```

### Response

```rust
pub struct PasswordInputResponse {
    pub response: Response,
    pub changed: bool,
    pub error_message: Option<String>,
    pub value: Option<String>,  // Some(text) when non-empty, None when empty
}

impl ComponentResponse for PasswordInputResponse {
    type DomainType = String;
    // ... standard trait methods
}
```

### External Error Injection

Callers can set validation errors externally (e.g., "Wrong password" from backend):

```rust
// After show(), caller can set error for next frame:
password_input.set_error(Some("Incorrect password"));

// Or clear it:
password_input.set_error(None);
```

This is needed because password validation happens asynchronously (backend task), not at input time.

### Text Access

```rust
impl PasswordInput {
    /// Get the current text value (for reading before submission).
    pub fn text(&self) -> &str;

    /// Clear the input (e.g., after failed unlock attempt or screen reset).
    pub fn clear(&mut self);
}
```

### Label Handling Decision

The component supports an **optional built-in label** via `with_label()`. Rationale:
- Most password fields need a label ("Password:", "Private Key:").
- The label is always above the input (per project convention).
- In `Grid` layouts where the label is in a separate column (like add_existing_identity_screen), callers omit `with_label()` and render the label themselves.

---

## 7. Usage Patterns

### 7.1 Wallet Unlock (simple)

```rust
struct MyScreen {
    password: Option<String>,
    password_widget: Option<PasswordInput>,
}

// In show():
let widget = self.password_widget.get_or_insert_with(|| {
    PasswordInput::new()
        .with_label("Password")
        .with_hint_text("Enter wallet password")
});
let response = widget.show(ui);
response.inner.update(&mut self.password);
```

### 7.2 Private Key Input (monospace)

```rust
let widget = self.key_widget.get_or_insert_with(|| {
    PasswordInput::new()
        .with_label("Private Key")
        .with_hint_text("WIF (51-52 chars) or hex (64 chars)")
        .with_monospace()
});
```

### 7.3 Dynamic List of Keys (add_existing_identity_screen)

The `Vec<String>` keys_input pattern does NOT use lazy-init `Option<PasswordInput>` because the list is dynamic (add/remove). Instead, use a `Vec<PasswordInput>` that is kept in sync with the data:

```rust
struct AddExistingIdentityScreen {
    keys_input: Vec<PasswordInput>,  // replaces Vec<String>
}

// Adding a key:
self.keys_input.push(
    PasswordInput::new()
        .with_hint_text("Private key (hex or WIF)")
        .with_monospace()
);

// In show() loop:
for (i, key_input) in self.keys_input.iter_mut().enumerate() {
    ui.label(format!("Private Key {}:", i + 1));
    key_input.show(ui);
}

// Collecting values for submission:
let keys: Vec<String> = self.keys_input.iter()
    .map(|w| w.text().to_string())
    .collect();
```

### 7.4 RPC Password (network config)

```rust
let widget = self.rpc_password_widget.get_or_insert_with(|| {
    PasswordInput::new()
        .with_hint_text("Core RPC password")
});
// No label -- rendered by external Grid column
```

---

## 8. Rendering Implementation Notes

### Eye Icon as Overlay

The eye icon is NOT a separate widget placed beside the TextEdit. It is painted as an overlay on top of the TextEdit's allocated rect. This approach:

1. Keeps the component as a single `ui.add()` call.
2. Avoids layout issues with horizontal grouping.
3. Matches how egui's built-in widgets handle internal icons.

Implementation sketch:

```rust
fn show(&mut self, ui: &mut Ui) -> InnerResponse<PasswordInputResponse> {
    // 1. Render label if configured
    if let Some(label) = &self.label {
        ui.label(label);
    }

    // 2. Build and add the TextEdit
    let text_edit = TextEdit::singleline(&mut self.text)
        .password(!self.revealing)
        .hint_text(&self.hint_text)
        .desired_width(self.desired_width.unwrap_or(f32::INFINITY))
        .margin(Margin { right: 32.0, ..Default::default() }); // Reserve space for icon

    let text_response = ui.add(text_edit);

    // 3. Calculate eye icon rect (right side of text field)
    let eye_rect = Rect::from_center_size(
        pos2(text_response.rect.right() - 16.0, text_response.rect.center().y),
        vec2(24.0, 24.0),
    );

    // 4. Sense interaction on eye rect
    let eye_id = text_response.id.with("eye");
    let eye_response = ui.interact(eye_rect, eye_id, Sense::click_and_drag());

    // 5. Update revealing state
    self.revealing = eye_response.is_pointer_button_down_on() && eye_response.hovered();

    // 6. Draw eye icon
    self.paint_eye_icon(ui, eye_rect, &eye_response);

    // 7. Show error below
    if let Some(err) = &self.error_message {
        ui.colored_label(DashColors::VALIDATION_WARNING, err);
    }

    // 8. Request repaint if revealing (to re-mask on release)
    if self.revealing {
        ui.ctx().request_repaint();
    }

    // ... build and return response
}
```

### Repaint on Reveal

When revealing, the component must call `ui.ctx().request_repaint()` each frame to ensure the UI updates immediately when the mouse button is released. Without this, there could be a frame delay before re-masking.

---

## 9. Accessibility

- **Min click target**: 24x24px icon within input that extends to at least 32px vertically due to input height. Meets WCAG AA.
- **Hover tooltip**: "Hold to reveal" on the eye icon.
- **No keyboard shortcut**: Acceptable limitation given egui's accessibility constraints. The password can still be typed and submitted without ever using the eye icon.
- **Color contrast**: Eye icon uses `text_secondary` (muted gray) by default and `DASH_BLUE` on hover/press. Both meet 3:1 contrast ratio against input background in both light and dark themes.
- **No ARIA**: egui does not support ARIA roles. Not applicable.

---

## 10. Migration Checklist

Replace all existing password/key input patterns with `PasswordInput`:

| Location | Current Pattern | Notes |
|---|---|---|
| `wallet_unlock_popup.rs` | `TextEdit + .password(!show) + StyledCheckbox` | Remove `show_password` field, checkbox |
| `wallet_unlock.rs` (trait) | `TextEdit + .password(!show) + StyledCheckbox` | Remove `show_password()` / `show_password_mut()` from trait |
| `single_key_send_screen.rs` | `TextEdit + .password(true) + checkbox "Show"` | Remove `show_password` field |
| `wallets_screen/mod.rs` | `TextEdit + .password(!show) + checkbox` | Remove `sk_show_password` field |
| `create_asset_lock_screen.rs` | `TextEdit + .password(!show)` via unlock trait | Remove `show_password` field |
| `asset_lock_detail_screen.rs` | `TextEdit + .password(!show)` via unlock trait | Remove `show_password` field |
| `import_mnemonic_screen.rs` | `TextEdit.password(true)` (no toggle) | Add reveal capability |
| `add_new_wallet_screen.rs` | `ui.text_edit_singleline` (NO masking!) | Fix security issue |
| `import_mnemonic_screen.rs` (password) | `ui.text_edit_singleline` (NO masking!) | Fix security issue |
| `add_existing_identity_screen.rs` (keys) | `ui.text_edit_singleline` (NO masking!) | Fix security issue, use Vec<PasswordInput> |
| `add_existing_identity_screen.rs` (voting/owner keys) | `ui.text_edit_singleline` (NO masking!) | Fix security issue |
| `network_chooser_screen.rs` (RPC password) | `ui.text_edit_singleline` (NO masking!) | Fix security issue |

---

## 11. Design Decisions and Rationale

**Hold-to-reveal vs toggle**: The user explicitly requested hold-to-reveal. This is more secure than a persistent toggle because the password is never left visible accidentally. It matches mobile banking patterns (e.g., Chase, Revolut).

**Eye inside vs adjacent**: Inside the field reduces horizontal space usage and is the dominant pattern in web/mobile UIs. Users expect the eye icon inside the input.

**No separate label widget**: The component optionally renders its own label to reduce boilerplate. Callers in Grid layouts skip the built-in label and render their own.

**Single component for passwords AND private keys**: Both are sensitive text that should be masked. The only difference is monospace font for keys. The `with_monospace()` builder handles this without needing a separate `PrivateKeyInput` component.

**No strength indicator**: Wallet passwords in Dash Evo Tool are user-chosen and not validated for strength. Adding a strength meter would be scope creep and is not requested.
