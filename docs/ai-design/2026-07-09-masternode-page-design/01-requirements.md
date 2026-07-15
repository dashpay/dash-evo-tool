# Masternodes Page — Requirements

**Repo:** `dash-evo-tool` · **Branch:** `feat/masternodes-tab` (based on PR #873) · **Date:** 2026-07-09
**Phase:** Requirements + UX (planning only; no Rust changes)
**Author:** Diziet (Product Designer)

Companion artifacts: `02-ux-spec.md` (journeys, wireframes), `wireframes.html` (visual mock).
Source of truth for the split rationale: `../identity-hub-parity-audit.md` § "Masternodes as a separate page".

---

## 1. Executive Summary

**Problem.** Masternode and evonode identity handling is buried inside the generic
*Identities → Load Identity* screen, reachable only by ticking *Show Advanced Options* and
selecting an identity type. That screen serves three audiences through one radio-plus-conditional
form. The masternode/evonode arm is a distinct job for a distinct audience — node operators whose
real payoff is **DPNS contested-name voting** — yet today these identities land in the same table,
and worse, they leak into the everyday-user **Identity Hub** picker where they are offered
user-centric actions (register a username, edit a social profile, add a contact) that are
meaningless for a collateral/voting identity.

**Solution direction.** Add a dedicated left-nav root tab **"Masternodes"** with a **card layout**
that (a) owns a masternode/evonode-only load flow lifted from the advanced-options arm, (b) lists
loaded masternodes/evonodes as cards showing ProTxHash, type, voter-identity readiness, key status,
and DPNS-voting status, (c) opens a per-node detail/voting view, and (d) is paired with a filter
that keeps masternode/evonode identities **out of the Identity Hub / Identities pickers** so those
surfaces stay everyday-user only.

**Key actors.** Priya (Power User / masternode operator) is the primary actor. Alex (Everyday User)
is a *contrast* actor — the design must keep the operator surface out of Alex's way and vice versa.

**No model changes.** `IdentityType` (User/Masternode/Evonode), `associated_voter_identity`, and the
three-way `PrivateKeyTarget` already provide the seam. This is a new view + filter + card layout over
existing model and backend plumbing, not a rewrite.

---

## 2. Stakeholder & Actor Analysis

### 2.1 Primary actor — Priya (Power User / masternode operator)

Canonical persona: `docs/personas/power-user.md`. Priya runs a Dash masternode, manages multiple
wallets, and understands ProTxHash, DIP3 owner/voting/operator keys, and derivation paths.

| Field | Value |
|---|---|
| **Goal** | Load her masternode/evonode identities into DET and use them to vote on DPNS contested names. |
| **Pain today** | The load path is buried behind *Show Advanced Options*; her nodes then sit in the same table as everyday user identities and appear in the Identity Hub with nonsensical user actions. |
| **Success metric** | Time to check masternode key paths **under 10s** (persona success metric); load a node and reach its voting view without touching the generic identity flow. |

### 2.2 Contrast actor — Alex (Everyday User)

Canonical persona: `docs/personas/everyday-user.md`. Alex never operates a node.

| Field | Value |
|---|---|
| **Goal** | Manage a personal identity (username, credits, DashPay). |
| **Relevance** | Alex must **never** be shown masternode load fields or see masternode identities in the Identity Hub picker. The Masternodes tab is a self-contained operator surface Alex can ignore. |

### 2.3 Secondary / supporting

- **DPNS contested-name voting** (backend `contested_names/vote_on_dpns_name.rs`, DPNS root screens):
  the downstream consumer of masternode voter keys. The Masternodes page hands off to / surfaces this.
- **Wallet subsystem** — the active wallet on the Masternodes page is the **funding source for Top up**
  (FR-9), not a key-derivation source. **Correction (investigated 2026-07-09):** voting/owner/payout keys
  cannot be auto-derived from a wallet — `derive_keys_from_wallets` is hard-gated to `IdentityType::User` in
  `backend_task/identity/load_identity.rs`; masternode keys are Core-side (tied to the node's ProRegTx), not
  part of any wallet's identity-auth HD tree. The "Try to derive from loaded wallet" checkbox does NOT carry
  over to the load form — see §9 note and US-6 retirement.
- **Secret seam / at-rest storage** — owner/voting/payout keys **enter unprotected (Tier-1 keyless)** at load
  time because the load flow has no password field (by design; confirmed also for the `identity_masternode_load`
  MCP tool). They are **not permanently plaintext**: a loaded node's keys can be sealed to **Tier-2 per-identity**
  encryption afterward via the existing `IdentityTask::ProtectIdentityKeys` (Argon2id + XChaCha20-Poly1305,
  per-secret object-password envelope; today reached from the Key Info screen's "Add password protection…").
  See `wallet_backend/secret_seam.rs` and `docs/ai-design/2026-06-19-secret-storage-seam/`. The design's job is
  to (a) make this accurate to the user via a non-blocking awareness note, and (b) surface the "add protection"
  affordance on the Masternodes page — **not** to design the crypto (which already exists).

---

## 3. Domain Notes

- **ProTxHash is the identifier.** For masternode/evonode identities the "Identity ID" is the ProTxHash,
  conventionally **hex**-encoded (`IdentityType::default_encoding` → Hex for MN/Evonode vs Base58 for User).
  The page must label the field **"ProTxHash"**, not "Identity ID".
- **Two node types.** `Masternode` (regular) and `Evonode` (HPMN / high-performance). Model discriminates
  via `IdentityType`. Badge colours already exist: Masternode → `PLATFORM_PURPLE`, Evonode → `DASH_BLUE`
  (`identity_picker_card.rs::draw_type_badge`).
- **Three key roles (DIP3).** Voting Private Key, Owner Private Key, Payout Address Private Key — the exact
  three inputs in the current advanced arm (`add_existing_identity_screen.rs:420-434`). All are optional at
  load time; without them the node is view-only.
- **A masternode carries a separate voter sub-identity.** `associated_voter_identity: Option<(Identity, IdentityPublicKey)>`.
  Its presence is what enables voting; its absence is the `NoVotingIdentity` error at vote time. The card must
  surface **voter-identity readiness** as a first-class status.
- **The real workflow is voting.** A masternode identity exists in DET primarily to vote on DPNS contested
  names (and hold owner/payout keys). Vote choices are **Abstain**, **Lock**, or **vote for a candidate**.
- **Identity status** (`IdentityStatus`: Active / Unknown / PendingCreation / NotFound / FailedCreation) applies
  and already has a colour mapping (green/gray/orange/red). Reuse it as a status dot on the card.
- **Key-protection tier is per-identity, opt-in.** MN/Evonode keys load as Tier-1 (unprotected/keyless) — the load
  form has no password field by design — and can be upgraded to Tier-2 (per-identity password protection) later
  via the existing `IdentityTask::ProtectIdentityKeys`. The Masternodes page surfaces this state and the upgrade
  affordance; it does not implement encryption. Protection is gated by the vault-key scheme, not identity type.
- **Provider withdraw destination rule.** Withdrawing a provider identity's credits with the **owner** key forces
  the destination to the node's **registered Core payout address**; with the **transfer/payout** key the
  destination is a **free** address. Existing withdraw-flow behaviour (FR-9) — surfaced, not redesigned.
- **Add-key purpose rule (all identity types).** The add-key purpose selector **excludes OWNER and VOTING** —
  Core-registered provider roles that cannot be added via Platform for any identity type. TRANSFER / AUTHENTICATION
  / ENCRYPTION / DECRYPTION are addable (FR-10).
- **Evonode token rewards are Evonode-only** (protocol rule). Plain Masternodes have none, so FR-11's "Claim token
  rewards" cross-link appears only for `IdentityType::Evonode`.

---

## 4. Functional Requirements

### FR-1 — Masternodes root tab *(Expert Mode gated — decision, 2026-07-09)*
A new left-nav root entry **"Masternodes"** (icon + label, matching the existing rail style), placed
adjacent to Identities / Identity Hub. Selecting it shows the Masternodes page. It persists as a root
screen and survives network switches like other root tabs.

**Visibility gate:** the entire tab — nav item AND screen access — is shown only when **Expert Mode** is
ON (`app_context.is_developer_mode()`, user-facing label "Expert mode" per `network_chooser_screen.rs:607`).
With Expert Mode off, the nav item does not render at all (not just disabled/hidden-behind-a-click) and the
route is unreachable, matching how other expert-only surfaces are gated in this codebase (`FeatureGate::DeveloperMode`,
`model/feature_gate.rs:69`). Rationale: masternode/evonode operation is a distinct, node-operator audience
(Priya persona) — Expert Mode is the existing mechanism DET already uses to separate that audience from
Alex (Everyday User), so this reuses an established pattern rather than inventing a new one.

### FR-2 — Empty state
When no masternode/evonode identities are loaded, show a centered card empty state (matching the
*No Identities Loaded* pattern in `03-identities-empty.png`) explaining what a masternode identity is
for (voting on DPNS contests, holding owner/payout keys) and offering a primary **"Load a masternode"**
action. Include the reassurance line about node connectivity in the existing empty-state voice.

### FR-3 — Card list of loaded masternodes
Present loaded masternode/evonode identities as a responsive card grid (reusing the
`identity_picker_card.rs` visual language: rounded surface card, monogram, type badge pill). Each card shows:
- **Identifier** — shortened ProTxHash (heading), with alias above it when set.
- **Type badge** — Masternode (purple) / Evonode (blue).
- **Voter-identity readiness** — "Voting ready" (voter identity present) or "No voting key" (absent).
- **Key status** — which of Voting / Owner / Payout keys are loaded (compact indicator, e.g. `V O P`
  with present keys emphasised).
- **DPNS-voting status** — a short line: open contests available to vote on, or scheduled/last vote state.
- **Identity status dot** — from `IdentityStatus` (Active/Unknown/NotFound…).
- The whole card is a single click target → opens the detail/voting view (FR-5).

### FR-4 — Load a masternode/evonode
A dedicated load flow (extracted from the advanced-options arm) with fields:
- **ProTxHash** (required) — labelled and hinted as ProTxHash; accepts hex or Base58.
- **Node type** — segmented toggle Masternode / Evonode (replaces the buried combo box; no "User" option here).
- **Alias** (optional) — local-only label, explicitly "not saved to Dash Platform".
- **Voting Private Key**, **Owner Private Key**, **Payout Address Private Key** — WIF or hex, all optional,
  always pasted manually (no auto-derive — see US-6 retirement in §9, these key roles cannot be derived from
  any wallet). On Testnet with a `.testnet_nodes.yml` fixture present, a **"Fill Random Masternode/Evonode"**
  dev-convenience button (FR-12) can autofill this section from a real test node.
- **Encryption password (optional)** — an optional password field (WIF-style show/hide eye + helper line). When
  set, the entered voting/owner/payout (and identity) keys are **sealed encrypted-at-rest at load time** (Tier-2);
  when left blank, keys load unprotected (Tier-1 keyless / obfuscation-only) and can be protected later from the
  Key Info screen. See **FR-8** for the plumbing this requires. Copy in §7.
- **Key-storage awareness** — a non-blocking inline note (Warning tone, not a blocking gate) explaining that,
  without a password, keys are stored unencrypted (obfuscation-only) and protection can be added later. Copy in §7.
- Primary **Load masternode** action, disabled until a ProTxHash is entered, with a disabled-tooltip
  explaining why (per `ResponseExt::disabled_tooltip`).

### FR-5 — Masternode detail / voting view
Opening a card shows a detail view with:
- Header: alias (if any) + shortened ProTxHash + type badge + copy-ProTxHash affordance + identity status.
- **Keys summary** — Voting / Owner / Payout presence, voter-identity ID (shortened, copyable), and the
  **protection tier** (Unprotected / Password-protected). When Tier-1, offer an **"Add password protection…"**
  action that dispatches the existing `IdentityTask::ProtectIdentityKeys` (no new crypto). This is the recourse
  the load-form awareness note points to. Also a **"Manage keys ›"** drill-in (FR-10) into the existing
  `KeyInfoScreen`.
- **DPNS voting section** — a **collapsing section, collapsed by default**, with the open-contest **count in
  the header** (`DPNS name contests to vote on (3)`) so operators see there's something to act on without
  expanding. Expanded, it lists active contested names this node can vote on, each with the three choices
  (Abstain / Lock / vote for a candidate identity), plus scheduled and past votes where available. This surfaces
  / hands off to the existing DPNS voting backend; it does not re-implement voting logic.
- **Credit actions row** — Withdraw / Top up / Transfer (FR-9), for both Masternode and Evonode.
- **Token rewards cross-link** — **Evonode only** (FR-11): "Claim token rewards ›" routing to the existing
  `ClaimTokensScreen`. Hidden for plain Masternode.
- **Remove** — a destructive action (danger button, confirmation dialog) that forgets the masternode from
  DET and also removes its associated voter identity (existing behaviour).

Grouping (top → bottom, to avoid clutter): header · credit-actions row (with the Evonode-only token-rewards
link) · Keys (with "Manage keys ›") · collapsible DPNS voting · Remove.

### FR-9 — Credit actions on the detail view (Withdraw / Top up / Transfer) *(reuse existing screens)*

The detail view exposes the node's credit operations as an actions row/dropdown (mirroring the legacy Identities
**Actions** affordance), for **both Masternode and Evonode**:
- **Withdraw** → reuse `withdraw_screen`, scoped to the selected node's `QualifiedIdentity`.
- **Top up** → reuse `top_up_identity_screen`, scoped to the node.
- **Transfer** → reuse `transfer_screen`, scoped to the node.

**MN/Evonode-specific withdraw behaviour (document, don't redesign):** withdrawing with the **owner** key forces
the destination to the node's **registered Core payout address**; withdrawing with the **transfer/payout** key
allows a **free** destination address. This is existing behaviour of the withdraw flow for provider identities —
surface it, don't reimplement it.

*Reuse note (Nagatha):* no new operation screens — pass the selected node's `QualifiedIdentity` into the three
existing screens. The only new work is the entry points (row/dropdown) on the detail view.

### FR-10 — Key-management drill-in (`KeyInfoScreen`) *(reuse existing screen)*

The Keys section offers **"Manage keys ›"** opening the **existing** `KeyInfoScreen` for the node: view private
key / WIF, sign message, add key, remove key, protect keys.

**Add-key constraint (not MN-specific — document it):** the add-key **purpose selector excludes OWNER and
VOTING** (those are Core-registered provider roles, un-addable via Platform for **any** identity type).
TRANSFER / AUTHENTICATION / ENCRYPTION / DECRYPTION are addable. This is an existing platform rule, surfaced here
so the operator isn't surprised.

*Reuse note (Nagatha):* route to the existing `KeyInfoScreen` scoped to the node; no new key UI.

### FR-11 — Evonode token-rewards cross-link (`ClaimTokensScreen`) *(Evonode only, reuse existing screen)*

On the detail view of an **Evonode** identity (NOT a plain Masternode), show **"Claim token rewards ›"** routing
to the existing Tokens **`ClaimTokensScreen`** scoped to that identity. **Hidden for Masternode.**

- Evonode-only is a **protocol rule** — a plain Masternode simply has no such rewards, so the action is shown
  only when `identity_type == Evonode`.
- Do **not** rebuild any claim UI on the Masternodes page — it is a cross-link/route only.

*Reuse note (Nagatha):* conditional entry point (`Evonode` only) that routes to `ClaimTokensScreen` with the
node's identity; no new claim logic.

### FR-12 — "Fill Random Masternode / Evonode" dev convenience (Testnet only, reuse existing logic)

Carry forward the existing dev-only quick-fill on the load form (FR-4): a single button, labelled to match the
Node-type toggle ("Fill Random Masternode" / "Fill Random Evonode"), that picks a random **real** testnet node
from a local `.testnet_nodes.yml` fixture and autofills ProTxHash + keys (**Voting + Owner for a Masternode;
Voting + Owner + Payout for an Evonode** — the regular-masternode fixture struct `MasternodeInfo` carries no payout
key, so `fill_random_masternode()` fills Voting + Owner only, verified `add_existing_identity_screen.rs:30-35,979-993`).
It does **not** fabricate a synthetic node — it is a curated-fixture quick-fill for developer testing, same as today.

- **Source (investigated 2026-07-09):** `add_existing_identity_screen.rs:203-208` — `fill_random_masternode()`
  (lines 979-993) and `fill_random_hpmn()` (lines 961-977), reading `load_testnet_nodes_from_yml(".testnet_nodes.yml")`
  into a `TestnetNodes { masternodes, hp_masternodes }` fixture struct.
- **Fixture facts (verified 2026-07-09):** `.testnet_nodes.yml` is **gitignored** (`.gitignore:17`), not tracked
  in git, and does **not exist** in this repo — it is not shipped, not hardcoded, and not something we currently
  have. It **does contain real private keys in plaintext** (`KeyInfo.private_key` for owner/voter/payout,
  `serde_yaml_ng`-parsed). It must be supplied locally by the developer; the loader returns `Ok(None)`
  gracefully only when the file is **absent** (no error, no crash). A **malformed** file returns `Err(_)`, which the
  *legacy* screen banners (`add_existing_identity_screen.rs:151-161`); the new Masternodes load form must map
  `Err(_)` → absent button (swallow, no banner) — a deliberate divergence, not verbatim reuse (see test-spec TC-FR12-04).
- **Visibility — MUST be conditional on fixture presence (decision, 2026-07-09):** the button does not render
  at all unless `load_testnet_nodes_from_yml(...)` returns `Some(_)` — never shown-but-disabled. On networks
  other than Testnet, or when the file is missing/unparseable, the button and its row are simply absent from
  the load form; no placeholder, no error state.
- **Gating today (current app):** visible when `show_advanced_options` is on + `network == Testnet` + the yml
  fixture loaded successfully. **Not gated by `developer_mode`** on the current screen.
- **Gating on the new page — simplified by FR-1's Expert Mode gate:** because the entire Masternodes tab now
  requires Expert Mode (FR-1) to even be reached, an additional per-button `developer_mode` check would be
  redundant for normal navigation. The button's own remaining condition is just **Testnet + fixture present**
  (dropping the now-unreachable `show_advanced_options` concept, which doesn't exist on this page — see FR-4).
  Flag for Nagatha: confirm whether a defense-in-depth `developer_mode` check is still worth adding at the
  button call-site (cheap, guards against any future non-nav entry point to this screen) — implementation
  judgment call, not re-litigated here.

*Reuse note (Nagatha):* reuse `fill_random_masternode()`/`fill_random_hpmn()` and the `.testnet_nodes.yml`
loader verbatim; only the entry point (one button instead of two, label follows the Node-type toggle) and the
condition (fixture-presence check controlling render, not just enabled state) are new.

### FR-6 — Filter masternode/evonode out of user-only pickers
Masternode/evonode identities (`IdentityType != User`) must **not** appear in the Identity Hub picker or the
generic Identities user-identity surfaces. This is the paired correction that keeps the everyday-user surface
coherent (audit § "1 new latent finding"). This is a filter, not a data migration.

**Extension (decision, §10.2, 2026-07-09):** once FR-4 ships, remove the Masternode/Evonode options from the
legacy buried arm (`add_existing_identity_screen.rs`'s Identity Type dropdown under Show Advanced Options) —
that dropdown becomes User-only. This is a removal, not just a filter, and it prevents two competing entry
points for loading the same kind of identity.

### FR-7 — Refresh
Provide a refresh affordance (top-right, matching `01-dashpay.png` header Refresh button) that re-fetches
masternode identity + voting state.

### FR-8 — Optional load-time key encryption password *(needs NEW plumbing — implementation scope)*

The load form (FR-4) offers an **optional "Encryption password"** field. Its behaviour:
- **Blank (default):** keys load **Tier-1 keyless** (obfuscation-only, not confidential) — current behaviour. The
  user can seal them later via the Key Info screen's "Add password protection…" (existing
  `IdentityTask::ProtectIdentityKeys`), also surfaced on the Masternodes detail view (FR-5).
- **Set:** the entered voting/owner/payout (and identity) keys are **sealed Tier-2 at load time**
  (`put_secret_protected` / `store_protected` — Argon2id + XChaCha20-Poly1305, per-secret object-password
  envelope) instead of only post-load.

**This is not a pure view change — it requires NEW plumbing (flag for the implementation plan / Nagatha):**
- `backend_task/identity/mod.rs::IdentityInputToLoad` (struct at `mod.rs:43`) currently has **no password field**
  — it carries only `voting_private_key_input` / `owner_private_key_input` / `payout_address_private_key_input`
  (`Secret`), `keys_input`, `derive_keys_from_wallets`, `selected_wallet_seed_hash`. A new optional password
  field must be added.
- `backend_task/identity/load_identity.rs` currently persists loaded keys unprotected. When a password is
  present it must route the persist through the **existing** seal path (`store_protected` /
  `put_secret_protected`, as used by `protect_identity_keys.rs:225` and `add_key_to_identity.rs:265`) so keys
  land Tier-2 at load — rather than only reachable post-load via `ProtectIdentityKeys`.
- No **new crypto**: reuse the existing protected-secret envelope. The work is threading the password param
  through `IdentityInputToLoad` → `load_identity` → the seal path, and validating it (non-empty when the box is
  used; the field is entirely optional).
- **MCP scope (decision, 2026-07-09):** the third `IdentityInputToLoad` constructor — the MCP
  `masternode_identity_load` tool (`src/mcp/tools/masternode.rs:180`, a confirmed keyless entry point, §2.3) —
  passes `encryption_password: None` this iteration (Tier-1 unchanged; FR-8 is GUI-scoped). A `TODO` records
  headless password parity as a follow-up.
- **Model/backend rules:** password is a `Secret`, never logged, never stored; validation of the plaintext
  password (e.g. min length, if any) belongs in `model/`, enforcement in the backend task per DET layering.

### FR-GLOBAL-NAV — Global wallet/identity switcher on every root page *(cross-cutting app-shell)*

> **Scope note.** This is a **cross-cutting app-shell change larger than the Masternodes page** — it touches
> every root screen's top panel, not just Masternodes. It is recorded here because it is the **foundation the
> Masternodes page sits inside**: the Masternodes tab must render the same global chrome as every other tab.
> Implementation should be scheduled as its own app-shell task; the Masternodes page consumes it.

The wallet + identity breadcrumb switcher (`src/ui/identity/breadcrumb_switcher.rs`, IDH-003) — today rendered
**only** on the Identity Hub (`hub_screen.rs:195`) — becomes a **global top-nav rendered on every root page**
(Dashpay, Identities, Identity Hub, Masternodes, Contracts, …) through the shared top panel. This realizes the
original IDH-003 design intent, which already states the breadcrumb "is always visible in the topbar of **every
tab**" (design-spec `2026-04-22-identity-dashpay-redesign/design-spec.md` §A.3) but was only wired into the Hub.

- **FR-GLOBAL-NAV-1 — One switcher, everywhere.** Every root screen renders the three-segment switcher
  (`Identities › 💼 wallet › 👤 identity`) in the top island's left region via
  `top_panel::add_top_panel_with_breadcrumb` (the seam already exists — both `add_top_panel` and the breadcrumb
  variant delegate to the same `render_top_island`). Behavior, styling, tooltips, dropdowns, and placeholder
  rules are identical on every page (design-spec §A.3 / §7 / §D).
- **FR-GLOBAL-NAV-2 — Selection interaction model (authoritative).** The switcher reads/writes the **app-scoped**
  selection (`AppContext::selected_wallet_hash` / `selected_identity_id`; `HubSelection` holds only within-session
  search buffers). Four rules govern how it behaves on every page:
  1. **Silent context change.** Selecting an object (wallet / identity / whatever the page exposes) in the top-nav
     silently updates the app-global selection. **No forced navigation** to another tab. *(This supersedes the
     earlier "route to Identity Home on identity selection" framing — see §9 resolved question.)*
  2. **Two-way binding where the page consumes the selection.** If the active page actively uses a selected
     object, the top-nav pill and the page stay in sync **both ways**. Canonical example: on a
     Send/Transfer-from-wallet page, changing the wallet in the top-nav changes the page's source wallet, and
     changing the source wallet on the page updates the top-nav pill. Same for identity where a page consumes it.
  3. **Blast-radius control (rollout strategy).** The nav renders on every page immediately, but a given pill is
     **interactive only on pages already wired to consume that selection**. On pages not yet wired, that pill
     renders **disabled/read-only** — dimmed, no caret, **no visible text tag**; the explanation lives in a
     **hover tooltip that tells the user how to change that selection** (e.g. "Change the active wallet from the
     Wallets tab", "Updates when you open a masternode"). Leave a `TODO` in code to wire it later. Interactivity
     rolls out page-by-page — no requirement to wire every page at once.
  4. **Per-page composition.** Show only the pills that make sense for the page. A page with no identity context
     (e.g. a Wallet page) shows **only the wallet pill**, no identity pill. The switcher is composable per page:
     `[wallet]`, `[wallet + identity]`, etc.
- **FR-GLOBAL-NAV-3 — The third pill is the identity/object relevant to THIS page's context (page-scoped).**
  The third segment is not hard-wired to "the app-global User identity" — it is **whatever identity/object the
  current page operates on**:
  - **On everyday-user pages** (Dashpay / Identities / Identity Hub): the app-global **User** identity.
  - **On the Masternodes page**: the **page-scoped masternode/evonode in view** (`Ⓜ mn-east-01 ▾`). Its dropdown
    lists the loaded masternode/evonode identities; it is **interactive and two-way bound** with the card grid
    and the detail view (opening a card sets the pill; picking from the pill opens that node).

  On the Masternodes page **both pills are interactive**: the **wallet pill** (funds Top up on this node — FR-9,
  two-way bound; NOT a key-derivation source, see §9 auto-derive correction) **and** the **masternode pill**
  (the node in view).

  **Why this does NOT violate FR-6 (the boundary — read carefully).** FR-6 forbids MN/Evonode identities from
  appearing in the **everyday-user Identity Hub / Identities picker**. The masternode-in-view selection here is a
  **separate, page-scoped selection**, distinct from the app-global User-identity selection. Picking a masternode
  in the Masternodes switcher does **not** write the app-global user-identity selection and therefore **never**
  makes that masternode appear in the identity pill on user pages, nor in the Hub picker. The Masternodes page's
  own switcher is not the everyday-user picker FR-6 governs. Two selections, two scopes, one clean boundary.
- **FR-GLOBAL-NAV-4 — Selection filtering already respects FR-6.** Because FR-6 filters MN/Evonode out of the
  identity picker/dropdown, the global identity pill's dropdown lists only User identities on every page — no new
  leak is introduced by making the switcher global.
- **FR-GLOBAL-NAV-5 — Sub-screen navigation stays in the content panel.** Pushed sub-screens (the load form, a
  masternode detail view) show their own lightweight back row **inside the content panel** (e.g. `‹ All masternodes`),
  keeping the global switcher single-line and unchanged across navigation (design-spec §A.3 "the topbar stays
  single-line").
- **FR-GLOBAL-NAV-6 — Leftmost breadcrumb is page-aware.** Segment-1 reflects the active tab and links to its
  root (`Masternodes › 💼 wallet › 👤 identity`) — confirmed answer to the earlier Q1.

**Flag for the implementation plan (Nagatha):** the mechanism is a **per-page capability declaring which
selections it consumes**, plus a **two-way binding** between that page and the selection, with
**disabled/tooltip rendering + `TODO` markers** on pages not yet wired. Crucially, the set of selections is more
than one axis: there is the **app-global User-identity selection** (consumed by the everyday-user pages) **and a
distinct page-scoped masternode/evonode selection** (consumed by the Masternodes page). These must be kept
separate so a masternode choice never bleeds into the app-global user-identity selection (FR-6 boundary). This is
the blast-radius-limited rollout — the nav appears everywhere on day one; interactivity per pill lands
page-by-page.

**Acceptance criteria (US-7 below).** This requirement is **Should** for the Masternodes deliverable but **Must**
as a shared prerequisite: the Masternodes page cannot ship its header without the global switcher existing.

---

## 5. Non-Functional Requirements

- **NFR-1 Reuse, do not reinvent.** Reuse `identity_picker_card.rs` (card + badge), the empty-state card
  pattern, `StyledButton`/`ComponentStyles` buttons, `MessageBanner`, breadcrumb header, and the left nav
  rail. Consult `src/ui/components/README.md` before adding any new widget.
- **NFR-2 No model changes.** No changes to `IdentityType`, `associated_voter_identity`, `PrivateKeyTarget`,
  or the DPNS voting backend. The page is a view + filter over existing types.
- **NFR-3 Design tokens only.** All colours/spacing/typography via `DashColors` / `Spacing` / `Typography` /
  `Shape` — no hardcoded values (see `docs/ux-design-patterns.md`).
- **NFR-4 Key-protection awareness (non-blocking) + reuse the existing protect path.** Surface, do not solve.
  MN/Evonode owner/voting/payout keys load **unprotected (Tier-1)** — the load flow has no password field, by
  design. The load form shows a one-line, actionable Warning-tone note that says protection can be added after
  loading. Do **not** design the encryption (it already exists as Tier-2 `IdentityTask::ProtectIdentityKeys`);
  do **not** gate the load flow on it. The detail view **surfaces the existing "Add password protection…"
  affordance** for the node's keys (reuse, not new crypto) and reflects the current protection tier.
- **NFR-5 i18n-ready copy.** All proposed strings are complete sentences with named placeholders, no fragment
  concatenation (per project string style).
- **NFR-6 Accessibility.** WCAG 2.1 AA: card is a single labelled click target (`WidgetInfo::labeled`),
  focus order top-to-bottom, disabled controls carry disabled-tooltips, status is never colour-only (pair the
  status dot with a text label). Note egui's limited screen-reader support (documented constraint).
- **NFR-7 Progressive disclosure — REVISED 2026-07-09 (supersedes the original wording).** This is a
  Power-User surface; it is a sibling root tab, and **is gated behind Expert Mode** (`is_developer_mode()`) per
  FR-1 — the original "not gated behind developer mode" language is corrected by explicit decision. Keep the
  everyday-user surfaces (Hub/Identities) unchanged apart from the FR-6 filter.

---

## 6. User Stories & Acceptance Criteria

**US-1 — Load a masternode by keys.**
*As a masternode operator, I want to load my masternode by its ProTxHash and DIP3 keys on a dedicated page,
so that I don't have to dig through the generic identity-load advanced options.*
- **Given** I open the Masternodes tab with no nodes loaded, **When** I click "Load a masternode", **Then** I
  see a form with ProTxHash, a Masternode/Evonode toggle, optional alias, and Voting/Owner/Payout key fields.
- **Given** the form, **When** the ProTxHash field is empty, **Then** the "Load masternode" button is disabled
  and its tooltip explains a ProTxHash is required.
- **Given** a valid ProTxHash and (optionally) keys, **When** I click "Load masternode", **Then** the node is
  loaded and appears as a card in the Masternodes list.
- **Given** I entered private keys, **When** I view the form, **Then** a non-blocking note tells me the keys are
  stored unencrypted at rest on this device.

**US-2 — See my masternodes at a glance.**
*As a masternode operator, I want a card list of my loaded masternodes showing type, voter readiness, key
status, and voting status, so that I can assess each node in seconds.*
- **Given** ≥1 loaded node, **When** I open the Masternodes tab, **Then** each node is a card showing shortened
  ProTxHash, type badge (Masternode/Evonode), voter-identity readiness, key status, DPNS-voting status, and an
  identity status dot with a text label.
- **Given** a node with no voter identity, **When** I read its card, **Then** it clearly shows "No voting key".
- **Given** a node with an alias, **When** I read its card, **Then** the alias is the heading and the ProTxHash
  is shown beneath it.

**US-3 — Open a masternode and vote.**
*As a masternode operator, I want to open a node and vote on the DPNS contests it can vote on, so that I can
fulfil my node's governance role.*
- **Given** a card, **When** I click it, **Then** its detail view opens showing keys summary, voter identity,
  and the DPNS voting section.
- **Given** the detail view with active contested names, **When** I choose Abstain / Lock / a candidate for a
  name, **Then** the vote is dispatched through the existing DPNS voting backend.
- **Given** a node whose voter identity is missing, **When** I open the voting section, **Then** I am told a
  voting key is required and how to add one (rather than a raw error).

**US-4 — Remove a masternode.**
*As a masternode operator, I want to remove a masternode from DET, so that I can stop tracking a node I no
longer operate.*
- **Given** a node's detail view, **When** I click "Remove masternode", **Then** a confirmation dialog with a
  specific verb label appears.
- **Given** the confirmation, **When** I confirm, **Then** the masternode and its associated voter identity are
  forgotten and the card disappears from the list.

**US-5 — Keep the everyday surface clean.**
*As an everyday user, I want my Identity Hub to show only my personal identities, so that I'm never offered
node-operator actions that don't apply to me.*
- **Given** loaded masternode/evonode identities, **When** I open the Identity Hub or Identities picker, **Then**
  those identities do **not** appear there.
- **Given** the same, **When** I open the Masternodes tab, **Then** they **do** appear there.

**US-6 — RETIRED (auto-derive does not apply to masternode keys).**
Investigated 2026-07-09: `derive_keys_from_wallets` is hard-gated to `IdentityType::User` in
`backend_task/identity/load_identity.rs` — masternode voting/owner/payout keys are Core-side keys tied to the
node's ProRegTx, not part of any wallet's identity-auth HD tree, so none of the three can be auto-derived. This
story and its "Try to derive from loaded wallet" checkbox do NOT carry over to the load form (FR-4); keys are
always pasted manually there. The wallet pill's real purpose on this page is unrelated — it is the **funding
source for Top up** (FR-9), reflected in US-7's acceptance criteria.

---

**US-7 — Switch wallet/identity from anywhere (silent + two-way, blast-radius-limited).**
*As any user, I want the same wallet/identity switcher on every page, so that I can see and change who I'm acting
as without leaving the current page.*
- **Given** I am on the Masternodes tab (or any root tab), **When** I look at the top panel, **Then** I see the
  page-aware switcher `Masternodes › 💼 wallet › 👤 identity` rendered with the Identity Hub's styling.
- **Given** the switcher on the Masternodes tab, **When** I switch wallets from the wallet pill, **Then** the
  app-global wallet context updates **in place (no navigation)**, and it becomes the funding source the next
  time I use **Top up** on a node (FR-9) — two-way: changing the source wallet from a Top-up flow also updates
  the pill. (NOT a key-derivation source — see §9 auto-derive correction; US-6 retired.)
- **Given** the Masternodes tab, **When** I open the **masternode pill** dropdown, **Then** it lists my loaded
  masternode/evonode identities, and choosing one opens that node — two-way bound with the card grid and detail
  view (opening a card updates the pill; picking from the pill opens the node).
- **Given** I pick a masternode in the Masternodes switcher, **When** I later open an everyday-user page
  (Dashpay / Identities / Identity Hub), **Then** the identity pill there shows my app-global **User** identity —
  the masternode never appears there (page-scoped selection; FR-6 boundary holds).
- **Given** a page that does not consume a given selection, **When** I hover its pill, **Then** it is dimmed with
  no caret and a **tooltip tells me how to change that selection** — there is no visible "read-only" text tag.
- **Given** a page with no identity/object context (e.g. a Wallet page), **When** I look at the switcher, **Then**
  it shows only the wallet pill (per-page composition), not a third pill.

**US-8 — Encrypt my node keys at load time.**
*As a masternode operator, I want to set an optional password when I load my node, so that its private keys are
encrypted at rest immediately instead of only after a separate step.*
- **Given** the load form, **When** I leave the encryption password blank, **Then** the node loads with keys
  unprotected (Tier-1) and I can protect them later from the Key Info screen / the detail view.
- **Given** the load form, **When** I enter an encryption password and load the node, **Then** the entered
  voting/owner/payout keys are sealed encrypted-at-rest (Tier-2) at load time.
- **Given** the password field, **When** I toggle the show/hide eye, **Then** I can reveal the password only
  while pressed (per the password-input pattern), and it is never logged or persisted in plaintext.
- **Given** a node loaded with a password, **When** I view its detail, **Then** its keys read
  "password-protected" and the "Add password protection…" action is not offered (already protected).

**US-9 — Move a node's credits.**
*As a masternode operator, I want to withdraw, top up, and transfer a node's Platform credits from its detail
view, so that I can manage its balance without leaving the Masternodes page.*
- **Given** a node's detail view (Masternode or Evonode), **When** I open the actions row, **Then** I can choose
  Withdraw, Top up, or Transfer, each opening the existing screen scoped to this node.
- **Given** a withdraw with the **owner** key, **When** I set it up, **Then** the destination is forced to the
  node's registered Core payout address; **Given** the transfer/payout key, **Then** I may choose a free address.

**US-10 — Manage a node's keys.**
*As a masternode operator, I want to open the key screen for a node, so that I can view a private key/WIF, sign a
message, or add/remove a key.*
- **Given** the Keys section, **When** I click "Manage keys ›", **Then** the existing `KeyInfoScreen` opens for
  this node.
- **Given** the add-key purpose selector, **When** I pick a purpose, **Then** OWNER and VOTING are not
  offered (Core-registered roles), while TRANSFER / AUTH / ENCRYPTION / DECRYPTION are.

**US-11 — Claim an evonode's token rewards.**
*As an evonode operator, I want to jump to token-reward claiming from the node's detail view, so that I can
collect rewards my evonode earned.*
- **Given** an **Evonode** detail view, **When** I look at the actions, **Then** "Claim token rewards ›" is
  shown and routes to the existing `ClaimTokensScreen` for this identity.
- **Given** a plain **Masternode** detail view, **When** I look at the actions, **Then** the token-rewards
  action is **not** shown.

## 7. Proposed Copy (i18n-ready)

- Tab label: `Masternodes`
- Empty-state heading: `No masternodes loaded`
- Empty-state body: `Load a masternode or evonode to vote on DPNS name contests and manage its owner and payout keys.`
- Empty-state primary button: `Load a masternode`
- Empty-state reassurance line *(canonical — resolves a wording drift found in test-spec review 2026-07-09;
  `wireframes.html`'s wording wins as the human-approved visual mock)*: `Have your node's ProTxHash to hand.
  Keys are optional — a node loads read-only without them.`
- Load form title: `Load a masternode`
- Load form subtitle: `Load a masternode or evonode that already exists on the Dash network.`
- ProTxHash label: `ProTxHash` · hint: `Enter the node's ProTxHash. You can find it in your masternode configuration.`
- Node type toggle: `Masternode` / `Evonode`
- Alias label: `Alias (optional)` · hint: `An alias helps you recognize this node inside Dash Evo Tool. It is not saved to the Dash network.`
- Key labels: `Voting private key`, `Owner private key`, `Payout address private key` · placeholder: `Private key (WIF or hex)`
- ~~Auto-derive toggle~~ / ~~No-wallet hint~~ — **removed** (leftover from a superseded pass; found and purged in
  test-spec review 2026-07-09). Auto-derive does not apply to these key roles — see §9 correction. There is no
  wallet-dependent copy on the load form; the wallet pill's job is unrelated (funds Top up, FR-9).
- Fill Random button *(FR-12, Testnet-only, rendered only when the fixture is present)*: `🎲 Fill Random
  Masternode` / `🎲 Fill Random Evonode` (label follows the Node-type toggle) · hint: `Testnet-only dev
  convenience — visible only when a local test-node fixture is found.`
- ProTxHash format error *(new, resolves test-spec gap #5, 2026-07-09; inline, on-blur, per project error-copy
  rules — what happened + what to do)*: `This doesn't look like a valid ProTxHash. Enter a hex or Base58
  ProTxHash from your masternode configuration.`
- Duplicate-node error *(new, resolves test-spec gap #5; surfaced at submit, MessageBanner Error)*: `This
  masternode is already loaded. Open it from the list instead of loading it again.` (Base58/hex ProTxHash or
  alias included per the project's Base58-IDs-are-allowed rule, e.g. "…already loaded as `mn-east-01`.")
- Encryption password label: `Encryption password (optional)` · placeholder: `Password to encrypt these keys` · helper: `Set a password to encrypt these keys on this device. Leave it blank to store them unencrypted and add protection later.`
- Key-storage note (Warning tone, actionable): `Set an optional password to encrypt these keys on this device. Without one, they are stored unencrypted and you can add protection later from the key screen.`
- Detail protection-tier labels: `Keys: unprotected` / `Keys: password-protected`
- Add-protection action: `Add password protection…`
- Load button: `Load masternode` · disabled tooltip: `Enter a ProTxHash to continue.`
- Card voter-ready: `Voting ready` / card voter-absent: `No voting key`
- Card voting status examples: `{count} contests to vote on` · `Vote scheduled` · `No open contests`
- Detail remove button: `Remove masternode` · confirm dialog verb: `Remove masternode`
- Voting section empty: `There are no open name contests for this node to vote on right now.`
- Missing voter identity at vote time: `This node has no voting key loaded. Add its voting private key to cast votes.`

---

## 8. Prioritized Backlog (MoSCoW)

**Must**
- FR-1 Masternodes root tab · FR-2 empty state · FR-3 card list · FR-4 load flow · FR-6 Hub/Identities filter.
- NFR-2 (no model changes), NFR-3 (tokens), NFR-4 (plaintext note), NFR-6 (a11y).

**Should**
- FR-5 detail/voting view (surfacing existing DPNS voting) · FR-7 refresh.
- ~~US-6 auto-derive parity~~ — **retired**, does not apply to masternode keys (see §9 correction).
- **FR-8 optional load-time encryption password** (needs new plumbing through `IdentityInputToLoad` →
  `load_identity` → the existing `store_protected` seal path — implementation scope for Nagatha's plan).
- **FR-9 credit actions** (Withdraw / Top up / Transfer) · **FR-10 Manage-keys drill-in** (`KeyInfoScreen`) ·
  **FR-11 Evonode-only token-rewards cross-link** (`ClaimTokensScreen`) — all **reuse existing screens** scoped to
  the selected node; only new entry points, no new operation UI.
- **FR-12 "Fill Random Masternode/Evonode" dev convenience** (Testnet-only, reuses existing
  `fill_random_masternode()`/`fill_random_hpmn()` + `.testnet_nodes.yml` fixture; visible only when the
  fixture is present — not shown-but-disabled; the page-level Expert Mode gate from FR-1 covers the rest).

**Could**
- ~~Per-key "auto-derived vs pasted" provenance indicator~~ — **moot**: no key role on this page is ever
  auto-derived (see §9 correction); all three are always pasted.
- Sort/filter of the card grid (by type, voter readiness, open-contest count) — aligns with Priya's
  "asset lock table is too compact / no sort or filter" pain point.

**Should**
- Surface the protection tier + **"Add password protection…"** action on the detail view (reuses the existing
  `IdentityTask::ProtectIdentityKeys` — no new crypto). This is the recourse the load-form note points to (NFR-4).

**Won't (this iteration)**
- Designing/building any **new** key-encryption mechanism — FR-8 reuses the existing Tier-2 envelope
  (`store_protected` / `put_secret_protected`); it only threads a password through to it.
- Making the load-time password mandatory — it is strictly optional; blank preserves today's Tier-1 behaviour.
- **Register DPNS name for MN/Evonode — DROPPED (out of scope).** In v0.10-dev this was gated to
  `identity_type = 'User'` (`database/identities.rs:344`); for MN/Evonode the button was a **silent no-op**, so
  it is not real parity. Adding functional DPNS-name registration for provider identities would be a **new
  feature**, not preservation — excluded here.
- Building any new operation screen — FR-9/10/11 **reuse** `withdraw_screen` / `top_up_identity_screen` /
  `transfer_screen` / `KeyInfoScreen` / `ClaimTokensScreen` scoped to the node; only entry points are new.
- Registering *new* masternode identities (this page is load/manage of existing on-chain nodes).
- Full in-page DPNS contest browser — the dedicated DPNS root screens remain the canonical voting surface;
  the detail view surfaces/hands off, it does not duplicate.

---

## 9. Open Questions & Assumptions

**Resolved (see §"Locked decisions"):** the four original open questions (nav placement, voting depth, filter
scope, auto-derive scope) are all locked as of 2026-07-09. **Auto-derive scope was subsequently superseded**
by a post-acceptance investigation — see decision 4's strikethrough entry in "Locked decisions" for the
correction (the load form has no auto-derive affordance at all; US-6 retired).

**Global-nav questions — all RESOLVED:**
1. **Segment-1 label** → **page-aware** (`Masternodes › 💼 wallet › 👤 identity`); segment-1 reflects the active
   tab and links to its root. (Confirmed; FR-GLOBAL-NAV-6.)
2. **Interaction on non-identity pages** → **silent context change + two-way binding, NOT route-to-Home.**
   Selecting an object in the nav silently updates the app-global selection with no forced navigation; where the
   page consumes that object the nav and page stay in sync both ways. On unwired pages the pill is read-only with
   a `TODO`. (Confirmed; supersedes the earlier "route to Identity Home" framing — see FR-GLOBAL-NAV-2.)
3. **Third-pill scope** → **page-scoped, page-aware object.** The third pill is the identity/object the current
   page operates on: the app-global **User** identity on everyday-user pages; the **page-scoped masternode/evonode
   in view** on the Masternodes page (interactive, two-way bound with the card grid + detail). This supersedes the
   earlier "Option (a) / read-only on Masternodes" resolution. It keeps FR-6 intact because the masternode
   selection is a *separate scope* from the app-global user-identity selection and never leaks into the
   everyday-user picker. Rationale in `02-ux-spec.md` §"Global nav — design question".

**Assumptions (documented):**
- No model or backend-task changes are needed; the page is a filtered view + card layout + relabelled load form.
- ProTxHash display uses the existing `shorten_id` helper and hex encoding for MN/Evonode.
- The plaintext-at-rest note is awareness-only and does not block the flow (per brief + NFR-4).
- Masternode/Evonode badge colours reuse the existing `draw_type_badge` mapping (purple / blue).

---

## 10. Requirements Quality Checklist

- [x] Primary actor (Priya) has stories addressing her primary goal (load + vote).
- [x] Every user story has testable Given/When/Then acceptance criteria.
- [x] ≥3 real-life scenarios covered across US-1…US-11 (load, glance, vote, remove, clean-surface, global-nav
      switching, encrypt-at-load, credit actions, key-mgmt drill-in, evonode token rewards). US-6 retired.
- [x] Edge/failure modes addressed: empty state, missing voter identity, no wallet loaded, disabled load button.
- [x] Priorities justified (Must = core operator flow + surface hygiene; Won't = out-of-scope crypto/registration).
- [x] No requirement without traceable justification (audit + personas + model).
- [x] Assumptions explicit; success metric tied to persona (≤10s to key paths).

---

## Locked decisions (accepted 2026-07-09)

The human reviewed the wireframes and confirmed all four open questions. Every answer matches the
wireframe as-drawn — no visual changes required. These decisions are now binding for implementation:

1. **Voting depth = INLINE.** FR-5 / the masternode detail view casts votes **directly in-page** via the
   DPNS-contest voting table + **Cast votes** button (wireframe D). It is **not** a deep-link to the DPNS
   Active Contests root screen. The detail view surfaces the existing DPNS voting backend inline.
2. **Filter scope = HUB PICKER ONLY.** FR-6 filters masternode/evonode identities out of the **Identity Hub
   picker only**. The legacy Identities table (`src/ui/identities/identities_screen.rs`) **keeps** showing
   MN/Evonode identities for now. Stripping them from the legacy Identities table is an **explicit deferred
   follow-up PR — out of scope for this iteration, and not to be treated as a regression** (supersedes
   Open Question §9.3 and the "Won't" note).
3. **Nav placement = BELOW IDENTITY HUB.** Left-nav order: Dashpay / Identities / Identity Hub / **Masternodes**
   / Contracts / Dash. Use a distinct node/server glyph (not the person glyph used by Identities). Resolves
   Open Question §9.1.
4. ~~**Auto-derive = all three key roles**~~ — **SUPERSEDED (2026-07-09, post-acceptance investigation).**
   This decision assumed "matching current behaviour" without verifying that behaviour existed for masternode
   keys. It does not: `derive_keys_from_wallets` is hard-gated to `IdentityType::User`; masternode voting/
   owner/payout keys are Core-side keys tied to the ProRegTx, never part of a wallet's HD tree, so none of the
   three is derivable today, for any identity type on this page. **Corrected decision: the load form has no
   auto-derive affordance; all three keys are always pasted manually (FR-4).** The wallet pill remains
   interactive on this page for an unrelated, verified reason — it is the funding source for **Top up**
   (FR-9). US-6 is retired, not confirmed. See §4c in 02-ux-spec.md and §9 below for the full correction.

---

🍬 **Findings tally** — surfaced during requirements analysis: **3** (Info severity):
(1) MN/Evonode identities leak into the everyday-user Identity Hub picker → FR-6 filter;
(2) load path is buried behind *Show Advanced Options* → FR-1/FR-4 extraction;
(3) MN/Evonode keys load unprotected (Tier-1) with no recourse shown to the user → NFR-4 actionable awareness note
+ surface the existing Tier-2 "Add password protection…" (`IdentityTask::ProtectIdentityKeys`) on the detail view.

---

## 10. Resolved gaps (test-spec review, 2026-07-09)

Marvin's Phase 1c test case specification (`03-test-case-spec.md`) surfaced 12 requirement gaps while writing
test cases against this document. Two were copy inconsistencies, fixed directly in §7 and in `02-ux-spec.md`
(empty-state reassurance line canonicalized; the retired auto-derive/no-wallet copy purged from §7). The
remaining decisions, made here so Nagatha's plan and Marvin's test cases build on settled ground rather than
open questions:

1. **DPNS card status-line precedence (FR-3).** Three possible strings can apply simultaneously
   (`{count} contests to vote on`, `Vote scheduled`, `No open contests`). **Precedence: open-contest count
   first (it's actionable), then scheduled, then none** — i.e. show `{count} contests to vote on` whenever
   `count > 0`, regardless of a pending scheduled vote; only show `Vote scheduled` when `count == 0` and a
   vote is pending; otherwise `No open contests`. **"Vote scheduled" is not a new concept** — reuse the same
   pending/scheduled-vote state the existing DPNS Scheduled Votes root screen already tracks (no new backend
   state; a display-layer read of existing data).
2. **Legacy buried Masternode/Evonode arm (`add_existing_identity_screen.rs`'s Identity Type dropdown under
   Show Advanced Options) — REMOVE, don't leave dangling.** Once FR-4 ships its own dedicated load flow, the
   old arm's Masternode/Evonode options are removed from that dropdown (User remains). This prevents two
   competing entry points for the same action, and matches the "carve masternode handling out of the generic
   identity flow" framing this whole design started from. Implementation scope for Nagatha's plan; extends FR-6.
3. **FR-8 password-strength rule.** No new policy is invented. The optional load-time password reuses the
   *same* validation (if any) the existing Key Info screen's "Add password protection…" flow already applies,
   since FR-8 routes through the identical `store_protected`/`put_secret_protected` seal path (§ FR-8). Nagatha
   confirms the existing rule at implementation time rather than this design inventing a new one.
4. **Masternode-pill state when navigating detail → list.** Already implicit in the wireframes, now stated
   explicitly: the pill reflects **the current screen's context**, not "last node opened." On the card-list /
   empty screens (A, B) it shows the placeholder `Choose a masternode ▾`; only the detail view (D) shows a
   specific node. Navigating from D back to the list (via `‹ All masternodes` or the pill's own dropdown)
   resets it to the placeholder — matches wireframe B exactly.

5. **No-wallet-loaded behavior when attempting Top up (FR-9).** Not a new copy/state to invent: Top up is an
   **existing reused screen** (`top_up_identity_screen`, per FR-9's reuse note) — whatever it already does
   today when no wallet is loaded (block, prompt, or otherwise) is what happens here too, unchanged. This
   design adds an entry point to that screen; it does not redefine its no-wallet behavior.
6. **Node-type toggle after Fill-Random autofill (FR-12).** Switching Masternode ↔ Evonode after using
   "Fill Random…" **clears** ProTxHash, Alias, and all key fields. A real node's identity is tied to one type
   — autofilled (or manually entered) data for one type is never valid for the other, so silently keeping it
   would be actively misleading, not a convenience.
7. **Past/scheduled votes on the detail view (FR-5) — out of scope, by design, not a gap.** The collapsible
   DPNS section covers **active, open contests only**, exactly as wireframe D draws it. Scheduled/past-vote
   history is not duplicated here — it already has a home in the existing DPNS Scheduled Votes root screen.
   Keeps the page focused; consistent with "reuse existing screens" rather than re-showing the same data twice.
8. **"Add voting key" (US-3, missing-voter-identity affordance) is a targeted action, not a re-run of FR-4's
   load form.** It opens a small, scoped key-input prompt that adds/updates the voter identity on the
   **already-loaded** node in place. It is a different flow from FR-4's load form and is therefore exempt from
   the duplicate-ProTxHash rejection below (that rejection guards *new* loads, not fixing up an existing one).
9. **Duplicate-ProTxHash load (FR-4) — reject, don't merge or duplicate.** Submitting a ProTxHash that's
   already loaded shows the duplicate-node error (§7 copy, added above) and does not create a second card or
   silently update the existing one. **Malformed-ProTxHash (FR-4)** — validated inline/on-blur (client-side
   shape check, hex or Base58), not only gated on emptiness; error copy added to §7 above.
10. **Network switch mid-sub-screen (load form or detail).** Matches TC-EDGE-05's existing rule ("card list
    scoped per active network"): switching network while on the load form or a node's detail view returns to
    the Masternodes **list** for the new network, rather than leaving a stale sub-screen referencing an
    identity that may not exist there. Consistent with root screens surviving network switches while
    identity-scoped sub-screens do not carry a now-foreign identity forward.
11. **Live de-gating fallback (FR-1).** If Expert Mode is turned off while the Masternodes tab is the active
    screen (no existing DET precedent found for a dev-gated *root tab* specifically — checked `app.rs` and
    found none to reuse), the app falls back to the **Identities** root screen — the nearest neutral,
    always-available screen, rather than leaving the user stranded on a tab that just disappeared from the nav.

No open items remain from Marvin's gap list that require a return to Phase 1a/1b (UX Design) — all eleven are
implementation-detail-level and are resolved above without changing any wireframe screen.
