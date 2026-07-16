# Just-In-Time Secret Access — `SecretAccess` Chokepoint (formal design)

**Feature:** Operation-scoped JIT secret access · DET-only (upstream `platform-wallet` untouched)
**Phase:** Architecture (design only — no implementation)
**Author:** Nagatha (Software Architect)
**Date:** 2026-06-02
**Base:** worktree fast-forwarded to `2272bae0`

> **SUPERSESSION.** This document **supersedes**
> `docs/ai-design/2026-06-02-signtime-unlock-ux/dev-plan.md` in its entirety. That plan
> designed a *gate-on-error, sign-time-unlock* prompt scoped to a single call site
> (single-key send), with HD and asset-lock signing explicitly **out of scope**. The user has
> since settled a broader cut: **all** secret consumers — HD signing, single-key signing,
> shielded `bind_shielded(seed)`, and DashPay contact-xpub derivation — route through one
> just-in-time chokepoint, and the eager session-residency model is retired for both wallet
> types at once. The dev-plan's narrow framing no longer holds.
>
> **What still applies from the superseded set:**
> - `requirements-and-ux.md` — FR-2 (self-explanatory prompt content), FR-4 (wrong-passphrase
>   recoverable), FR-5 (cancel aborts), FR-6 (session unlock — now the opt-in toggle), NFR-1
>   (secret confinement), NFR-2 (a11y) carry forward unchanged. FR-3 (auto-resume) is **replaced**
>   by in-flight await (§4): the operation never fails-and-retries; it suspends on the prompt.
>   FR-7 (multi-key) carries forward but is reframed as one-prompt-per-operation (§7).
> - `test-cases.md` — the **offline** TCs (cache MISS/HIT/unprotected/import-primes, wrong→typed,
>   secret-confinement sentinel across Display/Debug/AppAction/logs, prompt a11y, cancel-zeroizes)
>   carry forward, re-pointed at the new seam. The **gate-on-error-specific** TCs (TC-SITE-001
>   "keys off addr from error", TC-RESUME-002 "re-dispatch fires once", TC-CANCEL-005 "stash
>   dropped not hidden") are **obsoleted** — there is no error gate, no stash, no re-dispatch.
>   New TCs are required for the HD path, the await/cancel mechanics, and the session toggle (§9).

---

## 0. Ground truth (re-verified at `2272bae0`)

Every claim below was re-grepped against the synced base. The secret surface is wider than the
gate-on-error plan assumed, because that plan deliberately excised HD and shielded/DashPay.

### 0.1 Three eager secret residencies exist today — all must be retired or made transient

| # | Residency | Lifetime today | Populated at | Read by |
|---|---|---|---|---|
| R1 | `Inner.seeds: RwLock<BTreeMap<WalletSeedHash, Zeroizing<[u8;64]>>>` (`mod.rs:277`) | whole session | `provide_seed` (`mod.rs:531`) from the unlock chokepoint | `signer_for` (`mod.rs:1079`), `derive_private_key` (`mod.rs:1114`) |
| R2 | `Inner.single_key_unlocked: RwLock<BTreeMap<String,[u8;32]>>` (`mod.rs:201`) | whole session | `SingleKeyView::unlock_with_passphrase` (`single_key.rs:256`) | `raw_key_bytes` (`single_key.rs:293`) → `sign_with` (`single_key.rs:649`) |
| R3 | `Wallet::Open(OpenWalletSeed{ seed: [u8;64] })` inside `ctx.wallets` (`model/wallet/mod.rs:598`) | whole session (until lock) | `WalletSeed::open` / `open_no_password` (`model/wallet/mod.rs:633,653`) at the UI unlock seam | `seed_bytes()` (`mod.rs:751`) → `wallet_seed_snapshot` (`wallet_lifecycle.rs:421`), `first_open_wallet_seed` (`contact_requests.rs:521`), `initialize_shielded_wallet` (`shielded.rs:389`) |

R1 and R2 are named explicitly in the brief. **R3 is the residency the gate-on-error plan never
touched** and is the one that actually feeds DashPay and shielded today. A JIT design that retires
R1/R2 but leaves R3 holding a whole-session plaintext seed would be theatre. R3 is addressed in §6.

### 0.2 The unlock chokepoint already exists — the refactor inverts it

`AppContext::handle_wallet_unlocked` (`wallet_lifecycle.rs:302`) is the single place a decrypted HD
seed is handed to the backend: it snapshots the seed (`wallet_seed_snapshot`, `:421`) and pushes it
into R1 via `provide_seed` (`mod.rs:531`), then eagerly initializes shielded state (`:316`). It is
called from exactly three UI seams: `wallet_unlock.rs:33,100` and `wallet_unlock_popup.rs:180,252`,
plus the cold-boot bootstrap `bootstrap_loaded_wallets` (`wallet_lifecycle.rs:467`).

**The JIT refactor is an inversion of control on this chokepoint.** Today: *unlock pushes the seed
into a cache (eager)*. After: *an operation pulls the seed through `with_secret`, prompting only if
needed (lazy)*. `handle_wallet_unlocked` stops being a secret-distribution point; the unlock
**popup's role shrinks to verifying the passphrase and seeding the optional session cache** (§5).

### 0.3 The upstream signing seam is a per-operation async trait — confirmed, password-free

Upstream operations are generic over a signer with `async fn sign(&self, key: &K, data: &[u8]) ->
Result<BinaryData, ProtocolError>` (e.g. `…/rs-platform-wallet/src/wallet/identity/network/{dpns,
transfer,withdrawal,contract,profile,update,contact_requests}.rs:38-44` at pin `ddfa66e`). Upstream
has **no password concept** — confirmed. DET injects `WalletAssetLockSigner` (`asset_lock_signer.rs:52`)
which implements `key_wallet::signer::Signer` and **already owns a one-operation seed snapshot,
zeroized on drop** (`asset_lock_signer.rs:11-15,56`). This is not an obstacle to JIT — **it is the
template for it.** Its construction is the exact point where `with_secret` belongs.

### 0.4 Single-key send is still a stub (carried unchanged from the superseded plan)

`send_single_key_wallet_payment` returns `SingleKeyWalletsUnsupported`; single-key sends are
rejected at `core/mod.rs`. The superseded plan's Open Question Q3 (single-key send is larger than a
prompt) **still stands** and is re-surfaced in §10. The JIT *machinery* for single-key signing is
designed here and is testable against `sign_with` directly; the live send wiring remains gated.

### 0.5 Reuse inventory (the brief's mandate — confirmed present)

- `PasswordInput` (`components/password_input.rs:41`) — `Secret` backing (zeroized on drop,
  `model/secret.rs`), hold-to-reveal, `text()`/`take_secret()`/`secret()`/`clear()`/`set_error()`
  (`:100,130,125,110,120`). This is the passphrase field; do not build another.
- `WalletUnlockPopup` (`components/wallet_unlock_popup.rs:22`) — overlay, centered `Window`,
  focus-once, Enter/Escape/X/click-outside, `open()`/`close()` zeroize via `password_input.clear()`.
