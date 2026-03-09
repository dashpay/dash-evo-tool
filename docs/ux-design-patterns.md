# UX Design Patterns

Quick reference for Dash Evo Tool UI/UX conventions. All tokens defined in `src/ui/theme.rs`.

## 1. Design Tokens

### Spacing (`Spacing`)

| Token | px  | Usage                        |
|-------|-----|------------------------------|
| XXS   | 2   | Tight inline gaps            |
| XS    | 4   | Icon spacing, minor gaps     |
| SM    | 8   | Default item spacing         |
| MD    | 16  | Section spacing, form gaps   |
| LG    | 24  | Large section spacing        |
| XL    | 32  | Section separation           |
| XXL   | 48  | Major layout spacing         |
| XXXL  | 64  | Page-level spacing           |

Button padding: Small `16x8`, Medium `24x12` (default), Large `32x16`. Card padding: 20px. Form spacing: `16x8`.

### Typography (`Typography`)

| Token   | Size | Helper              |
|---------|------|---------------------|
| XS      | 12   | `caption()`         |
| SM      | 14   | `body_small()`      |
| BASE    | 16   | `body()`, `button()`|
| LG      | 18   | `body_large()`      |
| XL      | 20   | `heading_small()`   |
| XXL     | 24   | `heading_medium()`  |
| XXXL    | 30   | `heading_large()`   |
| Display | 36   | `heading_xlarge()`  |

Font family: Noto Sans (proportional). Monospace: `monospace()` at BASE size.

### Border (`Shape`)

| Token       | Value | Usage                    |
|-------------|-------|--------------------------|
| RADIUS_NONE | 0     | No rounding              |
| RADIUS_SM   | 6     | Banners, dialog buttons  |
| RADIUS_MD   | 12    | Buttons, cards, inputs   |
| RADIUS_LG   | 16    | Island panels            |
| RADIUS_XL   | 20    | Glass cards, hero sections|
| RADIUS_FULL | 255   | Pill / circular          |

Border widths: 1px standard (`BORDER_WIDTH`), 2px emphasis/focus (`BORDER_WIDTH_THICK`).

## 2. Color System

- All colors via `DashColors` -- never hardcode `Color32` values
- Theme-aware getters: `text_primary(dark_mode)`, `background(dark_mode)`, `surface(dark_mode)`, `border(dark_mode)`, etc.
- Dark mode: lighter surfaces for elevation, not shadows

| Semantic      | Constant          | Hex       |
|---------------|-------------------|-----------|
| Brand         | `DASH_BLUE`       | `#008de4` |
| Error         | `ERROR`           | `#eb5757` |
| Warning       | `WARNING`         | `#f1c40f` |
| Success       | `SUCCESS`         | `#27ae60` |
| Info          | `INFO`            | `#3498db` |
| Danger button | `DANGER_RED`      | `#c83c3c` |
| Validation    | `VALIDATION_WARNING`| `#ff9664`|
| Disabled      | `DISABLED`        | `#bdc3c7` |

**Network accents** (`network_accent(network, dark_mode)`):

| Network  | Light         | Dark          |
|----------|---------------|---------------|
| Mainnet  | `DASH_BLUE`   | `DASH_BLUE_DARK` |
| Testnet  | `TESTNET_ORANGE` | `TESTNET_ORANGE_DARK` |
| Devnet   | `DEVNET_RED`  | `DEVNET_RED_DARK` |
| Regtest  | `REGTEST_BROWN` | `REGTEST_BROWN_DARK` |

**Contrast**: WCAG AA minimum -- 4.5:1 normal text, 3:1 large text and UI components.

## 3. Buttons

Use `StyledButton` from `src/ui/components/styled.rs`. Never use bare `ui.button()`.

