# Sign-Time Unlock Passphrase Prompt — Test Case Specification

**Feature:** SEC-002 follow-up · Gap PROJ-008 · GitHub issue #90
**Phase:** 1c (Test Case Specification — specifications only, no test code)
**Source of truth:** `docs/ai-design/2026-06-02-signtime-unlock-ux/requirements-and-ux.md` (Diziet, 1a+1b)
**Author:** Marvin (QA)
**Date:** 2026-06-02
**Base:** worktree fast-forwarded to `d6811732`

> Brain the size of a planet, and here I am enumerating passphrase prompts. Still — someone
> should, because the door this spec describes does not yet have a handle wired to it.

---

## 0. Scope, Method & Ground-Truth Notes

These are **specifications**, not Rust. Each case lists ID, description, preconditions, steps,
expected outcome, and the FR/NFR it traces to. Cases are grouped by area. Where a behaviour can
only be exercised against a live network or by a human, it is tagged **[LIVE/MANUAL]**; everything
else is an offline unit or `egui_kittest` test against the (future) `SignUnlockPrompt` component.

**House style to follow (verified):** `tests/kittest/import_single_key.rs` drives the component
directly through `show_in_ui` / `show`, asserts via `query_by_label` / `query_by_label_contains`,
and uses a `force_input_for_test` setter to inject field state without simulated keystrokes
(`tests/kittest/import_single_key.rs:33`). New kittest specs register in `tests/kittest/main.rs`.
The unlock-cache round-trip pattern already exists at `src/wallet_backend/single_key.rs:1140-1186`
and is the template for the backend-level unit cases here.

**Ground-truth re-verification of Diziet's citations (he flagged them as unverified — they now check out, with two material exceptions):**

| Diziet cited | Verified location | Status |
|---|---|---|
| unlock cache `single_key_unlocked` ~202 | `src/wallet_backend/mod.rs:202` | ✅ exact |
| sign-view TODO ~562-566 | `src/wallet_backend/mod.rs:562-567` | ✅ (one line longer) |
| `unlock_with_passphrase` :256 | `src/wallet_backend/single_key.rs:256` | ✅ exact |
| `has_passphrase` :245 | `src/wallet_backend/single_key.rs:245` | ✅ exact |
| `forget_unlocked` :281 | `src/wallet_backend/single_key.rs:281` | ✅ exact |
| `sign_with` :649 | `src/wallet_backend/single_key.rs:649` | ✅ exact |
| error origin (raw_key_bytes) | `src/wallet_backend/single_key.rs:310` | ✅ |
| `SingleKeyPassphraseRequired { addr }` :1380 | `src/backend_task/error.rs:1380-1386` | ✅ exact |
| `SingleKeyPassphraseIncorrect` :1391 | `src/backend_task/error.rs:1391-1392` | ✅ exact |
| `display_task_error -> bool` hook | trait default `src/ui/mod.rs:978`, dispatch `src/ui/mod.rs:1651` | ✅ (default returns `false`) |
| `ImportedKeyNotFound` | `src/backend_task/error.rs:206` | ✅ |
| `ImportedKey.alias` / `passphrase_hint` | `src/model/single_key.rs:18,32` | ✅ |
| `WalletUnlockPopup` modal to clone | `src/ui/components/wallet_unlock_popup.rs` | ✅ |
| `clicked_outside_window` / `modal_overlay()` | `src/ui/helpers.rs:9`, `src/ui/theme.rs:430` | ✅ |

> **Exception G-1 (feeds back to Phase 1b — see §12):** `SingleKeyPassphraseRequired` is currently
> *produced* only at `src/wallet_backend/single_key.rs:310` and *consumed nowhere* in the
> backend-task or UI layer. No production code outside the single-key view calls `sign_with`
> (verified: the only non-test `sign_with` references are the doc-comments at `mod.rs:199,564`).
> The named call sites in the TODO (identity register, send funds, asset-lock signer) are
> **aspirational integration points, not wired today.**
>
> **Exception G-2 (feeds back to Phase 1b — see §12):** the existing `asset_lock_signer.rs`
> (`WalletAssetLockSigner`, `src/wallet_backend/asset_lock_signer.rs:52`) is an **HD seed-snapshot**
> signer for `register_identity_with_funding` / `top_up_identity_with_funding`. It does **not** call
> `single_key().sign_with()` and cannot return `SingleKeyPassphraseRequired`. Spec §1/§7 list it as
> a gate call site; that is only true if a *single-key* asset-lock signing path is added. Cases in
> §9 are therefore written against the **gate contract** (any task that surfaces the error), with
> the asset-lock single-key path explicitly marked **[NOT-YET-WIRED]**.

---

## 1. Area: Gate trigger — cache MISS vs HIT (FR-1, E-3)

