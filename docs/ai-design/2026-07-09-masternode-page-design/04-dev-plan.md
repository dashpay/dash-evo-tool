# Masternodes Page — Development Plan (Phase 1d)

**Repo:** `dash-evo-tool` · **Branch:** `feat/masternodes-tab` (base `v1.0-dev`) · **Date:** 2026-07-09
**Author:** Nagatha (Software Architect) · **Phase:** 1d — implementation decomposition. Planning only; no Rust written.

Inputs (all final, human-accepted 2026-07-09): `01-requirements.md` (FR-1…FR-12, FR-GLOBAL-NAV,
NFR-1…NFR-7, US-1…US-11, §10 Resolved gaps), `02-ux-spec.md`, `wireframes.html`, `03-test-case-spec.md`
(164 cases), `v010-masternode-features.md`. Grounded against the live tree: `src/ui/mod.rs`,
`src/ui/components/{top_panel,left_panel}.rs`, `src/ui/identity/breadcrumb_switcher.rs`,
`src/ui/state/hub_selection.rs`, `src/backend_task/identity/{mod,load_identity,protect_identity_keys}.rs`,
`src/model/feature_gate.rs`, `src/app.rs`.

> **Review fixes folded in (2026-07-09):** Fable/Adams reviewed this plan — APPROVE WITH FIXES, 0 CRITICAL/HIGH,
> 4 MEDIUM + 9 LOW (`fable-review.md`). PROJ-001, -002, -003, -005 (MEDIUM) and PROJ-004, -006, -007, -008, -009,
> -010, -011, -012, -013 (LOW) are folded into the affected tasks below and into `01-requirements.md` /
> `03-test-case-spec.md` / `02-ux-spec.md`. Changes are spec-level; the plan's architecture is unchanged.

Two structural facts from the live tree that shape the whole plan:
1. **The nav rail already accepts a per-entry `FeatureGate`** (`left_panel.rs:237-243` skips entries whose gate is
   unavailable; runtime-gate precedent is the `DashPay` entry). FR-1's Expert-Mode gate is therefore a *data*
   change (one gated entry), not new machinery.
2. **The top-panel breadcrumb seam already exists** (`top_panel::add_top_panel_with_breadcrumb` →
   `render_top_island`, `top_panel.rs:378-385`). The switcher itself (`breadcrumb_switcher::render`) is, however,
   **hub-hardwired**: a literal `"Identities"` segment-1, hub-only pill semantics, and effect application living
   inside `hub_screen.rs::apply_breadcrumb_effect`. Phase A generalizes exactly this.

---

## 1. System Layers & Responsibilities

Per DET's Module Placement Policy. Every task below names its layer(s); this is the map.

