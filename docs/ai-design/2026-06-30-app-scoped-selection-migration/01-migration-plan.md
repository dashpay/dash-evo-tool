# App-scoped selected-identity screen migration (W2–W5)

Status: PLAN — implementation contract for Bilby. Author: Nagatha.
Tracking: #842 (app-scoped selected identity). Builds on IDH-003 W0/W1
(merged at `cc2d84c9`).

## 1. Context & goal

The hub breadcrumb switcher (W1) made one identity the app-scoped "who am I
operating as" choice, persisted per network and held in `AppContext`. Every
operate-as screen, however, still picks its identity in isolation: it defaults
to `identities.first()`, to `None`, or to its own `identities[0]` re-default,
and never reads or writes the app-scoped selection. The goal of W2–W5 is to make
every operate-as screen obey the app-scoped selected identity on entry, and —
where the screen *changes who you are* — write the user's choice back so the
breadcrumb and every other surface stay in agreement. Recipient, target, and
group-member pickers must remain untouched: they name a different party, not you.

## 2. W0/W1 foundation (verified signatures — `cc2d84c9`)

`AppContext` (`src/context/mod.rs`):

```rust
// fields (mod.rs:123,127)
pub(crate) selected_identity_id: Mutex<Option<Identifier>>,
pub(crate) pending_identity_selection: Mutex<Option<Identifier>>,

// reads
pub fn selected_identity_id(&self) -> Option<Identifier>            // :1027
pub fn selected_wallet_hash(&self) -> Option<WalletSeedHash>        // :1022
pub fn resolve_selected_identity(&self) -> Option<QualifiedIdentity> // :1033

// writes (each writes both mutexes directly + persists both KV blobs once;
// never calls a sibling setter — no reconciliation recursion)
pub fn set_selected_identity(&self, id: Option<Identifier>)         // :1062
pub fn set_selected_hd_wallet(&self, hash: Option<WalletSeedHash>)  // :1090
pub fn set_selected_single_key_wallet(&self, hash: Option<SingleKeyHash>) // :1114
pub fn set_pending_identity_selection(&self, id: Identifier)        // :1124
pub fn take_pending_identity_selection(&self) -> Option<Identifier> // :1131
pub fn persist_selected_identity_kv(&self, id: Option<Identifier>)  // :1141
fn restore_selected_identity_from_kv(&self)                         // :1160 (private; ensure_wallet_backend)
```

Pure decision helpers (`src/model/selected_identity.rs`), the single source of
truth for precedence — no IO:

```rust
pub fn keep_if_loaded(selected: Option<Identifier>, loaded: &[Identifier]) -> Option<Identifier> // :37
pub fn resolve_selected(selected: Option<Identifier>, loaded: &[Identifier]) -> Option<Identifier> // :47
// resolve_selected = keep-if-loaded → else first loaded → else None
```

`IdentitySelector` (`src/ui/components/identity_selector.rs`) — the primary
migration vehicle. Two opt-in builder methods; default behaviour (neither
called) is byte-identical to pre-W0 (regression-locked by the two tests at
:347 and :360):

```rust
pub fn with_app_default(self, app_context: &'a Arc<AppContext>) -> Self // :112  READ: seed empty buffer from selected_identity_id(), only if that id is one of this selector's options
pub fn syncing_global(self, app_context: Arc<AppContext>) -> Self        // :120  SYNC: write the chosen id back via set_selected_identity on a real user change
```

Mechanism facts that constrain the plan (read from the widget body):
- `app_default_seed()` (:192) seeds **only when the buffer is empty** and the
  app-scoped id is in `self.identities`. So `with_app_default` is a **no-op on a
  screen that already pre-fills `identity_str` in `new()`** — such screens must
  instead seed their `new()`/refresh from `resolve_selected_identity()`.
- The seed path (`ui()` :235) sets the buffer + calls `on_change()` but does
  **not** mark the response changed and does **not** call `sync_to_global()`.
  Seeding therefore never writes back; only a genuine combo/text change does
  (:321 → `sync_to_global()` :205). This is why `with_app_default` + screen
  pre-fill conflict, and why SYNC is safe (entry never clobbers the global).
