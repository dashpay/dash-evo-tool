# Identity Home — action set redesign (2026-07-24)

Design record for collapsing the Identity Home tab's two action rows (6 buttons,
4 destinations) into one non-redundant row of 4. Read-only analysis; no code
changed by this note.

Source of truth reviewed: `src/ui/identity/home.rs`,
`src/ui/identities/{top_up_identity_screen,transfer_screen,withdraw_screen}`,
`docs/personas/everyday-user.md`, `docs/user-stories.md`,
`docs/ux-design-patterns.md`, `docs/ai-design/2026-04-22-identity-dashpay-redesign/design-spec.md` §B.2/§B.7/§B.9.

## 1. What is actually there today

`home_button_kind()` (home.rs:174) maps six rendered buttons onto four screens:

| Rendered button | Style | Opens |
|---|---|---|
| `Send` | primary blue | `TransferScreen` |
| `Receive` | primary blue | `TopUpIdentityScreen` |
| `Add contact` | outlined | Contacts tab (gated on social profile) |
| `Add funds` | ghost | `TopUpIdentityScreen` — same as `Receive` |
| `Send to wallet` | ghost | `WithdrawalScreen` — the only unique entry in its row |
| `Send to another identity` | ghost | `TransferScreen` — same as `Send` |

Two exact duplicate pairs, and the pairs are rendered at *different visual
weights*, so the same destination is simultaneously advertised as the most and
the least important thing on the screen.

### Why the duplication exists

Design-spec §B.2 specified the quick-actions row as **payment** affordances:

- `Send` → "Send Dash to a contact, username, or address." — the §B.7 Send sheet
  (recipient field with DPNS lookup). **Not implemented.** The only payment path
  that exists is per-contact (`Contacts` → row → `Pay` → `DashPaySendPayment`,
  which requires a `to_contact_id` and so cannot serve a recipient-less entry
  point).
- `Receive` → "Show a QR code or your username so someone can pay you." — a
  receive view. **Not implemented, and not implementable as specified**: an
  identity cannot be paid directly. Funding is always self-initiated from your
  own wallet via an asset lock (IDN-004). The nearest real capability, IDN-014
  "Receive a new deposit", derives a **wallet** address, so the funds land in
  the wallet and the user still has to finish the top-up wizard.

Rather than mark those two as gaps, the implementation re-pointed them at the
funding screens and rewrote the tooltips to describe the new behaviour (the
`T30` comments at home.rs:335 and :348). The secondary row then landed the
*correctly labelled* funding actions on top, producing the duplication.

## 2. Jobs to be done from this screen

An identity is an account whose balance is fuel for Platform operations. From
Home, its owner wants exactly four things:

1. **Put value in** — "I want to keep using Platform features." → `TopUp`
   (from wallet balance, a Platform address, an existing funding transaction, or
   a fresh deposit — all four are methods *inside* the one wizard).
2. **Take value out to Dash I can spend** — → `Withdrawal` (queued on Platform,
   settles to a Core address).
3. **Move value to another Platform participant** — → `Transfer` (destination
   toggle: another identity, or a Platform address).
4. **Connect with people** — → Contacts tab.

There is no fifth job. "Receive" is not a job an identity can do.

Frequency, for the Everyday User (Alex): fees are consumed continuously, so
topping up is the recurring action; withdrawing is rare but emotionally
load-bearing ("can I get my money back?"); identity→identity transfer is the
rarest. Hierarchy follows that ordering.

## 3. Recommended action set

One row, four buttons, four destinations — read left to right as money in →
money out → social.

| # | Label | Style | Destination | Tooltip |
|---|---|---|---|---|
| 1 | `Add funds` | Primary | `HomeScreenKind::TopUp` | `Move Dash from your wallet into this identity.` |
| 2 | `Send to wallet` | Secondary | `HomeScreenKind::Withdrawal` | `Move Dash out of this identity to a Dash address, such as one from your wallet.` |
| 3 | `Send to another identity` | Secondary | `HomeScreenKind::Transfer` | `Send Dash from this identity to another identity. You can also send to a Platform address.` |
| 4 | `Add contact` | Secondary | Contacts tab | unchanged (enabled + disabled copy both stay as-is) |

