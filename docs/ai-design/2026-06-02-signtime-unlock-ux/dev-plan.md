# Sign-Time Unlock Passphrase Prompt — Development Plan

**Feature:** SEC-002 follow-up · Gap PROJ-008 · GitHub issue #90
**Phase:** 1d (Architecture & Development Plan)
**Status:** Design only. No implementation. This is the contract Phase 2 (Bilby) builds from.
**Author:** Nagatha (Software Architect)
**Date:** 2026-06-02
**Base:** worktree fast-forwarded to `e6c6c017`

**Inputs:**
- `docs/ai-design/2026-06-02-signtime-unlock-ux/requirements-and-ux.md` (Diziet, 1a+1b) — chosen integration: **gate-on-error with auto-retry**.
- `docs/ai-design/2026-06-02-signtime-unlock-ux/test-cases.md` (Marvin, 1c, 45 cases + findings G-1..G-7).

> One observes the whole board before moving a single piece. Diziet designed the door; Marvin
> noticed it opens onto a wall. This plan first establishes where the wall actually is — verified
> against `e6c6c017`, not against the line numbers an earlier pass quoted — and only then lays
> the track. Every structural claim below was re-grepped after the base merge.

---

## 0. Ground Truth (re-verified at `e6c6c017`)

I treated G-1 and G-2 as hypotheses to be tested against the code, not as settled facts. Both
survive contact with the source. The findings are sharper than the spec assumed.

### 0.1 The signing surface — two disjoint paths

| Path | Secret material | Unlock mechanism | Sign entry point | Emits `SingleKeyPassphraseRequired`? |
|---|---|---|---|---|
| **HD seed** | 64-byte BIP-39 seed snapshot | `WalletUnlockPopup` → `wallet_seed.open(pw)` → `handle_wallet_unlocked` → `provide_seed` | `WalletAssetLockSigner` (`asset_lock_signer.rs:52`) via `signer_for(seed_hash)` (`mod.rs:1079`) | **No** — never calls `single_key()` |
| **Single key (imported WIF)** | per-key 32 bytes, AES-GCM under a per-key passphrase | `SingleKeyView::unlock_with_passphrase` → in-process `single_key_unlocked` cache | `SingleKeyView::sign_with` (`single_key.rs:649`) → `raw_key_bytes` (`:293`) | **Yes** — `single_key.rs:310` |

These paths share no signer. The sign-time prompt belongs **exclusively** to the single-key path.

### 0.2 G-1 confirmed — the gate is wired into nothing, and the in-scope operation is a stub

- `SingleKeyPassphraseRequired` is **produced** only at `single_key.rs:310` (inside `raw_key_bytes`,
  reached only through `sign_with`).
- It is **consumed** by no backend task and no screen. No `display_task_error` override matches it.
- Every non-test `sign_with` reference is a doc-comment (`mod.rs:198`, `mod.rs:669`). The only real
  callers are the unit tests inside `single_key.rs`.
- The one operation whose unit of action *is* an imported single key —
  `send_single_key_wallet_payment` (`send_single_key_wallet_payment.rs:14`) — currently returns
  `Err(TaskError::SingleKeyWalletsUnsupported)` unconditionally. It is a stub; it does not call
  `sign_with` at all. Single-key sends are also rejected at `core/mod.rs:218` and `:304`, and
  `refresh_single_key_wallet_info` (`:16`) is likewise stubbed.

**Verdict on G-1: the feature must INTRODUCE the single-key sign call site, not wire an existing
one.** There is no production `sign_with` caller to "hook". The honest framing of PROJ-008 is two
coupled pieces of work: (a) implement a real single-key send that signs via `sign_with`, and (b)
catch the resulting `SingleKeyPassphraseRequired` in the UI and auto-retry. Without (a) there is
nothing for (b) to react to.

### 0.3 G-2 confirmed — asset-lock signing is HD-only; it is OUT of scope

- `WalletAssetLockSigner` (`asset_lock_signer.rs:52`) owns a `Zeroizing<[u8;64]>` BIP-39 seed
  snapshot and derives via `derive_priv_ecdsa_for_master_seed` (`:78`). It is the **HD** signer.