- `sync_to_global()` resolves the id from the buffer and calls
  `set_selected_identity(Some(id))`. `set_selected_identity` reconciles the
  derived wallet to the identity's owner (clears to `None` for a wallet-less
  identity) — keystone #1.

## 3. Keystone rules (decided — do not relitigate)

- **K1 — wallet follows identity.** No operate-as screen reads
  `selected_wallet_hash` for signing; the signing wallet is derived from the
  identity (`crate::ui::identities::get_selected_wallet`). Wallet selection is
  display-only. `set_selected_identity` reconciles the derived wallet.
- **K2 — `dashpay_wallet_seed_hash()` is a trap.** It returns
  `associated_wallets.keys().next()` (lowest hash of *all* cloned wallets, not
  the owner). Never use it to derive the owning wallet. Wallet-scoped identity
  lists use `load_local_qualified_identities_for_wallet`.
- **K3 — GroupActions is session-local.** It picks one identity per session
  (multi-signer ambiguity); it must neither read from nor write to the
  app-scoped selection.

## 4. Classification rubric

| Category | Reads global on entry? | Writes global on user change? | When |
|---|---|---|---|
| **SYNC-on-change** | yes (seed) | yes | The screen performs a state-changing action **as** the selected identity, and that identity is the screen's sole operating identity. Changing it = changing who you are. |
| **READ-migrate (READ-only)** | yes (seed, guarded) | no | The identity is "which of mine" but the screen is wallet-primary or its candidate set is filtered, so silently re-pointing the global on a transient pick would be wrong. Seed for continuity; do not write back. |
| **SESSION-LOCAL** | no | no | Deliberately independent (group_actions — K3). |
| **N/A** | no | no | No operate-as identity input: the picker names a **recipient / target / group member** (a different party), or the operating identity is **inherited from the launcher**, or there is no identity input. |

### 4a. The SYNC vs READ-only decision rule (the crux)

> A selector **SYNCs** iff it answers "**who am I operating as**" for an action
> the screen signs/creates/sends on that identity's behalf, and that identity is
> the only operating identity on the screen. A selector is **N/A** iff it names
> a party that is **not you** — a recipient you send to, a target you act on
> (freeze/unfreeze/destroy), or a control-group member. A selector is
> **READ-only** iff it is "which of mine" but the screen is **wallet-primary**
> (the candidate list is one wallet's identities, possibly not the global
> wallet) or the candidate list is **capability-filtered** (e.g. EdDSA-only),
> so writing the global from a transient pick would mis-point the app.

This rule is self-validating against the W1 mechanism: the regression test
`default_selector_has_no_sync_target` (identity_selector.rs:360) is annotated
"the 9 no-sync sites stay inert". Applying the rule yields exactly **9 no-sync
`IdentitySelector` sites** (7 recipient/target/member + create_asset_lock
top-up READ-only + group_actions session-local) and **12 sync sites** — a clean
21-site partition. The hazard the test guards is real: a `syncing_global`
mistakenly added to a *target* picker (e.g. freeze) would, on selecting another
person's identity, hijack the global active identity **and** reconcile the
wallet to an identity you do not own (clearing it to `None`, K1).

## 5. Authoritative per-screen table

Operating identity = the identity that signs. "Picker" = the visible identity
input. `il` = `IdentitySelector`; `cb` = bespoke `ComboBox`; `—` = none.

### SYNC-on-change (12)