| Layer | This feature's responsibilities | Files (new *n* / modified *m*) |
|---|---|---|
| **model/** (pure, no IO) | ProTxHash shape validation (hex/Base58) as a stateless validator; **relocation** of the existing password-format validator into `model/` (see B0/PROJ-006 — it currently lives, private, in `backend_task/identity/protect_identity_keys.rs:156`, contrary to DET's Validation-placement rule). Global-nav page-scope enum lives in `ui/state`, not here (it is view state, per the discriminator). | `model/masternode_input.rs` *n* (ProTxHash validator); relocate `validate_protection_password` → `model/` *m* |
| **backend_task/** (async, authoritative enforcement) | Thread the optional load-time password through load; authoritative duplicate-ProTxHash rejection; masternode+voting refresh; reuse withdraw/top-up/transfer/protect/vote tasks unchanged. `TaskError` variants for duplicate/malformed. | `backend_task/identity/{mod.rs,load_identity.rs}` *m*; `backend_task/error.rs` *m* |
| **database/** | No new module. MN/Evonode rows already persist as `QualifiedIdentity`. Read paths reuse existing accessors; the per-node contest read (B1/PROJ-012) composes from the existing contested-names store. | — |
| **context/** (glue) | Wrapper read methods: masternode-only identity list for the active network; **User-only accessor consumed at the resolution layer** so `resolve_selected_identity()` and wallet-reconciliation can never resolve a masternode as the everyday-page identity (FR-6, PROJ-001); one-time sanitization of a stale persisted MN/Evonode selection; live-de-gating fallback helper. | `context/mod.rs` (or `context/identity_db.rs`) *m* |
| **ui/state/** (renders nothing) | The page-scoped nav model: which pills a page composes, which selections it consumes, and the **page-scoped masternode selection kept distinct from the app-global user-identity selection** (FR-6 boundary). Masternodes-screen view state. | `ui/state/global_nav.rs` *n*; `ui/state/masternodes_view.rs` *n* |
| **ui/components/** (renders egui) | Generalized global switcher built on the existing `BreadcrumbPill` modes (`Interactive`/`Subdued`/`Placeholder` — no new pill widget). | `ui/components/global_nav_switcher.rs` *n* (extracted from `breadcrumb_switcher.rs`) |
| **ui/masternodes/** (new screen domain) | The Masternodes root screen, empty state, card grid + card body, load form, detail/voting view, Fill-Random, entry points into reused screens. No business logic, no own validation. | `ui/masternodes/{mod,list_screen,load_form,detail_screen,card}.rs` *n* |
| **ui/mod.rs, app.rs** (shell) | New `RootScreenType::RootScreenMasternodes`, `ScreenType::Masternodes`, `Screen::MasternodesScreen`; root-screen registration; nav entry (gated); global-switcher render on every root screen. | `ui/mod.rs` *m*, `app.rs` *m*, `left_panel.rs` *m* |

**Reuse-first ledger (no new operation screens).** FR-9 → `withdraw_screen` / `top_up_identity_screen` /
`transfer_screen`; FR-10 → `KeyInfoScreen`; FR-11 → `ClaimTokensScreen`; FR-5 protect → `IdentityTask::ProtectIdentityKeys`;
FR-5 voting → `contested_names/vote_on_dpns_name.rs`; FR-12 → `fill_random_masternode()`/`fill_random_hpmn()` +
`.testnet_nodes.yml` loader; FR-3 card → `IdentityPickerCard` frame + `draw_type_badge`; header → `add_top_panel_with_breadcrumb`.
Each is an **entry point**, not a reimplementation — this is the QA dedup contract (TC-FR9-08, TC-FR11-04, TC-NAV-02).

---

## 2. Two-Phase Sequencing (coordinator decision)

- **Phase A — Global wallet/identity switcher on every root page (app-shell foundation).** Larger than this
  feature; lands and is reviewed as its own commit sequence *first*. The nav renders on every root page day one;
  interactivity is blast-radius-limited (Subdued + tooltip + `TODO` on unwired pages). The Masternodes page then
  *consumes* this, it does not build it.
- **Phase B — The Masternodes page**, built on top, inheriting the global nav.

Phase B B7 (page nav wiring) has a hard dependency on Phase A. **B1 now also depends on A2** (PROJ-005): the FR-6
filter lands in the durable context accessor consumed by both switcher generations, so B1 does not patch the file
A2 rewrites. All other Phase B tasks can proceed against a Subdued/placeholder header until B7 lands.

---

## 3. Phase A — Global Nav (app-shell)

### A1 — Page-nav model & the two-scope selection abstraction *(ui/state)*
**Files:** `ui/state/global_nav.rs` *n* (+ unit tests inline).
**Work:** Define `PageNavSpec` describing, per root page: segment-1 `(label, RootScreenType target)`
(page-aware, FR-GLOBAL-NAV-6); pill composition (`wallet?`, `identity/object?` — FR-GLOBAL-NAV-2 rule 4); and
per-pill consumption mode (`Consumed{two-way}` vs `Unwired{how-to-change tooltip}`, rule 3). Define
`IdentityPillScope` = `AppGlobalUser` **|** `PageScopedObject{ label, dropdown items, selected }` — this enum is
the structural guarantee behind the FR-6 boundary: the page-scoped variant **never** writes
`AppContext::selected_identity_id`. Pure state; renders nothing (placement discriminator → `ui/state`).
**Tests (TDD):** spec resolves correct pill set per page; `PageScopedObject` selection is isolated from app-global
identity; unwired-pill mode carries a non-empty tooltip.
**Satisfies:** TC-NAV-13, TC-NAV-14, TC-NAV-15 (composition/subdued/tooltip logic); foundation for TC-NAV-12, TC-FR6-07.

### A2 — Generalize `breadcrumb_switcher` into a page-aware global switcher *(ui/components)*
**Files:** `ui/components/global_nav_switcher.rs` *n* (extract/rewrite of `ui/identity/breadcrumb_switcher.rs`);
keep a thin hub-facing shim so `hub_screen.rs` compiles unchanged in behavior.
**Work:** `render(ui, ctx, spec: &PageNavSpec, view_state) -> GlobalNavEffect`. Segment-1 renders `spec` label +
links to `spec` root (replaces the literal `"Identities"`). Pills compose per `spec`: consumed pills render
`Interactive` (caret + dropdown) and emit two-way effects; unwired pills render `Subdued` (dimmed, **no caret, no
visible text tag**) with the how-to-change tooltip and a `// TODO: wire <selection> on this page` marker. Reuse
`BreadcrumbPill`/`IdentityPill` and the existing `BreadcrumbPillMode` verbatim — **no new pill widget** (NFR-1).
Generalize `BreadcrumbEffect` → `GlobalNavEffect` (adds `SelectPageObject(...)` for the page-scoped pill, kept
distinct from `SelectIdentity`). The identity-list source for the pill dropdown reads through the **User-only
context accessor** (B1), so the FR-6 filter lives in the accessor, not this component (PROJ-005).
**Tests:** segment-1 label is page-driven; a page-scoped-object selection produces `SelectPageObject`, never
`SelectIdentity`; subdued pill emits no effect on click.
**Satisfies:** TC-NAV-01, TC-NAV-02, TC-NAV-03, TC-NAV-13, TC-NAV-16.

### A3 — Render the switcher on every root screen + centralize effect application *(app-shell)*
**Files:** each root screen's top-panel call site → `add_top_panel_with_breadcrumb` with its `PageNavSpec`; a shared
`apply_global_nav_effect` (lift `hub_screen.rs::apply_breadcrumb_effect` to a shared helper) so wallet/identity
selection updates the **app-global** selection *silently, with no forced navigation* (rule 1). Every root screen not
yet wired supplies a Subdued spec (+ `TODO`). Hub keeps its existing interactive wallet/identity pills (regression).
**Work:** wiring + one shared effect applier. `SwitchWallet`→`set_selected_hd_wallet` (silent);
`SelectIdentity`→`set_selected_identity` (silent); `SelectPageObject`→ handled by the owning page (B7).
**Reconciliation semantics are not a pure wallet write (PROJ-010):** `set_selected_hd_wallet`
(`context/mod.rs:1152-1175`) reconciles the app-global *identity* to the new wallet's identities as a side effect
(keep-if-owned → first → `None`); on non-Hub pages this cross-axis mutation is real and intentional — A3 must
document it, and combined with B1's resolution-layer filter it must **never** reconcile onto an MN/Evonode.
Blast radius: only Hub (and, after B7, Masternodes) are interactive; all else Subdued.
**Tests:** kittest — switcher present on ≥2 non-Hub root screens; wallet selection does not navigate;
**wallet switch on a non-Hub page reconciles identity per existing rules and never onto a MN/Evonode**; unwired
identity pill on an everyday page never lists MN/Evonode (re-asserted in TC-NAV-17).
**Satisfies:** TC-NAV-06 (silent, no-nav), TC-NAV-15, TC-NAV-16; enables TC-NAV-12/17.

**Phase A rollout/TODO placement:** every root screen that does not yet consume a selection gets a `Subdued` pill and
an explicit `// TODO: wire wallet/identity selection consumption for <page>` at its `PageNavSpec` construction — the
page-by-page wiring backlog. Interactivity is opt-in per page; this bounds the app-shell blast radius.

---

## 4. Phase B — Masternodes Page (dependency order)

### B0 — FR-8 load-time key encryption plumbing *(model + backend_task)* — **no UI**
**Files:** `backend_task/identity/mod.rs` (`IdentityInputToLoad`, add `encryption_password: Option<Secret>` at
`mod.rs:43`); `backend_task/identity/load_identity.rs` (route persistence); `backend_task/identity/protect_identity_keys.rs`
(the seal path + `validate_protection_password:156`); **`src/mcp/tools/masternode.rs:180`** (the third
`IdentityInputToLoad` constructor — the MCP `masternode_identity_load` tool).
**Work:** when `encryption_password` is `Some`, seal the voting/owner/payout **and identity** keys via the existing
`store_protected`/`put_secret_protected` envelope (Argon2id + XChaCha20-Poly1305) at load time, through the
`wallet_backend/secret_seam.rs` chokepoint — **no new crypto, no second persistence path**. When `None`, current
Tier-1 keyless path is unchanged. Validate the password in the backend (enforcement) reusing the *existing* rule
(§10.3 — do not invent one); password is a `Secret`, never logged, never stored. Add typed `TaskError` variants for
duplicate/malformed ProTxHash here (used by B1/B4) rather than string parsing.
- **Password-validator placement (PROJ-006).** `validate_protection_password` is today a **private fn in
  `protect_identity_keys.rs:156`**, not a `model/` validator. Per DET's Validation-placement rule, pure
  password-format validation belongs in `model/`: **relocate it to `model/` (or expose `pub(crate)` if relocation
  is deferred)** so `load_identity` can call it — small, mechanical, in-scope for B0.
- **MCP constructor decision (PROJ-007).** Adding the field breaks `mcp/tools/masternode.rs:180` at compile time.
  **Decision: MCP passes `encryption_password: None` this iteration** (Tier-1 unchanged — matches FR-8's GUI-only
  scope; the tool is a confirmed keyless entry point, requirements §2.3). Leave a `TODO` for headless password
  parity as a follow-up; do not silently invent MCP password handling.
**Tests (RED-first):** blank→unprotected path; set→`put_secret_protected`; identity key also sealed (TC-FR8-10);
password never appears in logs; no plaintext at rest; MCP path compiles and stays Tier-1.
**Satisfies:** TC-FR8-01, TC-FR8-02, TC-FR8-04, TC-FR8-05, TC-FR8-09, TC-FR8-10.
**Depends on:** nothing. Start immediately (parallel to Phase A).

### B1 — Context read paths + FR-6 filter at the resolution layer + FR-7 refresh *(context + backend_task)*
**Files:** `context/mod.rs` (or `identity_db.rs`); `backend_task/identity/` (refresh).
**Work:**
- **New accessors:** `load_local_masternode_identities()` (active-network MN/Evonode — card-list + masternode-pill
  source) and a **User-only accessor** for the Hub picker + global identity pill.
- **FR-6 filter at the *resolution* layer, not the display call sites (PROJ-001 / PROJ-005 — R1-critical).** The leak
  is not confined to the two display sources (`breadcrumb_switcher.rs:148`, `hub_screen.rs:91,215`):
  `resolve_selected_identity()` (`context/mod.rs:1099-1105`) falls back to the **first loaded identity over ALL
  types** via `model::selected_identity::resolve_selected`, and `set_selected_hd_wallet` (`:1152-1175`) reconciles
  the app-global identity the same way. An operator whose only/first loaded identity is a masternode — exactly the
  Priya profile — would get it as the everyday-page operate-as identity with `IdentityPillScope` never involved.
  **Filter MN/Evonode inside the resolution path** (both the keep-if-loaded check and the first-loaded fallback, over
  all-loaded and over the per-wallet reconciliation source) via the User-only accessor. This is the durable seam and
  also feeds the two display sources; the legacy `identities_screen.rs` table stays untouched (locked decision #2).
- **One-time stale-selection sanitization (PROJ-001b).** Masternodes are pickable in the Hub *today*, so a persisted
  `selected_identity_id` may already point at an MN/Evonode. On context load, if the persisted selection resolves to
  `IdentityType != User`, clear it — otherwise the User-filtered pill and the MN-valued selection disagree and
  operate-as reads still resolve the masternode.
- **FR-7 refresh** (backend_task): re-fetch MN identity + **voting** state — compose from existing `refresh_identity`
  + the contested-names query; **do not** add a parallel fetcher.
- **Card DPNS read accessor (PROJ-012).** The card's per-node open-contest count + scheduled-vote state
  (TC-FR3-09/-10/-11, TC-DPNS-02) needs a **read** join over the existing contested-names store — name it here and
  compose from that store; if no existing query serves it, this is the second thin new piece alongside the refresh
  task (R4).
**Tests:** MN/Evonode absent from the picker/pill list **and** from `resolve_selected_identity()` even when it is the
only/first loaded identity; present in legacy table (control); User identity present in both; **stale MN persisted
selection cleared on load**; MN present in the masternode accessor.
**Satisfies:** TC-FR6-01…06, TC-NAV-12 (new preconditions TC-NAV-12b/12c), TC-NAV-17, TC-FR7-02, TC-FR7-03.
**Depends on:** **A2** (FR-6 filter lands in the durable context accessor both switcher generations consume; sequence
B1 after A2 to avoid the same-file collision — PROJ-005).

### B2 — Root tab: registration, Expert-Mode gate, nav, de-gating fallback *(ui/mod.rs + app.rs + left_panel)*
**Files:** `ui/mod.rs` (`RootScreenType::RootScreenMasternodes` + `to_int`/`from_int` round-trip with a fresh stable
integer + test, mirroring the IdentityHub precedent at `ui/mod.rs:161,194,205-215`; `ScreenType::Masternodes`;
`Screen::MasternodesScreen`; `create_screen`; `change_context` arm); `app.rs` root-screen registration (mirror the
hub chain at `app.rs:805-821`); `left_panel.rs` nav entry **gated `FeatureGate::DeveloperMode`**, positioned
**below Identity Hub** (locked decision #3), distinct node/server glyph (not `identity.png`).
**Work:** the gate reuses the existing per-entry `gate.is_available()` skip (`left_panel.rs:237-243`) — nav item and
route both absent when Expert Mode is off. **Live de-gating fallback (§10.11):** when Expert Mode flips off while
Masternodes is active, fall back to `RootScreenIdentities` (nearest neutral tab — no existing precedent to reuse, so
this is a small explicit guard in the screen-resolution path, analogous to the persisted-selection fallback at
`app.rs:829-833`). Network switch keeps the tab selected but resets any pushed sub-screen to the list (§10.10).
**Tests:** round-trip of the new variant; nav absent Expert-off / present + positioned Expert-on; distinct glyph id;
survives network switch; de-gating falls back to Identities.
**Satisfies:** TC-FR1-01…07, TC-FR1-05b, TC-EDGE-05, TC-EDGE-06.
**Depends on:** none structurally; do before B3–B7 (they need the screen to exist).

### B3 — Empty state + card grid + card body *(ui/masternodes)*
**Files:** `ui/masternodes/{list_screen,card}.rs`.
**Work:** Empty state (FR-2) reusing the `03-identities-empty.png` card pattern + exact §7 copy incl. the canonical
reassurance line. Card grid (FR-3) reusing `IdentityPickerCard` frame + `draw_type_badge` (purple/blue), extended
body rows: shortened ProTxHash / alias-as-heading, voter readiness, `V O P` key status across all 8 combinations,
DPNS status line with **count-first precedence** (§10.1 — `{count} contests` when `count>0`, else `Vote scheduled`
when pending, else `No open contests`; reads the per-node contest accessor from B1, reusing the existing
Scheduled-Votes state — no new backend concept), and the `IdentityStatus` dot+label (verified mapping, five states).
Whole card is one labelled click target (`WidgetInfo::labeled`, NFR-6). Responsive `minmax(260,1fr)`. Add the
top-right toolbar **Refresh** button here (FR-7).
**Tests (kittest):** empty↔grid boundary; badge colour/text per type; voter-ready vs no-voting-key; all 8 key combos;
precedence rows; five status states; single-click-target label; count == DB rows; Refresh button present/styled.
**Satisfies:** TC-FR2-01…07, TC-FR3-01…15, TC-FR7-01, TC-NFR6-01, TC-NFR6-03.
**Depends on:** B1 (list + contest read source), B2 (screen).

### B4 — Load form + validation + legacy-arm removal *(ui/masternodes + model)*
**Files:** `ui/masternodes/load_form.rs` *n*; `model/masternode_input.rs` *n* (ProTxHash validator);
`ui/identities/add_existing_identity_screen.rs` *m* (remove MN/Evonode options from the Advanced-Options Identity-Type
dropdown — User-only remains, §10.2 / TC-FR4-22).
**Work:** MN/Evonode-only form (FR-4): ProTxHash (required), Masternode/Evonode segmented toggle (default Masternode,
**no User option**), optional alias, V/O/P key inputs (reuse existing WIF-or-hex widget, hold-to-reveal), optional
encryption-password field (reuse `wallet_unlock.rs` hold-to-reveal; drives B0), always-visible Warning-tone note
(§7 copy), Load button disabled until ProTxHash present + disabled tooltip. **Validation delegates to model**
(NFR/layer rule): inline/on-blur ProTxHash shape check (hex or Base58) via the new `model/` validator; duplicate
detection is authoritative in the backend (B0/B1 typed error), surfaced as the §7 duplicate copy. Node-type toggle
**clears** ProTxHash/alias/keys (§10.6). **Explicitly no auto-derive affordance** (US-6 retired; assert absence).
**Tests:** field set incl. negative no-auto-derive assertion; toggle default/switch/clear; disabled+tooltip;
hex/Base58 accept; malformed inline; duplicate reject; friendly error banner + `.with_details`; cancel discards;
fresh form on reopen; legacy dropdown MN/Evonode removed.
**Satisfies:** TC-FR4-01…22, TC-EDGE-01, TC-EDGE-02, TC-EDGE-07, TC-EDGE-08, TC-FR8-03, TC-NFR4-01, TC-NFR6-02, TC-NFR6-04.
**Depends on:** B0 (password field target), B2.

### B5a — Detail view scaffold: header, actions row, keys, remove *(ui/masternodes, reuse-heavy)*
**Files:** `ui/masternodes/detail_screen.rs` *n*.
**Work:** Detail composition in the **corrected order (TC-FR5-01): Header → Actions row → Keys → DPNS → Remove.**
Header: alias (conditional) + shortened ProTxHash + copy-full-value + type badge + status. **Actions row (FR-9):**
entry points pushing the existing `WithdrawalScreen`/`TopUpIdentityScreen`/`TransferScreen` scoped to the node's
`QualifiedIdentity` (both MN and Evonode) — reuse, not reimplementation; Top up sources the wallet-pill wallet.
**Evonode-only** `Claim token rewards ›` cross-link → existing `ClaimTokensScreen` (hidden for Masternode, FR-11).
**Keys section:** V/O/P presence + voter-identity id (copyable) + protection tier (`unprotected`/`password-protected`);
`Add password protection…` → existing `IdentityTask::ProtectIdentityKeys` (offered only Tier-1); `Manage keys ›` →
existing `KeyInfoScreen` (FR-10, add-key selector already excludes OWNER/VOTING for all types — verify, don't add
logic). **Remove:** `ConfirmationDialog danger_mode(true)`, removes node + associated voter identity.
Content-panel `‹ All masternodes` back row (not in the global header). Add the detail-view **Refresh** button (FR-7).
**Tests:** section order (actions above keys); reuse identity scoping for all three credit actions + structural-reuse
assertion; Evonode-only cross-link present/absent; protection-tier display + conditional Add-protection; Manage-keys
opens `KeyInfoScreen`; Remove confirm deletes node+voter, isolation, back row; Refresh present on detail.
**Satisfies:** TC-FR5-01…05, TC-FR5-07, TC-FR7-04, TC-FR9-01…08, TC-FR10-01…13, TC-FR11-01…04, TC-FR8-06, TC-FR8-08,
TC-US4-01…07, TC-EDGE-04.
**Depends on:** B2, B3. **(PROJ-011)** For **TC-FR8-07** (detail reflects a *load-time-sealed* node, no redundant
Add-protection), the Tier-2-at-load precondition requires B0+B4 — that one case is verified in **B8's integration
pass**; B5a covers only the tier-display + conditional-action logic (reachable via a post-load `ProtectIdentityKeys`
seal), so TC-FR8-07 is listed under B8, not here.

### B5b — Detail view: inline DPNS voting + missing-voter path *(ui/masternodes, reuse voting backend)*
**Files:** `ui/masternodes/detail_screen.rs` (voting section).
**Work:** Collapsible section (**collapsed by default**, open-contest **count in header**). Expanded: per-contest
Abstain/Lock/Vote-for-candidate table + **Cast votes**, dispatching the existing
`contested_names/vote_on_dpns_name.rs` backend inline (locked decision #1 — **not** a deep-link). Candidate dropdown
scoped per contest. Active/open contests only (§10.7 — scheduled/past live on the existing Scheduled Votes screen,
not duplicated). **Missing-voter-identity (US-3/§10.8):** show the actionable §7 copy (never raw `NoVotingIdentity`)
+ `( Add voting key )` → a **scoped, in-place voter-key-input prompt** that updates the voter identity on the
already-loaded node (distinct from B4's load form; exempt from the duplicate-ProTxHash rejection). This is the
**§10.8-resolved design; TC-DPNS-10 is corrected in `03-test-case-spec.md`** from "load form opens pre-filled" to
"scoped prompt opens, node context pre-bound" so the pair TC-DPNS-10 / TC-DPNS-11 no longer contradict (PROJ-002).
Success banner auto-dismisses.
**Tests:** collapsed default; count in header; expand reveals choices; per-contest candidate scoping; Cast votes hits
the real backend with correct params; zero-open empty copy; missing-voter actionable message + scoped in-place prompt
(node pre-bound, NOT a load-form resubmission).
**Satisfies:** TC-DPNS-01…11, TC-EDGE-03.
**Depends on:** B5a.

### B6 — FR-12 Fill Random (Testnet-only, fixture-conditional) *(ui/masternodes, reuse loader)*
**Files:** `ui/masternodes/load_form.rs`.
**Work:** Reuse `fill_random_masternode()`/`fill_random_hpmn()` + `load_testnet_nodes_from_yml(".testnet_nodes.yml")`
(`add_existing_identity_screen.rs:961-993`). **One** button, label follows the toggle. **Render-conditional, not
disabled:** button+hint row present only when network == Testnet **and** the loader returns `Some(_)`; absent (0
widgets) otherwise.
- **Autofilled key set differs by node type (PROJ-003 — TC-FR12-07 corrected).** The regular-masternode fixture
  struct `MasternodeInfo { pro_tx_hash, owner, voter }` has **no payout field**, so `fill_random_masternode()` fills
  **ProTxHash + Voting + Owner only** (Payout stays blank). Only `fill_random_hpmn()` (Evonode / `hp_masternodes`)
  fills all three including Payout. This is the honest, low-cost option matching the fixture operators actually have;
  do **not** claim three-key autofill for the Masternode toggle. (Requirements FR-12 §7 and TC-FR12-07 corrected to
  match.)
- **Malformed-YAML is a deliberate behavior change, not verbatim reuse (PROJ-004 — TC-FR12-04).** The loader returns
  `Ok(None)` only for a **missing** file; a **malformed** file returns `Err(_)`, and the *legacy* screen banners it
  (`add_existing_identity_screen.rs:151-161`). The new form must **map `Err(_)` → absent button** (swallow; no
  banner, no panic) — a conscious divergence from the legacy screen. Add a `tracing::debug!` on the swallowed parse
  error so a broken fixture is diagnosable.
- Masternode toggle pulls `masternodes`; Evonode pulls `hp_masternodes`. Autofill respects the node-type clear rule
  (§10.6, shared with B4). **TC-FR12-09 decision — see §6:** add the defense-in-depth `is_developer_mode()` check.
**Tests:** render matrix (Testnet+fixture±toggle / missing / malformed→absent+no-banner / Mainnet / Devnet);
Masternode autofill = V+O (Payout blank); Evonode autofill = V+O+P; label follows toggle.
**Satisfies:** TC-FR12-01…08 (with the -07 correction), TC-FR12-09 (recorded decision), TC-FR12-10.
**Depends on:** B4.

### B7 — Wire the Masternodes page into the global nav *(ui/masternodes + ui/state)*
**Files:** `ui/state/masternodes_view.rs` *n* (page-scoped masternode selection); Masternodes screens' `PageNavSpec`.
**Work:** Provide the page's `PageNavSpec`: page-aware segment-1 `Masternodes`; **wallet pill Interactive + two-way**
(funds Top up — FR-9; changing it on a Top-up flow updates the pill and vice-versa); **masternode pill Interactive +
two-way** using `IdentityPillScope::PageScopedObject` — dropdown lists loaded MN/Evonode (B1 source), opening a card
sets the pill, picking from the pill opens that node's detail; placeholder `(no masternode yet)` on empty, `Choose a
masternode ▾` on list, specific node on detail, **reset to placeholder on `‹ All masternodes`** (§10.4). The
page-scoped selection lives in `masternodes_view.rs`, **never** in `AppContext::selected_identity_id` — this is the
FR-6 boundary in code (complementing B1's resolution-layer filter).
**Tests (critical):** card-click↔pill and pill↔detail two-way; dropdown content correctness; wallet-pill→Top-up and
Top-up→wallet-pill two-way; **masternode selection never leaks to app-global user-identity pill across Dashpay/
Identities/Hub (TC-NAV-12)**; pill placeholder/choose/reset states.
**Satisfies:** TC-NAV-04, TC-NAV-05, TC-NAV-07, TC-NAV-08, TC-NAV-09, TC-NAV-10, TC-NAV-11, TC-NAV-12, TC-NAV-18,
TC-FR5-06, TC-FR6-07, TC-FR9-04.
**Depends on:** **Phase A (A1–A3)**, B1, B3, B5a.

### B8 — Cross-cutting integration coverage & QA handoff *(tests/)*
**Files:** `tests/kittest/…`, optionally `tests/e2e/…`.
**Work:** Assemble the kittest/e2e suite mapping the remaining execution-verifiable cases not covered by unit tests
in B0–B7 (a11y sweep, network-switch edge cases, the FR-6 boundary end-to-end, and **TC-FR8-07** load-time-sealed
detail display which needs the full B0+B4+B5a chain — PROJ-011). No production code. QA-facing traceability closure.
**Satisfies:** TC-FR8-07, TC-NFR6-01…04 (sweep), TC-EDGE-05/06 end-to-end, regression net over TC-NAV-12.
**Depends on:** B2–B7.

---

## 5. Task → Test-Case Traceability

| Task | Layer(s) | Test cases satisfied |
|---|---|---|
| **A1** | ui/state | TC-NAV-13, -14, -15 |
| **A2** | ui/components | TC-NAV-01, -02, -03, -13, -16 |
| **A3** | app-shell | TC-NAV-06, -15, -16 |
| **B0** | model + backend_task | TC-FR8-01, -02, -04, -05, -09, -10 |
| **B1** | context + backend_task | TC-FR6-01…06, TC-NAV-12 (12b/12c), TC-NAV-17, TC-FR7-02, -03 |
| **B2** | ui/mod + app.rs + left_panel | TC-FR1-01…07, TC-FR1-05b, TC-EDGE-05, -06 |
| **B3** | ui/masternodes | TC-FR2-01…07, TC-FR3-01…15, TC-FR7-01, TC-NFR6-01, -03 |
| **B4** | ui/masternodes + model | TC-FR4-01…22, TC-EDGE-01, -02, -07, -08, TC-FR8-03, TC-NFR4-01, TC-NFR6-02, -04 |
| **B5a** | ui/masternodes (reuse) | TC-FR5-01…05, -07, TC-FR7-04, TC-FR9-01…08, TC-FR10-01…13, TC-FR11-01…04, TC-FR8-06, -08, TC-US4-01…07, TC-EDGE-04 |
| **B5b** | ui/masternodes (reuse voting) | TC-DPNS-01…11, TC-EDGE-03 |
| **B6** | ui/masternodes (reuse loader) | TC-FR12-01…10 |
| **B7** | ui/masternodes + ui/state | TC-NAV-04, -05, -07, -08, -09, -10, -11, -12, -18, TC-FR5-06, TC-FR6-07, TC-FR9-04 |
| **B8** | tests/ | TC-FR8-07, TC-NFR6 sweep, TC-EDGE-05/06 e2e, TC-NAV-12 regression |

All 164 cases map to at least one task. US-6 retired (0 cases). TC-FR7-01 (list Refresh) is now owned by B3 and
TC-FR7-04 (detail Refresh) by B5a — no longer footnote-only (PROJ-009). TC-FR8-07 moved from B5a to B8 (PROJ-011).

---

## 6. Risks & Open Implementation Decisions

**My decision on TC-FR12-09 (deferred to me by design).** *Add* the defense-in-depth `is_developer_mode()` check at
the Fill-Random button call-site. The whole tab is Expert-gated (FR-1), so under normal navigation the check is
redundant — **the decision therefore stands on future-proofing grounds** (a plaintext-private-key-reading dev tool
should be contained inside the Expert-Mode envelope regardless of any future non-nav entry point into this screen).
*(Corrected premise, PROJ-002: an earlier draft justified this by claiming the DPNS "Add voting key" affordance opens
the load form — under the §10.8-resolved design that affordance opens a **scoped in-place prompt**, not the load form,
so no second path into the load form / Fill-Random exists today. The check is still worth adding on future-proofing
alone; the recorded rationale is corrected.)*

**Risks to flag before implementation:**

- **R1 — FR-6 boundary is the highest-severity correctness item (critical, release-blocking).** The page-scoped
  masternode selection must never write `AppContext::selected_identity_id` (B7 / A1's `IdentityPillScope`), **and** —
  the gap Fable caught (PROJ-001) — the **resolution layer** must not resolve a masternode as the everyday-page
  identity via the first-loaded fallback or wallet reconciliation, and a stale MN selection persisted from a prior
  session must be sanitized on load. B1 now filters at `resolve_selected_identity()` + the reconciliation source and
  clears stale persisted MN selections; TC-NAV-12 gains preconditions 12b ("only a masternode loaded, nothing
  selected") and 12c ("masternode persisted as selection from a prior session"). Treat any failure here as blocking.
- **R2 — Phase A blast radius.** Making the switcher global touches every root screen's top panel; regression risk to
  the Hub and all tabs. Mitigation: Phase A lands and is reviewed independently; Subdued+`TODO` is the inert default,
  so unwired pages cannot misbehave. Do not begin B7 until A1–A3 are merged/green. **Also (PROJ-010):** a wallet
  switch reconciles the app-global identity as a side effect — A3 documents and tests this.
- **R3 — FR-8 secret path discipline (security).** The password must seal through the existing
  `secret_seam.rs` chokepoint / `protect_loaded_identity_keys` — **no second persistence path, no new crypto**.
  Password is a `Secret`; TC-FR8-04/05 (never logged, no plaintext at rest) are security assertions. Reuse the
  existing `validate_protection_password` rule (relocated to `model/`, PROJ-006); do not invent a policy (§10.3).
- **R4 — FR-7 refresh + the card contest-read may hide a genuine gap.** The rest of the plan is reuse; the two places
  a *new* thin piece may be required are re-fetching **voting** state (refresh) and the **per-node open-contest read**
  the card displays (PROJ-012). B1 must verify whether existing tasks/queries cover both; if not, add minimal
  composed pieces — do not reimplement contest fetching.
- **R5 — Inline DPNS voting reuse (B5b).** Casting votes inline (locked decision #1) reuses the vote backend, but the
  existing DPNS root screen currently *owns* contest fetching/rendering. Extracting a reusable voting-table widget
  may be needed; guard against a partial reimplementation that would fail the TC-FR9-08-style structural-reuse intent.
- **R6 — `.testnet_nodes.yml` handles real plaintext private keys.** FR-12's fixture is gitignored, absent from the
  repo, and Testnet-only. The render-conditional (not disabled) gate + Testnet + Expert-Mode + (my) dev-mode check
  keep it contained; never ship or hardcode the fixture, never log its contents.

**Artifact erratum (PROJ-013, non-blocking).** `wireframes.html` still draws the legacy **two** Fill-Random buttons
("Fill Random HPMN" / "Fill Random Masternode") and omits the missing-voter "Add voting key" affordance. FR-12/§7
(one button, label follows toggle) and ux-spec wireframe D are canonical; the HTML mock is stale for these two
details only. Recorded here so no implementer treats the mock as the source of truth for them.

---

🍬 **Findings tally (architecture, Phase 1d + review fold-in):** **6** architectural risks (Info/decision severity) —
R1 (scope-leak boundary, now covering the resolution layer per PROJ-001), R2 (app-shell blast radius + wallet
reconciliation), R3 (secret-path discipline), R4 (refresh + card-contest read gap), R5 (voting-widget reuse seam),
R6 (fixture secret containment). Plus one recorded decision (TC-FR12-09, premise corrected) and **13 Fable findings
folded in** (4 MEDIUM: PROJ-001/-002/-003/-005; 9 LOW: PROJ-004/-006…-013).

**Task count:** Phase A = 3 · Phase B = 9 (B0, B1, B2, B3, B4, B5a, B5b, B6, B7) + B8 integration = 10 · **13 total.**