| Variant   | Fill            | Text color | Stroke            |
|-----------|-----------------|------------|-------------------|
| Primary   | `DASH_BLUE`     | White      | None              |
| Secondary | White/surface   | `DASH_BLUE`| 1px `DASH_BLUE`   |
| Danger    | `ERROR`         | White      | None              |
| Ghost     | Transparent     | text_primary| None             |

| Size   | Padding  | Font size |
|--------|----------|-----------|
| Small  | 12x6     | 14 (SM)   |
| Medium | 16x8     | 16 (BASE) |
| Large  | 20x10    | 18 (LG)   |

- Corner radius: `RADIUS_MD` (12px)
- Disabled fill: `DISABLED`
- Hover: pointing hand cursor
- Min click target: 32x32px (WCAG AA), prefer 44x44px
- Usage: `StyledButton::primary("Label").show(ui)`

## 4. Dialogs and Modals

Reference: `ConfirmationDialog` in `src/ui/components/confirmation_dialog.rs`.

- **Button order**: right-to-left layout -- Confirm RIGHT, Cancel LEFT
- Overlay: `modal_overlay()` (black, alpha 120)
- Corner radius: 8px. Inner margin: 16px. Min width: 300px.
- Min button size: 80x32px
- Escape = cancel. X button = cancel.
- **Destructive actions**: `danger_mode(true)` -- red confirm button. Use specific verb labels ("Delete wallet" not "OK").
- For critical/irreversible: require type-to-confirm

## 5. Forms and Inputs

Reference: `AmountInput` in `src/ui/components/amount_input.rs`.

- Labels above inputs
- Validation on blur, not on every keystroke
- Errors inline below field in `VALIDATION_WARNING` color
- Invalid input sets domain value to `None`
- Input border: 1px unfocused (`input_stroke()`), 2px focused in `DASH_BLUE` (`input_stroke_focused()`), 2px `ERROR` on validation failure (`input_stroke_error()`)
- Styled text edits: `styled_text_edit_singleline()` / `styled_text_edit_multiline()`
- Follow Component pattern: lazy init, private fields, builder API, `ComponentResponse` trait (see `docs/COMPONENT_DESIGN_PATTERN.md`)

## 6. Messages and Errors

Reference: `MessageBanner` in `src/ui/components/message_banner.rs`.

- Use `MessageBanner::set_global()` -- never raw error strings in UI
- Error/Warning: persistent, manual dismiss only
- Success/Info: auto-dismiss after 5s
- Max 5 stacked banners (oldest evicted)
- User-friendly message + technical details via `.with_details(e)`
- Recovery suggestion via `.with_suggestion("...")`
- Structure: [What happened] + [What to do]. No blame language.
- MessageBanner logs automatically -- no additional logging needed
- Progress banners: `.with_elapsed()` for long-running operations
- Cleanup: `option_banner.take_and_clear()` (not `= None`)

## 7. Tables and Lists

- Text columns: left-aligned. Number/currency: right-aligned. Actions: center/right.
- Use `egui_extras::TableBuilder` with `Column` definitions
- Row height: 30-40px
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

- Focus indicator: 2px solid, 3:1 contrast ratio
- All interactive elements: click targets >= 32x32px
- Note: egui has limited a11y support -- no screen reader annotations available

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

- `island_central_panel()` for main content with responsive margins
- Width > 1200px: 24px margins. Width <= 1200px: 20px minimum.
- Use `ui.available_width()` for adaptive layouts
- `ScrollArea` for overflow content
- Modal windows resizable for content-heavy dialogs

---

## See Also

- `docs/COMPONENT_DESIGN_PATTERN.md` -- component implementation pattern
- `docs/personas/` -- user personas and progressive disclosure model
- `src/ui/theme.rs` -- design token definitions (`DashColors`, `Spacing`, `Typography`, `Shape`)
- `src/ui/components/styled.rs` -- button and card components
- `src/ui/components/message_banner.rs` -- message system
- `src/ui/components/confirmation_dialog.rs` -- dialog pattern