| # | File | Picker (line) | Current default | Picker is | Mechanism |
|---|---|---|---|---|---|
| 1 | `src/ui/contracts_documents/register_contract_screen.rs` | il :427 (`other_option(false)`) | `qualified_identities.first()` in `new()` :66 | operating | Seed `new()` from `resolve_selected_identity()` (fallback first); add `.syncing_global(self.app_context.clone())`. Keep the `response.changed()` key/wallet derivation (:440). |
| 2 | `src/ui/contracts_documents/update_contract_screen.rs` | il :446 | `first()` in `new()` :72 | operating | Same as #1. |
| 3 | `src/ui/contracts_documents/document_action_screen.rs` | il :266 | `None` (`selected_identity`) | operating | Seed `new()`/`render_*` from `resolve_selected_identity()`; add `.syncing_global(ctx)`. Keep `response.changed()` block (:279). |
| 4 | `src/ui/identities/register_dpns_name_screen.rs` | il :191 | `first()` in `new()` :77 | operating | Seed `new()` from `resolve_selected_identity()`; add `.syncing_global(ctx)`. Selector is gated `len>1` (:117) — single-identity case already correct. |
| 5 | `src/ui/tokens/tokens_screen/token_creator.rs` | il :148 **and** `add_identity_key_chooser` :223 (advanced) | `TokenCreatorUI.selected_identity = None` (mod.rs :1595) | operating | Seed `selected_identity` from `resolve_selected_identity()` when `None`; add `.syncing_global(ctx)` to the il (:148); in the advanced-mode chooser, write back via `set_selected_identity` on change (helper has no opt-in). |
| 6 | `src/ui/dashpay/add_contact_screen.rs` | il :257 (`.label("Identity:")`) | `None` :62 / :79 | operating (sender) | Seed in `new()`/`new_with_identity_id` from `resolve_selected_identity()`; add `.syncing_global(ctx)`. |
| 7 | `src/ui/dashpay/contacts_list.rs` | il :325 | `identities[0]` in `new()` :103 **and** `ui()` re-default :199–206 | operating (your identity) | Replace **both** `identities[0]` defaults with `resolve_selected_identity().or(first)`; add `.syncing_global(ctx)`. |
| 8 | `src/ui/dashpay/contact_requests.rs` | il :365 | `identities[0]` :101; `set_selected_identity()` :118 | operating (your identity) | Seed from `resolve_selected_identity()`; add `.syncing_global(ctx)`. Keep the change handler that clears lists + re-derives wallet (:376). |
| 9 | `src/ui/dashpay/send_payment.rs` | il :564 (Payment History) | `selected_identity` field :462 | operating (your identity) | Seed from `resolve_selected_identity()`; add `.syncing_global(ctx)`. Keep `response.changed()` → `refresh()` (:575). |
| 10 | `src/ui/dashpay/profile_screen.rs` | il :512 | `identities[0]` :152 + `ui()` re-default :240–245 | operating (edit your profile) | Replace both defaults with `resolve_selected_identity().or(first)`; add `.syncing_global(ctx)`. |
| 11 | `src/ui/dashpay/qr_code_generator.rs` | il :187 | `identities[0]` :77 | operating (share your identity) | Seed from `resolve_selected_identity()`; add `.syncing_global(ctx)`. |
| 12 | `src/ui/dashpay/qr_scanner.rs` | il :173 ("Select Your Identity" :164) | `identities[0]` :77 | operating (connect as you) | Seed from `resolve_selected_identity()`; add `.syncing_global(ctx)`. Keep the prev/new id diff at :168/:186. |

### READ-migrate, READ-only (3)

| # | File | Picker (line) | Current default | Mechanism | Why no sync |
|---|---|---|---|---|---|
| 13 | `src/ui/wallets/create_asset_lock_screen.rs` | il :398 ("Identity to top up") | `None` :94 | Add `.with_app_default(&self.app_context)` — it seeds **only if** the global id is one of this wallet's identities (the candidate list is wallet-scoped, :69). No `new()` pre-fill to change. | Wallet-primary: the screen is launched for a specific wallet that may not be the global wallet; topping up does not change who you operate as (K1 reconcile would mis-point). |
| 14 | `src/ui/tools/grovestark_screen.rs` | cb (`selected_identity: Option<String>` :53, ComboBox; `refresh_identities` :154) | `None` :134 | Manual seed: in `new()`/`refresh_identities`, set `selected_identity` from `resolve_selected_identity()` **iff** it is in the EdDSA-filtered list (:160), else first filtered, else `None`. Store the id string. | Capability-filtered (EdDSA-only): the global identity may be absent from the list; a developer tool should not push an EdDSA-only id as the app-wide active identity. SYNC deferred. |
| 15 | `src/ui/wallets/send_screen.rs` | cb `identity_source_selector` :1966 | `None` :424 | Manual seed: when the source list is built, default `selected_identity` from `resolve_selected_identity()` **iff** it is among this wallet's identities, else leave `None`. | Wallet-primary + transient funding source for one send; K1 reconcile of the global wallet would fight the screen's own wallet. |