- `register_identity` (`mod.rs:1224`) and `top_up_identity` (`mod.rs:1266`) are keyed by
  `WalletSeedHash` and obtain their signer **only** through `signer_for(seed_hash)` →
  `WalletAssetLockSigner`. There is no `seed_hash`-free, single-key-funded register/top-up path.
- Therefore neither identity registration nor top-up can today emit `SingleKeyPassphraseRequired`,
  and the asset-lock signer cannot either.

**Verdict on G-2: asset-lock signing is dropped from the prompt's call-site list.** Diziet's §1/§7
attribution is incorrect for the current codebase. A single-key-funded identity registration would
be a substantial separate feature (a new funding path that builds an asset lock from an imported
key's UTXOs and signs the funding sighash with `sign_with`); it is not in scope here and is not
assumed by this plan.

### 0.4 Substrate that already ships (do not redesign)

- `single_key_unlocked: RwLock<BTreeMap<String,[u8;32]>>` — session cache (`mod.rs:201`).
- `SingleKeyView::{has_passphrase (:245), unlock_with_passphrase (:256), forget_unlocked (:281),
  sign_with (:649)}`; `import_wif*` primes the cache (`:230-233`).
- Typed errors `SingleKeyPassphraseRequired { addr }` (`error.rs:1380`) and fieldless
  `SingleKeyPassphraseIncorrect` (`error.rs:1392`) — both already correctly typed, no String payload.
- UI seam `ScreenLike::display_task_error(&mut self, &TaskError) -> bool`, trait default returns
  `false` (`ui/mod.rs:978`); dispatch fan-out at `ui/mod.rs:1651`. Marvin's correction stands: the
  default is `false`, so a screen must *opt in* by overriding and returning `true` to suppress the
  banner.
- `PasswordInput` (`components/password_input.rs`) — zeroizing `Secret`, hold-to-reveal, undoer
  disabled, `clear()`/`take_secret()`/`set_error()` API.
- `WalletUnlockPopup` (`components/wallet_unlock_popup.rs`) — the modal scaffolding to clone:
  overlay (`DashColors::modal_overlay()`), `Align2::CENTER_CENTER` `Window`, `.open(&mut is_open)`
  for X, Escape handling (`:205`), `clicked_outside_window` (`:211`), focus-once (`:134`),
  right-to-left buttons with Unlock rightmost (`:155`).
- **Precedent for the stash pattern:** `SingleKeyWalletSendScreen` already carries a
  `FeeConfirmationDialog { pending_request: Option<WalletPaymentRequest>, ... }`
  (`single_key_send_screen.rs:43-50`) — a confirm-then-resume-the-stashed-request flow. The
  sign-unlock stash is the same shape one level up (it stashes the whole `AppAction`/`BackendTask`).
- `AppAction::BackendTask(BackendTask)` (`app.rs:268`) — the action a screen re-emits to resume.

---

## 1. Scope Reconciliation

### 1.1 In scope (needs the prompt)

**One call site: the single-key wallet send.** Concretely, `send_single_key_wallet_payment`
(`send_single_key_wallet_payment.rs`) must be implemented to sign its funding input(s) with
`single_key().sign_with(addr, sighash)`. When that key is passphrase-protected and cold,
`sign_with` returns `SingleKeyPassphraseRequired { addr }`, which propagates as a `TaskError`
through `run_backend_task` → `TaskResult::Error` → `AppState` → the visible
`SingleKeyWalletSendScreen::display_task_error`. That is the sole gate the prompt reacts to in v1.

This satisfies the operation-agnostic contract Marvin's TC-SITE-001 describes: the screen keys off
`addr` from the error, not off "send".

### 1.2 Explicitly out of scope

- **HD-seed operations** (HD wallet send, HD-funded identity register/top-up, DPNS, votes, token
  ops, DashPay). These unlock via `WalletUnlockPopup` + `wallet_seed.open`; they never call
  `single_key().sign_with` and never produce `SingleKeyPassphraseRequired`. The new prompt must not
  intercept them — `display_task_error` returns `true` **only** for `SingleKeyPassphraseRequired`.
- **Asset-lock signing / identity register / top-up from a single key (resolving G-2).** Dropped.
  No single-key funding path exists; building one is a separate feature. Diziet's call-site list is
  corrected to exclude these.

### 1.3 Dependency this plan surfaces (must be a user decision — see §7)

Implementing a real single-key send (the prerequisite for the gate to ever fire) is **larger than a
UI prompt**. It requires UTXO selection, transaction building, fee handling, and broadcast for
imported keys — currently all stubbed (`SingleKeyWalletsUnsupported`). The migration design
(`docs/ai-design/2026-05-18-platform-wallet-migration/single-key-mock.md`) deliberately mocked this
out. PROJ-008-as-written ("wire the prompt") cannot be demonstrated end-to-end without first
un-stubbing single-key send. This is the central open question in §7.

This plan is structured so the **prompt machinery (the genuinely reusable, testable PROJ-008
deliverable)** can be built and unit/kittest-verified **against the gate contract** independently of
the send implementation, with the live wiring gated behind the send work.

---

## 2. Architecture — Layer Map

The feature touches three layers; each has a crisp responsibility and API surface.

| Layer | Module(s) | Responsibility | New/changed surface |
|---|---|---|---|
| **Domain / backend signing** | `src/wallet_backend/single_key.rs`, `src/backend_task/core/send_single_key_wallet_payment.rs` | Produce `SingleKeyPassphraseRequired { addr }` from a real sign attempt. Owns the unlock cache. | `sign_with` already exists; the send task must *call* it. |
| **Error envelope (App Task System)** | `src/backend_task/error.rs` | Carry the typed gate error UI-ward. | **No new variant needed** — `SingleKeyPassphraseRequired { addr }` and `SingleKeyPassphraseIncorrect` already exist and are correctly typed (no String payload). |
| **Presentation** | `src/ui/components/sign_unlock_prompt.rs` (new), `src/ui/wallets/single_key_send_screen.rs` | Catch the error, prompt, unlock the cache, re-dispatch the stashed task; drop on cancel. | New `SignUnlockPrompt` component + screen-local stash + `display_task_error` override. |

**Key architectural property: the passphrase never crosses a layer boundary.** It lives only inside
`SignUnlockPrompt`'s `PasswordInput.Secret`, is consumed in-process by
`single_key().unlock_with_passphrase`, and is zeroized before the stashed task is re-dispatched. The
async boundary only ever carries the operation task (no secret) and the typed error (no secret).
This is the load-bearing reason the **gate-on-error** pattern beats mid-flight unlock-request, and
it is preserved by construction here.

---

## 3. Backend Gate

### 3.1 Where the gate fires

`AppContext::send_single_key_wallet_payment` is reworked from a stub into a real send. At the point
it must spend the imported key, it calls (per UTXO / per sighash):

```text
backend.single_key().sign_with(&addr, &sighash)?      // addr = the imported key's P2PKH address
```

On a cold protected key this returns `Err(TaskError::SingleKeyPassphraseRequired { addr })`. The `?`
propagates it unchanged up through `run_backend_task`. **No mapping, no re-wrapping, no String.**

### 3.2 Error variant — reuse, do not invent

`SingleKeyPassphraseRequired { addr: String }` (`error.rs:1380`) already:
- carries only the Base58 `addr` (a CLAUDE.md rule-6 handle, not a secret, not jargon),
- has a `#[error(...)]` Display that is user-appropriate,
- has no `#[source]` (correct — the "you must unlock" condition wraps no upstream error),
- has no user-facing-String-as-field smell.

`SingleKeyPassphraseIncorrect` (fieldless, `error.rs:1392`) is the wrong-passphrase variant returned
by `unlock_with_passphrase`. Also already correct.

**Action: none on `error.rs`.** A new variant would violate DRY and re-litigate a solved design.
Bilby must *not* add a parallel variant. If the removed-key copy decision (§7 Q4 / G-6) lands on a
dedicated message, that is a one-line `#[error]` tweak to the existing `ImportedKeyNotFound`
(`error.rs:206`) Display, not a new variant.

### 3.3 Propagation contract (unchanged App Task System)

```text
single_key().sign_with(addr) -> Err(SingleKeyPassphraseRequired{addr})
  → ? up through send_single_key_wallet_payment
  → run_backend_task returns Err
  → AppState sends TaskResult::Error(err)
  → AppState::update routes to visible_screen.display_task_error(&err)
  → SingleKeyWalletSendScreen returns true (suppress generic banner), opens prompt
```

No change to the channel, the spawn, or `TaskResult`. The gate is pure error propagation.

---

## 4. UI Integration — Gate-on-Error + Auto-Retry

### 4.1 The pending-task stash — where it lives and what it holds

The stash lives **on the triggering screen** (`SingleKeyWalletSendScreen`), not in `AppState` and
not in the component. Rationale (matches Diziet §4.1 and the existing `FeeConfirmationDialog`
precedent): the screen owns the inputs (recipients, amounts, options) that must survive the prompt,
and it is the natural owner of the `AppAction` it just emitted.

New screen fields:

```text
sign_unlock_prompt: Option<SignUnlockPrompt>,   // lazy; None when no gate is active
pending_signed_task: Option<BackendTask>,        // the task to re-dispatch on unlock
```

`BackendTask` is re-dispatchable by value: the screen reconstructs the same
`AppAction::BackendTask(...)` it produced when the gate fired. (The screen already holds the source
inputs, so it can rebuild the task; stashing the constructed `BackendTask` is the simplest and is
what the `FeeConfirmationDialog.pending_request` precedent does one level down.)

### 4.2 Catching the error (`display_task_error`)

`SingleKeyWalletSendScreen::display_task_error(&mut self, error: &TaskError) -> bool`:

```text
match error {
    TaskError::SingleKeyPassphraseRequired { addr } => {
        if self.sign_unlock_prompt is already open for `addr` { return true }   // E-2 dedupe
        stash the task that just failed into self.pending_signed_task
        self.sign_unlock_prompt = Some(SignUnlockPrompt::open_for(addr, alias, hint))
        true   // suppress the generic banner
    }
    _ => false  // everything else: default banner (HD ops, network errors, etc.)
}
```

The default (`false`) is preserved for all other errors, so HD-seed and unrelated failures are
untouched (TC-UNLOCK-005).

**Stashing the task on the error path.** `display_task_error` receives only the error, not the
originating task. Two clean options for Bilby; pick (A) unless it proves awkward:
- **(A) Re-derive on unlock.** The screen keeps the *source inputs* (it already does) and rebuilds
  the `BackendTask` when the prompt succeeds. Nothing extra to stash beyond a "resume requested"
  flag plus the inputs. Cleanest; no task duplication; survives the round-trip trivially (TC-RESUME-003).
- **(B) Remember-last-dispatched.** The screen records the `BackendTask` it dispatched this frame in
  `pending_signed_task` *before* spawning, and `display_task_error` promotes it to "armed". Mirrors
  `FeeConfirmationDialog.pending_request`.

Both keep the secret off the task. (A) is preferred because it cannot accidentally retain a stale
task across input edits.

### 4.3 The prompt lifecycle in `ui()`

Each frame, after the main screen body, the screen calls `self.sign_unlock_prompt`'s `show` (lazy
`Option`, standard component pattern). The component returns a `SignUnlockResult`:

