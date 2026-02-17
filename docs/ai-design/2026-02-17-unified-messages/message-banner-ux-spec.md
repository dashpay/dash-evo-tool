# MessageBanner Component -- UX Specification

## 1. Overview

The `MessageBanner` is a self-contained egui component that replaces all ad-hoc `error_message: Option<String>` fields across screens. Each screen owns one `MessageBanner` instance (not global). It displays a single message at a time with consistent styling based on severity.

---

## 2. Severity Levels

| Severity | Persistence        | Dismiss            | Use Case                                      |
|----------|---------------------|--------------------|-----------------------------------------------|
| Error    | Persistent          | Manual only        | Task failures, validation errors               |
| Warning  | Persistent          | Manual only        | Risky actions, degraded state                  |
| Success  | Auto-dismiss ~5s    | Manual or auto     | Completed operations                           |
| Info     | Auto-dismiss ~5s    | Manual or auto     | Informational feedback, neutral status updates |

All four types display a dismiss button. Success and Info also count down and auto-clear.

---

## 3. Visual Design

### 3.1 Layout Structure

```
+-----------------------------------------------------------------------+
| [Icon]  Message text here                              [5s] [Dismiss] |
+-----------------------------------------------------------------------+
```

The banner is a single horizontal row inside an `egui::Frame` with:
- **Corner radius**: 6px (`Shape::RADIUS_SM`)
- **Inner margin**: 10px horizontal, 8px vertical (matching existing pattern in `top_up_identity_screen`)
- **Outer spacing**: 10px below the banner (`ui.add_space(10.0)`)
- **Full available width**: The frame should expand to `ui.available_width()`

### 3.2 Content Arrangement (left to right)

1. **Icon character** (Unicode text, not image): Provides at-a-glance severity recognition.
   - Error: `!` (exclamation in circle, or literal `!` if unicode unavailable)
   - Warning: `!` (triangle-style -- visually distinct from error via color)
   - Success: Checkmark character
   - Info: `i` (info style)
   - Font size: `Typography::SCALE_BASE` (16px), bold
   - Color: Same as the text color for that severity

2. **4px gap** (`Spacing::XS`)

3. **Message text**: Left-aligned, wrapping permitted for long messages.
   - Font: `Typography::body()` (16px proportional)
   - Color: Severity-specific foreground color (see Section 3.3)

4. **Flexible space** (push remaining elements to the right)

5. **Countdown label** (Success and Info only): Shows remaining seconds, e.g., `(3s)`.
   - Font: `Typography::body_small()` (14px)
   - Color: Same as message text, at reduced opacity or using `text_secondary`

6. **Dismiss button**: Small text button labeled with an `x` character.
   - Uses `ui.small_button("x")`
   - Clicking sets the banner state to None

### 3.3 Color Palette

All colors are resolved through `DashColors` methods — the banner contains zero hardcoded color values. The banner uses a **tinted background + colored border + colored text** approach.

#### Color Resolution via DashColors

| Purpose | DashColors Method | Description |
|---|---|---|
| Text & border | `DashColors::message_color(type, dark_mode)` | Delegates to `error_color()` / `success_color()` / `warning_color()` / `info_color()` |
| Background tint | `DashColors::message_background_color(type, dark_mode)` | Severity color at 8% alpha (light) or 12% alpha (dark) |
| Countdown text | `DashColors::text_secondary(dark_mode)` | Standard secondary text color |

#### Resolved Values (for visual reference)

**Light Mode:**

| Severity | Background | Text & Border |
|----------|-----------|---------------|
| Error    | `ERROR` at 8% alpha | `DashColors::error_color(false)` → `DARK_RED` |
| Warning  | `WARNING` at 8% alpha | `DashColors::warning_color(false)` → dark amber |
| Success  | `SUCCESS` at 8% alpha | `DashColors::success_color(false)` → `DARK_GREEN` |
| Info     | `INFO` at 8% alpha | `DashColors::info_color(false)` → `DEEP_BLUE` |

**Dark Mode:**

| Severity | Background | Text & Border |
|----------|-----------|---------------|
| Error    | lighter red at 12% alpha | `DashColors::error_color(true)` → `rgb(255, 100, 100)` |
| Warning  | lighter amber at 12% alpha | `DashColors::warning_color(true)` → `rgb(255, 200, 100)` |
| Success  | muted green at 12% alpha | `DashColors::success_color(true)` → `rgb(80, 160, 80)` |
| Info     | light blue at 12% alpha | `DashColors::info_color(true)` → `rgb(100, 180, 255)` |

Dark mode uses higher alpha backgrounds (12% vs 8%) to maintain visibility against dark surfaces.

#### Border Stroke
- Width: `Shape::BORDER_WIDTH` (1px)
- Color: `DashColors::message_color(type, dark_mode)` — same as text color

### 3.4 Typography

- Icon: `Typography::SCALE_BASE` (16px), bold (`RichText::strong()`)
- Message body: `Typography::body()` (16px), normal weight
- Countdown: `Typography::body_small()` (14px), normal weight, secondary text color
- Dismiss button: egui default `small_button` styling

---

## 4. Placement

The banner renders at the **top of the screen's content area**, before the `ScrollArea`. This matches the existing convention in `top_up_identity_screen/mod.rs:531-552` and `add_new_identity_screen/mod.rs:1071-1092`.

```
+--------------------------------------------------+
| Top Panel (header / navigation)                   |
+--------------------------------------------------+
| Left Panel |  [ MessageBanner ]                   |  <-- here
|            |  +----- ScrollArea -----+            |
|            |  | Screen content       |            |
|            |  | ...                  |            |
|            |  +----------------------+            |
+--------------------------------------------------+
```