### SESSION-LOCAL (1)

| # | File | Picker (line) | Mechanism |
|---|---|---|---|
| 16 | `src/ui/contracts_documents/group_actions_screen.rs` | il :541 | **Leave the default `IdentitySelector` untouched** (no `with_app_default`, no `syncing_global`). Keep the screen's own `selected_identity`. Add a one-line code comment citing K3, and a regression test asserting it neither seeds from nor writes the global. |

### N/A — recipient / target / member / inherited / no input (≈13)

| File | Picker (line) | Why N/A |
|---|---|---|
| `src/ui/tokens/mint_tokens_screen.rs` | il :238 `.label("Recipient:")` `.exclude(self)` :245 | Recipient. Operating identity is fixed `identity_token_info.identity` (row-clicked). |
| `src/ui/tokens/transfer_tokens_screen.rs` | il :183 `.label("Recipient:")` `.exclude` :190 | Recipient. |
| `src/ui/tokens/freeze_tokens_screen.rs` | il :214 "Freeze Identity ID:" | Target you act **on**. Operating identity = `identity_token_info.identity`. |
| `src/ui/tokens/unfreeze_tokens_screen.rs` | il :218 "Identity ID to unfreeze:" | Target. |
| `src/ui/tokens/destroy_frozen_funds_screen.rs` | il :225 "Frozen Identity ID:" | Target. |
| `src/ui/tokens/tokens_screen/groups.rs` | il :206 `.exclude` :212 | Control-group member (defining who controls the contract). |
| `src/ui/identities/transfer_screen.rs` | il :172 "Receiver Identity ID:" `.exclude(self)` :179 | Recipient. Operating identity `self.identity` is passed to `new()` :82 (inherited from launcher). |
| `src/ui/identities/withdraw_screen.rs` | — | No picker. `self.identity` passed to `new()` :68 (inherited). |
| `src/ui/wallets/unshield_credits_screen.rs` | — | Wallet-scoped (`new(seed_hash)` :51); no identity. |
| `src/ui/tokens/tokens_screen/mod.rs` (main TokensScreen) | — | Lists balances across all identities; per-row actions carry their own `identity_token_info` (:1493). No single operating picker. |
| `src/ui/identity/home.rs` (:274), `src/ui/identity/contacts.rs` (:166), `src/ui/identity/settings.rs` (:660) | — | **Already app-scoped** (W1): they read `resolve_selected_identity()` directly. No change. settings.rs already pulls `incoming = resolve_selected_identity()` each frame and syncs its field (:660–669). |

Inherited-identity note: `transfer_screen` and `withdraw_screen` get their
operating identity from the launch site (the hub Home tab, `home.rs:274`, which
already uses `resolve_selected_identity()`), so they are transitively
app-scoped without their own migration. Confirm launch sites still pass the
active identity when wiring these.

### Counts

| Category | Count |
|---|---|
| SYNC-on-change | 12 |
| READ-migrate (READ-only) | 3 |
| SESSION-LOCAL | 1 |
| N/A (incl. 3 already-app-scoped hub tabs) | ≈13 |
| **Total identity-input sites enumerated** | **≈29** (21 `IdentitySelector` + 2 bespoke ComboBox + group/inherited/no-input) |