```
[ Add funds ]  [ Send to wallet ]  [ Send to another identity ]  [ Add contact ]
   primary          secondary              secondary                secondary
   money in        money out (L1)       money out (Platform)         social
```

- `Send` and `Receive` are **removed**, not renamed. Their `HomeButton` variants
  (`Send`, `Receive`) go with them.
- One primary only. Two blue buttons implied two equally-weighted entry points
  where in fact one was a duplicate of a ghost button.
- Use `StyledButton` / `ComponentStyles` (ux-design-patterns §3) rather than the
  ad-hoc `egui::Button` builders `primary_quick_action` / `ghost_action`; that
  also resolves the 40 px vs 36 px height mismatch between the current rows.
- Render with `ui.horizontal_wrapped` so four ~150 px buttons wrap instead of
  clipping on a narrow window.

### Is a bare `Send` still meaningful?

Not today. With `Send to wallet` and `Send to another identity` both present and
honestly labelled, a third bare `Send` answers neither "send what" nor "send
where", and its two candidate meanings live on two different screens with
different mechanics (a queued Core withdrawal vs an instant Platform transfer).

A bare `Send` becomes correct only under one condition: a single screen that
accepts any destination from an identity source. That screen already exists —
`WalletSendScreen` supports `SourceSelection::Identity` with `AddressKind::Core`
("Withdraw Credits"), `::Platform` ("Transfer to Address") and `::Identity`
("Transfer Credits"), auto-detecting the kind from what the user pastes. It
lacks only an identity-source preset (`WalletSendScreen::new` hard-codes
`SourceSelection::CoreWallet` and takes a `Wallet`). When that preset lands,
rows 2 and 3 collapse into one `Send` — `Send Dash from this identity. Paste a
Dash address, a Platform address, or an identity.` That is a follow-up, not part
of this fix.

### What replaces "Receive"

Nothing, at identity level. The deposit-QR capability stays where it already is
and works: the `Receive a new deposit` method inside the Add funds wizard, which
is always offered and needs no pre-existing balance. Surfacing it as a top-level
`Receive` would teach the wrong model — that people can pay an identity
directly — and would collide with the Wallet screen's `Receive`, which opens an
address + QR dialog. The redesign spec already reached the same conclusion for
the empty-wallet banner in §B.1: *"this app has no separate top-level Receive
screen, so the link goes to Wallets, where the user's receiving address lives."*

## 4. Persona walk-through of the proposed row

- **Alex (everyday).** Lands on Home, sees one blue button that adds money and
  three quiet ones. Every label names a destination he recognises (his wallet,
  someone else, a person). No word appears twice; no word means two things.
- **Priya (power).** Loses nothing: Platform-address transfer is still one click
  away inside `Transfer`, and the tooltip now says so, which the old `Send`
  tooltip did not.
- **Jordan (developer).** Unaffected — no developer-gated affordance was in
  either row.

## 5. Regression guard

Replace the unit test `primary_send_receive_mappings_are_stable`, which
currently pins `Send → Transfer` *and* `SendToAnotherIdentity → Transfer` as
intended behaviour, with an injectivity assertion over the action-row buttons:
no two of them may resolve to the same `HomeScreenKind`. (Scope it to the action
row — `PickUsernameHero` and `ChecklistPickUsername` legitimately share
`RegisterDpnsName`, in two different contexts that the module already takes care
never to show at the same time; see the `checklist_covers_profile` suppression
at home.rs:323.) `ALL_HOME_BUTTONS` and `all_buttons_list_is_exhaustive` need
the two removed variants dropped.

## 6. Adjacent debt found while reviewing — folded into this same fix

Scope was widened (2026-07-24) to fix all of the following in the same change,
not defer them:

