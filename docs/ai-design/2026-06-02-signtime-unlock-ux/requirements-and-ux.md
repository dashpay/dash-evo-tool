# Sign-Time Unlock Passphrase Prompt — Requirements & UX Spec

**Feature:** SEC-002 follow-up · Gap PROJ-008 · GitHub issue #90
**Phase:** 1a (Requirements) + 1b (UX) — combined
**Status:** Design only. No implementation. This is the spec the next phases build from.
**Author:** Diziet (Product Designer)
**Date:** 2026-06-02

> **Worktree note (verify after merge):** This worktree could not be fast-forwarded to
> `b2febb71` — the environment for this design pass exposed no shell. All `src/…:line`
> citations below were read from the worktree as-is and should be **re-verified after the
> base merge** before any implementation begins. The structural facts they describe (the
> unlock cache, the typed errors, the `display_task_error` hook) are load-bearing; the exact
> line numbers are not.

---

## 1. Executive Summary

### Problem statement
SEC-002 (Option C, **per-key passphrase**) shipped the storage and in-process unlock
substrate for imported single-key wallets: each protected key is AES-GCM-encrypted under its
own passphrase, and an in-process cache (`single_key_unlocked`) keeps a key unlocked for the
rest of the session once the user has typed its passphrase. What did **not** ship is the only
thing the user ever sees: **a prompt that asks for the passphrase at the moment a signature is
needed.** Today, if a protected key is not already in the cache, any operation that must sign
with it (register identity, send funds, asset-lock signing, and any future single-key signer)
simply fails with a typed `SingleKeyPassphraseRequired` error and no way forward. The TODO sits
at `src/wallet_backend/mod.rs:562-566`.

This is a dead-end, not a bug: the safety net is in place but the door has no handle.

### Key actors
- **Everyday User (Alex Torres)** — imported a private key (e.g. a paper-wallet sweep or a key
  a service gave them) and protected it with a passphrase. Does not know what "signing" means;
  knows they set a passphrase and expects to be asked for it when it matters.