```text
match prompt.show(ctx, backend.single_key(), addr) {
    Pending   => AppAction::None,                       // keep rendering modal
    Unlocked  => {                                       // cache now holds the key
        self.sign_unlock_prompt = None;                  // closed; secret already zeroized
        // FR-3 auto-resume: re-dispatch the stashed/rebuilt task
        let task = self.take_pending_or_rebuild();
        AppAction::BackendTask(task)                      // TC-RESUME-002
    }
    Cancelled => {                                        // Cancel / X / Esc / click-outside
        self.sign_unlock_prompt = None;
        self.pending_signed_task = None;                  // DROP, not hide (TC-CANCEL-001/005, E-5)
        AppAction::None                                   // inputs untouched
    }
}
```

The unlock call itself (`single_key().unlock_with_passphrase(addr, pw)`) happens **inside the
component's `show`** so the passphrase never leaves the component. The component maps the result:
`Ok(())` → `Unlocked`; `Err(SingleKeyPassphraseIncorrect)` → stay open, clear field, set inline
error from the variant's `Display` (TC-WRONG-002, not a hardcoded literal); `Err(ImportedKeyNotFound)`
→ close + surface via normal error path, no loop (E-6 / TC-EDGE-001).

### 4.4 Cancel and lifecycle hooks

