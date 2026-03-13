# UX Design Patterns

Quick reference for Dash Evo Tool UI/UX conventions — **when and how** to use each pattern. For exact values (pixel sizes, hex codes, padding), refer to the source files listed below; this document explains usage, not implementation constants.

## 1. Design Tokens

All design tokens (spacing, typography, shape, shadows) are defined in `src/ui/theme.rs`. Refer to that file for exact values.

### Spacing (`Spacing`)

Exact values in `src/ui/theme.rs`. Use semantic tokens — never hardcode pixel values.

| Token | Usage                        |
|-------|------------------------------|
| XXS   | Tight inline gaps            |
| XS    | Icon spacing, minor gaps     |
| SM    | Default item spacing         |
| MD    | Section spacing, form gaps   |
| LG    | Large section spacing        |
| XL    | Section separation           |
| XXL   | Major layout spacing         |
| XXXL  | Page-level spacing           |

### Typography (`Typography`)

Exact sizes in `src/ui/theme.rs`.

| Token   | Helper              |
|---------|---------------------|
| XS      | `caption()`         |
| SM      | `body_small()`      |
| BASE    | `body()`, `button()`|
| LG      | `body_large()`      |
| XL      | `heading_small()`   |
| XXL     | `heading_medium()`  |
| XXXL    | `heading_large()`   |
| Display | `heading_xlarge()`  |

Font family: Noto Sans (proportional). Monospace: `monospace()` at BASE size.

### Border (`Shape`)

Exact values in `src/ui/theme.rs`.

| Token       | Usage                     |
|-------------|---------------------------|
| RADIUS_NONE | No rounding               |
| RADIUS_SM   | Banners, dialog buttons   |
| RADIUS_MD   | Buttons, cards, inputs    |
| RADIUS_LG   | Island panels             |
| RADIUS_XL   | Glass cards, hero sections|
| RADIUS_FULL | Pill / circular           |

Width tokens: `BORDER_WIDTH` (standard), `BORDER_WIDTH_THICK` (emphasis/focus).

## 2. Color System

All color constants and hex values are defined in `src/ui/theme.rs` (`DashColors`). Refer to that file for exact values.

- All colors via `DashColors` -- never hardcode `Color32` values
- Theme-aware getters: `text_primary(dark_mode)`, `background(dark_mode)`, `surface(dark_mode)`, `border(dark_mode)`, etc.
- Dark mode: lighter surfaces for elevation, not shadows

**Semantic color constants**: `DASH_BLUE`, `ERROR`, `WARNING`, `SUCCESS`, `INFO`, `DANGER_RED`, `VALIDATION_WARNING`, `DISABLED`

**Password strength colors**: `STRENGTH_WEAK`, `STRENGTH_FAIR`, `STRENGTH_GOOD`, `STRENGTH_STRONG`

**Additional semantic colors**: `DANGER_HOVER`, `BUTTON_DISABLED`, `WARNING_BRIGHT`, `PLATFORM_PURPLE`, `ACTION_BUTTON_BLUE`, `HIGHLIGHT_GOLD`

**Interactive state colors** (theme-aware): `hover(dark_mode)`, `pressed(dark_mode)`, `selected(dark_mode)`, `disabled(dark_mode)`

**Network accents** (`network_accent(network, dark_mode)`): per-network light/dark variants for Mainnet, Testnet, Devnet, Regtest

**Contrast**: WCAG AA minimum -- 4.5:1 normal text, 3:1 large text and UI components.

## 3. Buttons

Use `StyledButton` from `src/ui/components/styled.rs`. Never use bare `ui.button()`.

**Variants**: Primary, Secondary, Danger, Ghost. **Sizes**: Small, Medium, Large. See `StyledButton` for exact fill/stroke/padding/font values.

- Corner radius: `RADIUS_MD`
- Disabled fill: `DISABLED`
- Hover: pointing hand cursor
- Min click target: WCAG AA compliant
- Usage: `StyledButton::primary("Label").show(ui)`
- **Styling**: Use `ComponentStyles` helpers -- `primary_button_fill()`, `primary_button_text()`, `primary_button_stroke()`, `secondary_button_fill()`, `secondary_button_text()`, `secondary_button_stroke()`, `danger_button_fill()`, `danger_button_text()`. Never style buttons ad-hoc at call sites.

## 4. Dialogs and Modals

Reference: `ConfirmationDialog` in `src/ui/components/confirmation_dialog.rs`.

- **Button order**: right-to-left layout -- Confirm RIGHT, Cancel LEFT
- Overlay: `modal_overlay()`
- Layout constants (corner radius, margins, min width, button size) defined in `ConfirmationDialog`
- Escape = cancel. X button = cancel.
- **Destructive actions**: `danger_mode(true)` -- red confirm button. Use specific verb labels ("Delete wallet" not "OK").
- For critical/irreversible: require type-to-confirm

## 5. Forms and Inputs

Reference: `AmountInput` in `src/ui/components/amount_input.rs`.

- Labels above inputs
- Validation on blur, not on every keystroke
- Errors inline below field in `VALIDATION_WARNING` color
- Invalid input sets domain value to `None`
- Input border: unfocused (`input_stroke()`), focused (`input_stroke_focused()`), validation error (`input_stroke_error()`)
- Styled text edits: `styled_text_edit_singleline()` / `styled_text_edit_multiline()`
- **Input border helpers**: `ComponentStyles::input_stroke()`, `input_stroke_focused()`, `input_stroke_error()` are the canonical helpers -- never construct input strokes manually
- Follow Component pattern: lazy init, private fields, builder API, `ComponentResponse` trait (see `docs/COMPONENT_DESIGN_PATTERN.md`)

### Password Inputs