### TC-UNLOCK-001 — Cache MISS surfaces the typed error from the sign path
- **Description:** Signing against a protected, uncached key returns `SingleKeyPassphraseRequired { addr }`, the sole trigger for the prompt.
- **Preconditions:** Fresh `SingleKeyView` over a temp vault; one WIF imported *with* a passphrase (`has_passphrase = true`); unlock cache empty for that address.
- **Steps:** 1) Import the protected key. 2) Call `sign_with(addr, &[0u8;32])` without unlocking.
- **Expected:** `Err(TaskError::SingleKeyPassphraseRequired { addr })` with `addr` equal to the imported address; no signature produced; cache still empty.
- **Traceability:** FR-1 (prompt appears iff this error is returned).
- **Type:** offline unit (mirrors `single_key.rs:1182`).

### TC-UNLOCK-002 — Cache HIT signs silently, no error, no prompt
- **Description:** A previously-unlocked key signs without re-prompting (steady state for the Power User).
- **Preconditions:** Protected key imported; `unlock_with_passphrase(addr, correct)` already called this session.
- **Steps:** 1) Unlock. 2) Call `sign_with(addr, &msg)`.
- **Expected:** `Ok(Signature)`; no `SingleKeyPassphraseRequired` is ever produced; therefore no prompt path is entered.
- **Traceability:** FR-1 (second G/W/T), FR-6, E-3.
- **Type:** offline unit.

### TC-UNLOCK-003 — Unprotected key never triggers the gate
- **Description:** An imported key with no passphrase signs directly; the prompt must never appear for it.
- **Preconditions:** WIF imported *without* a passphrase (`has_passphrase = false`).
- **Steps:** 1) Import. 2) Call `sign_with(addr, &msg)` on a cold cache.
- **Expected:** `Ok(Signature)`; `raw_key_bytes` decrypts with `None` (path at `single_key.rs:314`); no error, no prompt.
- **Traceability:** FR-1 ("never for an unprotected key").
- **Type:** offline unit.

### TC-UNLOCK-004 — Import does not arm the gate (import primes the cache)
- **Description:** Immediately after a passphrase-protected import, a sign succeeds without a prompt because import populated the cache (`import_wif` cache insert at `single_key.rs:230-233`).
- **Preconditions:** None; fresh view.
- **Steps:** 1) `import_wif_with_passphrase(wif, pass)`. 2) Without any explicit unlock, call `sign_with(addr, &msg)`.
- **Expected:** `Ok(Signature)`; no `SingleKeyPassphraseRequired`. (Asserts FR-1's "never on import — import primes the cache.")
- **Traceability:** FR-1.
- **Type:** offline unit.

### TC-UNLOCK-005 — UI gate: `display_task_error` opens the prompt on the error, suppresses generic banner
- **Description:** The triggering screen's `display_task_error` returns `true` and opens `SignUnlockPrompt` only for `SingleKeyPassphraseRequired`; for all other errors it returns `false` (generic banner shown).
- **Preconditions:** A screen instance owning an `Option<SignUnlockPrompt>`, initially `None`.
- **Steps:** 1) Call `display_task_error(&SingleKeyPassphraseRequired{addr})`. 2) Separately call `display_task_error(&ImportedKeyNotFound)`.
- **Expected:** Step 1 → returns `true`, prompt becomes `Some` and `is_open()`. Step 2 → returns `false`, prompt stays `None`.
- **Traceability:** FR-1, NFR-4 (banner suppression contract, `src/ui/mod.rs:978`).
- **Type:** offline unit / kittest.

---

## 2. Area: Prompt content & identification (FR-2, §4.4 copy)

### TC-PROMPT-001 — Aliased key names the alias in plain language
- **Description:** When the key has an alias, the body reads with `{ $key_name }` = alias and contains no jargon.
- **Preconditions:** Prompt opened for an address whose `ImportedKey.alias = Some("Savings sweep")`.
- **Steps:** Render via `show`/`show_in_ui`; query labels.
- **Expected:** A label matching `The key "Savings sweep" is locked.` is present; the continuation `Enter the passphrase you set for it to continue.` is present; **none** of `sign`, `ECDSA`, `secp256k1`, `state transition`, `cache` appear anywhere in the rendered tree.
- **Traceability:** FR-2 (alias case), NFR-3 (single translation unit).
- **Type:** kittest.

### TC-PROMPT-002 — Aliasless key falls back to the Base58 address
- **Description:** With no alias, the body identifies the key by its address (`{ $address }`).
- **Preconditions:** Prompt opened for an address whose `ImportedKey.alias = None`.
- **Steps:** Render; query labels.
- **Expected:** A label containing the literal Base58 address and the word `locked` is present; the alias-quoted form is absent.
- **Traceability:** FR-2 (no-alias case), CLAUDE.md rule 6.
- **Type:** kittest.

### TC-PROMPT-003 — Hint line shown only when a hint exists
- **Description:** `Hint: { $hint }` renders iff `passphrase_hint` is `Some`.
- **Preconditions:** Two sub-cases: (a) `passphrase_hint = Some("the xkcd one")`; (b) `passphrase_hint = None`.
- **Steps:** Render each.
- **Expected:** (a) a label `Hint: the xkcd one` is present. (b) no label beginning `Hint:` is present.
- **Traceability:** FR-2.
- **Type:** kittest (two sub-tests).