- `ScreenWithWalletUnlock` (`components/wallet_unlock.rs:9`) — the per-screen unlock-render trait.
- `SecretString` / `SecretBytes` (`platform_wallet_storage::secrets::secret`, pin `ddfa66e:40,165`):
  `SecretString::new(impl Into<String>)`, `.expose_secret() -> &str`; `SecretBytes::from_slice`,
  `.expose_secret() -> &[u8]`. Both zeroize. **The passphrase crosses the UI↔async boundary as
  `SecretString`.** DET already depends on the crate; do **not** roll a new secret type.

---

## 1. Goals, non-goals, and the invariant that drives the whole design

### 1.1 Goals

1. **Operation-scoped residency by default.** A decrypted secret exists only for the duration of one
   user action and is zeroized at action end.
2. **Opt-in session cache.** A per-prompt checkbox ("remember until I close the app", default OFF)
   promotes that secret to a session cache for the rest of the process.
3. **One chokepoint for all consumers.** HD signing, single-key signing, shielded `bind_shielded`,
   and DashPay derivation all obtain plaintext through `SecretAccess::with_secret`.
4. **The passphrase travels as `SecretString` across the UI↔async seam; the backend decrypts JIT.**
5. **egui never enters `wallet_backend`.** `SecretPrompt` is the only seam the UI implements.

### 1.2 Non-goals

- No change to upstream `platform-wallet` (it has no password concept; DET adapts on its side).
- No change to the at-rest vault model (`open_secret_store`, the SEC-003 empty-passphrase single-key
  vault is **out of scope** — do not "fix" it here).
- No un-stubbing of single-key send (still §10/Q-SEND, as in the superseded plan).
- No idle auto-lock timer (the toggle is "until app close" / manual lock only; an idle timer is a
  future, explicitly deferred — §5.4).

### 1.3 The load-bearing invariant

**A plaintext secret crosses no layer boundary and is never copied beyond the closure that consumes
it.** The async↔UI channel carries only (a) a `SecretPrompt` *request* (no secret), and (b) the
user's reply as a `SecretString` *passphrase* (a key-derivation input, not a derived key). The
derived 64-byte seed / 32-byte key exists only inside `with_secret`'s `Zeroizing` buffer, for the
length of one closure call. This is the same confinement property the superseded plan achieved for
single-key, generalised to every consumer.

---

## 2. Layer map

| Layer | Module(s) | Responsibility | New / changed surface |
|---|---|---|---|
| **Presentation (egui)** | `src/ui/components/secret_prompt_host.rs` (new), `src/app.rs`, reused `password_input.rs` / `wallet_unlock_popup.rs` | Drain prompt requests each frame; render the reused modal; return the typed reply. Implements `SecretPrompt`. | `EguiSecretPromptHost` impl + `AppState` drain loop integration. |
| **Boundary (the seam)** | `src/wallet_backend/secret_prompt.rs` (new) | UI-agnostic request/reply contract. The *only* thing the UI sees of the secret machinery. | `SecretPrompt` trait + `SecretPromptRequest` + `SecretScope` (DET-opaque). |
| **Domain / orchestration** | `src/wallet_backend/secret_access.rs` (new), `src/wallet_backend/mod.rs`, `single_key.rs`, `asset_lock_signer.rs` (→ `det_signer.rs`), `dashpay.rs`, shielded paths | `with_secret` chokepoint: cache lookup → prompt → decrypt JIT → run closure → zeroize. Owns the operation + session caches. | `SecretAccess` + the new `DetSigner`; retire R1/R2 caches and `provide_seed`. |
| **Persistence (vault)** | `platform_wallet_storage::secrets::SecretStore` (consumed) | Unchanged. Decryption-on-demand source of truth. | none. |

The seam is `secret_prompt.rs`. `wallet_backend` depends on it; egui depends on it; they never depend
on each other. (M-DONT-LEAK-TYPES, §8.)

---

## 3. `SecretPrompt` — the UI↔async boundary

### 3.1 Scope identifier (DET-opaque, no leaked types)

```rust
// src/wallet_backend/secret_prompt.rs

/// Which secret an operation needs. DET-opaque: carries no upstream type, no
/// plaintext, only opaque-but-copyable handles (CLAUDE.md rule 6) for prompt
/// copy. `Eq + Hash` so it keys the caches in `SecretAccess`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SecretScope {
    /// HD wallet seed, identified by DET's SHA256(seed) hash.
    HdSeed { seed_hash: WalletSeedHash },
    /// Imported single key, identified by its P2PKH address.
    SingleKey { address: String },
}
```

`WalletSeedHash` is a DET type (`model/wallet`), not an upstream one, so it may sit on this seam.
`SecretScope` is the cache key **and** the prompt-routing key.

### 3.2 The request and reply

```rust
/// A request for the user to supply a passphrase. Enqueued by the backend,
/// drained by `AppState` each frame. Carries NO secret.
pub struct SecretPromptRequest {
    pub scope: SecretScope,
    /// Human label for the modal body (alias / address / wallet name).
    pub display_label: String,
    /// Optional user-set hint (HD password hint or single-key hint).
    pub hint: Option<String>,
    /// Set on a re-ask after a wrong passphrase, so the host shows the inline
    /// error. None on first ask.
    pub retry_reason: Option<SecretPromptRetry>,
    /// One-shot reply channel. Dropping the sender == cancel (§7.3).
    pub reply: tokio::sync::oneshot::Sender<SecretPromptReply>,
}

pub enum SecretPromptRetry { WrongPassphrase }

/// The user's answer. The passphrase is a `SecretString` (zeroizing) — a
/// key-derivation INPUT, never a derived key.
pub struct SecretPromptReply {
    pub passphrase: platform_wallet_storage::secrets::SecretString,
    /// Session-toggle state at submit time (default false). Promotes the
    /// decrypted secret to the session cache (§5).
    pub remember_for_session: bool,
}
```

> **Why `SecretString` and not a derived-key type.** The reply carries the *passphrase*, decrypted
> nowhere yet. The backend decrypts JIT inside `with_secret`. The channel therefore never carries
> seed/key material — only the human input that unlocks it. This is strictly stronger than handing a
> decrypted seed across the channel and is the reason cancel can be a clean drop.

### 3.3 The trait

```rust
#[async_trait::async_trait]
pub trait SecretPrompt: Send + Sync {
    /// Ask the user for the passphrase for `request.scope`. Resolves when the
    /// user submits (Ok) or cancels (Err). Implementations MUST NOT block; the
    /// egui impl enqueues and awaits the oneshot.
    async fn request(&self, request: SecretPromptRequest) -> Result<SecretPromptReply, SecretPromptCancelled>;
}

/// The user dismissed the prompt, or the host was torn down. Carries no data.
#[derive(Debug, thiserror::Error)]
#[error("the passphrase prompt was cancelled")]
pub struct SecretPromptCancelled;
```

The trait owns the `oneshot` mechanics: `request()` builds the channel, enqueues
`SecretPromptRequest`, and `.await`s the receiver. A `RecvError` (sender dropped) maps to
`SecretPromptCancelled`. This keeps `tokio::sync::oneshot` out of every call site.

### 3.4 egui implementation — `EguiSecretPromptHost` + the AppState drain

`src/ui/components/secret_prompt_host.rs`:

- An `EguiSecretPromptHost { queue: Arc<Mutex<VecDeque<SecretPromptRequest>>>, egui_ctx }`
  implements `SecretPrompt::request` by pushing onto `queue`, calling `egui_ctx.request_repaint()`
  (so the modal appears immediately even if the UI is idle), and awaiting the oneshot.