- **Power User (Priya Nakamura)** — imports keys deliberately for specific purposes; wants
  single-key wallets to reach feature parity (her pain point #8) and wants **minimal friction**
  — one unlock per session, no re-typing.

### Solution direction
Intercept the existing typed `SingleKeyPassphraseRequired` error at the UI layer via the
already-present `Screen::display_task_error(&TaskError) -> bool` hook, open a **passphrase
unlock prompt** (a near-clone of the existing `WalletUnlockPopup`), call the existing
`SingleKeyView::unlock_with_passphrase`, and **automatically re-dispatch the original
operation** once the key is unlocked. No new backend signing path, no change to the sign API,
no change to SEC-002. The whole feature lives at the UI seam where errors already flow back.

### Recommended integration pattern (the one decision worth front-loading)
**Gate-on-error with auto-retry**, *not* mid-flight unlock-request. The backend task runs,
hits the locked key, returns `SingleKeyPassphraseRequired { addr }`; the UI catches it, prompts,
unlocks the in-process cache, and re-runs the **same** task. Rationale: the backend already
returns this typed error and the cache already makes the retry cheap — this pattern adds zero
new plumbing, keeps secrets off the async channel, and reuses the error path that exists today.
(Full justification in §6.)

---

## 2. Stakeholder & Actor Analysis

| Actor | Goal | Pain today | Success looks like |
|---|---|---|---|
| **Everyday User** | Use their protected imported key to send/register without thinking about cryptography | Operation silently fails or shows a confusing error with no path forward | A calm prompt: "Enter the passphrase for this key" → operation just continues |
| **Power User** | Sign many operations in a session with one unlock; full single-key parity | Single-key wallets are "second-class"; re-prompting would be friction | First sign of the session prompts; every subsequent sign for the same key is silent |
| **Security reviewer** (internal stakeholder) | Passphrase never leaks to logs, error text, async channels, or disk | — | Passphrase lives only in a zeroizing buffer, cleared on close; never in `TaskError`, never logged |
| **Next-phase engineer** (consumer of this spec) | Unambiguous integration point and component contract | TODO with no UX | One named component, one hook, one re-dispatch contract |

**Supporting systems (already shipped — do NOT redesign):**
- `WalletBackend.single_key_unlocked: RwLock<BTreeMap<String,[u8;32]>>` — the session cache
  (`src/wallet_backend/mod.rs:202`).
- `SingleKeyView::unlock_with_passphrase(addr, passphrase)` — decrypts and populates the cache;
  returns `SingleKeyPassphraseIncorrect` on a wrong passphrase
  (`src/wallet_backend/single_key.rs:256`).
- `SingleKeyView::has_passphrase(addr)` — whether a prompt is needed
  (`src/wallet_backend/single_key.rs:245`).
- `SingleKeyView::forget_unlocked(addr)` — explicit re-lock
  (`src/wallet_backend/single_key.rs:281`).
- `SingleKeyView::sign_with(addr, msg)` → returns `SingleKeyPassphraseRequired { addr }` when
  the cache misses on a protected key (`src/wallet_backend/single_key.rs:649`).
- Typed errors: `SingleKeyPassphraseRequired { addr }`, `SingleKeyPassphraseIncorrect`
  (`src/backend_task/error.rs:1380`, `:1392`).
- UI seam: `Screen::display_task_error(&mut self, &TaskError) -> bool` — returns `true` to
  suppress the generic error banner (`src/app.rs:1135`, dispatch in `src/ui/mod.rs:1666`).
- Reusable UI: `PasswordInput` (zeroizing `Secret`, hold-to-reveal, undo disabled —
  `src/ui/components/password_input.rs`) and `WalletUnlockPopup` (the modal pattern to clone —
  `src/ui/components/wallet_unlock_popup.rs`).

---

## 3. Requirements

### 3.1 Functional Requirements

**FR-1 — Prompt appears exactly when a sign is blocked.**
The prompt appears if and only if a backend operation returns
`SingleKeyPassphraseRequired { addr }` (i.e. a sign was attempted against a protected key whose
plaintext is not in the session cache). It never appears speculatively, never on import (import
primes the cache), and never for an unprotected key.

*Acceptance (Given/When/Then):*
- **G** an imported key at `addr` is passphrase-protected and not in the session cache
  **W** the user triggers send-funds / identity-register / asset-lock signing with that key
  **T** the unlock prompt opens, anchored over the triggering screen, with the operation paused.
- **G** the same key is already in the session cache **W** the user triggers the same operation
  **T** no prompt appears and the operation proceeds without interruption.

**FR-2 — Prompt content is self-explanatory to a non-technical user.**
The prompt shows: a title, a one-sentence explanation naming *which* key (by alias if present,
else by Base58 address — a permitted handle per CLAUDE.md rule 6), the saved passphrase **hint**
if one exists, a masked passphrase field, and Unlock / Cancel actions.

*Acceptance:*
- **G** the key has alias "Savings sweep" and hint "the xkcd one" **W** the prompt opens
  **T** it reads, in plain language, that the *Savings sweep* key is locked and shows the hint.
- **G** the key has no alias **W** the prompt opens **T** it identifies the key by its address.
- The prompt contains **no** jargon ("sign", "ECDSA", "secp256k1", "state transition", "cache").

**FR-3 — Correct passphrase unlocks and the operation resumes automatically.**
On a correct passphrase the key is unlocked into the session cache and the **original
operation is re-dispatched automatically** — the user does not re-initiate it.

*Acceptance:*
- **G** the prompt is open for a pending send **W** the user types the correct passphrase and
  confirms **T** the prompt closes and the send completes (or proceeds to its normal next step)
  without the user re-entering amount/recipient.

**FR-4 — Wrong passphrase is recoverable in place.**
A wrong passphrase keeps the prompt open, clears the field, and shows an inline, non-alarming
error. The user may retry without limit (see §3.2 NFR-5 for the rate-limit decision).

*Acceptance:*
- **G** the prompt is open **W** the user enters a wrong passphrase **T** the field clears, an
  inline message says the passphrase was not correct and to try again, and focus returns to the
  field. The pending operation is **not** cancelled.

**FR-5 — Cancel/abort cleanly aborts the operation.**
Cancel, the window X, the Escape key, and a click on the overlay all dismiss the prompt and
abandon the pending operation. Nothing is signed, no partial state is written, the user is
returned to the triggering screen with their inputs intact.

*Acceptance:*
- **G** a pending send with the prompt open **W** the user presses Escape **T** the prompt
  closes, nothing is sent, and the send screen still shows the entered amount and recipient.

**FR-6 — One unlock covers the session (mental model).**
After a successful unlock, subsequent signs for the **same** key in the same process run without
re-prompting (this is the cache behaviour that already ships). The prompt copy must set this
expectation so the user understands the unlock persists and is not per-operation.

*Acceptance:*
- **G** the user unlocked key `K` earlier this session **W** they perform a second operation
  with `K` **T** no prompt appears.
- The prompt copy communicates "unlocked until you close the app" (see §4.4 for exact strings).

**FR-7 — Multiple protected keys in one operation are each unlocked, in order.**
If a single operation needs two or more protected, uncached keys, the user is prompted for each
in turn (one prompt at a time), and the operation proceeds only once all are unlocked. (See §5
Edge Case E-1 for the recommended sequencing.)

**FR-8 — Explicit re-lock remains available (no regression).**
The existing "lock"/forget affordance (`forget_unlocked`) is unaffected; locking a key mid-session
means the next sign re-prompts. This spec does not add a re-lock UI but must not block one.

### 3.2 Non-Functional Requirements

**NFR-1 — Security: the passphrase never escapes the prompt.**
- Held only in the `PasswordInput`'s zeroizing `Secret`; cleared (`clear()`/zeroized) on every
  close path (success, cancel, X, Escape, click-outside) and on every wrong-passphrase retry.
- **Never** placed in a `TaskError`, an `AppAction`, a `BackendTask`, a log line, the
  `MessageBanner` details panel, or any string sent across the async channel. The async task
  receives a *passphrase-derived unlock having already happened in the cache*, never the
  passphrase itself.
- Re-affirms CLAUDE.md rule 7: no user-facing/secret `String` in error variants. The wrong-
  passphrase case uses the existing fieldless `SingleKeyPassphraseIncorrect`.

**NFR-2 — Accessibility (within egui's known limits).**
- Keyboard: field is auto-focused on open; **Enter** submits; **Escape** cancels; **Tab** moves
  Field → Unlock → Cancel in layout order (matches the global table in
  `docs/ux-design-patterns.md` §10).
- Focus is trapped in the modal while open (the overlay + modal `Window` already do this in the
  `WalletUnlockPopup` pattern); focus returns to the triggering control on close.
- Focus indicator uses `BORDER_WIDTH_THICK` / 3:1 contrast per the patterns doc.
- **Known limitation (must be recorded, not hidden):** egui exposes no screen-reader
  annotations (`docs/ux-design-patterns.md:172`). We therefore make the prompt legible *by text
  alone* — every element is a real, visible label; nothing relies on icon-only meaning; the hint
  and error are plain text. This is the best available a11y posture until egui gains a11y; flag
  for re-test when it does.

**NFR-3 — i18n-ready copy.**
All strings are complete sentences with **named** placeholders (`{ $key_name }`, `{ $hint }`),
no fragment concatenation, no grammar that assumes word order. Exact strings in §4.4.

**NFR-4 — Consistency.**
Visual and interaction parity with `WalletUnlockPopup` and `ConfirmationDialog`: same overlay
token (`modal_overlay()`), same corner radius / margins, same Escape=cancel / X=cancel /
click-outside=cancel rules, same primary(Unlock)/secondary(Cancel) button placement
(right-aligned, Unlock rightmost).

**NFR-5 — Retry policy: unlimited, no lockout (decision, open to override).**
The protected key is local, AES-GCM-encrypted, and gated by file permissions + the OS account;
there is no remote attacker and no account to lock. A retry counter or cooldown would punish the
forgetful user (the Everyday User's most likely failure) for no security gain. **Recommendation:
no retry limit, no cooldown.** (Listed as an open question in §6 in case Security wants a soft
cap.)

**NFR-6 — No new dependency on operation type.**
The prompt is operation-agnostic: it keys off `addr` from the error, not off "send" vs
"identity". Adding a future single-key signer requires zero prompt changes — only that the new
backend task surfaces `SingleKeyPassphraseRequired` (which it gets for free by calling
`sign_with`).

### 3.3 Persona mapping

| Requirement | Everyday User | Power User |
|---|---|---|
| FR-2 (plain content, hint) | **Critical** — this is their whole understanding of the feature | Nice-to-have |
| FR-3 (auto-resume) | **Critical** — they must not have to figure out "now what?" | Valued (no manual re-trigger) |
| FR-6 (session unlock) | Reassuring | **Critical** — minimal friction is their #1 ask |
| FR-7 (multi-key) | Rare for them | Plausible (they batch) |
| NFR-5 (no lockout) | **Critical** — forgetful retries must not lock them out | Indifferent |

Validated against the **least technical persona first** (Alex): if Alex, who does not know what
"signing" is, sees "*The key 'Savings sweep' is locked. Enter its passphrase to continue.*" with
their hint and an obvious Unlock button — and the send simply continues afterward — the feature
is usable. Everyone above Alex is then served.

---

## 4. Interaction Flow / UX Spec

### 4.1 Where the prompt lives
A **modal popup anchored center-screen over the triggering screen**, with a dimmed overlay —
identical placement to `WalletUnlockPopup`. It is **not** a new root screen and **not** a pushed
detail screen; it is owned by the triggering screen as an `Option<…>` field (lazy init, the
project's standard component pattern). Reasons:
- The operation's inputs (amount, recipient, identity selection) must survive the prompt → the
  prompt cannot navigate away from the screen that holds them.
- Auto-resume (FR-3) needs the original `AppAction`/`BackendTask` in hand → the triggering screen
  is the natural owner of that pending task.

### 4.2 The journey (happy path)

```
User on Send Funds screen (key K is protected, not yet unlocked this session)
        │
        ▼
  [Send]  ── AppAction::BackendTask(send) ──►  tokio::spawn
        │                                          │
        │                                   run_backend_task()
        │                                   single_key().sign_with(K, …)
        │                                   cache miss → Err(SingleKeyPassphraseRequired{addr:K})
        │                                          │
        │   ◄── TaskResult::Error(SingleKeyPassphraseRequired) ──┘
        ▼
  AppState routes to visible_screen.display_task_error(&err) -> true   (suppress generic banner)
        │   screen: stash the pending task, open SignUnlockPrompt for addr=K
        ▼
  ┌────────────────────────── Unlock prompt (modal) ──────────────────────────┐
  │  Enter passphrase → [Unlock]                                              │
  └───────────────────────────────────────────────────────────────────────────┘
        │ correct passphrase
        ▼
  single_key().unlock_with_passphrase(K, pw)  → Ok  (cache now holds K)
  prompt.clear()+close()
        │
        ▼
  screen re-dispatches the stashed AppAction::BackendTask(send)   ← auto-resume (FR-3)
        │
        ▼
  run_backend_task() → sign_with(K) hits the cache → operation completes normally
```

### 4.3 ASCII wireframe of the dialog

```
        ╔══════════════════════════════════════════════════════╗
        ║  Unlock imported key                            [ × ] ║
        ╟──────────────────────────────────────────────────────╢
        ║                                                      ║
        ║  The key "Savings sweep" is locked.                  ║
        ║  Enter the passphrase you set for it to continue.    ║
        ║                                                      ║
        ║  Hint: the xkcd one                                  ║   ← shown only if a hint exists
        ║                                                      ║
        ║  ┌────────────────────────────────────────┐  ( ◌ )  ║   ← masked field + hold-to-reveal eye
        ║  │ ••••••••••                              │         ║
        ║  └────────────────────────────────────────┘         ║
        ║                                                      ║
        ║  This key stays unlocked until you close the app.    ║   ← session mental model (FR-6)
        ║                                                      ║
        ║                              [ Cancel ]  [ Unlock ]  ║
        ╚══════════════════════════════════════════════════════╝
                     (dimmed overlay behind, click = cancel)
```

Error state (wrong passphrase) — field cleared, inline message, focus returned to field:

```
        ║  ┌────────────────────────────────────────┐  ( ◌ )  ║
        ║  │                                        │         ║   ← cleared
        ║  └────────────────────────────────────────┘         ║
        ║  That passphrase is not correct. Try again.         ║   ← inline, calm, no jargon
```

If the key has **no alias**, the first line uses the address instead:

```
        ║  The key bcV…q3 is locked.                          ║
        ║  Enter the passphrase you set for it to continue.    ║
```

### 4.4 Copy (i18n-ready, named placeholders)

| Slot | String | Notes |
|---|---|---|
| Title | `Unlock imported key` | Verb-first, plain. |
| Body (aliased) | `The key "{ $key_name }" is locked. Enter the passphrase you set for it to continue.` | One translation unit; `{ $key_name }` is alias. |
| Body (no alias) | `The key { $address } is locked. Enter the passphrase you set for it to continue.` | Base58 address as handle (rule 6). |
| Hint line | `Hint: { $hint }` | Rendered only when a hint exists. |
| Session note | `This key stays unlocked until you close the app.` | Sets the FR-6 mental model. |
| Field placeholder | `Passphrase` | |
| Reveal tooltip | `Hold to reveal` | Reuse `PasswordInput`'s existing affordance. |
| Primary button | `Unlock` | |
| Secondary button | `Cancel` | |
| Wrong-passphrase | `That passphrase is not correct. Try again.` | **Reuse the existing `SingleKeyPassphraseIncorrect` Display string** (`error.rs:1391`) — do not author a second copy. |

> The wrong-passphrase wording already lives on the typed variant. The screen should derive the
> inline message from the typed `SingleKeyPassphraseIncorrect` returned by
> `unlock_with_passphrase`, **not** hardcode a parallel literal — same anti-string-parsing
> discipline the codebase enforces elsewhere.

### 4.5 States (component state table)

| State | Trigger | Visual / behaviour |
|---|---|---|
| Hidden | default; cache hit | not rendered; zero overhead |
| Open / idle | `SingleKeyPassphraseRequired` caught | overlay + modal, field auto-focused, Unlock enabled only when field non-empty |
| Submitting | Enter / Unlock click | (optional) Unlock shows brief busy state while `unlock_with_passphrase` runs; it is local/fast so a spinner is optional, not required |
| Error | wrong passphrase | field cleared, inline error, focus back to field, operation still pending |
| Closing (success) | correct passphrase | clear secret, close, re-dispatch pending task |
| Closing (cancel) | Cancel / X / Escape / click-outside | clear secret, close, **drop** pending task, restore triggering screen |

---

## 5. Edge Cases

**E-1 — Multiple protected keys needed by one operation.**
*Recommendation: prompt sequentially, one key per prompt, in the order the backend reports them.*
The cleanest realisation under the gate-on-error pattern: the operation re-dispatches after each
unlock; if the *next* sign is for a *second* uncached key, the backend returns
`SingleKeyPassphraseRequired { addr: K2 }` and the same loop opens the prompt for `K2`. The user
sees one prompt at a time, each clearly naming its key. No batching UI needed; the error→prompt→
retry loop naturally drains all required keys. (A future optimisation could pre-collect all
needed addresses and prompt once with a stepper — out of scope; note it.)

**E-2 — Operation triggered while a prompt is already open.**
The prompt is modal with a focus-trapping overlay (FR-2/NFR-2), so the underlying screen cannot
dispatch a second operation while it is open. Defensive rule for the implementer: if a second
`SingleKeyPassphraseRequired` somehow arrives while a prompt is open for a *different* address,
queue it (do not stack modals); if it is for the *same* address, ignore the duplicate.

**E-3 — Session-cache hit (no prompt).**
Already covered by FR-1/FR-6 — the backend never returns `SingleKeyPassphraseRequired` when the
key is cached, so no prompt path is entered. This is the steady-state for the Power User after
their first unlock.

**E-4 — Passphrase retry limit.**
Per NFR-5: **none.** Local-only secret, no remote attacker, no account to lock; a cap would only
harm the forgetful Everyday User. Open question flagged in §6 for Security sign-off.

**E-5 — Cancel mid-operation must not leave partial state.**
Because the sign happens *before* any irreversible step (a state transition is only broadcast
after signing), cancelling at the prompt aborts before anything leaves the device. The implementer
must ensure the stashed pending task is **dropped**, not merely hidden, on cancel.

**E-6 — Key forgotten/removed between trigger and prompt.**
If the key was `forget`-ten (removed) after the operation started, `unlock_with_passphrase`
returns `ImportedKeyNotFound`. Surface this through the normal error path with a plain message
("This imported key is no longer available.") and close the prompt — do not loop.

**E-7 — Empty passphrase submitted.**
Unlock is disabled while the field is empty (or, if Enter is pressed on empty, it is a no-op).
Never call `unlock_with_passphrase` with an empty string.

**E-8 — Wallet/network switch while prompt is open.**
On `change_context` (network switch), close the prompt and drop the pending task — the operation
belonged to the previous network context. The secret is cleared as part of close (NFR-1).

---

## 6. Open Questions, Decisions & Assumptions

### Recommended integration pattern — decision and rationale
**Gate-on-error with auto-retry** over **mid-flight unlock-request**.

| | Gate-on-error (recommended) | Mid-flight unlock-request |
|---|---|---|
| New plumbing | none — reuses `display_task_error` + re-dispatch | new request/response channel from task → UI → task while task is suspended |
| Secret on async channel | never | risk: passphrase or a callback would have to cross the channel |
| Task complexity | task stays a pure async fn; no UI awareness | task must suspend, await a UI answer, resume — couples backend to UI |
| Cost of retry | trivial — second run hits the cache | n/a |
| Fit with existing code | the typed error and the hook already exist for exactly this | would require inventing a suspension mechanism |

One-line rationale: **the backend already returns the typed error and the cache already makes the
re-run cheap, so catching the error in the UI and re-dispatching is the lowest-coupling, most
secure path — no passphrase ever touches the async boundary.**

### New UI component? — **Yes, one small, justified component.**
`SignUnlockPrompt` (working name), a near-clone of `WalletUnlockPopup`. Reuse is maximal —
`PasswordInput` for the field, the same overlay/modal/focus/Escape/click-outside scaffolding,
the same button layout. It is **not** the same as `WalletUnlockPopup`: that one opens an HD
wallet seed (`wallet_seed.open`) and calls `handle_wallet_unlocked`; this one unlocks a single
imported key by address (`single_key().unlock_with_passphrase`) and re-dispatches a pending task.
A shared inner "passphrase modal body" widget could back both to avoid duplication — recommended
but not required. Catalog `src/ui/components/README.md` after building it.

### Open questions needing a user/stakeholder decision
1. **Retry limit (NFR-5 / E-4):** confirm *no* limit and *no* cooldown. (Design recommends none;
   Security may want a soft cap or an increasing delay. This is the one item that could change
   the spec.)
2. **Session note wording (FR-6):** is "until you close the app" the right mental model to commit
   to, or do we foresee an idle-timeout auto-lock later? If an auto-lock is planned, the copy
   should say "for a while" rather than promise the whole session. Defaulting to the literal,
   honest current behaviour ("until you close the app") until an auto-lock is actually designed.
3. **Multi-key UX (E-1):** is sequential one-at-a-time acceptable for v1, deferring a single
   batched stepper? (Design recommends sequential now.)

### Assumptions
- A1: imported single-key operations that need to sign are the only consumers of
  `SingleKeyPassphraseRequired` (HD wallets use the separate `WalletUnlockPopup` seed path). If a
  future signer reuses this error for a different secret type, the copy "imported key" must be
  revisited.
- A2: `unlock_with_passphrase` is fast enough (local AES-GCM) that no async spinner is mandatory.
- A3: the operation's signing step precedes any irreversible/broadcast step, so cancel is always
  safe (confirmed by the sign-then-broadcast ordering of state transitions).

---

## 7. Traceability

| Requirement | Substrate it depends on | UX element |
|---|---|---|
| FR-1 | `sign_with` → `SingleKeyPassphraseRequired`; `display_task_error` hook | §4.2 flow |
| FR-2 | `ImportedKey.alias`, `passphrase_hint`, `has_passphrase` | §4.3 wireframe, §4.4 copy |
| FR-3 | re-dispatch of stashed `AppAction::BackendTask` | §4.2, §4.5 closing-success |
| FR-4 | `unlock_with_passphrase` → `SingleKeyPassphraseIncorrect` | §4.3 error state, §4.4 |
| FR-5 | drop pending task; `clear()` secret | §4.5 closing-cancel, E-5 |
| FR-6 | `single_key_unlocked` session cache | §4.4 session note |
| FR-7 | error→prompt→retry loop | E-1 |
| NFR-1 | `Secret` zeroize; no String in `TaskError` | NFR-1, §4.4 note |
| NFR-2 | egui modal focus trap; patterns §10 | §4.5, NFR-2 |

---

## Candy Tally (findings surfaced)

This is a forward design, so "findings" = confirmed gaps/risks this spec resolves or flags.

- **Critical (1):** PROJ-008 — protected single-key operations are a dead-end with no unlock UI
  (the core gap this spec closes).
- **High (2):** (a) auto-resume is required or the Everyday User is stranded after unlocking;
  (b) passphrase must never cross the async channel — mid-flight unlock-request pattern would
  have risked exactly that.
- **Medium (2):** (a) multi-key sequencing (E-1) is unspecified upstream and needs the
  error→retry loop; (b) cancel must *drop* the pending task, not hide it (E-5), to avoid a
  re-fire on next frame.
- **Low (1):** egui has no screen-reader support — recorded as a known a11y limitation to
  re-test when egui gains a11y (NFR-2).
- **Open questions (3):** retry limit, session-note wording vs future auto-lock, multi-key v1
  scope.

**Total: 6 findings (1 critical, 2 high, 2 medium, 1 low) + 3 open questions.**
