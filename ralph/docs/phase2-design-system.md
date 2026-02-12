# Phase 2: Design System & App Shell — Decisions & Audit

## 2.1 Layout, Navigation & Visual Language Design (Run 29)

### Overall Layout: Three-Panel "Island" Design

Preserves the existing egui layout structure with modern web refinements:

```
+----------------------------------------------------------+
| Background (muted gray/dark)                              |
| +-----+ +---------------------------------------------+  |
| |     | | Top Bar (island)                             |  |
| |     | | [*] DashPay > Contacts    [Add] [Contracts]  |  |
| | Nav | +---------------------------------------------+  |
| |     | |                                             |  |
| | H   | | +------+ +----------------------------+    |  |
| | U   | | | Sub  | | Main Content (island)      |    |  |
| | I   | | | Nav  | |                            |    |  |
| | C   | | |      | |  Screen content here       |    |  |
| | T   | | |      | |                            |    |  |
| | W   | | +------+ +----------------------------+    |  |
| | S   | |                                             |  |
| |     | +---------------------------------------------+  |
| | NET |                                                   |
| +-----+                                                   |
+----------------------------------------------------------+
```

**Left Sidebar (fixed, 72px collapsed / 200px expanded):**
- 7 navigation items with Lucide icons + labels
- Items: DashPay, Identities, Contracts, Tokens, Wallets, Tools, Settings
- Active item highlighted with Dash Blue accent + white icon
- Network badge at bottom (Testnet/Devnet/Local — hidden on Mainnet)
- Developer mode badge when enabled
- Dash logo at very bottom (clickable -> dash.org)
- Collapsible to icon-only mode on narrow viewports (<1024px)

**Top Bar (sticky, within right content area):**
- Left: Connection status indicator (pulsating green dot when connected to Core, static red when disconnected, clickable to start Dash-Qt)
- Left: Breadcrumb navigation (e.g., "DashPay > Contacts")
- Right: Context-sensitive action buttons (network-accent colored)
- Right: Grouped dropdown menus for multi-action areas (Contracts, Documents)
- Network badge pill showing current network name + color

**Sub-Navigation Panel (conditional, 220px):**
- Appears for screens with subscreens: DashPay (4 tabs), DPNS (4 tabs), Tokens (3 tabs), Tools (9 items), Contracts (Document Query + DPNS tabs)
- Vertical list of sub-items with active highlighting
- Implemented as a secondary sidebar within the content area

**Main Content Area:**
- "Island" card with rounded corners (radius-lg), subtle border, elevated shadow
- Surface background (white light / dark-gray dark)
- Padding: 24px (lg)
- Scrollable content within the island

### Navigation Architecture

**Routing: @tanstack/react-router (file-based)**
- Root layout: `/_app` (sidebar + top bar + outlet)
- Main sections: `/dashpay`, `/identities`, `/contracts`, `/tokens`, `/wallets`, `/tools`, `/settings`
- Sub-routes: `/dashpay/contacts`, `/dashpay/profile`, `/tokens/search`, `/tools/platform-info`, etc.
- Modal/overlay screens: Route-based modals via `@tanstack/react-router` modal routes or React portals
- Screen stack behavior: preserved via router history (back button works)

**Navigation State:**
- Active section determined by current route path
- Breadcrumbs auto-generated from route hierarchy
- Context actions per route defined in route metadata

### Color System (Dash Brand + shadcn/ui)

**Override shadcn's default OKLCH neutral palette with Dash brand colors:**

**Brand Colors:**
- `--dash-blue`: #008de4 (primary action, links, active states)
- `--dash-deep-blue`: #012060 (gradient end, emphasis)
- `--dash-midnight`: #0b0f3b (darkest accent)