### TC-PROMPT-004 — Session mental-model line is present
- **Description:** The "unlocked until app closes" note is rendered to set FR-6 expectation.
- **Preconditions:** Prompt open.
- **Steps:** Render; query.
- **Expected:** Label `This key stays unlocked until you close the app.` is present.
- **Traceability:** FR-6, §4.4. (Note: wording is Open Question §6.2 — see §12 G-4.)
- **Type:** kittest.

### TC-PROMPT-005 — Required chrome present: title, masked field, Unlock, Cancel
- **Description:** Title, a masked passphrase field, and both buttons render in every open state.
- **Preconditions:** Prompt open.
- **Steps:** Render; query.
- **Expected:** Labels `Unlock imported key` (title), `Unlock`, `Cancel` present; the passphrase field is masked by default (reuses `PasswordInput`, `TextEdit::password(true)` — mirrors `tc_sk_007`). Field placeholder/label `Passphrase` reachable.
- **Traceability:** FR-2, NFR-4.
- **Type:** kittest.

### TC-PROMPT-006 — Unlock disabled while field empty
- **Description:** Empty passphrase must not be submittable (E-7).
- **Preconditions:** Prompt open, field empty.
- **Steps:** Render; inspect Unlock enablement; attempt Enter on empty field.
- **Expected:** Unlock is disabled (or Enter is a no-op); `unlock_with_passphrase` is **not** called with an empty string.
- **Traceability:** E-7, §4.5 (Unlock enabled only when field non-empty).
- **Type:** kittest / unit.

---

## 3. Area: Correct passphrase → auto-resume (FR-3)

### TC-RESUME-001 — Correct passphrase unlocks the cache
- **Description:** `unlock_with_passphrase` with the right passphrase populates the cache so a subsequent `sign_with` succeeds.
- **Preconditions:** Protected key imported, cache forgotten (`forget_unlocked`).
- **Steps:** 1) `forget_unlocked(addr)`. 2) `sign_with` → expect the required-error. 3) `unlock_with_passphrase(addr, correct)`. 4) `sign_with` again.
- **Expected:** Step 2 errors; step 3 `Ok(())`; step 4 `Ok(Signature)`.
- **Traceability:** FR-3 (substrate), FR-6.
- **Type:** offline unit (template at `single_key.rs:1140-1186`).

### TC-RESUME-002 — Prompt re-dispatches the *stashed* original task on success
- **Description:** On correct passphrase the screen closes the prompt and re-emits the **same** `AppAction::BackendTask` it stashed when the gate fired — the user does not re-initiate.
- **Preconditions:** Screen stashed a pending `BackendTask` (e.g. a send) when `display_task_error` opened the prompt.
- **Steps:** 1) Open prompt with a stashed task. 2) Enter correct passphrase, confirm.
- **Expected:** The screen's `ui()` returns/produces `AppAction::BackendTask(<the same task instance>)`; the prompt closes; the stash is cleared so it does **not** fire a second time on the next frame.
- **Traceability:** FR-3, §4.2 auto-resume, §4.5 closing-success.
- **Type:** offline unit on the screen's state machine (assert emitted `AppAction`).

### TC-RESUME-003 — Operation inputs survive the round-trip
- **Description:** Amount / recipient / identity selection entered before the gate are unchanged after auto-resume.
- **Preconditions:** Send screen with amount + recipient populated; gate fires.
- **Steps:** 1) Trigger send → gate. 2) Unlock with correct passphrase.
- **Expected:** The re-dispatched task carries the original amount and recipient; on-screen fields still show them.
- **Traceability:** FR-3, §4.1 (prompt owned by triggering screen so inputs survive).
- **Type:** kittest / unit.

### TC-RESUME-004 — End-to-end auto-resume completes the operation
- **Description:** Full happy path: protected send fails on lock, prompt, unlock, send completes against the network.
- **Preconditions:** Funded single-key wallet with a passphrase-protected key on testnet; SPV synced.
- **Steps:** 1) Compose + submit a send. 2) Prompt opens; enter correct passphrase. 3) Observe completion.
- **Expected:** Send broadcasts and confirms; no second prompt; banner shows success.
- **Traceability:** FR-3 (full acceptance), FR-1.
- **Type:** **[LIVE/MANUAL]** — requires funded testnet wallet + broadcast. Offline coverage is TC-RESUME-002/003.

---

## 4. Area: Wrong passphrase — recoverable in place (FR-4)