1. `Send to wallet` over-promises: `WithdrawalScreen` offers a blank
   `Address:` field and validates a hand-typed Core address. There is no "use an
   address from my wallet" affordance, so the user must fetch one from the
   Wallets screen themselves. Either add that affordance or soften the tooltip
   (done above) to not claim it.
2. Dead end for identities without a local TRANSFER/OWNER key:
   `WithdrawalScreen` renders a dark-red *"You do not have any withdrawal keys
   loaded for this {type} identity. Note that TRANSFER or OWNER keys are used
   for withdrawals."* Home does not gate the button, and the message is not
   written for the Everyday User. `identity.available_withdrawal_keys()` is
   available at Home — gate `Send to wallet` the same way `Add contact` is
   gated (disabled + `disabled_tooltip` explaining why).
3. PR #869 jargon leftovers in `top_up_identity_screen`:
   `WALLET_SELECTION_TOOLTIP` ("create the asset lock transaction",
   mod.rs:46), `"=> Waiting for Core Chain to produce proof of transfer of
   funds. <="` and `"=> Waiting for Platform acknowledgement <="`
   (by_receive_deposit.rs:187/190, by_using_unused_balance.rs:156/160,
   by_using_unused_asset_lock.rs:117), and `"Wallet Balance: {:.8} DASH"`
   (by_using_unused_balance.rs:28). Rewrite per the error-messages / i18n-ready
   string rules in CLAUDE.md.

   **Correction (QA finding, 2026-07-24):** this item originally claimed the
   rewrite would match "the sibling fix already applied to
   `add_new_identity_screen` in PR #869." That claim is false — independently
   verified that `add_new_identity_screen` (plus `wallets/create_asset_lock_screen.rs`
   and `dashpay/send_payment.rs`) still contain the identical unfixed jargon
   today. PR #869 touched those files but did not fix these specific strings
   (or they regressed since). The rewrite in `top_up_identity_screen` proceeded
   on its own merits against the CLAUDE.md rules directly, not by mirroring
   another screen. See §7 for the resulting follow-up.
4. Label→title discontinuity: `Add funds` opens a screen whose breadcrumb and
   heading say *Top Up Identity*. Align the screen's visible title/breadcrumb
   with the button label (pick one direction and make both match).
5. `docs/user-stories.md` uses the ID `IDN-013` for two different stories
   (identity key protection, and top up from Platform addresses). Renumber one.
6. §B.2's `Send` (payment sheet) and `Receive` (payment QR) are unimplemented
   but are not recorded as `[Gap]` stories anywhere. Add `[Gap]` entries so the
   catalog reflects that these were designed but never built, distinct from the
   redesigned action row documented here.

## 7. Implementation status and follow-ups

Items 1–6 above (and §3/§5) landed in commit `398ee3f4`. QA (independent
adversarial pass) caught two problems in the first implementation attempt,
both since fixed and re-verified: a kittest regression in
`withdraw_screen.rs` (two tests substring-matched the old dead-end message
text — updated to match the new wording without weakening what they guard),
and item 4 initially renaming only the breadcrumb while every heading and CTA
button downstream still said "Top Up Identity" (now fully aligned to "Add
funds"/"Add Funds" throughout).

Two items QA found are explicitly **not** part of this fix, left for a
separate follow-up:

- The jargon in `add_new_identity_screen`, `wallets/create_asset_lock_screen.rs`,
  and `dashpay/send_payment.rs` (see the correction in item 3 above) — same
  category of fix, different screens, kept out to avoid scope creep on an
  already-large change.
- Withdrawal-key gating is now checked three different ways across three
  surfaces: Home's `Send to wallet` button (this fix, `available_withdrawal_keys().is_empty()`),
  the `identities_screen.rs` "Withdraw" popup action (gated on balance only,
  no key check), and `WithdrawalScreen`'s own internal check (role-aware —
  Developer-role users need only *any* public key, not specifically
  TRANSFER/OWNER). Not a dead end in practice, just an inconsistency across
  entry points. Not requested by this fix's scope, not fixed here.