- **Cancel / X / Escape / click-outside** → `Cancelled` → drop the stash, zeroize secret
  (TC-CANCEL-001..005, TC-SEC-004).
- **`change_context` (network switch)** → the screen closes the prompt and drops the stash; the
  component's `close()` zeroizes the secret (TC-EDGE-003, E-8). The screen's existing
  `change_context` override gains: `self.sign_unlock_prompt = None; self.pending_signed_task = None;`.
- **Re-dispatch must fire once.** Clear the stash *as* you build the resume action so it cannot
  re-fire next frame (TC-CANCEL-005, TC-RESUME-002).

### 4.5 Multi-key sequencing (FR-7 / E-1)

No batching UI. The error→prompt→retry loop drains keys naturally: the re-dispatched send hits the
*next* uncached key, the backend returns `SingleKeyPassphraseRequired { addr: K2 }`,
`display_task_error` opens the prompt for K2, repeat (TC-MULTI-001/002). The defensive
"different-addr arrives while open" queue (TC-CONCUR-003) is, as Marvin flags in G-5, hard to reach
under this loop because focus is trapped; specify it as a **defensive no-op** (ignore the duplicate;
the loop will re-surface it after the current key resolves) rather than building a real queue.

---

## 5. New Component — `SignUnlockPrompt`