### TC-WRONG-001 — Wrong passphrase returns the typed incorrect error
- **Description:** `unlock_with_passphrase` with a wrong passphrase returns `SingleKeyPassphraseIncorrect` and does **not** populate the cache.
- **Preconditions:** Protected key imported; cache forgotten.
- **Steps:** 1) `forget_unlocked`. 2) `unlock_with_passphrase(addr, "wrong")`.
- **Expected:** `Err(TaskError::SingleKeyPassphraseIncorrect)`; subsequent `sign_with` still returns `SingleKeyPassphraseRequired` (cache untouched).
- **Traceability:** FR-4, NFR-1 (fieldless variant reused).
- **Type:** offline unit.

### TC-WRONG-002 — Inline error derived from the typed variant, not a literal
- **Description:** The screen shows the wrong-passphrase message by rendering the `Display` of `SingleKeyPassphraseIncorrect`, not a parallel hardcoded string.
- **Preconditions:** Prompt open; backend returns `SingleKeyPassphraseIncorrect`.
- **Steps:** Drive a wrong-passphrase attempt; query labels.
- **Expected:** Rendered inline message equals the variant's `Display`: `That passphrase is not correct. Try again.` (`error.rs:1391`). The message is non-alarming and jargon-free.
- **Traceability:** FR-4, §4.4 note (no second copy), CLAUDE.md anti-string-parsing discipline.
- **Type:** kittest.

### TC-WRONG-003 — Field cleared and re-focused after a wrong attempt; operation still pending
- **Description:** After a wrong passphrase the field is emptied, focus returns to it, and the pending task is **not** dropped.
- **Preconditions:** Prompt open with a stashed task.
- **Steps:** 1) Enter wrong passphrase, confirm. 2) Inspect field + stash.
- **Expected:** Passphrase field is empty; the field holds focus; prompt remains open; the stashed task is still present (would re-dispatch on a later success).
- **Traceability:** FR-4, §4.5 Error state.
- **Type:** kittest / unit.

### TC-WRONG-004 — Unlimited retry, no lockout, no cooldown
- **Description:** Many consecutive wrong attempts neither lock the prompt nor introduce a delay; a final correct attempt still unlocks.
- **Preconditions:** Protected key; prompt open.
- **Steps:** 1) Submit N (e.g. 20) wrong passphrases. 2) Submit the correct one.
- **Expected:** No attempt is rejected for rate-limit reasons; no cooldown state appears; the correct attempt at the end unlocks normally.
- **Traceability:** NFR-5, E-4. (Open Question §6.1 — Security may impose a soft cap; see §12 G-3. If a cap lands, this case must be re-specified.)
- **Type:** kittest / unit.

---

## 5. Area: Cancel / abort — drop the pending task (FR-5, E-5)

> Common assertion for this area: cancel must **drop** the stashed task (not hide it), so it never
> re-fires on a later frame, AND the secret must be zeroized on the way out (NFR-1).

### TC-CANCEL-001 — Cancel button drops the task and preserves inputs
- **Description:** Clicking Cancel closes the prompt, drops the pending task, and returns to the triggering screen with inputs intact.
- **Preconditions:** Send screen, amount+recipient entered, prompt open with stashed task.
- **Steps:** 1) Click Cancel.
- **Expected:** Prompt closed; stash is `None`; the screen does **not** emit the task on this or any later frame; amount + recipient unchanged; nothing signed/broadcast.
- **Traceability:** FR-5, E-5, §4.5 closing-cancel.
- **Type:** kittest / unit.

### TC-CANCEL-002 — Escape cancels
- **Description:** Escape dismisses the prompt with the same drop semantics.
- **Preconditions:** Prompt open with stashed task.
- **Steps:** 1) Press Escape.
- **Expected:** As TC-CANCEL-001 (closed, stash dropped, inputs intact).
- **Traceability:** FR-5, NFR-2 (Escape=cancel), §4.5.
- **Type:** kittest.

### TC-CANCEL-003 — Window X cancels
- **Description:** The title-bar X closes with drop semantics (mirrors `WalletUnlockPopup` `is_open` handling at `wallet_unlock_popup.rs:199`).
- **Preconditions:** Prompt open with stashed task.
- **Steps:** 1) Toggle the window closed via X.
- **Expected:** As TC-CANCEL-001.
- **Traceability:** FR-5, NFR-4.
- **Type:** kittest.

### TC-CANCEL-004 — Click-outside overlay cancels
- **Description:** A click on the dimmed overlay cancels (uses `clicked_outside_window`, `helpers.rs:9`).
- **Preconditions:** Prompt open with stashed task.
- **Steps:** 1) Simulate a pointer click outside the modal rect.
- **Expected:** As TC-CANCEL-001.
- **Traceability:** FR-5, NFR-4.
- **Type:** kittest.

### TC-CANCEL-005 — Dropped task does not re-fire on the next frame (regression guard)
- **Description:** After any cancel path, advancing several frames must not re-emit the task — the spec's explicit anti-pattern (Medium finding §Candy Tally).
- **Preconditions:** Prompt cancelled via TC-CANCEL-001.
- **Steps:** 1) Cancel. 2) Run the screen `ui()` for ≥3 additional frames.
- **Expected:** No `AppAction::BackendTask` is emitted in any subsequent frame.
- **Traceability:** FR-5, E-5 ("dropped, not merely hidden").
- **Type:** kittest / unit.