`IdentitySelector` partition: 12 SYNC + 9 no-sync (7 N/A recipient/target/member
+ #13 READ-only + #16 session-local) = 21 — matches the "9 no-sync sites"
regression-test invariant.

## 6. Domain-batched dev plan

Ordered mechanical (`IdentitySelector` + `.syncing_global`) first, bespoke last.
Each batch is independently committable and independently testable.

### Batch B1 — Contracts & Documents (3 SYNC)
- Files: `register_contract_screen.rs`, `update_contract_screen.rs`,
  `document_action_screen.rs`.
- Change: seed `new()` (and any refresh re-default) from
  `resolve_selected_identity()` (fallback `first()`); add
  `.syncing_global(self.app_context.clone())` to each `IdentitySelector`; leave
  the existing `response.changed()` key/wallet derivation intact.
- Tests: extend existing patterns — add a kittest (DB-seeded multi-identity, à
  la `identity_hub_switcher.rs`) asserting the contract screen defaults to the
  app-scoped id and that a picker change calls `set_selected_identity`. No live
  network.
- Commit: `feat(contracts): obey app-scoped selected identity in register/update/document screens (W2)`

### Batch B2 — DPNS / Identities (1 SYNC)
- Files: `register_dpns_name_screen.rs`.
- Change: seed `new()` from `resolve_selected_identity()`; add `.syncing_global`.
- Tests: **extend** `tests/kittest/register_dpns_name_screen.rs` — assert the
  default identity tracks the app-scoped selection (seed two identities, set the
  selection to the second, expect it pre-selected).
- Commit: `feat(identity): default DPNS registration to the app-scoped identity (W2)`

### Batch B3 — DashPay (7 SYNC)
- Files: `add_contact_screen.rs`, `contacts_list.rs`, `contact_requests.rs`,
  `send_payment.rs`, `profile_screen.rs`, `qr_code_generator.rs`,
  `qr_scanner.rs`.
- Change: replace every `identities[0]` / `None` default (in both `new()` **and**
  any `ui()`/refresh re-default — see contacts_list :199–206, profile_screen
  :240–245) with `resolve_selected_identity().or(first)`; add `.syncing_global`.
  Keep each screen's change handler (list-clear + wallet re-derive).
- Tests: **extend** `tests/kittest/dashpay_screen.rs` (+ the
  `identity_hub_contacts.rs` pattern). One representative seed-and-default
  assertion per screen; one write-back assertion (change picker →
  `resolve_selected_identity()` reflects it) on `contacts_list` as the canary.
- Commit: `feat(dashpay): sync DashPay screens with the app-scoped selected identity (W3)`

### Batch B4 — Tokens (1 SYNC; verify 6 N/A)
- Files: `tokens_screen/token_creator.rs` (+ `tokens_screen/mod.rs` for the
  `TokenCreatorUI.selected_identity` seed at :1595).
- Change: seed `TokenCreatorUI.selected_identity` from
  `resolve_selected_identity()` when `None`; add `.syncing_global(ctx)` to the
  simple-mode il (:148); in advanced mode, write back via `set_selected_identity`
  in the `add_identity_key_chooser` change path.
- Verify-only (no behaviour change): `mint`, `transfer_tokens`, `freeze`,
  `unfreeze`, `destroy_frozen_funds`, `groups` — confirm they keep the **default**
  `IdentitySelector` (no opt-in). Add a focused unit test per file is overkill;
  instead add one shared regression test (see B6).
- Tests: token_creator kittest seed-and-default assertion (simple mode) — may
  need a minimal token-context fixture; if unavailable, mark `#[ignore]` with a
  TODO and notify (test-infra gap, §7).
- Commit: `feat(tokens): default the token creator to the app-scoped identity (W4)`

### Batch B5 — Wallets & Tools (3 READ-only, bespoke)
- Files: `create_asset_lock_screen.rs` (add `.with_app_default(&self.app_context)`
  to the il :398), `grovestark_screen.rs` (manual EdDSA-guarded seed in `new()`
  / `refresh_identities`), `send_screen.rs` (manual wallet-membership-guarded
  seed of the source ComboBox).
- Change: READ-only seed; **no** `syncing_global` / write-back.
- Tests: unit test the seed guards (wallet-membership for create_asset_lock /
  send_screen; EdDSA-membership for grovestark) — pure model-ish, no UI harness
  needed where the guard is extractable; otherwise a small kittest.
- Commit: `feat(wallets,tools): seed wallet-scoped and tool screens from the app-scoped identity (W5)`

### Batch B6 — GroupActions guard + cross-cutting regression locks
- Files: `group_actions_screen.rs` (one-line K3 comment), plus tests.
- Change: no functional change to group_actions.
- Tests: (a) assert group_actions' selector is the default (no
  `with_app_default`/`syncing_global`) and that selecting an identity there does
  not call `set_selected_identity`; (b) a shared regression test asserting the
  N/A token recipient/target pickers (mint/freeze/etc.) keep `sync_target ==
  None` — complementing the existing `default_selector_has_no_sync_target`
  (identity_selector.rs:360). These lock the "9 no-sync sites" invariant against
  future drift.
