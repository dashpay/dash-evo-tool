# MessageBanner Component -- UX Specification

## 1. Overview

The `MessageBanner` renders user-facing messages with consistent styling based on severity. It operates in two modes:

- **Global mode**: Multiple banners stored in egui context data, rendered centrally by `island_central_panel()` before screen content. This is the primary mode — `AppState::update()` sets banners automatically for all backend task results.
- **Per-instance mode**: A screen owns a `MessageBanner` struct for screen-local messages. Shares rendering logic with global mode.

Multiple messages can be displayed simultaneously (capped at 5). Each banner is independently dismissible and independently timed.

---

## 2. Severity Levels

| Severity | Default Persistence | Dismiss | Use Case |
|----------|-------------------|---------|----------|
| Error | Persistent | Manual only | Task failures, validation errors |
| Warning | Persistent | Manual only | Risky actions, degraded state |
| Success | Auto-dismiss 5s | Manual or auto | Completed operations |
| Info | Auto-dismiss 5s | Manual or auto | Informational feedback, status updates |

All four types display a dismiss button (`x`). Success and Info also show a countdown label. Any banner can be switched to elapsed-time mode via `handle.with_elapsed()`, which disables auto-dismiss and shows time since creation.

---

## 3. Visual Design

### 3.1 Layout Structure

```text
+-----------------------------------------------------------------------+
| [Icon]  Message text here                              [5s] [x]       |
+-----------------------------------------------------------------------+
```

The banner is a horizontal row inside an `egui::Frame` with:
- **Corner radius**: `Shape::RADIUS_SM` (6px)
- **Inner margin**: 10px horizontal, 8px vertical
- **Outer spacing**: `Spacing::SM` below each banner
- **Full available width**: Frame expands to `ui.available_width()`

### 3.2 Content Arrangement (left to right)

1. **Icon** (Unicode): `⚠` (Error/Warning), `✓` (Success), `ℹ` (Info)
   - Color: Same as text color for severity
   - Style: `RichText::strong()`

2. **4px gap** (`Spacing::XS`)

3. **Message text**: Left-aligned. Long text may be clipped within the horizontal layout.
   - Font: Default egui label font (matches app-wide body text size)
   - Color: Severity-specific foreground

4. **Flexible space** (right-to-left layout for remaining elements)

5. **Annotation** (optional): Shows remaining seconds `(3s)` or elapsed seconds `(5s)`
   - Font: `Typography::body_small()` (14px)
   - Color: `DashColors::text_secondary(dark_mode)`

6. **Dismiss button**: `ui.small_button("x")`

### 3.3 Color Palette

All colors resolved through `DashColors` — zero hardcoded values.

| Purpose | Method |
|---------|--------|
| Text & border | `DashColors::message_color(type, dark_mode)` |
| Background tint | `DashColors::message_background_color(type, dark_mode)` |
| Annotation text | `DashColors::text_secondary(dark_mode)` |

Background uses low alpha (8% light, 12% dark) for subtle tinting. Border uses `Shape::BORDER_WIDTH` (1px) in the foreground color.

---

## 4. Placement

Global banners render at the top of the content area inside `island_central_panel()`, before any screen content:

```text
+--------------------------------------------------+
| Top Panel (header / navigation)                   |
+--------------------------------------------------+
| Left Panel |  [ Banner 1 ]                        |
|            |  [ Banner 2 ]                        |
|            |  +----- Screen Content -----+        |
|            |  | ...                      |        |
|            |  +--------------------------+        |
+--------------------------------------------------+
```

Banners remain visible regardless of scroll position because they render outside `ScrollArea`.

---

## 5. Behavior

### 5.1 Showing a Message

`MessageBanner::set_global(ctx, text, type)` adds a banner. If a banner with the same text already exists, the call is deduplicated (returns a handle to the existing banner).

### 5.2 Auto-Dismiss (Success, Info)

- Default duration: 5 seconds
- Countdown label: `(5s)`, `(4s)`, ..., `(1s)`
- Banner clears automatically when timer expires
- Component requests repaint every 1s for countdown updates

### 5.3 Elapsed-Time Mode

Calling `handle.with_elapsed()` switches a banner to elapsed-time display:
- Shows `(0s)`, `(1s)`, `(2s)`, ... counting up
- Auto-dismiss is disabled (banner persists until manually cleared)
- Used for long-running operations (e.g., identity refresh)

### 5.4 Manual Dismiss

- All severity types display a dismiss (`x`) button
- Clicking clears that specific banner immediately
- Other banners are unaffected

### 5.5 Message Replacement

`MessageBanner::replace_global(ctx, old_text, new_text, type)` finds a banner by old text and replaces it. If old text is not found, the new text is added as a new banner. If new text is empty, the old banner is removed.

### 5.6 BannerHandle Lifecycle

Handles returned by `set_global`/`replace_global` can outlive their banners. All handle methods return `Option` — `None` means the banner has been dismissed, expired, or cleared.

---

## 6. Edge Cases

| Scenario | Behavior |
|----------|----------|
| Very long message (300+ chars) | Text may be clipped within the horizontal layout. No explicit truncation or wrapping. |
| Empty string message | `set_global("")` is a no-op. Per-instance `set_message("")` clears. |
| Duplicate text | `set_global` returns handle to existing banner (idempotent). |
| More than 5 messages | Oldest message is evicted. |
| Rapid message replacement | Each call replaces/adds immediately. No debounce. |
| Theme change while showing | Colors re-evaluated each frame via `DashColors`. No stale colors. |
| Handle used after banner cleared | All methods return `None`. No panic. |
| Banner expired while screen not visible | Expired on next `show_global()` call when screen becomes visible. |

---

## 7. Accessibility

- **Color contrast**: Text on near-transparent backgrounds (8-12% alpha) meets WCAG 2.1 AA (4.5:1 minimum).
- **Not color-only**: Icon character provides non-color severity indicator.
- **Text selectable**: egui labels allow text selection.
- **Keyboard**: Dismiss button is focusable via egui's default tab-order.

---

## 8. What This Spec Does NOT Cover

- Toast/notification stacking beyond the 5-banner cap
- Animation or transitions (egui show/hide is instant)
- Sound or haptic feedback
- Message persistence across app restarts
- Changes to BackendTask/TaskResult/AppState routing architecture