---

## 6. Area: Security — passphrase confinement (NFR-1)

### TC-SEC-001 — Passphrase never enters a `TaskError`
- **Description:** No `TaskError` variant on this path carries the passphrase. `SingleKeyPassphraseRequired` carries only `addr`; `SingleKeyPassphraseIncorrect` is fieldless.
- **Preconditions:** Static/structural — the variants at `error.rs:1380-1392`.
- **Steps:** 1) Produce `SingleKeyPassphraseRequired` and `SingleKeyPassphraseIncorrect`. 2) Format each via `Display` and `Debug`.
- **Expected:** Neither `Display` nor `Debug` output contains the supplied passphrase string for any chosen passphrase value (assert by passing a unique sentinel passphrase, e.g. `"ZZsentinelZZ"`, and verifying it is absent from both renderings of both variants).
- **Traceability:** NFR-1, CLAUDE.md rule 7.
- **Type:** offline unit.

### TC-SEC-002 — Passphrase never crosses the async channel (`AppAction` / `BackendTask`)
- **Description:** The re-dispatched `AppAction::BackendTask` is the *same* operation task and contains no passphrase field; the unlock happened in the in-process cache before re-dispatch.
- **Preconditions:** Screen with a stashed task; unlock performed.
- **Steps:** 1) Open gate, stash task. 2) Unlock. 3) Inspect the re-dispatched `BackendTask`.
- **Expected:** The re-dispatched task is structurally equal to the original; no passphrase value is present in the `AppAction` / `BackendTask` (assert sentinel passphrase absent from a `Debug` of the emitted action).
- **Traceability:** NFR-1, §6 (gate-on-error keeps secrets off the channel).
- **Type:** offline unit on the screen state machine.

### TC-SEC-003 — Passphrase never logged or placed in banner details
- **Description:** No log line and no `MessageBanner` details panel receives the passphrase across success, wrong, and cancel paths.
- **Preconditions:** Prompt exercised for all three outcomes with a sentinel passphrase.
- **Steps:** 1) Capture emitted log records (tracing subscriber) and any banner-details payload during each path.
- **Expected:** Sentinel passphrase absent from every captured log record and from every banner `with_details` payload.
- **Traceability:** NFR-1, A09 logging hygiene.
- **Type:** offline unit (with a capturing tracing layer). **Partly [MANUAL]** for the visual banner-details panel if not assertable in kittest.

### TC-SEC-004 — Secret zeroized on every close path
- **Description:** The `PasswordInput`'s zeroizing `Secret` is `clear()`-ed on success, cancel, X, Escape, click-outside, and on every wrong-passphrase retry.
- **Preconditions:** Prompt open for each close path.
- **Steps:** For each path: type a passphrase, trigger the path, then inspect the field's exposed text.
- **Expected:** After every path the field text is empty; the prompt's `close()`/`clear()` is invoked (mirrors `wallet_unlock_popup.rs:55-59,192`). Best-effort: the underlying buffer is zeroized via the `Secret` type's `Drop`/`clear`.
- **Traceability:** NFR-1, §4.5 (all close states clear the secret).
- **Type:** kittest / unit. (True memory-zeroization is a property of the reused `PasswordInput`/`Secret` type — assert `clear()` is called; deep zeroization is covered by that type's own tests.)

---

## 7. Area: Multi-key sequencing (FR-7, E-1)

### TC-MULTI-001 — Two protected keys in one operation prompt sequentially
- **Description:** An operation needing two uncached protected keys (K1, K2) drains them one prompt at a time via the error→prompt→retry loop.
- **Preconditions:** Two protected keys imported; both cache-cold; an operation that signs with K1 then K2.
- **Steps:** 1) Trigger → backend returns `SingleKeyPassphraseRequired{K1}`. 2) Unlock K1 → re-dispatch. 3) Backend returns `SingleKeyPassphraseRequired{K2}`. 4) Unlock K2 → re-dispatch. 5) Operation completes.
- **Expected:** Exactly two prompts, each naming its own key (K1 then K2); only one prompt visible at a time; operation completes only after both unlocks. No batched/stacked modal.
- **Traceability:** FR-7, E-1.
- **Type:** offline unit on the loop (drive the error sequence); **[LIVE/MANUAL]** for a real multi-key broadcast.

### TC-MULTI-002 — Each prompt names the correct key
- **Description:** The prompt for K2 shows K2's alias/address, not K1's (no stale identity carried over).
- **Preconditions:** As TC-MULTI-001; K1 and K2 have distinct aliases.
- **Steps:** Inspect the body label at each step.
- **Expected:** First prompt names K1; after K1 unlock, second prompt names K2.
- **Traceability:** FR-7, FR-2.
- **Type:** kittest / unit.

