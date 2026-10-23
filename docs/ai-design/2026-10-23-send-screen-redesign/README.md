# Send Screen Redesign (2026-10-23)

This is a proposal for simplifying Wallet › Send. It is design only; no code has changed.

| File | Contents |
|---|---|
| [01-requirements.md](01-requirements.md) | Inventory of today's screen, UX findings, requirements `SR-xx`, acceptance criteria, deviations `D-xx`, dependencies |
| [02-ux-spec.md](02-ux-spec.md) | Anatomy, option catalogue, Auto resolution rules, defaults, Max, Summary, error copy, locked/network states, presets, disclosure, responsive layout, accessibility, edge cases |
| [03-wireframes.md](03-wireframes.md) | ASCII wireframes W1–W14 |
| [wireframes.html](wireframes.html) | The same W1–W14 rendered as interactive HTML mock-ups (open in a browser) |

User stories: SND-019 … SND-022 in [`docs/user-stories.md`](../../user-stories.md) (`[Gap]`).

Related open PRs: #1107 (quote-based Max, pooled Core accounts) and #1106 (Core→Shielded Max drain, "Finish now").

## Decisions (resolved open questions)

| # | Question | User decision |
|---|---|---|
| Q1 | Should To open on **Someone else** rather than "current wallet + Core/Platform"? (D-02, D-07) | **Yes.** Someone else / My wallets switch; the default is Someone else |
| Q2 | Should Advanced drop per-address amounts and use one Amount field? (D-11) | **Yes** |
| Q3 | Should Auto never spend identity credits, and spend shielded funds only for shielded destinations? (D-05) | **Yes** |
| Q4 | Should Auto never split a payment across sources and offer the largest single-source amount instead? (D-06) | **Yes** |
| Q5 | Should Advanced and multiple recipients be Expert/Developer only, and should fee strategies be reduced? (D-12, D-13) | **Expert-only: yes. Fee strategies: keep all 4.** They sit in a collapsed "Fee options ▸" row in the Summary card, with the default unchanged (SR-22) |
