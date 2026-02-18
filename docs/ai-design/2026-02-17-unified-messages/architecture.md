# MessageBanner Component -- Technical Architecture

## 1. Overview

`MessageBanner` is a unified message display component that replaces the ~50 screens' ad-hoc `error_message`, `backend_message`, and `message` fields with a single consistent banner system. It operates in two modes:

- **Global**: Multiple banners stored in egui context data, rendered centrally by `island_central_panel()`. Used for backend task results and screen-level messages.
- **Per-instance**: A `MessageBanner` struct owned by a screen, implementing the `Component` trait. Available for screen-local use cases.

Both modes share the same rendering function (`render_banner()`), ensuring visual consistency.

**File**: `src/ui/components/message_banner.rs`

---

## 2. Data Structures

### BannerState (private)

Internal state for a single banner message:

```rust
struct BannerState {
    key: u64,                             // Unique ID from AtomicU64 counter
    text: String,                         // Display text
    message_type: MessageType,            // Error | Warning | Success | Info
    created_at: Instant,                  // Monotonic timestamp for timing
    auto_dismiss_after: Option<Duration>, // None = persistent, Some = countdown
    show_elapsed: bool,                   // Show elapsed time instead of countdown
}
```

### MessageType

The unified enum in `src/ui/mod.rs` with four variants: `Error`, `Warning`, `Success`, `Info`. Colors are resolved via `DashColors::message_color(type, dark_mode)` and `DashColors::message_background_color(type, dark_mode)`.

### BannerHandle

A `'static` handle returned by `set_global` and `replace_global`. Identifies a banner by its internal `u64` key, allowing text updates and configuration changes after creation:

```rust
pub struct BannerHandle {
    ctx: egui::Context,  // egui Context is Arc<RwLock<...>>, cheap to clone
    key: u64,            // Unique key assigned at creation
}
```

All query/mutation methods return `Option` to handle the case where the banner has been dismissed or expired:

| Method | Signature | Purpose |
|--------|-----------|---------|
| `elapsed()` | `-> Option<Duration>` | Time since creation (looked up from context data, not stored on handle) |
| `set_message()` | `(&self, text: &str) -> Option<&Self>` | Update display text |
| `with_auto_dismiss()` | `(&self, Duration) -> Option<&Self>` | Set/override countdown duration; resets timer to now |
| `with_elapsed()` | `-> Option<&Self>` | Enable elapsed-time display mode (disables auto-dismiss) |
| `clear()` | `(self)` | Remove banner immediately (consumes handle) |

Methods that modify the banner (`set_message`, `with_auto_dismiss`, `with_elapsed`) return early without writing back to context data when the banner no longer exists.

---

## 3. Global API

### Multi-Message Support

The global store is a `Vec<BannerState>` in egui's temporary context data, keyed by `egui::Id::new("__global_message_banner")`. Multiple messages can coexist (capped at `MAX_BANNERS = 5`, oldest evicted first).

| Method | Signature | Behavior |
|--------|-----------|----------|
| `set_global` | `(ctx, text, type) -> BannerHandle` | Add message (dedup by text). Returns handle. |
| `replace_global` | `(ctx, old_text, new_text, type) -> BannerHandle` | Find by `old_text`, replace (resets timer, auto-dismiss, and `show_elapsed`). Falls back to `set_global` if not found. |
| `clear_global_message` | `(ctx, text)` | Remove specific message by text match. |
| `has_global` | `(ctx) -> bool` | Any messages present? |
| `show_global` | `(ui)` | Render all banners, handle auto-dismiss/elapsed, remove expired. |

**Deduplication**: `set_global` with the same text returns a handle to the existing banner without creating a duplicate. Key-based lookup is used after handle creation.

**Empty text**: `set_global("")` is a no-op (returns a dead handle). `replace_global(old, "", type)` clears the old message.

### Auto-Dismiss Defaults

| MessageType | Default |
|-------------|---------|
| Success | 5 seconds countdown |
| Info | 5 seconds countdown |
| Error | Persistent (manual dismiss only) |
| Warning | Persistent (manual dismiss only) |

### Elapsed-Time Mode

Calling `handle.with_elapsed()` on a banner switches it to elapsed-time display: shows `(Ns)` counting up from 0, and disables auto-dismiss. Used for long-running operations like identity refresh.

---

## 4. Per-Instance API

`MessageBanner` as a struct implements the `Component` trait:

```rust
pub struct MessageBanner {
    state: Option<BannerState>,
}
```

| Method | Purpose |
|--------|---------|
| `new()` | Empty banner |
| `set_message(text, type)` | Set/replace message (empty text clears) |
| `set_auto_dismiss(duration)` | Override auto-dismiss duration |
| `clear()` | Remove message |
| `has_message()` | Check if displaying |

`MessageBanner` also implements `Default` (equivalent to `new()`).

The `Component` trait implementation:
- `DomainType = BannerStatus` (enum: `Visible`, `Dismissed`, `TimedOut`)
- `Response = MessageBannerResponse` — struct with `pub status: Option<BannerStatus>` and private `changed: bool`
- `show(ui)` renders the banner and returns `InnerResponse<MessageBannerResponse>`
- `current_value()` returns `Some(Visible)` when a message is set

---

## 5. Rendering

Both global and per-instance paths call `render_banner()`:

```rust
render_banner(ui, text, message_type, annotation: Option<&str>) -> bool (dismissed?)
```