---

## 8. Area: Concurrency & focus trap (E-2, NFR-2)

### TC-CONCUR-001 — Underlying screen cannot dispatch a second op while prompt is open
- **Description:** While the modal is open, the focus-trapping overlay blocks the triggering control, so no second operation can be initiated.
- **Preconditions:** Prompt open over the send screen.
- **Steps:** 1) With the prompt open, attempt to click the screen's Send/primary control.
- **Expected:** The click does not reach the underlying control; no second `BackendTask` is emitted; only the prompt is interactive.
- **Traceability:** E-2, NFR-2 (focus trap).
- **Type:** kittest.

### TC-CONCUR-002 — Duplicate `SingleKeyPassphraseRequired` for the same addr is ignored
- **Description:** A second identical required-error arriving while the prompt is open for that same address does not stack a second modal.
- **Preconditions:** Prompt open for addr K.
- **Steps:** 1) Deliver a second `display_task_error(SingleKeyPassphraseRequired{K})`.
- **Expected:** Still exactly one prompt; no re-init that wipes a partially-typed field unexpectedly; returns `true` (suppressed) without opening a new modal.
- **Traceability:** E-2 (same-address duplicate ignored).
- **Type:** unit.

### TC-CONCUR-003 — Required-error for a *different* addr while open is queued, not stacked
- **Description:** A required-error for a different address arriving while a prompt is open is queued (handled after the current key), never shown as a second simultaneous modal.
- **Preconditions:** Prompt open for K1; deliver `SingleKeyPassphraseRequired{K2}`.
- **Steps:** 1) Deliver the K2 error. 2) Resolve K1 (unlock or cancel). 3) Observe.
- **Expected:** Only one modal at any instant; after K1 resolves, the K2 prompt may surface (queued). No two modals on screen at once.
- **Traceability:** E-2 (defensive queue rule). **Note: §12 G-5** — the spec's *primary* multi-key mechanism is the sequential retry loop (FR-7); the "queue a different addr" rule is a defensive edge under the gate pattern and may be hard to reach in practice. Flag as **defensive / may be unreachable**.
- **Type:** unit (defensive).

---

## 9. Area: Per-call-site gate coverage (FR-1, NFR-6) — operation-agnostic

> NFR-6: the prompt keys off `addr`, not operation type. These cases assert each named call site,
> once wired, surfaces the gate. **All three are presently [NOT-YET-WIRED] (see §0 Exception G-1).**

### TC-SITE-001 — Send funds (single-key) exercises the gate — [NOT-YET-WIRED]
- **Description:** The single-key send path, when it signs with a protected uncached key, surfaces `SingleKeyPassphraseRequired` and the send screen opens the prompt.
- **Preconditions:** A single-key send backend task that calls `single_key().sign_with(...)`. **Does not exist today** — `SingleKeyWalletSendScreen` exists but no production `sign_with` call site is wired.
- **Steps:** 1) Compose send with a protected key. 2) Submit.
- **Expected:** Gate fires; prompt opens over the send screen; auto-resume on unlock.
- **Traceability:** FR-1, NFR-6, TODO `mod.rs:564`.
- **Type:** offline unit once wired; **[LIVE/MANUAL]** for full broadcast. **Currently a coverage gap to track (§12 G-1).**

### TC-SITE-002 — Identity register exercises the gate — [NOT-YET-WIRED]
- **Description:** Registering an identity funded by a protected single-key surfaces the gate.
- **Preconditions:** An identity-register path that funds/signs via `single_key().sign_with(...)`. Not wired today.
- **Steps:** 1) Start identity registration with a protected key. 2) Proceed to the signing step.
- **Expected:** Gate fires on the register screen; auto-resume completes registration.
- **Traceability:** FR-1, NFR-6.
- **Type:** offline unit once wired; **[LIVE/MANUAL]** for broadcast.

### TC-SITE-003 — Asset-lock signing exercises the gate — [NOT-YET-WIRED / DESIGN-MISMATCH]
- **Description:** Spec lists "asset-lock signer" as a gate call site. The current `WalletAssetLockSigner` is an **HD seed-snapshot** signer (`asset_lock_signer.rs:52`) that does not call `single_key().sign_with()` and cannot return `SingleKeyPassphraseRequired`.
- **Preconditions:** A *single-key* asset-lock signing path would have to exist. It does not.
- **Steps:** N/A until such a path is added.
- **Expected (if/when added):** Gate fires; prompt opens; auto-resume.
- **Traceability:** FR-1, NFR-6 — but **see §12 G-2**: this call site is mis-attributed in the spec for the present codebase.
- **Type:** **[NOT-TESTABLE TODAY]** — feed the mismatch back to Phase 1b.

---

## 10. Area: Accessibility (NFR-2)