- `AppState::update` (`app.rs:1210`) gains a drain step beside the existing
  `task_result_receiver.try_recv()` loop (`app.rs:1300`): pop at most one request, store it as the
  *active* prompt, and render the reused modal (§5). Exactly one prompt is active at a time;
  additional requests stay queued (§7.2). The drain runs unconditionally each frame, so a request
  enqueued from a backend task on the tokio runtime surfaces on the next paint.

This mirrors the existing pattern precisely: backend tasks already talk to `AppState` through a
channel drained in `update()`. `SecretPrompt` adds a second, symmetric channel that flows the other
way (UI → backend) for the reply. No new runtime, no egui types in `wallet_backend`.

---

## 4. `SecretAccess` — the chokepoint

### 4.1 Shape

```rust
// src/wallet_backend/secret_access.rs

/// The just-in-time secret chokepoint. Held by `WalletBackend::Inner`.
/// Clone is O(1) (Arc inner) — M-SERVICES-CLONE.
#[derive(Clone)]
pub struct SecretAccess { inner: Arc<SecretAccessInner> }

struct SecretAccessInner {
    /// The encrypted vault — decrypt-on-demand source of truth.
    secret_store: Arc<SecretStore>,
    /// Single-key index (alias/hint/has_passphrase) for prompt copy.
    single_key_index: Arc<RwLock<BTreeMap<String, ImportedKey>>>,
    /// HD wallet meta (alias / password hint) for prompt copy.
    wallet_meta: WalletMetaView,
    /// The UI seam. `dyn` because the host is chosen at construction
    /// (egui host in app, a test double in tests — M-MOCKABLE-SYSCALLS).
    prompt: Arc<dyn SecretPrompt>,
    /// OPT-IN session cache. Empty by default. A scope lands here only when the
    /// user ticked "remember for session". Zeroized values; cleared on app
    /// close, network switch, and manual lock (§5.4).
    session: RwLock<HashMap<SecretScope, SessionSecret>>,
    network: Network,
}

/// A session-promoted plaintext secret. Variant matches the scope kind.
enum SessionSecret {
    HdSeed(Zeroizing<[u8; 64]>),
    SingleKey(Zeroizing<[u8; 32]>),
}
```

### 4.2 The core method

```rust
impl SecretAccess {
    /// Run `f` with the plaintext secret for `scope`, obtaining it
    /// just-in-time. Resolution order:
    ///   1. operation cache (built fresh per call — see §4.3);
    ///   2. session cache, IFF the user previously opted in;
    ///   3. else prompt via `SecretPrompt`, receive a `SecretString`,
    ///      decrypt JIT, run `f`, then zeroize.
    /// `f` receives the plaintext by reference inside a `Zeroizing` guard; it
    /// MUST NOT clone it out. Returns `f`'s value, or `SecretAccessError`
    /// (cancelled / decrypt failed / vault error).
    pub async fn with_secret<R>(
        &self,
        scope: &SecretScope,
        f: impl FnOnce(SecretPlaintext<'_>) -> Result<R, TaskError>,
    ) -> Result<R, TaskError>;

    /// Batch form: keeps ONE decrypted secret alive across several closure
    /// calls for the SAME scope within one operation (one prompt, N signs).
    /// This is the "operation cache" made explicit and bounded (§7.1).
    pub async fn with_secret_session<R>(
        &self,
        scope: &SecretScope,
        f: impl AsyncFnOnce(&SecretSession<'_>) -> Result<R, TaskError>,
    ) -> Result<R, TaskError>;
}

/// Borrowed plaintext, kind-tagged. No `Clone`, no `Deref` to the raw bytes
/// without an explicit `expose_*`. Lives only inside the closure.
pub enum SecretPlaintext<'a> {
    HdSeed(&'a Zeroizing<[u8; 64]>),
    SingleKey(&'a Zeroizing<[u8; 32]>),
}

/// Within-operation handle: `sign(...)`-style helpers borrow the held
/// plaintext without re-prompting (§7.1). Dropped (zeroized) at op end.
pub struct SecretSession<'a> { /* &held plaintext */ }
```

> **On `AsyncFnOnce`.** The batch form must hold the plaintext across `await`s (a payment signs
> several inputs, each an upstream async call). Rust 1.92 (project MSRV, `CLAUDE.md`) supports
> `async` closures; `with_secret_session` takes an `AsyncFnOnce`. If the closure ergonomics prove
> awkward in review, the fallback is an explicit RAII guard returned to the caller
> (`let guard = sa.acquire(scope).await?; guard.sign(...).await?;`) that zeroizes on drop. Either
> shape preserves "one prompt per operation, secret zeroized at op end." **Decision deferred to
> Bilby's first task spike; flag for Nagatha re-review if the guard form is chosen** (it changes the
> consumer signatures in §6).

### 4.3 What an "operation" is, and the cache lifecycle

- **Operation = one `with_secret`/`with_secret_session` call.** The "operation cache" is not a
  process-global map; it is the **plaintext bound to a single chokepoint call's stack/closure**. It
  is created when `with_secret` decrypts (or reads the session cache), lives for the closure, and is
  zeroized when the closure returns — `Zeroizing` drop, no manual clearing required, no entry in any
  long-lived map. This is the default and is why the default residency is operation-scoped.
- **Session cache** is the only long-lived plaintext store, and it is **empty unless the user opts
  in**. On a `remember_for_session = true` reply, `with_secret` inserts the freshly decrypted
  plaintext into `session` before running the closure; subsequent `with_secret` calls for the same
  scope hit step 2 and never prompt.
- **No "operation cache map" survives a call.** This is the central difference from R1/R2, which were
  process-lifetime maps. There is exactly one optional long-lived map (`session`), gated behind an
  explicit, defaulted-off user choice.

### 4.4 Resolution algorithm (single secret)

```
with_secret(scope, f):
  1. if let Some(s) = session.read().get(scope):        // opt-in cache hit
         return f(s.borrow())                            // no prompt, no vault read
  2. loop:                                               // re-ask on wrong passphrase
       req = build_request(scope, label, hint, retry)    // copy from index/meta
       reply = prompt.request(req).await
                 .map_err(|_cancel| TaskError::SecretPromptCancelled)?   // §7.3
       plaintext = decrypt_jit(scope, reply.passphrase)  // vault read + AES-GCM
                     match { Ok(p) => p,
                             Err(WrongPassphrase) => { retry = WrongPassphrase; continue }
                             Err(other) => return Err(other) }
       if reply.remember_for_session:
           session.write().insert(scope, SessionSecret::from(&plaintext))   // promote
       let out = f(plaintext.borrow());                  // run with Zeroizing borrow
       // plaintext (the operation copy) zeroizes here on scope exit
       return out
```

`decrypt_jit` is the only place the vault is touched for plaintext:
- **HD:** `SecretStore.get(WalletId(seed_hash), "seed.v1")` → decode `StoredSeedEnvelope` →
  AES-GCM-decrypt with the passphrase → `Zeroizing<[u8;64]>`. (Mirrors `wallet_seed_store.rs`; the
  decrypt currently happens in `WalletSeed::open` at the model layer — that path is rerouted, §6.1.)
- **Single key:** `SecretStore.get(single_key_namespace, label_for_address(addr))` →
  `SingleKeyEntry::decode` → `entry.decrypt(Some(passphrase))` → `Zeroizing<[u8;32]>`. This is
  exactly today's `unlock_with_passphrase` body (`single_key.rs:256-276`) minus the cache insert.