- Commit: `test(identity): lock session-local and no-sync identity pickers (W5)`

## 7. Risks & test-infra gaps

- **TI-1 — `WalletFixture` builder is the gating test-infra need.** IT-SWITCH-01/02
  (wallet dropdown + wallet-scoped identity list) are blocked because there is no
  fixture for a **loaded HD `Wallet` in `AppContext::wallets` with a matching
  `wallet_hash`** (documented in `tests/kittest/identity_hub_switcher.rs:6–16`;
  the current path only seeds identities via
  `insert_local_qualified_identity(.., &None)`). The SYNC write-back tests that
  need to observe **K1 wallet reconciliation** (set identity → derived wallet
  follows) cannot be fully exercised until this builder exists. Build
  `WalletFixture` before, or as the first step of, B6. Until then, write-back
  tests assert only `resolve_selected_identity()` movement, not wallet
  reconciliation, and carry a TODO.
- **TI-2 — per-frame identity-table load in the hub.** `hub_screen.rs:209–210`
  carries `TODO(IDH-003 follow-up)` to fold `landing()`'s load and the
  breadcrumb switcher's per-frame `load_local_qualified_identities()` into one
  shared snapshot. The SYNC migration adds **no** new per-frame DB load (the
  selector seeds from in-memory `selected_identity_id()`), but each migrated
  screen still calls `load_local_qualified_identities()` per frame as today —
  do not regress this into extra loads; prefer seeding from already-loaded
  vectors.
- **R1 — wallet-primary screens must not SYNC (K1 interaction).** `create_asset_lock`
  (#13) and `send_screen` (#15) are launched for a **specific** wallet whose
  identity list may differ from the global wallet. If either were given
  `syncing_global`, picking an identity would call `set_selected_identity`, which
  reconciles the **global** wallet to that identity's owner — fighting the
  screen's own wallet and possibly clearing it to `None`. They are READ-only by
  design. Do not "upgrade" them to SYNC.
- **R2 — target/recipient pickers must stay default.** Adding `syncing_global`
  to a freeze/unfreeze/destroy **target** or a mint/transfer **recipient** would
  let selecting another party's identity hijack the global active identity and
  K1-reconcile the wallet to an identity the user does not own. The B6 regression
  lock and the existing `default_selector_has_no_sync_target` test defend this.
- **R3 — seed-vs-pre-fill conflict.** `with_app_default` is inert on any screen
  that pre-fills `identity_str` in `new()` (the buffer is non-empty). For all 12
  SYNC screens that pre-fill, seed via `resolve_selected_identity()` in
  `new()`/refresh and rely on `syncing_global` for write-back — do **not** expect
  `with_app_default` to do the reading there.
- **R4 — grovestark capability filter.** The candidate list is EdDSA-only; the
  seed must guard membership or the selection silently won't take.

## 8. Surprises vs the prior MemCan rulings

The earlier audit (memory 5ebc3af6) named six "READ-migration" screens:
register_contract, update_contract, document_action, group_actions,
token_creator, grovestark. This plan refines that umbrella term, which predates
the W1 `syncing_global` mechanism:
- register_contract, update_contract, document_action, **token_creator** are
  **SYNC-on-change**, not READ-only — they are operate-as signers, so a user
  change must propagate. ("Require READ-migration" is satisfied by SYNC, which
  also reads.)
- group_actions is **SESSION-LOCAL** (confirmed, K3).
- grovestark is **READ-only** (confirmed) — capability-filtered tool screen.

The audit's "~24 READ-migrate screens" (memory b2115c58) was a loose upper
bound; the precise partition is 12 SYNC + 3 READ-only + 1 session-local, with
the token action screens' visible pickers reclassified as **N/A recipient/target
pickers** (their operating identity is row-scoped via `identity_token_info`, not
app-scoped) — a correction the earlier audit did not draw.