`src/ui/components/sign_unlock_prompt.rs`. A near-clone of `WalletUnlockPopup`, differing only in
what it unlocks and what it reports back.

### 5.1 State

```text
pub struct SignUnlockPrompt {
    is_open: bool,
    addr: String,                 // the imported key's P2PKH address (the gate key)
    key_name: Option<String>,     // ImportedKey.alias, if any (FR-2)
    hint: Option<String>,         // passphrase_hint, if any (FR-2)
    password_input: PasswordInput,
    error_message: Option<String>,// derived from SingleKeyPassphraseIncorrect.Display (FR-4)
    focus_requested: bool,
}
```

Private fields only (component pattern). No public mutable state.

### 5.2 API

```text
impl SignUnlockPrompt {
    pub fn open_for(addr: impl Into<String>, key_name: Option<String>, hint: Option<String>) -> Self;
    pub fn is_open(&self) -> bool;
    pub fn addr(&self) -> &str;                 // E-2 same-address dedupe / TC-MULTI-002
    pub fn close(&mut self);                     // zeroizes secret (calls password_input.clear())
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        single_key: SingleKeyView<'_>,           // borrow to call unlock_with_passphrase in-process
        // addr/key_name/hint are already on self
    ) -> SignUnlockResult;
}

pub enum SignUnlockResult { Pending, Unlocked, Cancelled }
```

`show` performs the unlock attempt internally (Enter or Unlock click, only when field non-empty —
E-7/TC-PROMPT-006), so the `Secret` never leaves the component (NFR-1 / TC-SEC-002/003).

### 5.3 Copy (i18n-ready, from Diziet §4.4; named placeholders)

| Slot | String |
|---|---|
| Title | `Unlock imported key` |
| Body (aliased) | `The key "{ $key_name }" is locked. Enter the passphrase you set for it to continue.` |
| Body (no alias) | `The key { $address } is locked. Enter the passphrase you set for it to continue.` |
| Hint (only if present) | `Hint: { $hint }` |
| Session note | `This key stays unlocked until you close the app.` |
| Field placeholder | `Passphrase` |
| Reveal tooltip | `Hold to reveal` (already on `PasswordInput`) |
| Primary | `Unlock` · Secondary | `Cancel` |
| Wrong passphrase | derive from `SingleKeyPassphraseIncorrect.Display` — do **not** hardcode a parallel literal |

