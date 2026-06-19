# Secret Storage Seam — UX Disclosure Spec (Phase 1b)

**Author:** Diziet (Product Designer)
**Date:** 2026-06-19
**Status:** Design artifact for the implementer. No code here.
**Scope:** UX and exact user-facing copy for the four "Diziet items" in the
secret-storage-seam plan. The architecture is approved and is **not** reopened
here — this document only decides what the user sees, when, and in what words.

## Source of truth

- Execution plan: `/home/ubuntu/.claude/plans/snazzy-marinating-sun.md` (UX section)
- Full design: `/home/ubuntu/.claude/plans/snazzy-marinating-sun-agent-ae6181c0dc23bdba8.md`
  ("Diziet items", "Migration", `WalletMeta.uses_password` flip)
- Persona: `docs/personas/everyday-user.md` (Alex Torres)
- Surfaces this copy lands in: `src/ui/components/message_banner.rs`
  (`MessageBanner::set_global`, `with_details`), the existing unlock modal
  `src/ui/components/passphrase_modal.rs` / `wallet_unlock_popup.rs`

## The situation, stated plainly for the persona

DET is moving every wallet secret onto one storage seam and dropping its own
per-wallet encryption. The accepted interim consequence: **a password-protected
wallet, once migrated, is no longer encrypted under its password at rest** — it
falls back to file-permission protection (`0600`) plus an empty-passphrase vault
until upstream per-secret encryption lands. After migration the wallet no longer
asks for its password to unlock.