The `annotation` parameter is generic — it receives either a countdown string `"(3s)"`, an elapsed string `"(5s)"`, or `None` for persistent banners. This is computed by `process_banner()` which handles the lifecycle logic.

### Visual Structure

```text
+-----------------------------------------------------------------------+
| [Icon]  Message text here                              [5s] [x]       |
+-----------------------------------------------------------------------+
```

- Frame: `DashColors::message_background_color()` fill, `DashColors::message_color()` border
- Icon: Unicode character (⚠ for Error/Warning, ✓ for Success, ℹ for Info)
- Text: `DashColors::message_color()` foreground
- Annotation: `DashColors::text_secondary()` color, `Typography::body_small()` font
- Dismiss: `ui.small_button("x")`
- Spacing: `Spacing::SM` below each banner

All colors are resolved via `DashColors` methods — zero hardcoded `Color32` values.

---

## 6. AppState Integration

`AppState::update()` in `src/app.rs` sets global banners automatically for all task results:

```text
TaskResult::Error(message)
  → MessageBanner::set_global(ctx, &message, MessageType::Error)
  → screen.display_message(&message, MessageType::Error)  // for side-effects

TaskResult::Success (default)
  → MessageBanner::set_global(ctx, "Success", MessageType::Success)
  → screen.display_task_result(result)
```

The call to `screen.display_message()` is retained alongside `set_global` so screens can perform side-effects (e.g., resetting step state on error, clearing refresh handles) without being responsible for rendering.

### Rendering Point

`island_central_panel()` in `src/ui/components/styled.rs` calls `MessageBanner::show_global(ui)` before screen content. All screens render through this function, providing a single consistent insertion point.

---

## 7. Usage Example: IdentitiesScreen

Demonstrates the `BannerHandle` + elapsed-time pattern for a long-running refresh operation:

```rust
// Struct: stores handle instead of separate refreshing/timestamp fields
refresh_banner: Option<BannerHandle>,

// Starting refresh (in ui()):
let handle = MessageBanner::set_global(ctx, "Refreshing identities...", MessageType::Info);
handle.with_elapsed();
self.refresh_banner = Some(handle);

// On success (in display_task_result() — no ctx parameter available):
if let Some(handle) = self.refresh_banner.take() {
    handle.clear();
}
MessageBanner::set_global(
    self.app_context.egui_ctx(),
    "Successfully refreshed identity",
    MessageType::Success,
);

// On error (in display_message() — side-effect only):
if let MessageType::Error = message_type
    && let Some(handle) = self.refresh_banner.take()
{
    handle.clear();
}
```

---

## 8. Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Global banners via egui context data | All screens render through `island_central_panel()`, making global placement consistent. Eliminates per-screen rendering boilerplate. |
| Multi-message support (max 5) | Backend tasks may produce multiple results; they should all be visible. Cap prevents unbounded growth. |
| Text-based dedup at creation, key-based lookup after | Prevents duplicate "Success" banners from rapid task completions while allowing handle-based text updates. |
| `BannerHandle` as `'static` struct | Handles can be stored in screen fields (e.g., `refresh_banner: Option<BannerHandle>`) without lifetime issues. |
| `elapsed()` looks up from context data | Avoids data redundancy — `created_at` lives only in `BannerState`, not duplicated on the handle. |
| All handle methods return `Option` | Handles may outlive their banners (dismissed, expired). Callers must handle the `None` case. |
| `Instant` for timing | Monotonic, immune to system clock changes. Correct for timeout and elapsed-time logic. |
| Per-instance implements `Component` trait | `DomainType = BannerStatus` allows screens to react to banner lifecycle events (dismiss, timeout) through the standard component interface. |
| All colors via `DashColors` methods | Zero hardcoded colors. Theme changes propagate automatically. |
| Retained `screen.display_message()` call | Screens still need side-effect hooks (reset step state, clear refresh handles). The global banner handles display; `display_message()` handles side-effects only. |

---

## 9. Migration Status

3 of ~50 screens migrated as proof-of-concept:

| Screen | Old Pattern | Migration Notes |
|--------|-------------|-----------------|
| `TopUpIdentityScreen` | Framed banner + Dismiss | Removed `error_message` field and 20-line Frame block. `display_message()` retains step-reset side-effect. |
| `IdentitiesScreen` | Timed badge with DateTime | Removed `backend_message` field and helpers. Uses `BannerHandle` with `with_elapsed()` for refresh tracking. |
| `MintTokensScreen` | Bare colored_label + status enum | Changed `ErrorMessage(String)` to `Error` (no payload). Banner displays via global API. |

Old and new patterns coexist without conflict because the global banner renders above screen content via `island_central_panel`.

---

## 10. Pre-Migration Analysis

Before the MessageBanner was implemented, the codebase had:

- **~50 screens** with no unified message rendering
- **4 distinct rendering patterns**: Framed Banner, Timed Badge, Status Enum, Bare colored_label
- **8+ different error colors**: `Color32::RED`, `DARK_RED`, `from_rgb(255,100,100)`, `from_rgb(220,80,80)`, `DashColors::error_color()`, `DashColors::ERROR`, etc.
- **Two competing `MessageType` enums**: active 3-variant in `mod.rs`, dead 4-variant in `theme.rs`

The dead `theme.rs::MessageType` was deleted. The active enum gained a `Warning` variant. Color methods were consolidated into `DashColors::message_color()` and `message_background_color()`. A new `info_color()` helper was added to complement the existing `error_color()`/`success_color()`/`warning_color()` methods.
