# IDN — Identities

Environment: PR892 build (`/data/target/debug/dash-evo-tool` @ `57195d54`), isolated data dir
`/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`, wallet `QA Wallet 1`. App was
already running (PID 989399) when this pass started; reused per campaign instructions.

## Environment status at start of this pass (worse than DEV.md's snapshot — read before the
## individual story write-ups)

`CAMPAIGN-CONTEXT.md`'s known blocker was re-confirmed present (masternode-list/quorum-sync
failure blocking Platform proof verification for any query — same
`SdkError { source_error: Proof(ContextProviderError(Config("masternode list not yet synced
(quorums unavailable)"))) }` signature DEV.md documented). But this session additionally shows
the **wallet-storage-layer failure** `ALK.md`'s "App-restart failure" section flagged as a
live, unresolved risk: `det.log` at the start of this pass already contained repeated
```
WARN dash_evo_tool::backend_task: Wallet backend initialization deferred
  error=Could not access wallet data. Check available disk space and restart the application.
```
and Settings > Networks showed **four simultaneous red banners** — "SPV sync failed", "We
couldn't finish preparing your wallet. Try restarting the app.", "Your wallet is still starting
up.", and (new, Identities-specific) "Could not load your identities from this device." The
Wallets screen confirms this is not just a display glitch: `QA Wallet 1`'s balance renders as
**0 DASH** with "Sync Status: Core: Error, Addresses: never synced" — i.e., in this session the
wallet backend never wired at all, so even the local-DB-cached balance views that earlier
categories (WAL/SND/ALK) relied on as environment-independent do not render. Per campaign
instructions this was not re-diagnosed or restarted-and-retried beyond the single fresh-launch
check already performed by the main loop; all findings below are attributed to this
already-documented, dual-symptom blocker where applicable. Screenshot:
`screenshots/IDN-000-identities-empty-state-blocked-banners.png`.

Confirmed via direct SQLite inspection (`det-app.sqlite`) that this pass made **zero persistent
changes**: `identities` table has 0 rows before and after, `wallets`/`meta_wallet` unchanged —
consistent with every write-path attempted below being blocked before reaching persistence.

---

## IDN-001: Register a new identity — **BLOCKED** (wizard navigation/validation confirmed
## working; one independent UI bug found en route)

**Persona:** Alex, Priya, Jordan. Acceptance criteria: "Multi-stage confirmation flow. Identity
funded from an asset lock."

### Steps and observed result

