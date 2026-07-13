# Provider-Key Account Usage Scenarios & the Orphaned-Wallet Skip Loop

Two separable topics, deliberately kept in one doc because both trace back to the
same upstream area (`rs-platform-wallet-storage`, PR #3968 / #4072):

- **Section A** — how DET would eventually *use* persisted provider-key accounts
  (BLS operator keys, EdDSA platform-node keys) once
  [dashpay/platform#4113](https://github.com/dashpay/platform/issues/4113) lands.
  **Informational. Not committed work. Nothing here is built.**
- **Section B** — a live defect in DET's persisted-wallet skip path. Candidate
  follow-up issue; **untracked today** (no platform issue number exists).

> Citations are pinned to DET at branch `docs/platform-wallet-migration-design`
> and platform at DET's `Cargo.lock` pin
> `44c20e35f572800d62e95ac9fcaa50697c25d1bd` (branch `dash-evo-tool`), except
> where a line is explicitly marked as living at platform `HEAD` (post-#4072).
> READ-ONLY design — no production code in this document.

---

## 0. Headline findings (read first)

1. **DET cannot name `ProviderKeyAccountEntry` today.** The `Cargo.lock` pin
   (`Cargo.lock:1880`) predates #4072 (merged 2026-07-10), which introduced the
   type. Every Section-A scenario is gated behind a pin bump before a single line
   of DET code can reference it.
2. **DET has no operator-key or platform-node-key surface at all.** The masternode
   load form collects exactly three keys — Voting, Owner, Payout — all pasted
   manually (`src/ui/masternodes/load_form.rs:296-308`, placeholder
   `"Private key (WIF or hex)"` at `:16`). There is no BLS operator field and no
   EdDSA platform-node field anywhere. Section A is **new surface**, not a
   replacement of an existing one.
3. **A locked DET decision partially blocks Section A — and its code comment
   over-generalizes it.** `§Locked-decisions #4`
   (`docs/ai-design/2026-07-09-masternode-page-design/02-ux-spec.md:415-418`)
   retired auto-derive and is scoped **explicitly and only** to
   Voting/Owner/Payout. But the code doc-comment generalizes it to *"masternode
   keys **never** live in a wallet's HD tree"* (`load_form.rs:3`, repeated at
   `:172`). That blanket claim is **false once provider-key accounts exist
   upstream** — operator BLS and platform-node EdDSA keys *are* HD-derived. A
   future implementer reading only the comment would wrongly conclude the whole
   feature is off the table. **Re-scope the comment; the decision itself stands
   for V/O/P.** (Finding UX-1.)
4. **Section B's "unrecoverable closed loop" is NOT confirmed — the common case
   appears to self-heal.** Static tracing (below, §B.2) shows W2
   (`ensure_upstream_registered`) *repairs* an empty-manifest row on the next
   boot/unlock, because a skipped row is never inserted into the in-memory
   manager, so `create_wallet_from_seed_bytes` re-inserts and re-`store`s a full
   manifest. The briefed terminal error `WalletRegistrationXpubMismatch` is
   **not reachable** on this path. See §B.2 — this materially reduces severity
   and must be reproduced before anything is filed.
5. **The residual defects are real, narrower, and still worth fixing** (§B.3):
   a permanently-immortal orphan row when *no DET sidecar seed exists* for it,
   a **stale warning banner** that survives a successful self-heal for the whole
   session, and banner advice that is **literally unexecutable**
   (`WalletAlreadyImported` fires first).
6. **DET throws away the one handle it needs.** Upstream's skip outcome already
   carries the corrupt row's `wallet_id`; DET discards it —
   `.map(|(_wallet_id, reason)| (None, ...))` (`src/wallet_backend/mod.rs:572`).
   The `Option<WalletSeedHash>` in `LoadedWallets.skipped`
   (`src/wallet_backend/loader.rs:45`) is therefore **structurally always
   `None`**, despite its doc claiming otherwise. (Finding UX-2.)
7. **The eviction primitive mostly exists upstream already.**
   `SqlitePersister::delete_wallet(wallet_id)` cascade-deletes by `wallet_id`
   with no in-memory dependency (`persister.rs:398`, platform HEAD). It is simply
   **not on the `WalletPersister` trait** — `traits.rs:318` names `list_wallets`
   and `delete_wallet` as *deferred contract candidates*. The upstream ask is
   smaller than "design a new API".

---

# Section A — Future provider-key-account usage scenarios

**Status: informational.** None of this is built, scheduled, or committed. The
purpose is to make the eventual DET integration land well, and to give #4113 a
consumer-side justification.

## A.1 Personas in scope

| Persona | Relevance |
|---|---|
| **Priya** (Power User — masternode operator, `docs/personas/power-user.md`) | Primary. Already lists *"Masternode key management — access provider voting, owner, operator, and platform node key paths"* as **Primary Goal #7**, and *"Time to check masternode key paths: under 10 seconds"* as a success metric. Provider-key persistence is squarely her unmet need. |
| **Jordan** (Platform Developer) | Secondary. Repeatable devnet/testnet node bring-up. |
| **Alex** (Everyday User) | **Out of scope.** Must never see a Node Keys surface. Gate behind Expert Mode, as the Masternodes root tab already is. |

Note Priya's persona doc **already promises** operator and platform-node key
paths. DET does not deliver them today. Section A closes a gap the personas
assert exists.

## A.2 User stories

Each carries its concrete DET-side prerequisites. **All share prerequisite P0.**

> **P0 — Pin bump.** DET's `platform` pin must move past #4072 so
> `ProviderKeyAccountEntry` is nameable, *and* past #4113 so the accounts are
> actually persisted rather than silently dropped. Until both land, every story
> below is unimplementable.

---

**US-A1 — Operator-key recovery after reinstall**

> As a **masternode operator (Priya)**, I want my BLS operator key to come back
> automatically when I reinstall DET and restore my wallet, so that I do not have
> to hunt for the key in my node's config file or re-enter a seed I have already
> restored.

*Why this needs #4113:* the operator key is HD-derived into a provider-key
account. If that account registration is not persisted, a restored wallet rebuilds
with no record it ever had one — the key is silently absent, exactly the drop
#4113 fixes.

*Acceptance (sketch):*
- **Given** a wallet with a registered provider-key account, **when** DET is
  reinstalled and the wallet restored from its recovery phrase, **then** the
  operator key is derivable without further user input.
- **Given** the same, **when** the wallet is loaded watch-only at cold boot,
  **then** the operator *public* key is visible with no password prompt (the
  private key stays behind the JIT secret chokepoint).

*DET prerequisites:* P0 · new Node Keys UI surface · new `backend_task` to read
provider-key accounts · **re-scope `§Locked-#4`** (Finding UX-1).

---

**US-A2 — View derived platform-node key for ProRegTx construction**

> As a **masternode operator (Priya)**, I want to see the EdDSA platform-node key
> DET derives for my wallet, so that I can put the correct value into my ProRegTx
> and my node's configuration without generating a key by hand.

*Acceptance (sketch):*
- **Given** an Expert-Mode wallet, **when** Priya opens Node Keys, **then** the
  platform-node public key and its derivation path are shown, copyable, in under
  10 seconds from the wallet screen (her stated metric).
- **Given** a locked wallet, **when** she reveals the *private* half, **then**
  the JIT chokepoint prompts once — reusing the existing sign-time unlock
  pattern, no new crypto.

*DET prerequisites:* P0 · Node Keys UI · derivation-path display · hold-to-reveal
+ chokepoint reuse (both already exist in the masternode key screen).

---

**US-A3 — Pre-registration key preview**

> As a **masternode operator (Priya)**, I want to preview the operator and
> platform-node keys DET *would* derive **before** I broadcast a ProRegTx, so that
> I can commit to a node identity knowing the keys are already safely backed by my
> recovery phrase.

*Why it matters:* today the operator key is generated outside DET and pasted in;
if the operator loses it, the masternode is unrecoverable without a
ProUpRegTx. Deriving it from the seed makes "my recovery phrase is my backup"
true for node keys too — the single biggest safety win in Section A.

*DET prerequisites:* P0 · Node Keys UI · read-only derivation preview (no
registration side-effect).

---

**US-A4 — Repeatable node keys on testnet**

> As a **Platform developer (Jordan)**, I want deterministic, seed-derived node
> keys on Testnet/Devnet, so that I can tear down and rebuild a node without
> tracking a separate key file per environment.

*DET prerequisites:* P0 · Node Keys UI (Testnet) · no new backend surface beyond
US-A2.

---

**US-A5 — Bundle the watch-only registration ask (upstream)**

> As a **DET maintainer**, I want `PlatformWalletManager::register_watch_only_wallet`
> exposed upstream, so that imported single keys can be refreshed and spent.

Not a provider-key story, but it belongs in the **same upstream ask**: it needs a
public constructor over the *same private `register_wallet` body* that #4113-adjacent
work already touches. It is blocked and documented in-tree at
`src/backend_task/core/mod.rs:190-198` (refresh) and `:281-293` (send), and is the
reason `CoreTask::RefreshSingleKeyWalletInfo` returns
`TaskError::SingleKeyWalletsUnsupported` (`:199-201`).

**Recommendation:** when #4113's DET-side follow-up is filed upstream, bundle
`register_watch_only_wallet` into it — one upstream conversation, one review, one
pin bump, two features unblocked.

## A.3 The `§Locked-#4` collision (must be resolved before any of A.2 is built)

| | |
|---|---|
| **What is locked** | Auto-derive of **Voting / Owner / Payout** keys is retired; the load form asserts the *absence* of a derive affordance (`TC-FR4-01`, `03-test-case-spec.md:84`). Rationale: `derive_keys_from_wallets` is hard-gated to `IdentityType::User` in `backend_task/identity/load_identity.rs` (field threaded at `:80`, type gate at `:111`). |
| **What is NOT locked** | Anything about **operator BLS** or **platform-node EdDSA** keys. `§Locked-#4` never mentions them. |
| **The trap** | `load_form.rs:3` and `:172` state the blanket *"masternode keys never live in a wallet's HD tree"* — an over-broad rendering of a V/O/P-scoped decision. |

**Action (cheap, do it independently of #4113):** amend the two doc-comments to say
what the decision actually says — *"Voting/Owner/Payout keys are never
wallet-derived (§Locked-#4); provider-key accounts (operator/platform-node) are a
separate, unaddressed surface."* This costs nothing now and prevents a future
implementer from discarding Section A on the strength of a comment.

---

# Section B — The persisted-wallet skip path

**Status: candidate follow-up issue. Untracked — no platform issue number exists.**
File it once this doc is reviewed. **Reproduce first (§B.5) — the severity below
is materially lower than initially briefed.**

## B.1 The setup

Wallet registration (`src/context/wallet_lifecycle/registration.rs:101-172`) is a
non-atomic 5-step sequence:

| Step | What | Failure mode |
|---|---|---|
| 1 | Reject duplicate (`:116-121`) | — |
| 2 | Seed-envelope vault write (`:131`) | **fail-closed** |
| 3 | Wallet-meta sidecar write (`:142`) | **fail-closed** |
| 4 | In-memory insert + address bootstrap (`:145-159`) | — |
| 5 | Upstream persistor write (`:169`) | **async, best-effort** — logged at `warn`, never surfaced (`:180-208`) |

Steps 2-3 are fail-closed; step 5 is not. A crash or failure between them leaves
DET's sidecars intact but the upstream row absent or (per the platform TODO)
half-written with a **permanently empty account manifest**.

Upstream acknowledges this exact state, unfiled:

> `TODO(product decision needed, task #14): a crash between wallet-row creation and
> first-account-registration leaves this row with a permanently empty manifest. It
> is not corrupted or lost — every future load correctly skips it as
> MissingManifest — but there is no recovery path today: no re-registration flow,
> no eviction, no surfacing to the user.`
>
> — `packages/rs-platform-wallet-storage/src/sqlite/persister.rs:1070-1078` (platform HEAD)

Such a row is skipped on every load as `MissingManifest`
(`src/wallet_backend/loader.rs:55`), so it never enters `id_map`.

## B.2 Correction: the common case appears to SELF-HEAL

The briefed claim — that re-registration hard-rejects with
`WalletRegistrationXpubMismatch`, closing an unrecoverable loop — **does not hold
under static tracing.** The trace:

1. W2 `ensure_upstream_registered` (`mod.rs:799-815`) fires at cold boot for every
   **open** wallet, inside a prompt-free JIT seed scope
   (`src/context/wallet_lifecycle/bootstrap.rs:117`). Its doc is explicit:
   *"an already-bootstrapped wallet that was never registered upstream still gets
   registered here"* (`bootstrap.rs:66-67`).
2. `id_map` lacks the seed hash (the skip guaranteed that) → it proceeds to
   `register_wallet_from_seed` (`mod.rs:712`).
3. `pwm.get_wallet(&wallet_id)` (`:738`) → **`None`**. A skipped row is *never
   inserted into the in-memory manager* — the placeholder empty wallet is built
   and then dropped one layer up (`persister.rs:1064-1068`). So the
   `resolve_registered_wallet` branch that raises `WalletRegistrationXpubMismatch`
   (`mod.rs:857-863`) **is not reached**.
4. → `create_wallet_from_seed_bytes` (`:752`). `WalletAlreadyExists` is raised by
   the **in-memory** `wm.insert_wallet`, not the persistor
   (`wallet_lifecycle.rs:259-269`, platform HEAD) — and the in-memory map is empty
   for this wallet, so the insert **succeeds**.
5. → the create path `store`s a full changeset for the same `wallet_id`,
   **overwriting the empty manifest**. `resolve_registered_wallet` then finds the
   wallet, the xpub matches, `id_map` is populated.

**Net: the row is repaired on the next boot** (unprotected wallet: prompt-free;
protected wallet: at the next unlock gesture, which is when it becomes open).

> ⚠️ This is **static analysis only** — it was not executed (see §B.5). It should
> be confirmed by a reproduction test *before* an issue is filed, because it
> changes the bug from "unrecoverable data loss" to "misleading UI + a narrow
> immortal-row leak".

## B.3 What is actually still broken

**B-1 — The immortal orphan (no sidecar seed).** The self-heal in §B.2 requires
DET to *hold the seed*. A persisted row whose seed DET no longer has — sidecar
deleted, wallet removed locally, a row belonging to a foreign seed — can never be
healed (no seed → no derivation → no W2) and can never be **evicted**:
- `remove_wallet` (`src/context/wallet_lifecycle/removal.rs:38-57`) drives upstream
  eviction only `if let Some(wallet_id) = upstream_id`, sourced from
  `registered_wallet_id` → the `id_map` the skip guarantees stays empty. So the
  subtask never spawns.
- Even if it did, upstream `PlatformWalletManager::remove_wallet`
  (`wallet_lifecycle.rs:452`) early-returns `WalletNotFound` when the wallet is
  absent from the in-memory map (`:485-487`) — always true for a skipped row — and
  DET maps that error to `Ok(())` (`mod.rs:984`). **A guaranteed silent no-op.**

  Such a row is skipped forever, counted in the banner forever, and removable by
  nothing short of deleting the sqlite file.

**B-2 — The banner is stale, and its advice is unexecutable.**
Copy (`mod.rs:2482`):
> *"1 saved wallet couldn't be opened. Re-add it from its recovery phrase to restore it."*

Two independent defects:
- **Stale.** The banner is raised during the load pass (`mod.rs:496`), which runs
  *before* W2 self-heals the row. `raise_skipped_wallets_banner` clears only on a
  **subsequent load pass** reporting zero skips (`:2497-2506`). If no second pass
  runs after W2, a **persistent Warning banner accuses DET of losing a wallet that
  is, by then, present and healthy** — for the rest of the session.
- **Unexecutable.** If the user *follows* the advice and re-adds from the recovery
  phrase, `register_wallet` rejects it with `WalletAlreadyImported`
  (`registration.rs:116-121`) — the meta sidecar is still there. The instruction
  cannot be carried out as written. Walking this as Priya: she is told her wallet
  is gone, told to restore it, and the restore is refused. She has no next move.

**B-3 — DET discards the corrupt row's identity.** `mod.rs:572` maps upstream's
`(wallet_id, reason)` to `(None, reason)`, so `LoadedWallets.skipped`'s
`Option<WalletSeedHash>` is *always* `None` (`loader.rs:45`) — its doc comment
("present only when the skipped wallet could be matched") describes a matching
step that does not exist. DET therefore cannot name, target, or evict the bad row
even though upstream handed it the id.

## B.4 Proposed fix shape (do not implement from this doc)

**Platform side** — smaller than it looks; the primitive already exists.

1. **Promote `delete_wallet` (and `list_wallets`) onto the `WalletPersister`
   trait.** `traits.rs:318` already names them as *deferred contract candidates*.
   `SqlitePersister::delete_wallet(wallet_id) -> DeleteWalletReport`
   (`persister.rs:398`) already cascade-deletes purely by `wallet_id`, with a
   safe-by-default pre-delete backup, and has **no in-memory-manager dependency**.
2. **Add a manager-level purge that does not require in-memory presence** — the
   gap `PlatformWalletManager::remove_wallet` cannot fill:

   ```rust
   /// Evict a persisted wallet by id WITHOUT requiring it to be registered
   /// in memory. Unlike `remove_wallet`, succeeds for a row that `load()`
   /// skipped (e.g. `MissingManifest`) and therefore never entered the map.
   pub async fn purge_persisted_wallet(
       &self,
       wallet_id: &WalletId,
   ) -> Result<DeleteWalletReport, PlatformWalletError>;
   ```
3. **Keep the `wallet_id` in the skip outcome as a documented, stable handle** —
   it is already returned; just contract it, so consumers may rely on it to drive
   (2). Optionally answer the `task #14` product question in the same PR: eviction
   is the recovery path; no TTL needed.

**DET side.**

1. **Stop discarding the id** (`mod.rs:572`): carry the upstream `wallet_id` into
   `PersistedLoadSkip` / `LoadedWallets.skipped`, and fix the now-false doc on
   `loader.rs:45`.
2. **Route orphan eviction through the new purge**, not `remove_upstream_wallet`
   — and stop swallowing `WalletNotFound` as success (`mod.rs:984`) on that path.
   `remove_wallet` (`removal.rs:49`) should fall back to the skip-recorded
   `wallet_id` when `registered_wallet_id` yields `None`.
3. **Re-evaluate the banner after W2.** Re-run the skip check (or clear the
   banner) once cold-boot reconciliation has completed, so a self-healed wallet
   does not leave a permanent false warning.
4. **Make the advice executable.** Only two honest options for the residual
   unrecoverable case:
   - *If the row self-heals* → say so, calmly, and clear on completion. Ideally
     **show nothing at all** — a defect that repairs itself before the user can
     act on it does not warrant a persistent Warning banner.
   - *If it cannot* → the banner must offer the **action**, not a instruction the
     app will then refuse: a "Repair wallet" affordance that performs
     *evict-then-re-register* internally (the user already proved seed ownership;
     do not ask them to type it again). Per
     `docs/ai-design/2026-06-17-.../04-design-addendum.md` house pattern:
     remediation CTAs belong next to the problem, not in prose.

   Copy should never instruct a user toward a path the app hard-rejects.

## B.5 Before filing — reproduce

The severity swing between "unrecoverable" (as briefed) and "self-heals with a
stale banner" (as traced) is large enough that the issue must not be filed on
static reading alone. Minimum repro:

1. Register a wallet; force step 5 to leave an empty-manifest row (inject a
   failure between wallet-row creation and first account registration).
2. Restart. Assert: banner raised, `id_map` empty, wallet skipped.
3. Let cold boot complete (unprotected wallet → W2 runs prompt-free).
4. **Assert whether the row is repaired.** This single assertion decides the
   severity and the shape of the fix.
5. Separately, delete the meta sidecar and confirm the row is then immortal
   (B-1) — that path needs no W2 and should reproduce cleanly.

DET already has the harness for this: `src/context/wallet_lifecycle/tests.rs`
drives create → persist → real `load_from_persistor_seedless` → gate (`:599`,
`:783`, `:2562`), and `tests.rs:1551`/`:1772` already exercise the
"`id_map` stays empty and every seed-keyed operation degrades" shape.

---

## Open questions

1. **Does W2 actually repair the row?** (§B.5 step 4.) Everything downstream
   depends on this answer.
2. **Is `§Locked-#4` re-openable for provider keys?** Section A is dead without
   it. The decision text says V/O/P only, so this should be a clarification rather
   than a reversal — but it needs an explicit human "yes".
3. **Should the skipped-wallets banner exist at all** for a self-healing
   condition? Current recommendation: no — replace with a log line, and reserve
   the banner for the genuinely unrecoverable B-1 case, where it can carry a real
   action.
4. **Does #4113 alone unblock Section A**, or is a further upstream ask needed to
   *read back* provider-key accounts (not just persist them)? Unknown at DET's pin;
   verify after the bump.

## Assumptions

- Platform HEAD citations (`persister.rs`, `traits.rs`, `wallet_lifecycle.rs`)
  reflect post-#4072 upstream and are **not** what DET compiles against today.
  Re-verify after any pin bump.
- No code was executed for this document (design-only, no shell). Every dynamic
  claim — above all §B.2 — is static analysis and is flagged as such.

---

🍬 **Findings tally (UX / requirements)** — **5 confirmed**

| # | Severity | Finding |
|---|---|---|
| UX-1 | **Medium** | `load_form.rs:3`/`:172` over-generalize a V/O/P-scoped locked decision into a blanket "masternode keys never live in a wallet's HD tree", which would mislead a future implementer into discarding provider-key derivation entirely. |
| UX-2 | **Medium** | `LoadedWallets.skipped`'s `Option<WalletSeedHash>` is structurally always `None` (`mod.rs:572`); its doc comment (`loader.rs:45`) describes a matching step that does not exist. Dead field + false doc. |
| UX-3 | **High** | Skipped-wallets banner advice is **unexecutable**: following it hits `WalletAlreadyImported` (`registration.rs:116-121`). The user is told to do the one thing the app refuses. |
| UX-4 | **Medium** | The same banner is **stale after self-heal** — raised pre-W2, cleared only by a later load pass that may never run; a healthy wallet keeps a Warning banner all session. |
| UX-5 | **Medium** | Orphan eviction is a **guaranteed silent no-op**: `removal.rs:49` gates on an `id_map` the skip keeps empty, and upstream `remove_wallet` would return `WalletNotFound` anyway — which DET maps to `Ok(())` (`mod.rs:984`). |

Additionally, one **briefed claim not confirmed**: the
`WalletRegistrationXpubMismatch` closed loop (§B.2) — that branch is unreachable
for a skipped row. Recorded here so the correction is not lost.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