Alex (the Everyday User) does not know what AES-GCM, a vault, or a seam is. Alex
knows two things, and we must speak to exactly those two: **(1) "I set a password
on my wallet"** and **(2) "the app stopped asking me for it."** A change in that
contract that goes unexplained reads as either a bug ("did it forget my
password?") or a breach ("is my wallet open to anyone now?"). Both produce the
support request the persona's success metrics say we must drive to zero. The
disclosure exists to convert a silent, alarming change into an expected,
understood one.

---

## Decision summary

| # | Item | Decision |
|---|------|----------|
| 1 | Per-wallet password vestigial after migration | Stop asking (`uses_password=false`). One-time per-wallet notice at the migrating unlock. |
| 2 | Single-key per-key passphrase (SEC-002) | Identical treatment to item 1. Same notice family, key-flavored copy. |
| 3 | One-time interim at-rest disclosure | Non-gating, informational. Surfaces *with* the item-1/item-2 notice at the migrating unlock — not at app start, not a separate modal. |
| 4 | SEC-201 (Enter-consume papercut) | Cross-reference only. Not fixed here. Noted that migration runs the modal more often. |

Design principles applied: **error prevention over recovery** (explain before
the user notices and worries), **progressive disclosure** (one short sentence
the user must read; the technical "why" is one optional click away), and
**calm, actionable tone** (project i18n + error-message rules).

---

## Item 1 — Per-wallet password becomes vestigial

### What the user experiences

1. Alex opens a password-protected wallet as always and is prompted to unlock —
   **the same unlock modal as today** (`wallet_unlock_popup.rs`). Nothing new
   here; the migration needs this one passphrase entry and reuses the existing
   flow. (This is the lazy-migration unlock from the plan's Migration section B.)
2. On successful unlock, migration runs inside the decrypt scope and flips
   `uses_password=false`.
3. **Immediately after the wallet finishes unlocking**, a single global
   info-style notice appears (see Copy A). It is the only new surface the user
   sees.
4. On every subsequent open, that wallet **unlocks without a password prompt**.
   This is expected because the notice in step 3 told Alex it would happen.

### Why at the migrating unlock, and once per wallet

- **At unlock, not app start:** the change is per-wallet and only becomes true at
  the moment that specific wallet migrates. A startup banner would fire before
  the fact is true, for wallets that may never be opened, and would be generic
  noise. Tying the notice to the unlock makes it causally legible: "I just
  unlocked, and *this* is what changed about *this* wallet."
- **Once per wallet, not once globally:** Alex may have one wallet with a
  password and one without. The fact only applies to the protected one, and only
  at its migration. A per-wallet one-time notice (keyed on the same `uses_password`
  flip that drives the migration — fire when the flip happens, never again) is
  the precise scope. After the flip, `uses_password` is already `false`, so the
  notice naturally never re-fires for that wallet.
- **Not gating:** the password is *already* vestigial by the time we could ask
  for acknowledgement — the wallet is unlocked and migrated. Gating would be a
  speed bump in front of a decision the user cannot change and was made for them
  by an approved plan. Informational respects their time (the persona expects
  unlock in seconds) while still being honest.

### Copy A — per-wallet password notice (HD-seed wallet)

> **Banner type:** `MessageType::Warning` (see note on type below)
> **Surface:** `MessageBanner::set_global`, shown once when this wallet migrates.
> **Details (optional, via `with_details`):** Copy D (the shared "why").

```
"{wallet}" no longer needs its password to open. Your wallet stays on this device, protected by your computer's account. Full password protection will return in a future update.
```

- Placeholder: `{wallet}` = the wallet alias/name (`WalletMeta.alias`). One named
  placeholder, complete sentences, no fragment concatenation — i18n rule
  satisfied.
- No jargon: no "encryption", "vault", "seam", "AES", "at rest". "Protected by
  your computer's account" is the truthful, persona-legible rendering of "file
  permissions + OS user account" — Alex understands "my computer login keeps my
  files private."
- Structure is *what happened* + *current state* + *what to expect*, mirroring
  the project error-message rule even though this is not an error.

---

## Item 2 — Single-key per-key passphrase (SEC-002) becomes vestigial

Treatment is **identical** to item 1: stop prompting for the per-key passphrase,
retain the decode reader for migration, surface the same one-time notice at the
migrating unlock — only the noun changes (an *imported key*, not a *wallet*).

### Copy B — per-key passphrase notice (imported single key)

> **Banner type:** `MessageType::Warning`
> **Surface:** `MessageBanner::set_global`, shown once when this key migrates.
> **Details (optional):** Copy D.

```
The imported key "{key}" no longer needs its passphrase to use. It stays on this device, protected by your computer's account. Full passphrase protection will return in a future update.
```

- Placeholder: `{key}` = the key's user-facing label (the imported-key
  alias/address shown in the UI). Single named placeholder.
- "Passphrase" (not "password") matches the term the single-key import flow uses,
  so the word the user typed is the word they read back.
- If a wallet and an imported key migrate in the same session, the two notices
  are distinct messages (different text), so `set_global`'s text-dedup does not
  collapse them — each fact is reported once.

---

## Item 3 — One-time disclosure of the interim at-rest regression

### Decision: fold the disclosure into the item-1/item-2 notice, non-gating

The plan's recommended default is "non-gating informational." I am refining
*placement*: rather than a third, free-standing notice (which would mean Alex
sees a password notice **and** a separate security notice and has to reconcile
them), the regression disclosure **is** the item-1/item-2 notice plus its
optional details. Copy A and Copy B already state the regression in
persona-legible terms — "protected by your computer's account" and "full
protection will return." The deeper, honest "why" lives in the details panel
(Copy D) for anyone who clicks, and in the logs.

### Why non-gating, for the Everyday User specifically

- **The decision is already made and irreversible for the user.** An "I
  understand" gate implies a choice. There is none: the architecture is approved,
  migration is automatic, the password is vestigial the instant the wallet
  unlocks. A gate in front of a non-choice teaches users to click through
  acknowledgements without reading — it *erodes* the weight of future, real
  consent dialogs.
- **The persona transacts in seconds and opens the wallet 2–5×/week.** A modal
  wall on unlock fights the "unlock in seconds" expectation and, on the second
  reading, becomes friction the user resents and dismisses blindly.
- **Honesty without alarm.** We are not hiding the regression — Copy A/B states
  it in plain language, Copy D gives the full technical truth one click away, and
  it is logged. That satisfies the disclosure obligation without an alarm that
  the persona ("did something go wrong with my funds?") would over-read.

### A note on banner type — why `Warning`, not `Info`

`message_banner.rs` auto-dismisses `Info`/`Success` on a **short** timer and
`Warning`/`Error` on a **long** timer (`DEFAULT_AUTO_DISMISS_SHORT` vs
`_LONG`). A security-relevant, one-time, must-actually-be-read disclosure should
not vanish on the short timer before Alex has read it. `Warning` gives the longer
dwell and the ⚠ glyph signals "read me, this matters" without the ⛔ alarm of an
error. This is **not** an alarm about a failure — tone in the copy stays calm and
forward-looking ("will return in a future update"). If the implementer finds
`Warning`'s long auto-dismiss still too short for a paragraph the user must read,
prefer a **manually-dismissed** (non-auto) banner over downgrading to `Info`.
The priority order is: *the user reads it once* > *it doesn't nag*.

### Copy D — shared technical detail (details panel, optional click)

> **Surface:** `with_details(...)` attached to Copy A and Copy B. Goes to the
> collapsible details panel and the log. This is the one place where slightly
> more precise language is allowed, because it is opt-in for a curious user — but
> it still avoids raw internals.

```
This wallet's secrets are now stored in a shared protected location on this device, guarded by your computer's account and file permissions rather than by your wallet password. This is a temporary step while a stronger, built-in protection is being finished. Your keys never leave this device. To keep this wallet extra safe in the meantime, make sure your computer account is password-protected and not shared.
```

- This is the only string that gives the user a concrete *self-help* action
  ("make sure your computer account is password-protected"), satisfying the
  project rule that messages offer something the user can do themselves — even
  though the primary banner is informational. It never says "contact support."
- Still no "AES", "vault", "seam", "0600", "empty passphrase". "Shared protected
  location," "file permissions," and "computer account" are the truthful,
  legible renderings.

---

## Item 4 — SEC-201 (passphrase-modal Enter-consume) — cross-reference only

**Not designed or fixed here**, per the plan. Recorded so the implementer and QA
hold the context:

Migration makes the existing unlock modal (`passphrase_modal.rs`) run on **every
protected-wallet unlock that triggers a migration**, and protected wallets are
exactly the ones that migrate lazily. So the known Enter-consume papercut
(SEC-201) becomes **more visible** during the migration window — more users will
hit the modal, possibly hit Enter, during this rollout. This raises the value of
fixing SEC-201 soon, but it is a separate change. If SEC-201 is unfixed when this
ships, expect a modest uptick in Enter-key friction reports concentrated around
first-unlock-after-update; that is the migration surfacing an existing bug, not a
regression introduced by this work.

---

## Surfacing matrix (for the implementer)

| Trigger | Condition | Copy | Banner type | Once? | Details |
|---|---|---|---|---|---|
| Protected HD wallet finishes lazy migration at unlock | `uses_password` flips `true→false` (HD seed) | Copy A | Warning (or manual-dismiss) | Once per wallet | Copy D |
| Protected imported key finishes lazy migration at unlock | per-key passphrase flips to vestigial | Copy B | Warning (or manual-dismiss) | Once per key | Copy D |
| App start | — | none | — | — | — |
| No-password wallet eager migration | silent (no UX change for the user) | none | — | — | — |

Notes:
- **Eager (no-password) migrations produce no notice.** Nothing changes from the
  user's point of view — the wallet never asked for a password and still doesn't.
  Surfacing a security notice there would alarm users about a change they cannot
  perceive and that does not affect their (already password-free) wallet.
- **Headless / MCP:** password wallets do not lazily migrate without a GUI unlock,
  so none of these notices fire headlessly. No copy is needed for the headless
  path; the legacy reader serves silently (per plan Migration section C).
- **"Once" is naturally enforced by the migration itself:** the notice fires on
  the `uses_password` flip; after the flip the condition is permanently false, so
  re-firing is impossible without a fresh legacy wallet. No separate "seen" flag
  is strictly required, though the implementer may add one defensively.

## i18n compliance checklist (all strings above)

- [x] Complete sentences, no fragment concatenation.
- [x] Named placeholders only (`{wallet}`, `{key}`), no positional grammar
      assumptions.
- [x] No logic embedded in text.
- [x] No jargon in the persona-facing banner copy (A, B); the one slightly
      more technical string (D) is opt-in and still jargon-free.
- [x] Each string is a single, extractable translation unit.

## Persona walk-through (validation)

Alex, mainnet, one password-protected wallet, updates DET and opens the wallet:

1. Sees the familiar unlock prompt, types the password. *No surprise.*
2. Wallet opens. A calm ⚠ notice says the wallet won't need its password to open
   anymore, it's still on this device protected by the computer account, and full
   protection is coming back. *Understood, not alarmed — Alex was told before
   noticing the prompt was gone.*
3. (Curious once) clicks details, reads Copy D, makes sure the laptop login is
   set. *Given a concrete action; feels in control.*
4. Next week, opens the wallet — no password prompt. *Expected. No support
   ticket.* Success metric "support requests about unexplained changes" → held
   at zero.

The least-technical persona understands every screen. If Alex can use it,
the Power User and Platform Developer (who understand the underlying change) can.

---

## Candy tally (confirmed UX findings surfaced)

| Severity | Count | Finding |
|---|---|---|
| Medium | 1 | Silent disappearance of the password prompt after migration would read as a bug/breach to the Everyday User — requires the one-time per-wallet notice (Copy A). |
| Medium | 1 | Banner-type default (`Info`) auto-dismisses too fast for a must-read one-time security disclosure; recommend `Warning` long-dwell or manual-dismiss (item 3 type note). |
| Low | 1 | Two separate notices (password + regression) would force the user to reconcile them; consolidated into one notice + details to reduce cognitive load (item 3 placement). |
| Low | 1 | Single-key passphrase needs distinct copy from the wallet notice so `set_global` text-dedup doesn't collapse them when both migrate in one session (Copy B). |

**Total: 4 findings — 2 Medium, 2 Low.**