A wrong passphrase is detected as the existing typed condition (single-key
`SingleKeyPassphraseIncorrect`; HD's `WalletSeed::open` error reshaped to a typed variant, §6.1) and
loops to re-ask **without** closing the modal (FR-4 preserved).

### 4.5 Error type

```rust
// extends TaskError (backend_task/error.rs) — typed, no String payloads
TaskError::SecretPromptCancelled            // user dismissed; aborts the op cleanly
TaskError::SecretDecryptFailed { #[source] ... }   // vault/AES error (not wrong-pass)
// wrong-passphrase stays the existing SingleKeyPassphraseIncorrect / new HdPassphraseIncorrect
// (handled inside the loop, not surfaced unless re-ask is itself cancelled)
```

`SecretPromptCancelled` Display is Everyday-User copy ("You cancelled. Nothing was changed. Try the
action again when you're ready.") and carries no secret, no jargon. The passphrase appears in **no**
`TaskError` variant (§8 / NFR-1).

---

## 5. The reused prompt + the session toggle

### 5.1 The prompt is a thin evolution of `WalletUnlockPopup`, not a new component

`EguiSecretPromptHost` renders a modal that **reuses `PasswordInput` verbatim** and **clones the
`WalletUnlockPopup` chrome** (overlay `DashColors::modal_overlay()`, centered `Window`, focus-once,
Enter/Escape/X/click-outside). The only additions over `WalletUnlockPopup` are:

1. It is driven by a `SecretPromptRequest` (scope + label + hint + retry) rather than a
   `&Wallet`, and it reports back through the `oneshot`, not by mutating wallet state.
2. It carries the **session toggle** checkbox (§5.3).

**Recommended consolidation (and the reason this is an "evolution"):** extract the shared modal body
from `WalletUnlockPopup` into a private `passphrase_modal(ui, &mut PasswordInput, hint, error,
extra: impl FnOnce(&mut Ui)) -> PassphraseModalOutcome` helper in `components/`, then render both the
existing wallet-unlock popup and the new secret prompt through it. `WalletUnlockPopup` keeps its
public API; `EguiSecretPromptHost` passes a closure that draws the session checkbox in `extra`. If
the extraction proves fiddly, a straight clone with a `// SHARED-CHROME` marker is acceptable for v1
(same call the superseded plan made), but the extraction is preferred now because there are **two**
real call sites and a third latent one (`wallet_unlock.rs` inline render).

### 5.2 Where the unlock popup's role goes

`handle_wallet_unlocked` no longer distributes a seed (R1 is gone, §6.1). The unlock popup
(`wallet_unlock_popup.rs`) and `ScreenWithWalletUnlock` (`wallet_unlock.rs`) keep their job of
*verifying the passphrase and marking the wallet `Open` in the UI* (which gates display), but they
**stop being the secret-to-backend pipe**. Two coherent options:

- **(5.2-A) Keep the explicit unlock popup as a pure "verify + optionally seed the session cache"
  affordance.** When the user unlocks via the existing popup and the session toggle is on, the popup
  calls `secret_access.remember_session(scope, plaintext)`. When off, unlocking only flips the UI
  `Open` state; the first signing op will prompt via `with_secret`. **Recommended** — preserves the
  familiar "unlock my wallet" gesture and makes the toggle reachable there too.
- **(5.2-B) Remove the standalone unlock popup entirely** and rely solely on JIT prompts. Cleaner in
  principle but removes the ability to pre-unlock before an operation and changes muscle memory.
  **Not recommended for this cut** (it widens blast radius into every `ScreenWithWalletUnlock`
  consumer). **Open question Q-UNLOCK (§10).**

Either way, `Wallet::Open` ceases to be a *plaintext-seed residency* (R3) — see §6.1.

### 5.3 The toggle: placement, default, scope

- **Placement:** a single checkbox inside the reused modal, below the `PasswordInput`, above the
  Unlock/Cancel row. Copy: **"Keep this wallet unlocked until I close the app."**
  (single-key variant: "Keep this key unlocked…"). i18n-ready, named-placeholder-free, one unit.
- **Default:** **OFF** (operation-scoped). The reply's `remember_for_session` is `false` unless
  ticked. The default residency is therefore transient by construction.
- **Scope of "remember":** **per-secret** (`SecretScope`), not global. Ticking it for wallet A's
  seed does not unlock wallet B or an imported key. The `session` map is keyed by `SecretScope`, so
  granularity is exactly per-secret. (A future "remember all" global toggle could promote-on-unlock
  for every loaded scope, but is out of scope — §10 Q-GLOBAL.)

### 5.4 "Forget": when the session cache clears

- **App close** — process exit zeroizes (the `session` map's `Zeroizing` values drop). Primary path.
- **Manual lock** — a "Lock wallet" / "Lock key" action calls `secret_access.forget(scope)` (the
  successor to `forget_unlocked`, `single_key.rs:281`), removing+zeroizing that scope's entry.
- **Network switch** — `change_context` clears the whole `session` map (a per-network `WalletBackend`
  is rebuilt anyway; the old one drops, zeroizing). Belt-and-suspenders: `SecretAccess::forget_all()`
  on teardown.
- **Idle auto-lock timer** — **deferred** (non-goal §1.2). The toggle copy is the honest literal
  ("until I close the app"), not "for a while," exactly as the superseded plan's Q2 recommended.

---

## 6. Migrating every consumer onto `with_secret`

This is the heart of the cut. Each current secret reader is mapped to a `with_secret` scope. R1, R2,
R3, `provide_seed`, `signer_for`, `derive_private_key`, `single_key_unlocked`,
`unlock_with_passphrase`-as-cache-primer all retire.

### 6.1 HD seed — R1 + R3 retired; the seed becomes operation-scoped

**Retire R1 (`Inner.seeds`) and `provide_seed`.** `handle_wallet_unlocked` (`wallet_lifecycle.rs:302`)
stops calling `provide_seed`; the eager shielded init it triggers (§6.4) is also reworked.

**Retire R3 as a plaintext residency.** `Wallet::Open(OpenWalletSeed.seed)` is the model-layer
plaintext seed. The cleanest cut that respects "the backend decrypts JIT" is:

- `WalletSeed::open(password)` (`model/wallet/mod.rs:633`) no longer stores the decrypted seed in an
  `Open` variant for the whole session. Instead, unlock **verifies** the passphrase (decrypt-and-
  discard, or decrypt-into-session-cache iff the toggle is on) and flips a UI-visible `is_open`
  flag. The plaintext seed is no longer parked in `ctx.wallets`.
- Every reader that did `guard.seed_bytes()` now routes through `with_secret(HdSeed{seed_hash}, …)`.

> **Scope honesty.** Reshaping `WalletSeed` is the largest single piece of this cut because
> `seed_bytes()` (`mod.rs:751`) and `is_open()` (`mod.rs:716`) have many readers. A pragmatic
> staging (Bilby T-series, §9): first introduce `SecretAccess` and route the **backend** consumers
> (signing, shielded, DashPay) through it while `Wallet::Open` still exists; then, in a dedicated
> follow task, collapse `Wallet::Open`'s plaintext into the session cache so R3 is fully retired.
> The first stage already delivers operation-scoped residency for the *consumers the brief
> enumerates*; the second closes R3. **This staging is a recommendation; flag Q-R3 (§10) for the
> user to confirm R3 retirement is in this cut vs. a fast follow.**

HD consumers and their new form:

| Current | File:line | New |
|---|---|---|
| `signer_for(seed_hash)` builds `WalletAssetLockSigner` from R1 | `mod.rs:1079` | **Becomes `DetSigner` (§6.2)**: no eager seed; on each `sign`, `with_secret(HdSeed,…)` derives. For the batch asset-lock/identity flows, `with_secret_session` holds one decrypted seed across the upstream call. |
| `derive_private_key(seed_hash, path)` from R1 | `mod.rs:1114` | `with_secret(HdSeed{seed_hash}, |SecretPlaintext::HdSeed(seed)| derive(path, seed))` — one-shot credit-output key. |
| `send_payment` → `signer_for` | `mod.rs:1138` | `with_secret_session(HdSeed,…)` wrapping `send_to_addresses(&det_signer)`; one prompt, all inputs. |
| `register_identity` / `top_up_identity` → `signer_for` | `mod.rs:~1244/~1286` | same: `with_secret_session(HdSeed,…)` wraps `*_with_funding(&det_signer)`. `ensure_identity_funding_accounts` (which needs the seed to derive funding accounts) runs **inside** the same held-secret scope so it does not re-prompt. |
| `create_asset_lock_proof` → `signer_for` + `derive_private_key` | `mod.rs:1176`+ | one `with_secret_session(HdSeed,…)` covering both the signer and the credit-output derivation — single prompt for the whole asset-lock build. |

### 6.2 The `DetSigner` — implements the upstream async signer, pulls JIT

Rename/evolve `asset_lock_signer.rs` → `det_signer.rs` (or keep the file, add the JIT variant). Today
`WalletAssetLockSigner::new(seed, network)` is handed an already-snapshotted seed. The JIT signer is
constructed from a **borrowed held secret** inside a `with_secret_session` scope:

```rust
// inside with_secret_session(HdSeed{seed_hash}, |held| async {
//     let signer = DetSigner::from_held(held, network);   // borrows the Zeroizing seed
//     wallet.core().send_to_addresses(.., &signer).await  // upstream async sign() calls land here
// })
```

`DetSigner` implements `key_wallet::signer::Signer` (the asset-lock/payment seam) **and** the
identity-operation `async fn sign(&self, key:&K, data) -> Result<BinaryData, ProtocolError>` seam
(§0.3). Each upstream `sign` derives from the *held* seed (no re-prompt, no re-decrypt — the
operation already holds it). On drop at scope end, the held `Zeroizing` zeroizes. **This is the
existing `WalletAssetLockSigner` lifetime contract (`asset_lock_signer.rs:11-15`) with the seed
source changed from "snapshot at construction" to "borrow the held JIT secret."**

**Single-key flows through the same `DetSigner` seam** for the identity/DPNS/etc. signer trait when
the signing identity key is an imported single key: `with_secret(SingleKey{address},…)` yields the
32 bytes, `DetSigner::from_single_key(&bytes)` signs. (Live wiring of single-key *funding* remains
gated, §0.4 / §10.) For raw single-key ECDSA (`sign_with`, `single_key.rs:649`), `raw_key_bytes`
(`single_key.rs:293`) is rewritten to call `with_secret(SingleKey{address}, |k| ecdsa(k, msg))`
instead of consulting `single_key_unlocked`.

### 6.3 Single key — R2 retired

- **Retire `Inner.single_key_unlocked` (R2)** and `unlock_with_passphrase`'s cache-insert role
  (`single_key.rs:271-275`). The vault read + `entry.decrypt(Some(passphrase))` logic moves into
  `SecretAccess::decrypt_jit` for the `SingleKey` scope (§4.4).
- **`raw_key_bytes` / `sign_with`** no longer return `SingleKeyPassphraseRequired` to the UI as a
  *gate*; instead the missing secret is obtained inline via `with_secret`, which prompts. The typed
  `SingleKeyPassphraseIncorrect` is reused inside the re-ask loop (§4.4). `SingleKeyPassphraseRequired`
  becomes vestigial for the prompt flow (it may remain for non-interactive callers, e.g. MCP/CLI,
  which get the error rather than a prompt — §10 Q-HEADLESS).
- `import_wif` no longer "primes the cache" (`single_key.rs:230-233` per the superseded plan) — there
  is no operation cache to prime; a freshly imported key signs immediately because the just-imported
  passphrase can seed the session cache if the user opted in, else the next sign prompts.

### 6.4 Shielded `bind_shielded(seed)` — JIT-derived Orchard keys

`initialize_shielded_wallet` (`shielded.rs:372`) currently reads the plaintext seed from R3
(`shielded.rs:389-399`) and calls `derive_orchard_keys(&seed_bytes,…)` (`:402`); upstream's
`bind_shielded` (FFI `shielded_sync.rs:218`, host `wallet.bind_shielded(&shielded_seed,…)`) consumes
a shielded seed. Migration:

- The eager shielded init at unlock (`wallet_lifecycle.rs:316-331`) is **removed** — it forced a
  whole-session seed residency purely to warm shielded state. Shielded keys are derived **on first
  shielded operation** (shield / unshield / shielded send), not at unlock.
- That operation calls `with_secret(HdSeed{seed_hash}, |SecretPlaintext::HdSeed(seed)| {
  derive_orchard_keys(seed, network, 0) })`, then binds the resulting key set (DET's
  `ShieldedWalletState`, and upstream `bind_shielded` where the FFI path is used). The derived
  Orchard **viewing/spending key set** is what persists in `shielded_states` (as today); the **seed**
  does not.
- Background shielded *sync* (scanning) uses viewing keys already in `shielded_states` and needs no
  seed — so retiring eager init does not break sync; only the first *spend/bind* prompts.

> **Security note (Smythe).** Orchard spending keys derived from the seed live in `shielded_states`
> for the session today and will continue to. This design does not change that residency (it is
> derived-key state, not the seed). If the user wants shielded spending keys themselves to be
> operation-scoped, that is a larger follow-on (re-derive per spend); flagged Q-SHIELDED (§10).

### 6.5 DashPay contact-xpub derivation — JIT seed

`derive_contact_xpub_material(&seed_bytes,…)` (`dashpay.rs:105`) is called from
`contact_requests.rs:321` with a seed obtained via `first_open_wallet_seed` (`:521`), which reads R3.
Migration:

- `first_open_wallet_seed` is **removed**. The caller resolves the relevant `seed_hash` (it already
  has the `QualifiedIdentity`/wallet association) and wraps the derivation:
  `with_secret(HdSeed{seed_hash}, |SecretPlaintext::HdSeed(seed)| derive_contact_xpub_material(seed, network, account_index, sender, recipient, &ecdh))`.
- The signature of `derive_contact_xpub_material` is unchanged (`&[u8;64]`), so only its *source*
  moves behind the chokepoint. The receive-side derivation (per SEC-001, `incoming_payments`) maps
  identically.

### 6.6 The unlock chokepoint after the cut

`handle_wallet_unlocked` (`wallet_lifecycle.rs:302`) shrinks to: mark the wallet `Open` for display,
and — **iff** the session toggle was set during the unlock gesture — promote the verified seed into
`SecretAccess`'s session cache via a new `remember_session(scope, plaintext)`. It no longer calls
`provide_seed`, no longer eagerly inits shielded. The three `wallet_unlock*.rs` call sites
(`:33,100`, `:180,252`) and the cold-boot `bootstrap_loaded_wallets` (`:467`) adjust to the new
shrunken contract. **Crucially, watch-only rehydration (`rehydration-rewire/design.md`) is
unaffected**: that design already made load seedless and watch-only; this design supplies the seed
JIT for signing, which is exactly the "seed enters memory only on demand" property
`rehydration-rewire` §5.3 asked for — `provide_seed` was its placeholder for "unlock-time seed
provisioning"; **this design replaces that placeholder with operation-time provisioning** and is
strictly more conservative (the seed is in memory for one op, not the whole session).

---

## 7. Cancellation, reentrancy, batch

### 7.1 Batch (one operation, many signs → one prompt)

`with_secret_session` holds the decrypted secret for the whole closure (§4.2). A payment that signs N
inputs, an asset-lock build that signs the funding sighash **and** derives the credit-output key, an
identity registration that derives funding accounts **and** signs — all run inside one
`with_secret_session` scope and prompt at most once. The held secret zeroizes when the scope ends.
(Maps the superseded plan's "operation cache → one prompt" onto an explicit, bounded scope rather
than a process-lifetime map.)

### 7.2 Reentrancy / second request while a prompt is open

`AppState` keeps **one active prompt**; further `SecretPromptRequest`s sit in the queue (§3.4). So a
second operation that needs a *different* secret while the first prompt is open does not race — its
request is drained only after the first resolves. Within a single operation, §7.1 means there is no
second request at all. This **serializes** prompts by construction: at most one modal, FIFO drain.

A pathological case — two independent backend tasks both needing secrets concurrently — resolves as:
task A's prompt shows, task B's request waits in the queue; when A's user submits/cancels, B's prompt
shows next. Neither task busy-waits (both are parked on their `oneshot`). This is the desired UX (one
question at a time) and is safe (each `oneshot` is independent).

### 7.3 Cancellation = drop the reply sender = clean abort

- User dismisses the modal (Cancel / X / Escape / click-outside): the host **drops the
  `oneshot::Sender`** without sending. The awaiting `with_secret` sees `RecvError` →
  `SecretPromptCancelled` → the consuming backend task returns `Err(TaskError::SecretPromptCancelled)`
  → normal `TaskResult::Error` → a calm banner. **No partial state**: the secret was never decrypted
  (or was decrypted and immediately dropped), no signing began, nothing persisted.
- Mid-operation cancel (the prompt appears during a multi-sign op and the user cancels): the held
  scope unwinds, `Zeroizing` zeroizes, the upstream call was never issued (the signer construction is
  inside the scope, after the secret is obtained). For an op that already broadcast one tx and needs
  a second sign — not possible under §7.1, because one op prompts once *before* the first sign; if a
  *future* op genuinely needs mid-stream prompts, it must be designed to be idempotent/resumable, and
  that is out of scope here. Flag Q-MIDSTREAM (§10) only if such an op is introduced.
- Network switch / app close during an open prompt: host drops the queue's senders → every parked
  `with_secret` cancels cleanly; `session` zeroizes (§5.4).

### 7.4 Wrong passphrase

Handled inside the `with_secret` loop (§4.4): re-ask with `retry_reason = WrongPassphrase`, modal
stays open, `PasswordInput.set_error(...)` shows the typed `*PassphraseIncorrect` Display, field
cleared (`PasswordInput.clear()` zeroizes the prior attempt). No retry cap by default (local
AES-GCM, no remote attacker, no account to lock — same rationale as the superseded plan's D4);
a soft cap is a Smythe option (Q-RETRYCAP, §10).

---

## 8. Fund-safety, secret hygiene, type boundaries

### 8.1 Fund-safety invariants preserved

- **published-xpub == scanned-xpub account-xpub gate stays untouched.** This design changes *when the
  seed is decrypted*, never *which* wallet/xpub is used. `bip44_account_xpub_encoded` (`mod.rs:503`)
  and the `rehydration-rewire` WalletId-match gate are orthogonal and unmodified.
- **Watch-only rehydration interaction:** strictly improved. Seedless load (`rehydration-rewire`)
  brings wallets back watch-only with no seed in memory; this design keeps the seed out of memory
  until a *signing operation*, then for only that operation (default). The two compose: cold boot →
  watch-only display (no seed) → user signs → one-op seed (or session cache if opted in).

### 8.2 Secret hygiene (NFR-1)

- **Passphrase** crosses the seam only as `SecretString` (zeroizing). It enters **no** `TaskError`
  (§4.5), **no** `AppAction`/`BackendTask`, **no** logs, **no** banner details. The `SecretPromptReply`
  is consumed inside `with_secret` and dropped.
- **Derived plaintext** (64-byte seed / 32-byte key) exists only inside `Zeroizing` guards within
  `with_secret`/`with_secret_session`/the session cache; never serialized, never logged, never in an
  error.
- **`SecretPlaintext` / `SecretSession`** have no `Clone`, no `Debug` that exposes bytes, no `Deref`
  to raw bytes — access is via explicit `expose_*` returning a borrow with the closure's lifetime.
- **In-process channel trust boundary:** the `oneshot` and the request queue live entirely within the
  one process; no IPC, no serialization. The trust boundary is the process. The reply carries a
  passphrase (input), not a derived key, minimizing the value-at-risk even within that boundary.
- **Zeroization points (exhaustive):** (a) `SecretString` passphrase on `SecretPromptReply` drop;
  (b) the operation-scoped `Zeroizing` plaintext on closure exit; (c) `PasswordInput.Secret` on every
  `clear()`/close/wrong-attempt and on drop; (d) session-cache `Zeroizing` values on `forget`/
  network-switch/app-close.

### 8.3 M-DONT-LEAK-TYPES

- `SecretAccess`, `DetSigner`, `SecretSession`, `SecretPlaintext`, `WalletId`, and all upstream
  `platform_wallet` / `key_wallet` types stay **inside the `wallet_backend` seam**.
- The **only** types the UI sees are the `secret_prompt.rs` seam: `SecretPrompt`, `SecretPromptRequest`,
  `SecretPromptReply`, `SecretScope`, `SecretPromptRetry`, `SecretPromptCancelled`, plus the reused
  `SecretString` (already an allowed DET dependency). `WalletSeedHash` on `SecretScope` is a DET type,
  not upstream — permitted.
- `SecretString`/`SecretBytes` are `platform_wallet_storage` types; per the module's documented
  M-PLATFORM-WALLET-FIRST-PARTY exception (`mod.rs:6-10`), they may appear on the `wallet_backend`
  surface. They sit on the seam deliberately (the brief mandates reuse), and the UI already imports
  the crate.

---

## 9. Task breakdown for Bilby (Phase 2)

Ordered by dependency. Each ≥100 lines or batched. TC references: existing IDs from
`signtime-unlock-ux/test-cases.md` where they survive; **[NEW-TC]** where the expanded HD/await/toggle
scope needs cases that did not exist (the superseded suite was single-key + gate-on-error only).
**(S)** = Smythe security review required.

- **T1 — `secret_prompt.rs` seam (S).** `SecretPrompt` trait, `SecretScope`, `SecretPromptRequest`,
  `SecretPromptReply` (`SecretString` field), `SecretPromptRetry`, `SecretPromptCancelled`. No egui,
  no upstream types beyond `SecretString`. Unit: a `TestPrompt` double (scripted replies/cancel).
  Satisfies: NFR-1 confinement contract (TC-SEC-001/002/003 re-pointed), [NEW-TC] cancel=drop. ~150 lines.

- **T2 — `secret_access.rs` chokepoint (S).** `SecretAccess`, `with_secret`, `with_secret_session`
  (resolve `AsyncFnOnce` vs RAII-guard spike — §4.2; flag Nagatha if guard chosen), `SecretPlaintext`,
  `SecretSession`, session cache (opt-in, per-scope, zeroizing), `decrypt_jit` for both scopes (move
  the single-key decrypt from `unlock_with_passphrase`, the HD decrypt from `WalletSeed::open`),
  re-ask loop, `forget`/`forget_all`, `remember_session`. New `TaskError` variants
  `SecretPromptCancelled`, `SecretDecryptFailed`, `HdPassphraseIncorrect`. Satisfies: TC-UNLOCK-001..004
  (re-pointed), TC-WRONG-001/002 (re-pointed), [NEW-TC] session-promote/forget, [NEW-TC] op-scope
  zeroize, [NEW-TC] batch-one-prompt. ~300+ lines (batched). Dep: T1.

- **T3 — `EguiSecretPromptHost` + AppState drain + reused modal (S).** Host impl (queue + repaint +
  oneshot await); `AppState::update` drain beside `task_result_receiver` (`app.rs:1300`); reused
  `PasswordInput`; clone/extract `WalletUnlockPopup` chrome into shared `passphrase_modal` (§5.1);
  session checkbox (default OFF, §5.3); inline error from typed `*PassphraseIncorrect`. Catalog row in
  `src/ui/components/README.md`. Satisfies: TC-PROMPT-001..006, TC-A11Y-001..005, TC-WRONG-002/003,
  TC-CANCEL-001..004 (re-pointed), TC-SEC-004, [NEW-TC] toggle default-off, [NEW-TC] FIFO serialize
  two requests. kittest in `tests/kittest/` (house style: `force_input_for_test`, `query_by_label`).
  ~300+ lines (batched). Dep: T1, T2.

- **T4 — `DetSigner` JIT signer (S).** Evolve `asset_lock_signer.rs` → JIT: construct from a *held*
  secret inside a `with_secret_session` scope; implement both the `key_wallet::signer::Signer` seam
  and the identity-op `async fn sign` seam (§6.2); HD and single-key sources. Satisfies: [NEW-TC]
  HD-sign-derives-JIT, [NEW-TC] held-secret-no-reprompt, secret-zeroize-on-scope-exit. ~200 lines.
  Dep: T2.

- **T5 — Migrate HD backend consumers (S).** `signer_for`/`derive_private_key`/`send_payment`/
  `register_identity`/`top_up_identity`/`create_asset_lock_proof` (`mod.rs:1079..1300`) onto
  `with_secret_session`; delete `provide_seed` (`mod.rs:531`), `Inner.seeds` (`mod.rs:277`),
  `signer_for`'s eager read; shrink `handle_wallet_unlocked` (`wallet_lifecycle.rs:302`).
  Satisfies: [NEW-TC] each HD op prompts once / cancel aborts / session-cache skips prompt;
  regression: `assert_can_sign` test (`mod.rs:1095`) reframed to "with_secret yields a signer."
  ~250 lines (batched). Dep: T4.

- **T6 — Migrate single-key signing (S).** `raw_key_bytes`/`sign_with` (`single_key.rs:293,649`) onto
  `with_secret(SingleKey,…)`; delete `Inner.single_key_unlocked` (`mod.rs:201`) and the cache-insert
  in `unlock_with_passphrase` (`single_key.rs:271-275`); `import_wif` cache-prime removed; `forget`
  → `SecretAccess::forget`. Satisfies: TC-UNLOCK-002 (session) / TC-WRONG-001 (re-pointed),
  [NEW-TC] no-cache-prime. ~180 lines. Dep: T4.

- **T7 — Migrate shielded + DashPay (S).** Shielded: remove eager init at unlock
  (`wallet_lifecycle.rs:316`); derive Orchard keys on first shielded op via `with_secret(HdSeed,…)`
  (`shielded.rs:372-402`). DashPay: route `derive_contact_xpub_material` (`dashpay.rs:105`,
  `contact_requests.rs:321`) through `with_secret`; delete `first_open_wallet_seed`
  (`contact_requests.rs:521`). Satisfies: [NEW-TC] shielded-first-spend-prompts,
  [NEW-TC] dashpay-derive-JIT. ~200 lines (batched). Dep: T4.

- **T8 — [GATED Q-R3] Retire `Wallet::Open` plaintext residency (S).** Reshape `WalletSeed::open`
  (`model/wallet/mod.rs:633`) to verify-not-park; reroute remaining `seed_bytes()` readers; `is_open`
  becomes a UI-display flag, not a plaintext-present flag. Large blast radius — staged per §6.1.
  Satisfies: R3 fully retired; [NEW-TC] no-plaintext-seed-in-ctx.wallets. Size: large; specify when
  Q-R3 resolved. Dep: T5, T7.

- **T9 — [GATED Q-SEND] Single-key send live wiring.** Un-stub `send_single_key_wallet_payment`;
  signs via the `with_secret(SingleKey,…)` path. Inherits the superseded plan's T5 framing.
  Dep: T6, Q-SEND.

- **T10 — Docs + supersession housekeeping (batched).** Mark
  `signtime-unlock-ux/dev-plan.md` superseded; update `docs/user-stories.md` (FR-6 session unlock now
  the toggle); `components/README.md` (the prompt host + shared `passphrase_modal`); `SECRETS`/security
  notes. Dep: T8 (or T7 if R3 deferred).

**Buildable now without a user decision: T1–T7, T10 (T10 partial). Gated: T8 (Q-R3), T9 (Q-SEND).**
Smythe review: T1, T2, T3, T4, T5, T6, T7, T8, T9 (all touch secret handling).

---

## 10. Open questions (need a user / stakeholder decision)

1. **Q-R3 — Retire `Wallet::Open` plaintext seed in this cut, or fast-follow?** Retiring R3 (T8) is
   the largest piece and touches many `seed_bytes()` readers. Recommend: route the brief's enumerated
   consumers through `with_secret` now (T5–T7), and land R3 retirement (T8) as a tightly-scoped
   follow so the bulk of the cut ships without the wide blast radius. **User confirm.**
2. **Q-UNLOCK — Keep the explicit unlock popup (verify + optional session-seed, 5.2-A) or remove it
   (5.2-B)?** Recommend 5.2-A (preserves "unlock my wallet" muscle memory, gives the toggle a second
   home). **User confirm.**
3. **Q-SEND — Single-key send: in or out of THIS deliverable?** Unchanged from the superseded plan's
   pivotal Q3. The single-key *signing machinery* (T6) is built and testable against `sign_with`;
   live *send* (T9) needs UTXO selection / tx build / fee / broadcast. Recommend: machinery now, live
   send tracked separately. **User confirm.**
4. **Q-SHIELDED — Should Orchard spending keys themselves be operation-scoped?** This design keeps
   derived shielded key state session-resident (as today) and only makes the *seed* JIT. Per-spend
   re-derivation is a larger follow. Recommend: out of scope now. **User confirm.**
5. **Q-HEADLESS — Non-interactive callers (MCP/CLI) have no prompt host.** For those, `with_secret`
   must fail with a typed "secret required, no interactive prompt available" error rather than hang.
   Recommend: a `NullSecretPrompt` that immediately cancels, surfacing `SecretPromptCancelled` (or a
   dedicated `SecretPromptUnavailable`) to the MCP error envelope. **User confirm** the headless UX.
6. **Q-RETRYCAP — Wrong-passphrase soft cap?** Default: none (local AES-GCM, no remote attacker).
   Smythe may want a soft cap. **Security decision.**
7. **Q-MIDSTREAM — Any operation that must prompt mid-stream after partial broadcast?** None today
   (§7.3). If introduced later, it must be idempotent/resumable. Flag only if such an op appears.

---

## Candy tally 🍬 (architecture findings by severity)

- **CRITICAL (1):** R3 — the `Wallet::Open` whole-session plaintext seed (`model/wallet/mod.rs:598`,
  read by DashPay `first_open_wallet_seed` and shielded `initialize_shielded_wallet`) is a third
  eager residency the gate-on-error plan never addressed; a JIT design that ignores it leaves the
  primary signing-seed plaintext resident all session. Must be retired (T8) for the cut to mean
  anything.
- **HIGH (2):** (1) the upstream per-op `async fn sign` seam + the existing one-op-snapshot
  `WalletAssetLockSigner` are the natural JIT injection point — `DetSigner` evolves it from
  snapshot-at-construction to borrow-the-held-secret (§6.2), so no new signing path is invented;
  (2) `with_secret_session` must hold one decrypted secret across the upstream `await`s of a
  multi-sign op (one prompt per operation) — the `AsyncFnOnce`-vs-RAII-guard choice is load-bearing
  for consumer signatures and needs a Bilby spike (§4.2).
- **MEDIUM (3):** (1) eager shielded init at unlock (`wallet_lifecycle.rs:316`) exists *only* to warm
  state and forces seed residency — removing it (derive-on-first-spend) is both a hygiene win and a
  prerequisite for operation-scoped seeds; (2) `handle_wallet_unlocked` inverts from
  secret-distributor to verify-and-optionally-seed-session — the unlock popup's role shrinks, not
  grows; (3) the reply must carry the *passphrase* (`SecretString`), not a derived key, so cancel is
  a clean sender-drop and the channel never holds key material.
- **LOW (2):** (1) reuse `SecretString`/`SecretBytes` and `PasswordInput`/`WalletUnlockPopup` rather
  than inventing — the shared `passphrase_modal` extraction now has two real call sites and is worth
  doing; (2) the gate-on-error TCs (TC-SITE-001, TC-RESUME-002, TC-CANCEL-005) are obsoleted by the
  await model — a finding that prevents Bilby from re-implementing a now-wrong contract.
- **Open questions (7):** Q-R3, Q-UNLOCK, Q-SEND, Q-SHIELDED, Q-HEADLESS, Q-RETRYCAP, Q-MIDSTREAM.

**Total: 8 findings (1 critical, 2 high, 3 medium, 2 low) + 7 open questions.**

---

## Resolved Decisions (Wave plan)

Settled by user + Smythe before Wave 1 build. Where these differ from the body above, **these win**.

### Four settled decisions

1. **Residency is operation-scoped by default; a "remember" toggle promotes to a session cache.**
   The policy is modelled as `RememberPolicy { None, UntilAppClose, For(Duration) }` (replaces the
   body's `remember_for_session: bool` on `SecretPromptReply`, which now carries `remember:
   RememberPolicy`). The GUI later wires only `None` + `UntilAppClose`; `For(Duration)` is
   data-model-only for now (unused by UI). Session-cache entries **carry a TTL/expiry and honor it on
   access** (expired entries are evicted and force a re-prompt). Default is `None`.
2. **Secret-on-the-wire is `platform_wallet_storage`'s `SecretString`.** The reply carries the
   passphrase as `SecretString` plus the chosen `RememberPolicy`. No new secret type. No secret in
   `TaskError` / `AppAction` / logs / `Debug`.
3. **The chokepoint lives behind `secret_prompt` (UI seam) + `secret_access` (orchestration).** Only
   `secret_prompt`'s types are UI-facing; `SecretAccess` / `DetSigner` / upstream types stay in the
   `wallet_backend` seam (M-DONT-LEAK-TYPES).
4. **Typed errors only.** New `TaskError` variants `SecretPromptCancelled`, `SecretDecryptFailed`,
   `HdPassphraseIncorrect`; wrong-passphrase re-ask loop reuses `SingleKeyPassphraseIncorrect` /
   `HdPassphraseIncorrect`. No user-facing `String` fields; no error-string parsing.

### Smythe must-fixes (baked into Wave 1)

1. **Closure form, not RAII guard.** `with_secret(scope, FnOnce(SecretPlaintext))` and
   `with_secret_session(scope, AsyncFnOnce(&SecretSession))` confine plaintext to the closure; no
   storable guard can be parked across awaits. (Resolves the §4.2 `AsyncFnOnce`-vs-guard spike in
   favour of `AsyncFnOnce` — **no consumer-signature change vs. the guard form is needed; Nagatha
   re-review NOT triggered.**)
2. **Borrow-only, no bare `[u8;N]` copies on the consumer path.** The closure borrows
   `&Zeroizing<…>` via `SecretPlaintext` (no `Clone`, no `Deref` to raw bytes — access via explicit
   `expose_*`). `DetSigner` borrows the held secret rather than snapshotting. The one place a copy is
   made is the cache-hit op-lift (an op-scoped `Zeroizing` copy taken to release the cache lock before
   the closure's `await`, avoiding a re-entrancy deadlock); it zeroizes on op exit, exactly like the
   prompt path's owned plaintext.
3. **Boxed session-cache secrets, poison-safe clear.** Cached plaintext is `Box<Plaintext>` so a
   `HashMap` rehash moves only the pointer. `forget` / `forget_all` recover a poisoned lock
   (`into_inner`) so a panicked reader can never strand a plaintext.
4. **Never prompt for unprotected scopes.** `with_secret` checks `scope_has_passphrase` first;
   unprotected HD wallets (`uses_password = false`) and unprotected imported keys resolve via
   `decrypt_jit(scope, None)` with no prompt.
5. **Secret-confinement sentinel tests.** A sentinel passphrase + sentinel seed are asserted absent
   from every emitted error `Display`, `Debug`, and from `DetSigner`/`SecretPromptCancelled` debug.

### Wave 1 scope delivered (additive, no removals)

`src/wallet_backend/secret_prompt.rs` (T1), `secret_access.rs` (T2), `det_signer.rs` (T4), and the
three new `TaskError` variants. The eager model (R1 `inner.seeds`/`provide_seed`, R2
`single_key_unlocked`, R3 `Wallet::Open`) and all consumer rewiring (T3 UI host, T5–T9) are **Waves
2–4** and untouched here. `DetSigner` compiles + is unit-tested via the mock prompt but is not yet
swapped into call sites (`#![allow(dead_code)]` on the module until Wave 2).