1. Identities (empty state) > "Create my first identity" → `Identities > Create Identity`
   wizard. Step 1 ("Choose which wallet"): `QA Wallet 1` correctly pre-selected, shown as
   "QA Wallet 1 — 0 DASH" (the 0 DASH reflects the wallet-not-wired state above, not the
   wallet's real balance).
2. Step 2 ("Choose your funding method") dropdown: two options — **"Recover an unfinished
   funding"** and **"Receive a new deposit"**. Selected "Receive a new deposit" → step 3
   appeared titled "Deposit received. Choose how much to use, then continue." but rendered
   **no content at all** (no address, no QR, no amount field, no error) — likely because
   generating a fresh deposit address needs a wired wallet backend, but the screen gives no
   explicit "can't generate address right now" message the way other blocked flows do (see
   below). Minor UX gap, not re-tested in a healthy environment to confirm it's blocker-specific.
3. Selected "Recover an unfinished funding" instead → step 5 showed **"Couldn't load your
   unfinished funding."** with a **"Retry"** button — a clean, well-typed empty/error state.
   Clicked Retry twice; same message reproduced consistently, no crash, no hang.
   Screenshot: `screenshots/IDN-001-2-recover-unfinished-funding-couldnt-load-retry.png`.
4. Tested "Show Advanced Options" (step numbering shifts to 5 steps: wallet, identity index,
   key selection, funding method, funding step). Identity Index field defaults to `0` as
   recommended. Key Selection Mode dropdown offers **Default (Recommended)** / **Advanced**.
   Screenshot: `screenshots/IDN-001-1-create-identity-wizard-advanced-key-mode.png`.

### Bug found: "+ Add Key" button in Advanced key-selection mode is a no-op

Switched Key Selection Mode to "Advanced" — a "+ Add Key" button appeared. Clicked it twice;
**no new key row ever appeared**, no banner, no log line. Traced via source: `add_new_identity_
screen/mod.rs`'s `add_identity_key()` has a silent early return (`let Ok(backend) =
self.app_context.wallet_backend() else { return; }`) that fires whenever the wallet backend
isn't wired — guaranteed in this environment. Source review also surfaced a second, deeper
issue independent of this session's environment problem: even with a wired backend, the first
click misses the identity-key cache (the default 5 keys are pre-warmed, but "+Add Key" always
requests the 6th), and the async warm-completion handler (`ensure_correct_identity_keys()`)
unconditionally rebuilds the visible key list from a **fixed 5-entry default set**, discarding
the newly-requested key rather than appending it — so the first click is a functional no-op
even outside this campaign's degraded environment. Worth a real ticket; not fixed here per
campaign rules (observe/document only).

**Verdict: BLOCKED** — reasoning: "blocked by known environment issue: Testnet masternode-list/
quorum-sync failure prevents Platform proof verification, see CAMPAIGN-CONTEXT.md /
scenarios/ALK.md and scenarios/DEV.md for full diagnosis" (compounded this session by the
wallet-storage-layer failure documented above, which prevents the wizard from ever reaching a
fundable state at all). Wizard navigation, step sequencing, and the "Recover an unfinished
funding" empty-state error are all confirmed working correctly. The Advanced-mode "+ Add Key"
no-op is a real, independently-reproducible defect (see above) — flagged for product
awareness, not counted against the BLOCKED verdict since it's a secondary/advanced path, not
the story's core acceptance criteria.

---

## IDN-002: Load existing identity by ID — **FAIL** (silent hang on the exact acceptance-
## criteria flow; two alternate lookup methods on the same screen degrade gracefully)

**Persona:** Priya, Jordan. Acceptance criteria: "Enter identity ID and private key. Identity
details are fetched and displayed."

No known-real testnet identity ID fixture was found (checked `memcan:recall` for project
`dash-evo-tool` and the `dash-platform` skill's docs — no identity ID fixture, only the
well-known **DPNS contract ID** `GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec`, which is a valid
32-byte Base58 `Identifier` format but points to a contract, not an identity). Used it as a
syntactically-valid input to exercise the load flow's behavior — since proof verification is
broken for every kind of Platform query in this environment, the exact ID's real-world validity
does not change the failure mode being tested here.

`Identities > "I already have an identity — load it"` → `Load Existing Identity` screen, three
tabs: **"Identity ID & private key"** (the story's exact flow), **"From my wallet"**, **"My
username"**.

### "Identity ID & private key" tab — silent hang, zero feedback

1. Typed the DPNS contract ID into the "Identity ID" field (no private key — the field is
   present but the "Load Identity" button enables on ID format alone). Button turned solid
   blue (enabled).
2. Clicked "Load Identity". **Nothing happened**: no banner (info or error), no navigation, no
   new line in `det.log` — reproduced 3 times, including one immediate re-screenshot right
   after the click to rule out a flash-and-vanish. Screenshot:
   `screenshots/IDN-002-1-load-identity-by-id-silent-noop.png`.
3. Traced via source: the click handler (`add_existing_identity_screen.rs`) sets an info
   banner ("Loading identity...") **synchronously, before** dispatching the async backend
   task — so a banner should always appear immediately regardless of what happens next. Its
   absence points to `AppContext::run_backend_task`'s generic `ensure_wallet_backend` pre-check
   (used by this screen, unlike the direct `wallet_backend()` calls other screens use) hanging
   indefinitely inside the wallet-backend construction/lock rather than erroring — so the task
   never reaches the point where it would send a `TaskResult` back to the UI at all.

### "From my wallet" tab — works correctly, clean typed error

Selected `QA Wallet 1`, clicked "Search Wallet for Identities". This **dispatched correctly**:
`det.log` shows real gap-limited key-derivation attempts (indices 0–11) and a clean completion
with a proper typed error banner: **"No identities found up to wallet index 5. Try a higher
search range."** (`NoWalletIdentitiesFound { max_index: 5 }`). Screenshot:
`screenshots/IDN-002-2-search-wallet-for-identities-clean-typed-error.png`. This confirms the
silent hang above is not simply "everything on this screen is equally broken" — a sibling
button on the identical screen, under the identical environment condition, completes and
reports cleanly.

### "My username" tab — doubles as IDN-010, degrades gracefully (see IDN-010 below)

**Verdict: FAIL** for the story's stated acceptance-criteria flow ("Enter identity ID and
private key") — it hangs with **zero** user-facing feedback, which is a strictly worse failure
mode than the clean typed/generic errors every other blocked flow in this campaign shows
(including the other two tabs on this exact screen). This is flagged as an independent defect,
not purely environment fallout, precisely because sibling code paths on the same screen, in the
same session, degrade correctly. Should be re-tested once the environment blocker is resolved
to see if the hang persists on a healthy backend — if it does, it's a P1 (a legitimate ID+key
pair would leave a user staring at a frozen button forever with no explanation).

---

## IDN-003: Load evonode/masternode identity — reclassified `[Superseded by MN-001]` in the
## corrected catalog

**Reconciliation note**: PR892's real catalog (`docs/user-stories.md` in the PR892-build
worktree) tags this story `[Superseded by MN-001]`, not `[Implemented]` — the new MN
category's MN-001 ("Load a masternode by keys") now owns this capability. `progress.md`
tracks IDN-003 as N/A accordingly. The FAIL finding below (same silent-hang defect class as
IDN-002, on the exact "Load a masternode" flow this story describes) is kept as directly
relevant context for whoever tests MN-001 — the underlying screen and bug are the same one
MN-001 will exercise.

## IDN-003 (original write-up, kept as context for MN-001): Load evonode/masternode identity — **FAIL** (same silent-hang defect class as
## IDN-002; format validation and node-type toggle confirmed working)

**Persona:** Priya. Acceptance criteria: "Enter protx hash to load the associated identity."

No masternode/evonode identity fixture was found via `memcan:recall` (consistent with
`DEV.md`'s finding of no `.testnet_nodes.yml` fixture in this environment; real registration
needs ~1000 tDASH collateral this environment doesn't have). Masternodes screen: "No
masternodes loaded" empty state, "Load a masternode" button — matches `DEV-006`'s prior
screenshot.

### Steps and observed result

1. "Load a masternode" → form with **Masternode / Evonode** node-type toggle (clicked
   "Evonode", switched cleanly), ProTxHash field, optional Alias, and three optional private
   key fields (Voting/Owner/Payout).
2. Typed `not-a-valid-protxhash` into ProTxHash, clicked "Load masternode". Got a clean inline
   validation error: **"This doesn't look like a valid ProTxHash. Enter a hex or Base58
   ProTxHash from your masternode configuration."** — no crash, precise and actionable.
   Screenshot: `screenshots/IDN-003-1-load-masternode-protxhash-validation-then-silent-noop.png`
   (taken after the subsequent step below; the validation-error state was confirmed visually
   before proceeding).
3. Replaced with a syntactically-plausible 68-hex-character string (passed format validation —
   no error shown). Clicked "Load masternode". **Same silent-hang signature as IDN-002**: no
   banner, no navigation, no new log line, reproduced across waits of 2s/5s/15s after the
   click.

**Verdict: FAIL** for the same reasoning as IDN-002 — the ProTxHash format validation and the
Masternode/Evonode node-type toggle both work correctly (clean, actionable feedback), but the
actual "Load masternode" submission hangs with zero user feedback once given a well-formed
input. Per source review conducted for IDN-002, this screen's load button funnels through the
same `IdentityTask::LoadIdentity` task as the ID+key tab (with a `reconcile_pending_load`
backstop specifically built for this early-return scenario) — but no backstop resolution was
observed even after a 15-second wait, so either the backstop itself isn't firing in this
degraded environment or the underlying task truly never completes. Re-test once the environment
blocker is resolved.

---

## IDN-004 through IDN-009, IDN-013: Identity-detail operations — **BLOCKED** (no identity
## reachable to operate on)

**Stories:** IDN-004 (Top up identity credits), IDN-005 (Withdraw credits to Core address),
IDN-006 (Transfer credits between identities), IDN-007 (Add key to identity), IDN-008 (View
identity keys and details), IDN-009 (Refresh identity state), IDN-013 (Top up identity from
Platform addresses).

All of these are reached from an identity's detail screen (Identities > select a loaded
identity > …), which does not exist as a navigation target when zero identities are loaded
locally — confirmed via direct SQLite check (`identities` table: 0 rows, matching the
Identities screen's persistent "Welcome to Identities" empty state throughout this pass). Since
IDN-001 (register), IDN-002 (load by ID), and IDN-003 (load by ProTxHash) — the only three ways
to populate that list — all failed to produce a loaded identity in this environment (two via
silent hang, one via the environment blocker), there is no UI surface for IDN-004–009/013 to
exercise beyond what IDN-012's source review already established structurally (see below).

**Verdict for all seven: BLOCKED** — reasoning: "blocked by known environment issue: Testnet
masternode-list/quorum-sync failure prevents Platform proof verification, see
CAMPAIGN-CONTEXT.md / scenarios/ALK.md and scenarios/DEV.md for full diagnosis" (transitively,
via IDN-001/002/003's inability to produce a loaded identity to act on). Not independently
re-tested; no additional UI surface exists to test without an identity.

---

## IDN-010: Search identity by DPNS name — **BLOCKED** (dispatches and fails cleanly — same
## masternode-list-sync signature as DEV.md)

**Persona:** Alex, Priya. Acceptance criteria: "Enter username and retrieve associated
identity."

This is the `Identities > "I already have an identity — load it" > "My username"` tab (no
separate search screen exists elsewhere). Entered `alice` (the field's own placeholder example:
"Enter 'alice' to look up 'alice.dash'"), clicked "Search by Username".

`det.log` shows a real dispatch: 7 retries against 7 different DAPI endpoints, each failing with
the now-familiar `SdkError { source_error: Proof(ContextProviderError(Config("masternode list
not yet synced (quorums unavailable)"))) }`, then a clean (if generic) banner: **"An unexpected
error occurred. Please try again later."** with the technical detail available via "Show
details". Screenshot: `screenshots/IDN-010-1-search-by-username-FAIL-quorums-unavailable.png`.

**Verdict: BLOCKED** — reasoning: "blocked by known environment issue: Testnet masternode-list/
quorum-sync failure prevents Platform proof verification, see CAMPAIGN-CONTEXT.md /
scenarios/ALK.md and scenarios/DEV.md for full diagnosis." Unlike IDN-002/003's ID/ProTxHash
load buttons, this one **does** dispatch and fail gracefully with visible retry activity and a
banner — reinforcing that the ID+key and ProTxHash load buttons' silent hangs are a distinct,
narrower defect rather than "this whole category is universally broken the same way."

---

## IDN-012: Register identity from Platform addresses — **BLOCKED** (confirmed implemented and
## correctly gated in source; unreachable because the live balance cache never populates)

**Persona:** Priya, Jordan. Acceptance criteria: "Alternative funding method in identity
registration wizard. Uses existing Platform address balance."

The Create Identity wizard's funding-method dropdown only offered "Recover an unfinished
funding" / "Receive a new deposit" — no "Use a Platform address" option, despite `ALK.md`
documenting a real, non-zero Platform balance (0.01985204 DASH) on this same wallet's DIP-17
address earlier in the campaign.

Source review (`src/ui/identities/funding_common.rs`, `add_new_identity_screen/mod.rs`)
confirms this is **implemented, not a gap**: `FundingMethod::UsePlatformAddress` exists and is
gated behind `wallet.platform_address_info.values().any(|info| info.balance > 0)` — an
in-memory cache populated by a periodic (~15s) push from the wallet-backend coordinator
(`wallet_backend/event_bridge.rs` → `AppContext::apply_platform_address_push`), not fetched
on-demand by the screen itself. Because the wallet backend never wired in this session (see
environment status above), that cache stays empty regardless of what balance is actually
persisted in SQLite, so the option is correctly absent rather than shown-and-broken.

**Verdict: BLOCKED** — reasoning: "blocked by known environment issue: Testnet
masternode-list/quorum-sync failure prevents Platform proof verification, see
CAMPAIGN-CONTEXT.md / scenarios/ALK.md and scenarios/DEV.md for full diagnosis" (specifically,
the compounding wallet-backend-not-wired symptom prevents the live Platform-balance cache this
feature depends on from ever populating). Feature confirmed implemented via source, correctly
gated, not reachable for a live UI exercise in this environment. Worth a follow-up pass once the
environment recovers.

---

## IDN-013a: Password-protect an identity's signing keys (SEC-001) — **BLOCKED** for live UI
## (no identity reachable); source review confirms the feature is fully implemented

**Persona:** Priya, Jordan. Acceptance criteria: see `docs/user-stories.md` — the story between
IDN-008 and IDN-009 in PR892's catalog (disambiguated as `IDN-013a` in this campaign's
`progress.md` because of the genuine duplicate-ID defect noted at the top of this file).

**Disambiguation reminder:** this is a *different* story from `IDN-013b` ("Top up identity from
Platform addresses", already tested — see the "IDN-004 through IDN-009, IDN-013" section above,
carried into `progress.md` as `IDN-013b`, BLOCKED, no identity reachable). Not re-tested here.

### Why this is BLOCKED for live UI

The entire "Key Protection" section lives on an identity's Key Info screen
(`src/ui/identities/keys/key_info_screen.rs`), reachable only via `Identities > <a loaded
identity> > Keys`. As established exhaustively above (IDN-001/002/003), this data dir has **zero
loaded identities** — confirmed again via direct SQLite check at the start of this session
(`identities` table: 0 rows) and via the live environment-blocker banners still present
(`WalletBackendNotYetWired` — see IDN-014 below for a fresh live re-confirmation this session).
There is no navigation path to the Key Info screen without a loaded identity, so the "Key
Protection" section itself could not be exercised live.

**Verdict: BLOCKED** — reasoning: "no identity reachable, see scenarios/IDN.md" (same root
cause as IDN-001 through IDN-013b: the Testnet wallet-backend/masternode-list-sync environment
blocker, `ALK.md`/`DEV.md`, has prevented every registration/load path from producing a loaded
identity in this data dir across this entire campaign).

### Read-only source review (no edits made) — confirms the feature is implemented as specified

Per the task's own guidance (and this project's `CLAUDE.md`, which references
`IdentityTask::ProtectIdentityKeys` as an already-shipped part of the secret-storage seam), a
source review was done to confirm the feature exists and matches every acceptance-criteria
bullet. It does, in detail:

1. **"Applies only to identities with vault-stored keys… hidden entirely for HD-backed
   identities"** — `key_info_screen.rs::compute_protection_status()` (line ~1057) looks up each
   key's `SecretScheme` via `IdentityKeyView`; HD-wallet-derived keys have no vault entry at all
   (they resolve via the wallet's own seed), so they fall into the `_ => {}` arm and never
   contribute to `protected`/`unprotected` counts. With both counts at 0 the status is
   `NoVaultKeys`, and `render_key_protection_section()` (line 1083) returns immediately without
   drawing anything — the section is structurally absent, not just disabled, for such
   identities.
2. **"Identity keys default to keyless… headless/MCP signing keeps working"** —
   `wallet_backend/secret_access.rs` line 683 and 725 explicitly document new keys are sealed
   "unprotected (prompt-free → headless/MCP signing works)" by default.
3. **"Collapsible 'Key Protection' section (closed by default)… 'Add password protection…' /
   'Remove password protection…'"** — `egui::CollapsingHeader::new("Key Protection")
   .default_open(false)` (line 1096); `render_protection_idle()` (line 1131) picks the button
   label from the current `IdentityProtectionStatus` — `"Add password protection…"` when
   Unprotected/Mixed(finish), `"Remove password protection…"` when Protected.
4. **"Opting in shows a danger warning… forgotten password unrecoverable… automatic tools can no
   longer sign… then new password + confirmation + optional hint"** —
   `open_add_confirm()` (line 1154) builds a `ConfirmationDialog` in `danger_mode(true)` with
   exactly this wording verbatim: *"If you forget the password, these keys cannot be recovered.
   There is no reset option."* and *"Automatic tools (such as scripts or the command-line
   interface) will no longer be able to sign with this identity without the password."* On
   confirm, `render_new_password_form()` (line 1216) collects new password, confirmation
   (validated via `validate_single_key_passphrase`), a live zxcvbn strength bar, and an optional
   plain-text hint field labelled *"visible in plain text. Do not use the password itself as a
   hint."*
5. **"Once protected, every signing operation asks for the password just-in-time, with an
   optional 'keep unlocked until I close the app'. A wrong password re-asks with no oracle."** —
   `ui/components/secret_prompt_host.rs` line 132 defines the per-scope checkbox label for
   `SecretScope::IdentityKey`: *"Keep this key unlocked until I close the app."*
   `wallet_backend/secret_access.rs` has dedicated tests (`ScriptedAnswer::remember(...,
   RememberPolicy::UntilAppClose)`, lines ~2141-2142, 2669-2670) exercising exactly this
   just-in-time + remember-until-close flow, plus fail-closed tests proving a locked protected
   key never leaks via a keyless read (line 2003: *"a password-free read of a protected identity
   key must fail"*) and that the background sweep skips a locked protected identity rather than
   prompting (line 2015).
6. **"Headless/MCP signing of a protected identity fails with a calm, actionable message… no
   env-var/flag fallback"** — `backend_task/identity/mod.rs` line 479's doc comment states
   plainly: *"headless/MCP signing yields `SecretPromptUnavailable`."* That variant
   (`backend_task/error.rs` line 1987) is fieldless by design ("never any secret") with
   `Display`: *"This wallet is protected by a passphrase, which can only be entered in the app
   window. Open Dash Evo Tool and run this action there."* — calm, actionable, no technical
   jargon, matching the project's error-message conventions. (Minor observation, not a defect:
   this shared variant's wording says "wallet" rather than "identity"; it is reused verbatim for
   both wallet-passphrase and identity-key-protection headless failures. Worth a follow-up to
   confirm the copy reads correctly in the identity context, but functionally it correctly
   blocks headless signing with an actionable message and no password fallback — the story's
   actual requirement.)
7. **"Opting out asks for the current password and reverts keys to keyless; signing is
   prompt-free again, including headless."** — `open_remove_confirm()` (line 1176) +
   `render_verify_password_form()` (line 1260) collect the current password;
   `IdentityTask::UnprotectIdentityKeys` (backend_task/identity/mod.rs line ~488) is documented
   as verifying the password before "revert[ing] every password-protected (Tier-2) vault-stored
   key of this identity back to keyless (Tier-1)... idempotent... crash-safe;" after which
   "signing is prompt-free again, including headless/MCP" (doc comment, verbatim).
8. **"One password protects all of an identity's keys; separate from wallet password;
   Argon2id + XChaCha20-Poly1305, no new crypto, no plaintext on disk."** —
   `ProtectIdentityKeys { identity_id, password, hint }` (backend_task/identity/mod.rs ~476)
   seals every keyless vault key of the identity under **one** per-identity object password; the
   doc comment for the variant explicitly says this reuses the shipped Tier-2 seam. This matches
   `CLAUDE.md`'s own description of the `put_secret_protected`/`get_secret_protected` chokepoint
   (Argon2id + XChaCha20-Poly1305) — no separate/new crypto path was found in this review.

`backend_task/identity/protect_identity_keys.rs` additionally carries a substantial unit-test
suite (idempotency on an already-protected identity, the `IdentityKeysProtected{count:0}`
false-positive regression guard, crash-safety ordering) — further evidence this is a mature,
already-shipped feature rather than a stub.

**Conclusion:** live UI testing is correctly BLOCKED by the same "no identity reachable"
condition that has blocked every identity-detail story in this campaign. The source review found
**no gaps** — every acceptance-criteria bullet has a corresponding, specifically-worded
implementation, matching this project's `CLAUDE.md` claim that `ProtectIdentityKeys` is
already-shipped. No PR892 source was modified during this review.

---

## IDN-014: Fund identity by receiving a deposit to a shown QR/address — **FAIL** (step 3/2
## still renders zero content; re-verified fresh this session, root cause confirmed live)

**Persona:** Priya, Jordan. Acceptance criteria: "Choosing 'Receive a new deposit' shows a
scannable deposit address (QR + copyable text) and the minimum amount to send. Once enough
arrives the amount field pre-fills… I can switch funding methods at any time… A build/broadcast
failure leaves my deposit safe in the wallet…"

This story is directly reachable without any pre-existing identity — it is part of the Create
Identity wizard's funding-method step, exercised fresh this session (not reused from IDN-001's
notes) per the task's instruction to re-verify.

### Steps and observed result

1. `Identities` (empty state, same 4 red environment banners as documented above —
   `WalletBackendNotYetWired` still present this session) > "Create my first identity" >
   `Identities > Create Identity`. This build's wizard shows only one wallet (`QA Wallet 1`), so
   step 1 is "Choose your funding method" directly (no separate wallet-selection step, unlike
   IDN-001's write-up from an earlier pass — minor wizard-numbering difference, not a defect).
2. "Select how to fund" dropdown offers **"Recover an unfinished funding"** and **"Receive a new
   deposit"** (identical to IDN-001's finding). Selected **"Receive a new deposit"**.
3. Step "2. Deposit received. Choose how much to use, then continue." appeared — and rendered
   **zero content**: no address, no QR code, no copyable text, no minimum-amount field, no error
   or loading message of any kind. Reproduced after a 5-second wait (ruling out an async
   loading delay) and after a full-page scroll-down (ruling out off-screen content). Screenshot:
   `screenshots/IDN-014-1-receive-new-deposit-step2-blank.png`.
4. Confirmed the dropdown itself is not a dead end: re-opening "Select how to fund" while on
   this blank step still lists all three options (including switching back to "Recover an
   unfinished funding"), consistent with the "I can switch funding methods at any time" bullet —
   the *navigation* isn't broken, only the deposit-address content on this specific step.
5. Correlated with `det.log`: my "Receive a new deposit" selection immediately produced
   ```
   23:11:11 WARN dash_evo_tool::backend_task: Wallet backend initialization deferred
     error=Could not access wallet data. Check available disk space and restart the application.
   ```
   (repeated again at 23:11:31) — i.e. the screen silently attempted to generate a fresh deposit
   address, the attempt failed because the wallet backend is not wired in this session, and the
   failure was swallowed with **no user-facing feedback whatsoever** (contrast with "Recover an
   unfinished funding," which shows a clean "Couldn't load your unfinished funding." + Retry
   button for the same underlying cause, per IDN-001's write-up).

### Verdict

**FAIL** — not BLOCKED. Per the task's explicit guidance: this flow is directly reachable
without a pre-existing identity, so "no identity reachable" is not the applicable reasoning here.
The acceptance criteria's very first bullet — "shows a scannable deposit address (QR +
copyable text) and the minimum amount to send" — is unmet: the step renders nothing at all. This
reproduces IDN-001's earlier finding on the exact same build/session family, confirming it is
not a one-off flake. Root cause (from `det.log` correlation) is the same wallet-backend-not-wired
environment condition documented throughout this campaign, but unlike most other blocked flows
in this campaign (which degrade to a clean typed/generic error), this one degrades to **total
silence** — the same "swallowed failure, zero feedback" defect class already flagged as a
cross-cutting UX gap in IDN-002/IDN-003 (silent-hang buttons) and now confirmed here on a screen
render rather than a button click. Worth flagging as a P2 UX defect independent of the underlying
environment issue: even a healthy backend user hitting a transient address-generation error would
see nothing.

---

## IDN-015: Automatic identity discovery after sync — **PASS** for the auto-trigger mechanism
## (live `det.log` evidence from this exact running session), supplemented by source review for
## sub-behaviors not observable live (nothing to discover in this wallet)

**Persona:** Alex, Priya. Acceptance criteria: "After the network is ready, every unlocked
wallet is searched automatically once per session. The search uses a rolling five-index
lookahead, going deeper each time an identity is found... Already-loaded identities are
refreshed... while any alias the user assigned is preserved. Locked, password-protected wallets
are skipped without prompting."

### Method chosen: det.log evidence from the current running process (no restart performed)

A restart was judged **not safe/practical**: at the time of testing, the live environment
blocker was actively present (4 red banners, `WalletBackendNotYetWired` — same screenshot
context as IDN-014 above), and the task's own guidance is to restart "only if... a clean restart
seems low-risk." Restarting into a session that's already mid-blocker risks losing the very
log evidence needed and does not meaningfully improve on evidence already available: `det.log`
for **this exact currently-running process** (PID confirmed via `pgrep`, hash-verified binary)
already contains a full, successful automatic-discovery run from earlier in its own uptime —
before the backend later regressed into the `WalletBackendNotYetWired` state documented
elsewhere in this campaign. This is live evidence, not a stale log from a previous process.

### Live log evidence

```
22:24:28.643023  Masternode list synced; starting Platform sync coordinators
22:24:28.643171  SyncEvent: SyncComplete(tip=2504940, cycle=0)
22:24:28.643224  Starting automatic identity discovery for all open wallets wallet_count=1
22:24:28.643354  Starting gap-limited identity discovery for wallet
                  seed=0523... seed_window=None allow_prompt=false
22:24:57.223249  Gap-limited identity discovery complete
                  seed=0523... found=0 stored=0
```

This confirms, live and unambiguously:
- The scan fires automatically, immediately on Platform readiness (masternode list `Synced` →
  `SyncComplete` event), with no user action required — matching "after the network is ready...
  automatically."
- It covers "every open wallet" (`wallet_count=1`, matching this data dir's single wallet, `QA
  Wallet 1`) in one sweep.
- `allow_prompt=false` on this automatic sweep — matching "skipped without prompting" for locked
  wallets (this data dir's one wallet is unprotected/unlocked, so the skip path itself wasn't
  exercised, but the flag confirms the code path is wired for it — see source review below).
- It ran **exactly once** in this log (grepped the full 2582-line file for repeat
  "Starting automatic identity discovery" lines — only one match) — matching "once per session,"
  even though the app has remained running and the banner-flapping condition has recurred
  multiple times since.
- `found=0 stored=0` is the expected, correct result for `QA Wallet 1` — consistent with every
  other story in this campaign (IDN-002's "From my wallet" tab, IDN-012) independently confirming
  this wallet holds no on-chain identities up to at least index 5.

### Source review — confirms the specific mechanics not observable live

Since no identity was ever found in this wallet, the "rolling window that goes deeper" and
"alias preserved on refresh" bullets could not be observed in action live. Read-only source
review (no edits) confirms both are implemented exactly as specified:

- **Once-per-session latch**: `context/wallet_lifecycle/bootstrap.rs::queue_all_wallets_identity_
  discovery()` gates on a single `AtomicBool` (`identity_autodiscovery_fired`), swapped to `true`
  on first fire; cleared only by `stop_spv()` on reconnect. Fired specifically "when Platform
  becomes reachable (masternode list `Synced`)" — exactly the trigger observed live above.
- **Locked wallets skipped without prompting**: the sweep snapshots only `self.open_wallets()`
  ("a locked protected wallet hydrates closed... and is skipped so the background sweep cannot
  trigger a passphrase prompt") and additionally passes `allow_prompt=false` into
  `discover_identities_gap_limited`, which (per `discover_identities.rs` line ~108-116) treats a
  locked-wallet auth-key derivation failure (`TaskError::AuthKeyUnlockRequired`) as "skip the
  whole wallet" rather than prompting. A wallet unlocked later is separately covered by
  `queue_unlocked_wallet_identity_discovery()`, gated on Platform already being ready, with
  `allow_prompt=true` (safe because the user is present for the unlock).
- **Rolling five-index lookahead**: `model/identity_discovery.rs` defines
  `IDENTITY_GAP_LIMIT: u32 = 5` and `should_continue_scan(current_index, highest_found)`:
  with no hits yet, probes `0..5`; each new hit at index `h` extends the window to
  `..= h + 5`, so "each new discovery extends the window" exactly as the acceptance criteria
  describes. Unit tests in the same file directly assert this (`for i in 0..IDENTITY_GAP_LIMIT {
  assert!(should_continue_scan(i, None)) }`, etc.). A `IDENTITY_SCAN_HARD_CAP = 100` bounds
  worst-case fan-out.
- **Alias preserved on refresh**: `discover_identities.rs::upsert_discovered_identity()` — when
  an identity is already known (`Some(existing)`), it explicitly carries `qualified_identity.
  alias = existing.alias` onto the freshly-fetched identity before persisting via
  `update_local_qualified_identity()`, matching "any alias the user assigned is preserved."

### Verdict

**PASS**, based on a hybrid of live evidence (the auto-trigger firing correctly, once, on
Platform readiness, covering the one open wallet, with the no-prompt flag correctly set) and
source review (confirming the rolling-five-index and alias-preservation mechanics that this data
dir's zero-identity wallet cannot exercise live). This is a genuinely stronger evidence basis
than most other BLOCKED stories in this campaign, because the core automatic-trigger behavior
*did* run to completion, successfully, inside the live process — it is not purely inferred from
source. No restart was performed (judged unsafe given the live environment blocker); no PR892
source was modified.

---

## IDN-016: Identities and their keys preserved across an app upgrade — **BLOCKED** (no
## pre-upgrade legacy fixture exists; out of scope to fabricate one), supplemented by a
## read-only source review as supporting context

**Persona:** Alex, Priya. Acceptance criteria: identities/keys/alias/wallet-link carried across
an upgrade via first-launch import; an unreadable identity reported in a banner (not dropped
silently) without blocking readable identities, wallet migration, or scheduled-vote import;
the unreadable-identities report persists until acknowledged; a combined banner when both
identities and scheduled votes are unreadable; deletions after upgrade stay deleted (import runs
once).

### Why this is BLOCKED

This story exercises a **first-launch-after-upgrade migration path**: it needs a genuine
pre-upgrade, old-format identity store (written by a version of the app *before* this migration
code existed) to import from. This QA data dir (`/data/tmp/det-qa-pr892-data`) was created fresh
directly against the PR892 build — there is no prior-version data to migrate, so the "first
launch after an upgrade" precondition cannot occur here. Building such a fixture would require
running an older app version first to produce legacy-format storage, which is out of scope for
this QA pass (and explicitly out of scope per this task's own instructions, which also
prohibit fabricating or corrupting data to simulate this).

**Verdict: BLOCKED** — reasoning: "no pre-upgrade legacy identity-storage fixture exists to
exercise this migration path; would require running a prior app version first, out of scope for
this QA pass."

### Read-only source review (optional, done as supporting context; no edits made)

`src/backend_task/migration/v093_upgrade.rs` implements a v0.9.3→current upgrade path with a
substantial dedicated unit-test suite that specifically exercises this story's edge cases:

- `a_second_launch_after_an_unreadable_identity_preserves_user_edits_and_deletions` (line ~1401)
  — a test whose name alone confirms the "deletions stay deleted, import runs once" bullet is
  implemented and tested.
- `src/context/migration_status.rs` defines a `MigrationState` enum with **separate** variants
  `SucceededWithUnreadableIdentities { count }` and `SucceededWithUnreadableVotes { count }`, plus
  a **combined** `SucceededWithUnreadableIdentitiesAndVotes { identities, votes }` variant whose
  doc comment states verbatim: *"a single banner names both problems, its single acknowledge..."*
  — i.e. the exact "one banner names both remedies... neither report can bury the other" bullet.
  Doc comments also confirm the report is "durable and re-published on every launch until
  acknowledged."
- `src/database/legacy_import.rs` line 77's doc comment: unreadable identities are recorded "as a
  durable warning and [left] in the legacy file — never deleted" — matching "the previous
  version's data is never deleted, so a later build can still import it."
- Test fixtures in `v093_upgrade.rs` construct a real v0.9.3-shaped SQLite schema (including a
  `scheduled_votes` table, line 378) and deliberately poison one identity/vote row to test the
  "one bad row doesn't block the rest" bullet (e.g. line ~1330's comment: "The app-data pass
  (scheduled votes, top-up history) can fail hard — one malformed blob is enough. That failure is
  [contained]").

This is consistent with the task's framing that this feature is expected to already be
implemented — the source review found a mature, test-covered migration path, not a stub. This is
supporting context only; **no live UI exercise was possible or attempted**, consistent with the
BLOCKED verdict above.

---

## Summary

| Story | Verdict |
|---|---|
| IDN-001 | BLOCKED (wizard/validation work; independent "+Add Key" no-op bug found) |
| IDN-002 | FAIL (ID+key load button silently hangs; other two tabs on same screen degrade cleanly) |
| IDN-003 | FAIL (same silent-hang defect; ProTxHash validation + node-type toggle both PASS) |
| IDN-004 | BLOCKED (no identity reachable) |
| IDN-005 | BLOCKED (no identity reachable) |
| IDN-006 | BLOCKED (no identity reachable) |
| IDN-007 | BLOCKED (no identity reachable) |
| IDN-008 | BLOCKED (no identity reachable) |
| IDN-009 | BLOCKED (no identity reachable) |
| IDN-010 | BLOCKED (dispatches + fails cleanly on known masternode-list-sync error) |
| IDN-011 | N/A (Gap, not implemented — pre-existing) |
| IDN-012 | BLOCKED (confirmed implemented + correctly gated in source; live cache never populates) |
| IDN-013a | BLOCKED for live UI (no identity reachable); source review confirms full implementation |
| IDN-013b | BLOCKED (no identity reachable) |
| IDN-014 | FAIL (step 2's deposit address/QR renders zero content; directly reachable, not identity-gated) |
| IDN-015 | PASS (live log confirms once-per-session auto-trigger; source confirms 5-index rolling window + alias preservation) |
| IDN-016 | BLOCKED (no pre-upgrade legacy fixture exists; source review confirms mature, tested implementation) |

Two genuinely new, environment-independent-looking findings from the original pass: **IDN-002**
and **IDN-003** both hang completely silently (no banner, no log line, no timeout) on their core
acceptance-criteria action, despite the source code setting a "Loading..." banner synchronously
before dispatch — this is a materially worse failure mode than every other blocked flow tested in
this pass (all of which show a retry trail and/or a clean typed/generic error banner). The
**"+Add Key"** no-op in the Create-Identity wizard's Advanced mode is a second, narrower defect
confirmed via source to be a real bug (cache-miss + unconditional-rebuild race) independent of
this environment's issues. Everything else in this category traces cleanly back to the two
already-documented environment failures (`ALK.md`'s wallet-storage-open failure and `DEV.md`'s
masternode-list/quorum-sync failure), both of which were present and worse than DEV.md's
snapshot at the start of this pass (see environment status section above).

This second pass (IDN-013a, 014, 015, 016) adds one more environment-independent finding in the
same "silent failure" class: **IDN-014**'s deposit-address step renders nothing at all on
failure, with zero user feedback, mirroring IDN-002/003's silent-hang pattern but on a screen
render rather than a button click. It also adds the campaign's **strongest positive live result**
in this category: **IDN-015**'s automatic identity-discovery trigger was directly observed
completing successfully inside the live running process (not merely inferred from source),
because that particular subsystem's readiness gate (masternode-list sync) was transiently
satisfied earlier in this session before the wallet-backend-wiring regression resurfaced.
IDN-013a and IDN-016 remain BLOCKED for live UI for the same structural reasons as the rest of
this category, but both received read-only source reviews finding mature, thoroughly-tested
implementations consistent with `CLAUDE.md`'s own description of these features as
already-shipped.

No PR892 application source was modified in either pass. QA Wallet 1 and the DIAG throwaway
wallet were left untouched. This second pass made no wallet/identity/database writes: the
"Receive a new deposit" attempt (IDN-014) failed before any persistence, and IDN-013a/015/016
involved no live mutating actions (013a and 016 never advanced past navigation/log inspection;
015 was pure log/source observation).