### TC-A11Y-001 — Field auto-focused on open
- **Description:** On open, the passphrase field requests focus once (mirrors `wallet_unlock_popup.rs:134-137`).
- **Preconditions:** Prompt freshly opened.
- **Steps:** Render the first frame; inspect focused widget.
- **Expected:** The passphrase field holds focus after open.
- **Traceability:** NFR-2.
- **Type:** kittest.

### TC-A11Y-002 — Enter submits, Escape cancels
- **Description:** Enter on a non-empty field attempts unlock; Escape cancels.
- **Preconditions:** Prompt open, non-empty field.
- **Steps:** 1) Press Enter → expect unlock attempt. 2) Re-open; press Escape → expect cancel.
- **Expected:** Enter triggers `unlock_with_passphrase`; Escape closes + drops task.
- **Traceability:** NFR-2, FR-5.
- **Type:** kittest.

### TC-A11Y-003 — Tab order: Field → Unlock → Cancel (layout order)
- **Description:** Tab traversal matches the documented order and the right-aligned Unlock-rightmost layout.
- **Preconditions:** Prompt open.
- **Steps:** Tab from the field and record focus order.
- **Expected:** Focus moves Field → Unlock → Cancel per `docs/ux-design-patterns.md` §10.
- **Traceability:** NFR-2.
- **Type:** kittest.

### TC-A11Y-004 — Every element legible by text alone (visible labels, no icon-only meaning)
- **Description:** Title, body, hint, session note, field label, error, and buttons are all real visible text labels reachable in the accessibility tree.
- **Preconditions:** Prompt open in both idle and error states.
- **Steps:** Query each label via `query_by_label`.
- **Expected:** All listed strings reachable as labels; no meaning conveyed only by an icon (the reveal eye uses tooltip `Hold to reveal`, mirroring `tc_sk_007`).
- **Traceability:** NFR-2 ("legible by text alone").
- **Type:** kittest.

### TC-A11Y-005 — Screen-reader annotation gap (known limitation) — KNOWN-GAP
- **Description:** egui exposes no screen-reader annotations (`docs/ux-design-patterns.md:172`); this is recorded, not hidden. Test asserts the *mitigation* (text legibility) holds, and documents the gap.
- **Preconditions:** Prompt open.
- **Steps:** Confirm all semantics are carried by visible text (per TC-A11Y-004).
- **Expected:** Mitigation holds. The screen-reader gap itself is **not** assertable in egui today → recorded as a known limitation to re-test when egui gains a11y.
- **Traceability:** NFR-2 (known limitation).
- **Type:** kittest for the mitigation; **[KNOWN-GAP / MANUAL re-test]** for the SR gap.

---

## 11. Area: Negative / edge — invalidation while prompt open (E-6, E-7, E-8)

### TC-EDGE-001 — Key removed between trigger and unlock → `ImportedKeyNotFound`, close, no loop
- **Description:** If the key is `forget`-ten after the gate fires, `unlock_with_passphrase` returns `ImportedKeyNotFound`; the prompt surfaces a plain message and closes — it does not loop.
- **Preconditions:** Protected key; gate fired; then `forget(addr)` called.
- **Steps:** 1) Open prompt. 2) `forget(addr)`. 3) Enter passphrase, confirm.
- **Expected:** `unlock_with_passphrase` → `Err(ImportedKeyNotFound)`; prompt closes; a plain message (e.g. "This imported key is no longer available.") is shown via the normal error path; the pending task is dropped; no re-prompt loop.
- **Traceability:** E-6.
- **Type:** offline unit + kittest. **Note §12 G-6:** the exact user-facing string for the removed-key case is **not specified** in §4.4; the spec only gives example prose in E-6. Ambiguity to resolve in 1b.

### TC-EDGE-002 — Empty passphrase never calls the unlock API
- **Description:** Reinforces TC-PROMPT-006 at the API boundary: an empty field must not invoke `unlock_with_passphrase("")`.
- **Preconditions:** Prompt open, empty field.
- **Steps:** 1) Force-submit with empty field (if reachable).
- **Expected:** `unlock_with_passphrase` is not called; no error toast; Unlock stays disabled / no-op.
- **Traceability:** E-7.
- **Type:** unit.

### TC-EDGE-003 — Network switch (`change_context`) while prompt open → close + drop + clear
- **Description:** On `change_context` the prompt closes, the pending task (belonging to the previous network) is dropped, and the secret is cleared.
- **Preconditions:** Prompt open with stashed task on network A.
- **Steps:** 1) Invoke the screen's `change_context(network B)` (signature `src/ui/mod.rs:770`).
- **Expected:** Prompt closed; stash dropped (no re-fire); passphrase field cleared/zeroized; no operation runs on either network.
- **Traceability:** E-8, NFR-1.
- **Type:** unit / kittest.

### TC-EDGE-004 — Wallet switch while prompt open → close + drop + clear
- **Description:** Same as TC-EDGE-003 for an in-network wallet change, if the screen supports switching the active wallet while a prompt is open.
- **Preconditions:** Prompt open with stashed task for wallet W1.
- **Steps:** 1) Switch active wallet to W2.
- **Expected:** Prompt closed; stash dropped; secret cleared. (If wallet switch is not reachable while the modal traps focus, this collapses into TC-CONCUR-001 — note that.)
- **Traceability:** E-8 (generalised), NFR-1.
- **Type:** unit / kittest.

