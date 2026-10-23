# Send Screen Redesign (2026-10-23)

This is a proposal for simplifying Wallet › Send. It is design only; no code has changed.

| File | Contents |
|---|---|
| [01-requirements.md](01-requirements.md) | Inventory of today's screen, UX findings, requirements `SR-xx`, acceptance criteria, deviations `D-xx`, dependencies |
| [02-ux-spec.md](02-ux-spec.md) | Anatomy, option catalogue, Auto resolution rules, defaults, Max, Summary, error copy, locked/network states, presets, disclosure, responsive layout, accessibility, edge cases |
| [03-wireframes.md](03-wireframes.md) | ASCII wireframes W1–W14 |

User stories: SND-019 … SND-022 in [`docs/user-stories.md`](../../user-stories.md) (`[Gap]`).

Related open PRs: #1107 (quote-based Max, pooled Core accounts) and #1106 (Core→Shielded Max drain, "Finish now").

## Open questions

| # | Question | Recommendation |
|---|---|---|
| Q1 | Should To open on **Someone else** (empty address field) rather than "current wallet + Core/Platform"? (D-07) | **Yes.** With the literal default plus Auto, the form starts as a send to yourself, and Send is enabled as soon as an amount is typed |
| Q2 | Should Advanced drop per-address amounts and keep only "tick the addresses to spend from"? (D-11) | **Yes.** It is the most error-prone part of today's screen; one Amount field is enough |
| Q3 | Should Auto never spend identity credits, and spend private (shielded) funds only for private destinations? (D-05) | **Yes.** Both would surprise users: identities pay for usernames, and unshielding reveals amounts |
| Q4 | When no single balance covers the amount, should Auto offer the largest single-source amount rather than splitting across Core and Platform? (D-06) | **Yes, no split.** A split is two payments that can partly fail |
| Q5 | Should Advanced and "+ Add another recipient" show only in Expert/Developer view, with the fee choice reduced to Sender / Recipient? (D-12, D-13) | **Yes.** They are used by few people, and "first/last input/output" means nothing to most users |