**Semantic Colors (mapped to CSS variables):**
- `--primary`: Dash Blue (#008de4) — replaces shadcn's neutral primary
- `--primary-foreground`: White
- `--destructive`: Error Red (#eb5757)
- `--success`: Green (#27ae60) — custom addition
- `--warning`: Orange (#f1c40f) — custom addition
- `--info`: Blue (#3498db) — custom addition

**Network Accent Colors:**
- Mainnet: Dash Blue (#008de4 / #0071b6 dark)
- Testnet: Orange (#ffa500 / #cc8400 dark)
- Devnet: Dark Red (#8b0000 / #6f0000 dark)
- Local/Regtest: Brown (#8b4513 / #6f370f dark)
- Applied to: top bar action buttons, active nav highlights, network badges

**Light Mode:**
- Background: #f0f2f7 (soft blue-gray)
- Surface: #ffffff
- Input bg: #f8fafc
- Border: #e2e8f0 (light), #f0f5fb (very light)
- Text primary: #111921
- Text secondary: #64788c

**Dark Mode:**
- Background: #121212
- Surface: #202020
- Input bg: #282828
- Border: #3c3c3c (normal), #323232 (light)
- Text primary: #f0f0f0
- Text secondary: #a0a0a0

### Typography (Noto Sans, shadcn defaults + overrides)

**Font Family:** "Noto Sans", system-ui, sans-serif (matching egui's Noto Sans)
**Monospace:** "JetBrains Mono", ui-monospace, monospace (for JSON, hex, code display)

**Scale (matching egui theme.rs):**
- `text-xs`: 12px — captions, badges
- `text-sm`: 14px — secondary text, table cells
- `text-base`: 16px — body text, inputs, buttons
- `text-lg`: 18px — large body
- `text-xl`: 20px — section headings
- `text-2xl`: 24px — page headings
- `text-3xl`: 30px — display headings
- `text-4xl`: 36px — hero/display

### Spacing Scale (matching egui Spacing constants)

- `space-0.5`: 2px (xxs)
- `space-1`: 4px (xs)
- `space-2`: 8px (sm)
- `space-4`: 16px (md)
- `space-6`: 24px (lg)
- `space-8`: 32px (xl)
- `space-12`: 48px (xxl)
- `space-16`: 64px (xxxl)

### Border Radii

Override shadcn's `--radius: 0.625rem` to match egui:
- `rounded-sm`: 6px
- `rounded-md`: 12px
- `rounded-lg`: 16px (island panels, cards)
- `rounded-xl`: 20px
- `rounded-full`: 9999px (pills, badges)

### Shadow System (matching egui Shadow struct)

- `shadow-sm`: 0 2px 4px rgba(0,0,0,0.03) — subtle elements
- `shadow-md`: 0 4px 12px rgba(0,0,0,0.05) — popups, dropdowns
- `shadow-lg`: 0 8px 24px rgba(0,0,0,0.06) — large panels
- `shadow-elevated`: 0 12px 32px rgba(0,0,0,0.07) — island panels, cards
- `shadow-glow`: 0 0 20px rgba(0,141,228,0.12) — primary element glow

### Content Layout Patterns

**List View (Identities, Wallets, Tokens, Contacts):**
- Sortable data table via @tanstack/react-table
- Column headers with sort indicators
- Row hover state with subtle highlight
- Row actions via context menu (right-click) or action column (kebab menu)
- Alternating row stripe for readability
- Empty state: centered illustration + message + CTA button

**Detail View (Identity detail, Wallet detail):**
- Header with title + status badge + action buttons
- Tabbed content sections
- Key-value display grid for metadata
- Collapsible sections for advanced info

**Form/Wizard View (Create wallet, Register identity, Token creator):**
- Multi-step wizard with step indicators
- Form fields with inline validation (red border + error text)
- Required field indicators
- Submit button disabled until valid
- Loading state on submission

**Action Screen (Send payment, Top up, Transfer):**
- Input form (amount, destination, options)
- Fee preview section
- Wallet unlock step (if needed)
- Confirmation step with summary
- Progress indicator during broadcast
- Success/error result with details

### Modal/Dialog Patterns

**Confirmation Dialog (shadcn AlertDialog):**
- Semi-transparent overlay backdrop (rgba(0,0,0,0.47))
- Centered card with title, message, and action buttons
- Confirm (primary or destructive) + Cancel buttons
- Escape key dismisses
- Focus trapped within dialog

**Wallet Unlock Popup (shadcn Dialog):**
- Password input with show/hide toggle
- Wallet name displayed
- Error message on failed attempt with hint
- Auto-focus on password field
- Enter key submits, Escape cancels
- Password zeroized on close (security)

**Fee Confirmation Dialog (shadcn Dialog):**
- Fee breakdown table (base fee, multiplier, total)
- Confirm + Cancel buttons
- Identity/wallet context shown

**Toast Notifications (shadcn Sonner):**
- Bottom-right position
- Auto-dismiss: 5s for success/info, persistent for errors
- Types: success (green), error (red), warning (amber), info (blue)
- Dismissible by click

### Loading & Error States

**Loading:**
- Skeleton loader for initial data fetch (shimmer effect)
- Inline spinner for in-progress actions (button spinner)
- Full-page spinner for app initialization
- Progress bar for known-duration operations (SPV sync)

**Error:**
- Inline error messages (red text below inputs)
- Error banners (red background + icon at top of content area)
- Error toast for async operation failures
- Expandable error details (technical info collapsed by default)

**Empty States:**
- Centered layout with muted icon + descriptive text + action button
- e.g., "No wallets yet" -> "Create Wallet" button
- e.g., "No identities loaded" -> "Add Identity" button

### Responsive Behavior (Desktop-First)

- >=1280px: Full layout (sidebar expanded 200px + sub-nav 220px + content)
- 1024-1279px: Sidebar collapsed to icons (72px), sub-nav as overlay/sheet
- <1024px: Sidebar as hamburger drawer, sub-nav integrated into content
- Minimum supported width: 800px (Tauri window minimum)

### Accessibility

- All interactive elements keyboard-focusable (tab order)
- ARIA labels on icon-only buttons
- Focus visible indicator (ring) on all focusable elements
- Color contrast >=4.5:1 for text (WCAG AA)
- Role attributes on navigation, main content, dialogs
- Skip-to-content link
- Screen reader announcements for toasts and status changes

---

## 2.7 App Shell & Design System Quality Audit (Run 41)

### Overall Assessment: SOLID (B+)
322 tests pass, lint clean, typecheck clean. Good foundation with proper ARIA attributes,
semantic HTML, focus management in dialogs, and comprehensive theme variable system.

### Issues Found:

**Accessibility (2 issues):**
1. WalletUnlockDialog.tsx:133 — password visibility toggle has `tabIndex={-1}`, removing it
   from keyboard tab order. Users cannot reach show/hide password via keyboard.
2. Light mode color contrast: `--muted-foreground` (#64788c) on `--muted` (#f8fafc) background
   = 4.36:1, below WCAG AA 4.5:1 minimum. On white bg = 4.56:1 (barely passes).
   Darkening to ~#5a6d80 would achieve ~5.1:1 on muted bg.

**Bug (1 issue):**
3. DesignSystem.tsx:326 — EmptyState uses non-existent `action` prop instead of correct
   `actionLabel` + `onAction` props. Button silently doesn't render. TypeScript doesn't
   catch it because JSX allows extra props.

**No Issues (areas that passed):**
- Dark/light theme visual consistency: All CSS variables properly dual-defined
- Dialog keyboard handling: Escape via Radix, Enter key in WalletUnlockDialog
- Screen reader support: role="status", role="alert", role="navigation", role="main", aria-current, aria-expanded, aria-invalid, aria-describedby all properly used
- Component API consistency: Dialogs use onOpenChange+onResult, inputs use onChange, actions use onClick — patterns are coherent
- NetworkChooserScreen password toggle: Has proper aria-label (lines 531-533)
- Auto-focus in WalletUnlockDialog: Working via useEffect + setTimeout
- Test coverage: 322 tests across 23 test files — every component has tests