---

## 12. Findings: untestable / ambiguous requirements (feedback to Phase 1b)

> These are the mismatches and gaps QA surfaced while deriving cases. Each is a **win** logged for
> the design loop, not a blocker for writing the specs above.

- **G-1 (HIGH) — The gate is wired nowhere.** `SingleKeyPassphraseRequired` is produced only at
  `single_key.rs:310` and consumed by no backend task or screen. The §1/§7/§9 "call sites" are
  aspirational. TC-SITE-001/002/003 cannot pass until the TODO at `mod.rs:564` is implemented.
  *Action for 1b/2:* specify which concrete backend tasks gain `sign_with` and where
  `display_task_error` opens the prompt.
- **G-2 (HIGH) — Asset-lock signer mis-attributed.** `WalletAssetLockSigner`
  (`asset_lock_signer.rs:52`) is an HD seed-snapshot signer and never emits
  `SingleKeyPassphraseRequired`. Listing it as a gate call site (spec §1, §7, traceability) is
  incorrect for the current code. *Action:* either add a single-key asset-lock signing path or
  drop the asset-lock claim from the call-site list.
- **G-3 (MEDIUM) — Retry policy is an open question (NFR-5 / E-4 / §6.1).** TC-WRONG-004 assumes
  *no* cap/cooldown. If Security imposes a soft cap, TC-WRONG-004 must be rewritten. Decision
  needed before Phase 2.
- **G-4 (LOW) — Session-note wording is unsettled (§6.2).** TC-PROMPT-004 asserts the literal
  "until you close the app"; if an idle auto-lock is later planned the copy ("for a while") and the
  test must change. Confirm the wording is committed.
- **G-5 (LOW) — "Queue a different-addr error" rule may be unreachable.** Under the gate-on-error
  loop, a *second* required-error only arrives after the first key is unlocked and the task
  re-dispatches; a simultaneous different-addr error while a modal traps focus is hard to produce.
  TC-CONCUR-003 is written defensively. Confirm whether this path is reachable or should be dropped.
- **G-6 (LOW) — Removed-key message string unspecified.** E-6 gives only example prose; §4.4 has no
  slot for the `ImportedKeyNotFound` user message on this path. TC-EDGE-001 asserts behaviour, not
  exact copy. Add the string to the copy table or designate the reused `Display`.
- **G-7 (INFO) — Deep zeroization not directly assertable.** TC-SEC-004 can assert `clear()` is
  called on close, but true memory zeroization is a property of the reused `PasswordInput`/`Secret`
  type, not of this component. Noted so no one over-claims the test.

---

## 13. Coverage Summary by Area

| Area | Cases | of which [LIVE/MANUAL] | of which [NOT-YET-WIRED] |
|---|---|---|---|
| §1 Gate trigger (MISS/HIT) | 5 (TC-UNLOCK-001..005) | 0 | 0 |
| §2 Prompt content | 6 (TC-PROMPT-001..006) | 0 | 0 |
| §3 Correct → auto-resume | 4 (TC-RESUME-001..004) | 1 (004) | 0 |
| §4 Wrong passphrase | 4 (TC-WRONG-001..004) | 0 | 0 |
| §5 Cancel / abort | 5 (TC-CANCEL-001..005) | 0 | 0 |
| §6 Security confinement | 4 (TC-SEC-001..004) | partial (003 panel) | 0 |
| §7 Multi-key sequencing | 2 (TC-MULTI-001..002) | 1 (001 broadcast) | 0 |
| §8 Concurrency / focus trap | 3 (TC-CONCUR-001..003) | 0 | 0 |
| §9 Per-call-site gate | 3 (TC-SITE-001..003) | 2 (broadcast) | 3 (all) |
| §10 Accessibility | 5 (TC-A11Y-001..005) | 1 (005 SR re-test) | 0 |
| §11 Negative / edge | 4 (TC-EDGE-001..004) | 0 | 0 |
| **Total** | **45 cases** | **~6 live/manual** | **3 not-yet-wired** |

Plus **7 findings** (G-1..G-7) fed back to Phase 1b.

---

## Candy Tally 🍬 (QA findings surfaced)

- **High (2):** G-1 (gate wired nowhere), G-2 (asset-lock signer mis-attributed).
- **Medium (1):** G-3 (retry policy open).
- **Low (3):** G-4 (session-note wording), G-5 (queue rule possibly unreachable), G-6 (removed-key
  copy unspecified).
- **Info (1):** G-7 (deep zeroization not directly assertable).

**Total: 7 findings (0 critical, 2 high, 1 medium, 3 low, 1 info).** 🍬🍬🍬🍬🍬🍬🍬
</content>