Reference: `src/ui/components/wallet_unlock.rs`.

- Mask input: `TextEdit::singleline(&mut text).password(!show_password)`
- Hold-to-reveal: use `egui::Button` with eye icon; toggle via `ui.input(|i| i.pointer.any_pressed())` -- show cleartext only while pressed
- Password strength: display colored bar using `STRENGTH_WEAK`/`FAIR`/`GOOD`/`STRONG` colors
- Show validation errors only after user interaction, never on initial focus for untouched fields
- See `docs/ai-design/2026-03-09-password-input/ux-spec.md` for full spec

## 6. Messages and Errors

Reference: `MessageBanner` in `src/ui/components/message_banner.rs`.

- Use `MessageBanner::set_global()` -- never raw error strings in UI
- Error/Warning: persistent, manual dismiss only
- Success/Info: auto-dismiss (timeout defined in `MessageBanner`)
- Max stacked banners defined in `MessageBanner` (oldest evicted)
- User-friendly message + technical details via `.with_details(e)`
- Recovery suggestion via `.with_suggestion("...")`
- Structure: [What happened] + [What to do]. No blame language.
- MessageBanner logs automatically -- no additional logging needed
- Progress banners: `.with_elapsed()` for long-running operations
- Cleanup: `option_banner.take_and_clear()` (not `= None`)

## 7. Tables and Lists

- Text columns: left-aligned. Number/currency: right-aligned. Actions: center/right.
- Use `egui_extras::TableBuilder` with `Column` definitions
- Alternating rows: `DashColors::stripe(dark_mode)` for tables > 5 rows
- Sortable column headers where applicable

## 8. Loading and Progress

| Duration | Indicator                                       |
|----------|-------------------------------------------------|
| < 1s     | None                                            |
| 1-10s    | `egui::Spinner` + descriptive text              |
| > 10s    | Progress banner with `.with_elapsed()`          |

- Disable triggering action during load (prevent double-submit)

## 9. Navigation

- Top panel: breadcrumb location + network indicator + connection status
- Left panel: icon-based section menu, nested submenus
- Screen stack: `AppAction::PushScreen` / `PopScreen` for modal/detail screens
- Root screens persist in `AppState.main_screens` (BTreeMap by `RootScreenType`)

## 10. Keyboard and Accessibility

| Key            | Action                          |
|----------------|---------------------------------|
| Enter          | Submit form / confirm dialog    |
| Escape         | Dismiss modal / cancel          |
| Tab/Shift+Tab  | Focus traversal (layout order)  |

- Focus indicator: `BORDER_WIDTH_THICK`, WCAG 3:1 contrast ratio
- All interactive elements: WCAG AA compliant click targets
- Note: egui has limited a11y support -- no screen reader annotations available

### Cursor Icons and Tooltips

Use `ResponseExt` methods (defined in `src/ui/theme.rs`) instead of raw `.on_hover_text()`. These enforce the correct cursor automatically.

| Method | Cursor | When to use |
|---|---|---|
| `.clickable_tooltip("text")` | `PointingHand` | Interactive elements with click handlers (buttons, links, clickable labels) |
| `.info_tooltip("text")` | `Help` (?) | Non-interactive elements showing explanatory text (status labels, settings, column headers) |
| `.disabled_tooltip("text")` | `NotAllowed` | Disabled elements explaining why the action is unavailable |

- Import: `use crate::ui::theme::ResponseExt;`
- `ComponentStyles` button constructors (`add_primary_button`, etc.) set `PointingHand` automatically -- no `clickable_tooltip` needed for the cursor. You still need `disabled_tooltip` when the button can be disabled
- Never use bare `.on_hover_text()` or `.on_disabled_hover_text()` -- always use the `ResponseExt` methods above
- `ResponseExt` is the general extension point for `egui::Response` behavior policies; future methods may enforce hover effects, accessibility, or other conventions

## 11. Progressive Disclosure

Reference personas in `docs/personas/`.

| Persona         | Visibility                                    |
|-----------------|-----------------------------------------------|
| Alex (everyday) | Default view -- essential features only       |
| Priya (power)   | Expand/collapse per section, not global toggle|
| Jordan (dev)    | Behind developer mode setting                 |

- Hide features used by <20% of users behind expandable sections
- Always visible: balance, send/receive, transaction history, navigation

## 12. Responsive Layout

- `island_central_panel()` for main content with responsive margins (breakpoints defined in implementation)
- Use `ui.available_width()` for adaptive layouts
- `ScrollArea` for overflow content
- Modal windows resizable for content-heavy dialogs

## 13. Shadows

Reference: `Shadow` in `src/ui/theme.rs`. Tokens and their exact values defined there.

| Token       | Usage                        |
|-------------|------------------------------|
| `small()`   | Subtle depth                 |
| `medium()`  | Cards, panels                |
| `large()`   | Elevated elements            |
| `elevated()`| Major cards, hero panels     |
| `inner()`   | Glass morphism (white tint)  |
| `glow()`    | Primary element emphasis     |

Dark mode uses lighter surfaces for elevation rather than shadows.

---

## See Also

- `docs/COMPONENT_DESIGN_PATTERN.md` -- component implementation pattern
- `docs/personas/` -- user personas and progressive disclosure model
- `src/ui/theme.rs` -- design token definitions (`DashColors`, `Spacing`, `Typography`, `Shape`)
- `src/ui/components/styled.rs` -- button and card components
- `src/ui/components/message_banner.rs` -- message system
- `src/ui/components/confirmation_dialog.rs` -- dialog pattern
- `src/ui/components/wallet_unlock.rs` -- password input pattern
- `docs/ai-design/2026-03-09-password-input/ux-spec.md` -- password input UX spec