### 5.4 Shared modal body — recommended, optional

`WalletUnlockPopup` and `SignUnlockPrompt` share ~90% scaffolding (overlay, `Window`, focus-once,
Enter/Escape/X/click-outside, right-to-left Unlock/Cancel). Extracting a private
`modal_password_body(ui, &mut PasswordInput, &mut focus_requested, error) -> ModalChrome` helper (or
a thin `PassphraseModal` struct) into `components/` would remove the duplication. **Recommended but
not required for v1** — the two call sites differ enough (one opens an HD seed and calls
`handle_wallet_unlocked`; the other unlocks a single key by address and reports `Unlocked`) that a
straight clone is acceptable if the extraction proves fiddly. If cloned, leave a `// SHARED-CHROME:
mirror of wallet_unlock_popup.rs` marker so the duplication is intentional and findable.

### 5.5 Catalog

After building, add a row to `src/ui/components/README.md` Dialog Components table:
`| SignUnlockPrompt | sign_unlock_prompt.rs | SignUnlockResult | Per-key passphrase unlock for imported single keys |`.

---

## 6. Decisions

| # | Decision | Resolution | Trace |
|---|---|---|---|
| D1 | Integration pattern | **Gate-on-error + auto-retry.** Confirmed by ground truth: secret stays off the async channel by construction; cache makes retry free. | Diziet §6 |
| D2 | New error variant? | **No.** Reuse `SingleKeyPassphraseRequired { addr }` + `SingleKeyPassphraseIncorrect`; both already typed and String-free. | §3.2 |
| D3 | Stash location | **On the triggering screen** (`SingleKeyWalletSendScreen`), not `AppState`, not the component. | §4.1 |
| D4 | Retry limit | **None / no cooldown** (Diziet NFR-5). Local AES-GCM secret, no remote attacker, no account to lock. **Flag for Smythe** (G-3): if Security wants a soft cap, TC-WRONG-004 and the component gain a counter. | §7 Q1 |
| D5 | Multi-key | **Sequential via the retry loop**, one prompt at a time. Defensive different-addr "queue" is a no-op ignore, not a real queue (G-5). | §4.5 |
| D6 | Secret confinement | Passphrase lives only in `PasswordInput.Secret`; consumed in-process; zeroized on every close path and every wrong attempt. Never in `TaskError`, `AppAction`, `BackendTask`, logs, or banner details. | NFR-1 / TC-SEC-001..004 |
| D7 | Asset-lock / identity register | **Out of scope** (G-2). No single-key funding path exists. | §0.3, §1.2 |
| D8 | Single-key send implementation | **Prerequisite, larger than the prompt.** Prompt machinery built + tested against the gate contract; live wiring gated behind un-stubbing send. **User decision needed.** | §1.3, §7 Q3 |

**Security review (Smythe) flags:** D4 (retry policy), D6 confinement tests (TC-SEC-001..004), and
the eventual single-key send signing path (when D8 is taken) all warrant Smythe sign-off.

---

## 7. Open Questions (need a user / stakeholder decision)

1. **Retry limit (G-3 / NFR-5).** Confirm *no* cap and *no* cooldown. Design and this plan
   recommend none; Security (Smythe) may want a soft cap. This is the one decision that changes the
   component's state and TC-WRONG-004.
2. **Session-note wording (G-4 / §6.2).** Commit to "This key stays unlocked until you close the
   app." now, or hedge to "for a while" in anticipation of a future idle auto-lock? Recommend the
   honest literal until an auto-lock is actually designed.
