# Identity + DashPay Redesign Wireframe

This directory contains the design specification and interactive wireframe for the unified
Identities section of Dash Evo Tool 2. The redesign collapses the current two left-nav
entries — Dashpay and Identities — into a single **Identities** section with four tabs:
Home, Contacts, Activity, and Settings.

The critical distinction throughout this design: **Identity** is the primary on-chain Dash
Platform object (keys, DPNS usernames, credit balance). **Social profile** is optional
extended metadata — display name, bio, avatar — layered on top via a DashPay Profile
document. Many identities never have a social profile. The Contacts tab is gated on a social
profile existing. The nav label remains `Identities` (plural, unchanged from the codebase).

## How to view

Serve locally to avoid font-loading CORS restrictions:

```
cd docs/ai-design/2026-04-22-identity-dashpay-redesign
python3 -m http.server 8000
```

Then open `http://localhost:8000/wireframe.html` in Chromium or Firefox.

Google Fonts (Noto Sans) is loaded via `<link>`. If your network blocks it, the fallback
stack (system-ui / Segoe UI / Helvetica) takes over — the layout is unaffected.

## Controls

| Control | Location | What it does |
|---|---|---|
| Persona toggle | Top-right of page header | Switches between Alex / Priya / Jordan. Arrow-key navigation supported. Flips `body[data-persona]` which CSS uses to show/hide `.adv` (Priya+Jordan) and `.dev` (Jordan only) elements. Default: Alex. |
| Theme toggle | Top-right of page header | Flips `<html data-theme>` between `light` and `dark`. Updates `aria-pressed`. No persistence — resets on reload. |
| Advanced expanders | Inside frames | Native `<details>/<summary>` — click to expand/collapse. Frame 8 Advanced is open by default (Priya context). |

## Frames

| # | Caption |
|---|---|
| 1 | App chrome zoom — Alex (subdued wallet pill) and Priya (interactive wallet pill), one-row switcher layout |
| 2 | Onboarding empty state |
| 3 | Identity picker — grid shown when ≥ 2 identities are loaded (4 cards + add-new card) |
| 4 | Identity Home — Alex, social profile set |
| 5 | Identity Home — Priya, no social profile |
| 6 | Contacts tab — gated state (no social profile) |
| 7 | Activity tab |
| 8 | Send sheet — compose step |
| 9 | Settings — Priya, Advanced expanded |

## Placeholder token legend

No unresolved `{{PLACEHOLDER_TOKEN}}` strings remain in `wireframe.html`. Every dynamic
value from `design-spec.md` is rendered with representative sample data:

| Token in design-spec.md | Wireframe sample value | Design-spec section |
|---|---|---|
| `{amount}` | `2.450 DASH`, `0.500 DASH`, etc. | §B.2, §B.7 |
| `{handle}` | `@alex.dash`, `@priya.dash` | §A.2, §B.2 |
| `{wallet_name}` | `Main Wallet`, `Masternode Ops` | §A.3 |
| `{fiat_code}` | `USD` | §B.2, §B.7 |
| `{fiat_amount}` | `214.30`, `43.70` | §B.2, §B.7 |
| `{fee_amount}` | `0.00002 DASH` | §B.7 |
| `{total_amount}` | `0.50002 DASH` | §B.7 |
| `{credit_amount}` | `50,000 credits` | §B.7 |
| `{counterparty_name}` | `@carol.dash`, `@dave.dash` | §B.6, §B.7 |
| `{network_name}` | `Mainnet`, `Testnet` | §D tooltip 15 |
| `{max}` | `200` | §B.8 |
| `{reason}` | `voting` | §B.3 |

## Screenshot capture

Capture all 8 frames in light and dark mode using Playwright (requires `npx playwright`):

```
npx playwright screenshot \
  --full-page \
  http://localhost:8000/wireframe.html \
  wireframe-full.png
```

For individual frames at 1280x800, use the Playwright Node API targeting each
`section[aria-labelledby]` element, iterating over personas `alex`, `priya`, `jordan` and
themes `light`, `dark`. This produces up to 54 PNGs (9 frames x 3 personas x 2 themes).

## Design decisions recorded since initial commit

- **Shadow alphas**: wireframe shadow CSS values (`0.08`–`0.30`) are intentionally brighter
  than `theme.rs` egui alpha bytes (`8`–`30` / 255 ≈ `0.031`–`0.118`). The wireframe is the
  visual target; `theme.rs` needs updating in the implementation PR. See design-spec.md §E.
- **Secondary Home actions**: Add funds / Send to wallet / Send to another identity are
  visible to all personas, no `.adv` gate. Persona-specific funding paths are inside the
  wizard, not on the Home row. See design-spec.md §G9.
- **Local nickname vs. DPNS aliases**: `QualifiedIdentity.alias` is renamed `Local nickname`
  in Settings — not deprecated. See design-spec.md §G7.
- **Identity pill dropdown ordering**: Local nickname → DPNS username → shortened ID.
  Search activates at 7+ identities. Drag-reorder deferred. See design-spec.md §G6.
- **One-row switcher**: wallet pill and identity pill are displayed side by side (`flex-direction: row; flex-wrap: wrap`) rather than stacked. Degrades to two rows via `flex-wrap: wrap` at narrow widths. The picker page (F3) has no switcher — the grid itself is the selector.
- **Identity picker card heading hierarchy**: display name preferred over DPNS handle, which is preferred over shortened Identity ID. This matches the priority order already established for the pill dropdown (see §A.3 / §G6) and avoids a separate rule set.
- **Picker avatar sizing**: 72×72 px chosen as a midpoint between the 40 px contact list avatar and the 96 px hero avatar, giving enough surface for a legible monogram glyph without dominating the card at ≥ 260 px width.
- **Picker card hover elevation**: shadow increases from `--shadow-small` to `--shadow-medium` on hover — same elevation step used by all other interactive cards in the design. No border-color change on standard cards (the add-new card switches from dashed to solid Dash Blue instead, since that border is its defining visual element).

## Known limitations

- Static visual reference only — not a clickable prototype.
- No real network calls; all data is hard-coded sample values.
- Persona toggle and theme toggle work; tab switching and dropdown interactions do not.
- Send sheet Retry / Review flow is shown statically; button states are for illustration.
- Google Fonts require a network connection; system fallback activates offline.

## Links

- [design-spec.md](./design-spec.md) — full UX specification (IA, screens, wording audit,
  tooltip catalog, visual direction)
- [docs/personas/everyday-user.md](../../personas/everyday-user.md) — Alex Torres persona
- [docs/personas/power-user.md](../../personas/power-user.md) — Priya Nakamura persona
- [docs/personas/platform-developer.md](../../personas/platform-developer.md) — Jordan Kim
- [src/ui/theme.rs](../../../src/ui/theme.rs) — authoritative token source
  (DashColors, Spacing, Shape, Shadow, Typography)
- [docs/ux-design-patterns.md](../../ux-design-patterns.md) — UI/UX reference card