The banner must be rendered **outside** the `ScrollArea` so it remains visible regardless of scroll position. This is consistent with current best practice in the codebase.

---

## 5. Behavior

### 5.1 Showing a Message

Calling `banner.set_message("text", MessageType::Error)` replaces any currently displayed message. There is no queue. The new message immediately takes effect.

For auto-dismissing types (Success, Info), the component records the timestamp when the message was set (using `Instant::now()` or egui frame time).

### 5.2 Auto-Dismiss (Success, Info)

- Duration: 5 seconds
- The countdown label shows remaining whole seconds: `(5s)`, `(4s)`, ..., `(1s)`
- When the timer expires, the message clears automatically on the next frame
- The screen must call `banner.show(ui)` each frame (standard egui immediate mode)
- The component internally checks elapsed time and clears itself

### 5.3 Manual Dismiss

- All severity types display a dismiss button
- Clicking the dismiss button immediately clears the message
- No confirmation needed

### 5.4 Message Replacement

- Setting a new message while one is showing replaces it immediately
- If the old message was Error (persistent) and the new one is Success (auto-dismiss), the Success behavior applies
- Timer resets on replacement

### 5.5 Screen Navigation

- When the user navigates away from a screen and returns, persistent messages (Error, Warning) should still be visible if the screen struct was retained (root screens in `main_screens` BTreeMap)
- Auto-dismiss messages that expired while the screen was not visible should be gone on return
- Modal/detail screens pushed onto `screen_stack` are destroyed when popped, so their messages naturally disappear

---

## 6. Component API (Behavioral Spec)

This describes the public interface the component should expose. Not a full Rust implementation, but a behavioral contract for the architect and implementer.

```
MessageBanner
  State:
    - message: Option<(String, MessageType, Instant)>

  Methods:
    - new() -> Self                                     // empty, no message
    - set_message(text: &str, msg_type: MessageType)    // set/replace message
    - clear()                                           // manually clear
    - show(ui: &mut Ui)                                 // render; auto-dismiss check happens here

  MessageType (unified, replaces both existing enums):
    - Error
    - Warning
    - Success
    - Info
```

The component does NOT implement the `Component` trait from `component_trait.rs` because it has no domain data to bind via `update()`. It is a simpler display-only widget. Screens call `show(ui)` and `set_message(...)` directly.

---

## 7. Integration Points

### 7.1 ScreenLike Trait

The existing `display_message(&mut self, message: &str, message_type: MessageType)` method on `ScreenLike` is the integration point. Screens that adopt `MessageBanner` implement it as:

```
fn display_message(&mut self, message: &str, message_type: MessageType) {
    self.banner.set_message(message, message_type);
}
```

### 7.2 Replacing Existing Fields

Each screen replaces its ad-hoc fields:
- `error_message: Option<String>` -> removed
- `info_message: Option<String>` -> removed
- `message: Option<(String, MessageType)>` -> removed
- `backend_message: Option<(String, MessageType, DateTime<Utc>)>` -> removed

All replaced by a single:
- `banner: MessageBanner`

### 7.3 Sync Error Display (Validation)

For validation errors set during `ui()`:

```
if some_validation_fails {
    self.banner.set_message("Invalid input: ...", MessageType::Error);
}
```

---

## 8. Edge Cases

| Scenario | Behavior |
|----------|----------|
| Very long message (300+ chars) | Text wraps within the frame. Frame grows vertically. No truncation. |
| Empty string message | Treated as no message; banner not shown. |
| Rapid message replacement | Each call to `set_message` replaces immediately. No debounce. |
| Multiple error fields on one screen | All consolidated into a single banner. If multiple errors need display, concatenate them with newlines before calling `set_message`. |
| Screen with multiple independent sections | Each section could own its own `MessageBanner` if needed, but the default is one per screen. |
| Theme change while message is showing | Colors re-evaluate each frame via `ui.ctx().style().visuals.dark_mode`. No stale colors. |

---

## 9. Accessibility

- **Color contrast**: All text/background combinations meet WCAG 2.1 AA contrast ratio (4.5:1 minimum). The colored text on tinted backgrounds achieves this because the backgrounds are near-transparent (8-12% alpha) over the page background.
- **Not color-only**: The icon character provides a non-color severity indicator (distinct shapes for error vs warning vs success vs info).
- **Text is selectable**: egui labels allow text selection by default.
- **Keyboard**: The dismiss button is focusable and activatable via keyboard in egui's default tab-order. No special focus management needed.
- **Screen readers**: Not directly applicable (egui does not have native screen reader support), but the text content is programmatically accessible via egui's accessibility layer if enabled.

---

## 10. What This Spec Does NOT Cover

- Toast/notification stacking (out of scope -- one message per screen)
- Animation or transitions (egui does not support CSS-style transitions; show/hide is instant)
- Sound or haptic feedback
- Message persistence across app restarts
- Global overlay banners
- Changes to BackendTask/TaskResult/AppState architecture

---

## 11. Migration Strategy (UX Perspective)

The visual result after migration should be:
1. Every screen shows messages in exactly the same visual style
2. Error messages appear in the same position (top of content, before scroll area)
3. Success messages auto-clear after 5 seconds with a visible countdown
4. No more inline `colored_label` errors scattered at arbitrary positions in screen layouts
5. The Warning severity becomes available for the first time (currently missing from `MessageType` in `mod.rs`)

Screens that currently use `colored_label` for inline validation hints (e.g., "Field is required") are a separate concern and should remain inline. The `MessageBanner` replaces only the screen-level status/result messages.