3. **Single-key send: in or out of THIS deliverable? (the pivotal one, §1.3 / D8.)** The gate cannot
   fire until `send_single_key_wallet_payment` is un-stubbed to sign via `sign_with`. Options:
   - **(3a) Prompt-only now.** Build + unit/kittest the `SignUnlockPrompt` and the
     `display_task_error`/stash/auto-retry state machine against the *gate contract* (drive the
     error directly, as Marvin's offline cases do). Defer live send wiring. PROJ-008's reusable
     deliverable lands; live demo waits on the send feature.
   - **(3b) Full vertical slice.** Implement single-key send end-to-end too (UTXO selection, tx
     build, fee, broadcast) so the gate fires for real on testnet. Substantially larger; overlaps
     the migration design's deliberately-mocked single-key surface.
   Recommend **(3a)** for this phase, with (3b) tracked as a dependent feature. **User must choose.**
4. **Removed-key message (G-6).** `ImportedKeyNotFound` has no bespoke copy for the "removed between
   trigger and unlock" case. Reuse its existing `Display`, or author a dedicated message? Recommend
   reuse unless it reads poorly to the Everyday User.

---

## 8. Task Breakdown for Bilby (Phase 2)

Tasks are ordered by dependency. Each names the TC IDs it satisfies and flags Smythe review. Tasks
T1–T4 are the **prompt machinery** and are independent of the send implementation (they test against
the gate contract). T5 is the **live wiring**, gated behind Open Question Q3.

### T1 — `SignUnlockPrompt` component (new)
- **Files:** `src/ui/components/sign_unlock_prompt.rs` (new); register in `src/ui/components/mod.rs`;
  add catalog row to `src/ui/components/README.md`.
- **Scope:** struct + `open_for`/`is_open`/`addr`/`close`/`show` + `SignUnlockResult`; clone
  `WalletUnlockPopup` chrome (overlay, Window, focus-once, Enter/Esc/X/click-outside, RTL buttons);
  reuse `PasswordInput`; copy from §5.3; Unlock disabled while empty; inline error from
  `SingleKeyPassphraseIncorrect.Display`; zeroize on every close path; `unlock_with_passphrase`
  called in-process.
- **Satisfies:** TC-PROMPT-001..006, TC-WRONG-001..003, TC-A11Y-001..005, TC-SEC-004, TC-EDGE-002.
- **Smythe:** yes (secret confinement, zeroize-on-close).
- **Size:** ~250–350 lines + kittest.

### T2 — `SingleKeyWalletSendScreen` gate integration (stash + `display_task_error` + lifecycle)
- **Files:** `src/ui/wallets/single_key_send_screen.rs`.
- **Scope:** add `sign_unlock_prompt: Option<SignUnlockPrompt>` and the pending-task stash (§4.1,
  option A preferred); override `display_task_error` to open the prompt only for
  `SingleKeyPassphraseRequired` (return `true`) and pass through everything else (return `false`);
  drive the prompt in `ui()` (Pending/Unlocked/Cancelled → §4.3); drop stash on cancel; clear prompt
  + stash in `change_context`; resolve `alias`/`hint` for the `addr` from `single_key().list()` /
  `ImportedKey`.
- **Depends on:** T1.
- **Satisfies:** TC-UNLOCK-005, TC-RESUME-002/003, TC-WRONG-003, TC-CANCEL-001..005, TC-CONCUR-002/003,
  TC-EDGE-003/004, TC-MULTI-002.
- **Smythe:** yes (re-dispatch carries no secret — TC-SEC-002).
- **Size:** ~150–250 lines.

### T3 — Offline state-machine + security unit tests
- **Files:** inline `#[cfg(test)]` in the component / screen; kittest in `tests/kittest/`
  (`sign_unlock_prompt.rs`, registered in `tests/kittest/main.rs`); follow the
  `tests/kittest/import_single_key.rs` house style (`force_input_for_test`, `query_by_label*`).
- **Scope:** drive the gate contract directly (no live send): cache MISS/HIT/unprotected/import-primes
  (TC-UNLOCK-001..004 — mirror `single_key.rs:1140-1186`); correct→cache populated (TC-RESUME-001);
  wrong→typed incorrect, cache untouched (TC-WRONG-001); re-dispatch fires once, drops on cancel
  (TC-RESUME-002, TC-CANCEL-005); secret confinement with sentinel passphrase across Display/Debug,
  AppAction, logs (TC-SEC-001/002/003).
- **Depends on:** T1, T2.
- **Satisfies:** the offline two-thirds of Marvin's suite.
- **Smythe:** yes (TC-SEC-001..003 are the security assertions).
- **Size:** batched, ~300+ lines across files.

### T4 — Shared modal-chrome extraction (optional, recommended)
- **Files:** `src/ui/components/` (new private helper or `PassphraseModal`); refactor
  `wallet_unlock_popup.rs` and `sign_unlock_prompt.rs` onto it.
- **Scope:** only if the duplication from T1 is worth removing; otherwise close with the
  `// SHARED-CHROME` marker decision. No behavioural change.
- **Depends on:** T1.
- **Satisfies:** maintainability (no TC; regression-covered by T3 + existing wallet-unlock tests).
- **Smythe:** no.
- **Size:** ~100–150 lines net (mostly moves).

### T5 — [GATED on Q3] Single-key send signs via `sign_with` (live wiring)
- **Files:** `src/backend_task/core/send_single_key_wallet_payment.rs`, and the single-key send
  rejections at `src/backend_task/core/mod.rs:218,304`.
- **Scope:** un-stub the send; build/sign the funding input(s) via `single_key().sign_with(addr,
  sighash)`; let `SingleKeyPassphraseRequired` propagate. **This is the larger send feature** (UTXO
  selection / tx build / fee / broadcast) — likely itself decomposed; out of scope unless the user
  takes (3b).
- **Depends on:** Q3 decision; consumes T1–T3 once live.
- **Satisfies:** TC-SITE-001 (currently [NOT-YET-WIRED]); TC-RESUME-004 [LIVE/MANUAL];
  TC-MULTI-001 [LIVE/MANUAL].
- **Smythe:** yes (new signing path).
- **Size:** large; specify separately when Q3 is resolved.

**Task count for Bilby in this phase: 4 buildable now (T1–T4), plus T5 gated on a user decision.**

---

## 9. Traceability (plan → requirements → tests)

| Requirement | Plan section | Tests |
|---|---|---|
| FR-1 gate appears iff `SingleKeyPassphraseRequired` | §3, §4.2 | TC-UNLOCK-001..005 |
| FR-2 self-explanatory content | §5.3 | TC-PROMPT-001..005 |
| FR-3 auto-resume | §4.3 (T2) | TC-RESUME-001..004 |
| FR-4 wrong passphrase recoverable | §4.3, §5 | TC-WRONG-001..004 |
| FR-5 cancel drops task | §4.4 | TC-CANCEL-001..005 |
| FR-6 session unlock | §0.4 cache, §5.3 note | TC-UNLOCK-002, TC-PROMPT-004 |
| FR-7 multi-key sequential | §4.5 | TC-MULTI-001..002 |
| NFR-1 secret confinement | §2, §4.3, §5.2, D6 | TC-SEC-001..004 |
| NFR-2 a11y | §5 (clone of `WalletUnlockPopup`) | TC-A11Y-001..005 |
| NFR-5 retry policy | D4, Q1 | TC-WRONG-004 |
| NFR-6 operation-agnostic | §1.1 (keys off `addr`) | TC-SITE-001 |

---

## Candy Tally 🍬 (architecture findings surfaced)

- **High (2):** G-1 confirmed and sharpened — the gate is unwired *and* its only in-scope operation
  (`send_single_key_wallet_payment`) is a `SingleKeyWalletsUnsupported` stub, so the feature must
  **introduce** the `sign_with` call site, not hook an existing one; G-2 confirmed — asset-lock /
  identity-register signing is HD-seed-only (`signer_for` → `WalletAssetLockSigner`) and is **dropped
  from scope**.
- **Medium (2):** the single-key send prerequisite (D8/Q3) is materially larger than the prompt and
  must be a user decision before any live demo; the pending task must be **dropped, not hidden**, on
  cancel/network-switch or it re-fires (encoded in T2 + TC-CANCEL-005).
- **Low (2):** no new `TaskError` variant is needed (reuse the two correct existing ones) — a finding
  that *prevents* a likely duplicate-variant mistake; shared modal chrome (T4) is worth extracting
  but optional, marked so the clone is intentional.
- **Open questions (4):** retry limit (Smythe), session-note wording, single-key-send scope (the
  pivotal one), removed-key copy.

**Total: 6 findings (0 critical, 2 high, 2 medium, 2 low) + 4 open questions.**
