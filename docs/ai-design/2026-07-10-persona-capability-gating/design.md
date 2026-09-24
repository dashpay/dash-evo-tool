# Persona / Capability Gating — Design

**Status:** Design (Phase 1). No code changes proposed here — this document precedes implementation.
**Date:** 2026-07-10
**Author:** Nagatha (architecture)
**Reviewed:** independently audited by `fable` (project-reviewer); findings PROJ-001…007 folded in (rev3). User refinements folded in (rev4). See the changelog at the foot of this document.
**Motivating trigger:** The new Masternodes tab (PR #876) was gated behind `FeatureGate::DeveloperMode`. The project's own persona model says masternode operation is a **Power User** activity, not a developer one — so the gate is a conceptual mismatch, not merely a propagation bug. That exposed a deeper problem: DET has two half-built "mode" systems and no type that models the documented progressive-disclosure levels.

**Refined brief (2026-07-10, from the user):** the target is **one generic gate mechanism** invoked with a feature identifier, which grants or denies access by **composing however many checks actually apply** — user role, server/platform capability (already present via `Shielded`), and, in future, possibly *context* predicates (a wallet being loaded, an identity existing, …). The user is explicit that those context predicates are **illustrations of the shape of future need, not a spec**: the design must derive from *actual source review* what the app needs today and stay general enough to extend later **without pre-building speculative machinery** (YAGNI). The generality is to come from a clean composition primitive, not from a catalogue of imagined checks. This document is written to be independently auditable — every claim about current behaviour carries a `file:line` citation.

---

## 1. Current state

### 1.1 Two disconnected "mode" mechanisms

DET currently carries **two independent notions of user mode**, only one of which is wired to anything.

**(A) The live axis — `.env`-backed `developer_mode` boolean.**

- Stored in `AppContext` as a bare `AtomicBool` (`src/context/mod.rs:65`, initialised at `mod.rs:334`).
- Sourced from the `.env` file's `DEVELOPER_MODE` key, parsed into `Config::developer_mode: Option<bool>` (`src/config.rs:18`, loaded `config.rs:296`, saved `config.rs:211`).
- Toggled at runtime via `AppContext::enable_developer_mode(bool)` (`mod.rs:398`) and read via `is_developer_mode() -> bool` (`mod.rs:738`).
- Persisted **to `.env`**, not to `AppSettings`: the Settings screen's "Expert mode" checkbox writes both the atomic and the config file (`src/ui/network_chooser_screen.rs:607`–`625`).
- Consumed at **~43 UI/context callsites** (via `is_developer_mode()` / `FeatureGate::DeveloperMode`) — fable's independent inventory found 43 and confirmed the classification below is empirically monotone across all of them — **plus one non-accessor consumer the grep missed** (see §1.3, `[REVIEW PROJ-002]`).

**(B) The dead axis — persisted `AppSettings.user_mode`.**

- `UserMode { Beginner, Advanced }` enum (`src/model/settings.rs:22`), stored in the `AppSettings` blob (`settings.rs:220`), round-tripped through the positional bincode wire form (`settings.rs:278`, `settings.rs:299`, `settings.rs:316`).
- **Consumed nowhere in production.** `grep` across `src/` finds `user_mode` / `UserMode` only inside `settings.rs` and `settings_db.rs`, and every `settings_db.rs` reference (`:176`, `:190`, `:251`–`269`) is inside a `#[cfg(test)]` module. It is written, migrated, and defaulted to `Advanced` (`settings.rs:246`) — but no UI or gate ever reads it.
- **Critical consequence** (`[REVIEW PROJ-001]`): because the default is `Advanced` and *no code path ever writes a different value*, **every existing user's persisted blob stores the literal string `"Advanced"`**, regardless of whether they are an Everyday or an expert user. This string is therefore a **legacy sentinel, not a signal of user intent** — a fact that dictates the migration mapping in §5.2.
- `AppSettings.show_evonode_tools: bool` (`settings.rs:218`) is in the same state: persisted, never consumed.

So the enum that *looks* like the persona/role model (`UserMode`) is orphaned, while the real gating is done by a boolean that lives in the wrong place (the `.env` config, not user settings) and carries a misleading name.

### 1.2 `FeatureGate` conflates two axes into one flat enum

`FeatureGate` (`src/context/feature_gate.rs:19`) has three variants whose predicates are **semantically different kinds of check**, collapsed into one `is_available()` match (`feature_gate.rs:32`):

| Variant | Predicate | Axis it really tests |
|---|---|---|
| `Shielded` | Platform protocol version defines all five shielded state transitions (`feature_gate.rs:34`–`51`) | **Capability** (network/protocol) |
| `DashPay` | `true` (placeholder, `feature_gate.rs:52`) | — |
| `DeveloperMode` | `ctx.is_developer_mode()` (`feature_gate.rs:53`) | **User role** |

`Shielded` asks "does the connected network support this feature?" `DeveloperMode` asks "which role has the user selected?" These are orthogonal questions, but the enum treats them as interchangeable members of one set, and there is no way to express a feature that needs **more than one** check ("requires Power User *and* a protocol that supports X"). This is exactly the composition the refined brief asks for.

### 1.3 `is_developer_mode()` is overloaded — four buckets, not three

The callsites do not all mean the same thing. Source review (mine + fable's) separates them into **four** distinct concerns, currently indistinguishable behind one boolean:

1. **Disclosure** — show/hide UI complexity: the left-nav "dev" label (`src/ui/components/left_panel.rs:119`, `:307`), the Wallets System-account tab and hidden-balance rows (`src/ui/wallets/wallets_screen/mod.rs:1150`, `:1537`, `:1563`), fee display (`mod.rs:1663`), the DashPay raw-field views (`src/ui/dashpay/contact_details.rs:298`). → **Power-role.**
2. **Behavioural capability / signing override** — `AppContext::state_transition_options()` enables `allow_signing_with_any_security_level` only in dev mode (`src/context/mod.rs:757`). Not a UI-disclosure concern; it changes signing behaviour. → **Developer-role.**
3. **"Show test buttons / bypass key checks"** — the **~10** (not 15; `[REVIEW PROJ-006]`) token/identity screens with `let has_keys = if is_developer_mode() { … }` or an `is_dev_mode` local: `src/ui/tokens/claim_tokens_screen.rs:352`, `transfer_tokens_screen.rs:428`, `direct_token_purchase_screen.rs:437`, `set_token_price_screen.rs:915`, `token_action_screen.rs:472`, `update_token_config.rs:920`, `src/ui/identities/withdraw_screen.rs:409`, `transfer_screen.rs:610`, and the two helpers `src/ui/helpers.rs:520`, `:595`. These use dev mode as a proxy for "let me proceed even without the expected key." → **Developer-role permission concern** (see §7/OQ4).
4. **Experimental / stability flags wearing a disclosure costume** (`[REVIEW PROJ-004]`) — gates whose comments say "dev mode only" but whose *intent* is "this feature is not stable yet": the DashPay/shielded **Pay** button ("requires SPV which is dev mode only", `src/ui/dashpay/contacts_list.rs:842`), `contact_profile_viewer.rs:125`, `dashpay_subscreen_chooser_panel.rs:19`, and the wallet send-screen paths `src/ui/wallets/send_screen.rs:2055`, `:2106`, and the shielded tab (`src/ui/wallets/shielded_tab.rs:196`). When the underlying feature stabilises these should unlock for **everyone regardless of role** — so they belong on the **capability axis** (an `Experimental` check), **not** role reclassification. Misfiling them as Developer-role disclosure would permanently hide stabilised features from ordinary users.

**Also missed by an accessor-only grep** (`[REVIEW PROJ-002]`): `AppContext::new` reads `config.developer_mode` **directly** (not through `is_developer_mode()`) to disable UI animations — `developer_mode_enabled` at `mod.rs:313` drives the `animate` match at `mod.rs:315`–`321`. This is a genuine consumer of the axis that a `grep is_developer_mode` inventory does not surface; it must be re-pointed at the new role (behaviour-preserving mapping: disable animations at `>= Power`, matching today's `dev == true`).

Bucket 1 is disclosure; 2 and 3 are Developer-role; 4 is capability/experimental. The current flat boolean cannot express any of these distinctions.

### 1.4 Summary of the mismatch

`docs/personas/README.md:17`–`25` already specifies a three-level progressive-disclosure model (Default / Detailed / Developer tools) and explicitly states (`README.md:15`) that "developer mode" is mislabelled power-user mode. The code models none of this: it has a misnamed boolean doing power-user gating, an orphaned `Beginner/Advanced` enum, and a `FeatureGate` enum that cannot compose the role axis with the capability axis.

### 1.5 Verification: `developer_mode` is strictly binary; no hidden multi-state

The refined brief asked to *verify*, not assume, that today's gating is binary. Confirmed by exhaustive read:

- **`AppContext.developer_mode`** is an `AtomicBool` at every touch point: declared `AtomicBool` (`mod.rs:65`), initialised from `config.developer_mode.unwrap_or(false)` (`mod.rs:313`, `:334`), mutated by `enable_developer_mode(enable: bool)` via `store(enable, …)` (`mod.rs:398`–`399`), read by `is_developer_mode() -> bool` (`mod.rs:738`), branched on in `state_transition_options()` (`mod.rs:757`), and read once via the `config` local to gate animations (`mod.rs:313`–`321`). No third state exists anywhere on this field.
- **`Config::developer_mode`** is `Option<bool>` (`config.rs:18`) — nullable binary, where "unset" collapses to `false` (`mod.rs:313`).
- **The only enum-typed "mode" in settings** is `UserMode { Beginner, Advanced }` (`settings.rs:22`) — two states, and (per §1.1B) it has **no production reader**. So even the one multi-variant type that exists is (a) only two-valued and (b) dead. There is no latent three-or-more-state gating hiding in the tree.

Conclusion: the user's recollection holds. The migration starts from a genuine binary, and the multi-state model is net-new — with the bonus that a dead 2-state field (`user_mode`) is already sitting in the persistence layout to reuse (§5.2).

---

## 2. Requirements

Derived from the three persona documents, the disclosure table in `docs/personas/README.md:21`–`25`, and the refined brief.

**R1 — Three ordered user roles, matching the documented personas.**
`README.md` describes strictly **cumulative** levels: Detailed view contains the Default view's features; Developer tools is "Everything in detailed view, **plus** …" (`README.md:25`). The model must represent these three roles and honour "at least role X" semantics.

**R2 — Masternode operation belongs to Power User (validated, no 4th persona).**
The team lead asked whether masternodes warrant a fourth persona. They do not. `docs/personas/power-user.md` is unambiguous: Priya is a "part-time masternode operator" (`power-user.md:9`, `:11`), primary goal #7 is "Masternode key management" (`power-user.md:30`), a secondary success metric is "Time to check masternode key paths" (`power-user.md:58`), and "Provider key paths (voting, owner, operator, platform node)" is listed under "What Priya Needs That Alex Does Not" (`power-user.md:77`). Masternode operation is an **operator/advanced-user** activity — full visibility and control without writing SDK code — which is precisely the Power User definition (`power-user.md:20`). It is not a Platform-Developer activity (Jordan's differentiators are Devnet config, faucet, bulk identity creation, raw protocol inspection — `platform-developer.md:66`–`75`; none of these is masternode operation). **Conclusion: gate Masternodes at `Power`, retire its `DeveloperMode` gate. Three personas remain sufficient.**

**R3 — Capability gating stays independent and unchanged.**
`Shielded`'s protocol-version check (`feature_gate.rs:34`–`51`) must keep working exactly as-is. This effort **adds** a role axis; it does not replace the capability axis.

**R4 — One generic gate, invoked by feature-id, composing N checks.**
Per the refined brief: a single mechanism, called with a feature identifier, evaluates access as the **conjunction of whichever checks apply** to that feature — zero, one, or several. A feature needing "role ≥ X **AND** capability Y" is the two-check case; "always available" is the zero-check case; a single role check is the one-check case.

**R5 — Extensible without speculative machinery (YAGNI).**
Adding a new *kind* of check later (e.g. a context predicate) must cost one new check variant plus its evaluator — no redesign. But no context-predicate variants are added now, because source review (§1, §6) shows the app needs exactly these check kinds today: user role, platform capability, and — surfaced by review — an experimental/stability flag (§1.3 bucket 4). Generality lives in the composition primitive, not in a pre-built catalogue.

**R6 — Backward-compatible persistence.** *(partly superseded — see the Round 2 addendum below.)*
Existing users' saved settings must not break — both the positional bincode `AppSettings` blob (`settings.rs:267` — field order *is* the on-disk format) and the `.env DEVELOPER_MODE` key. The migration must not silently change any existing user's effective role (`[REVIEW PROJ-001]`).

**R7 — Single source of truth.**
The user role must have one authoritative persisted home and one runtime accessor. Today it has two persisted homes (`.env` live, `AppSettings.user_mode` dead) and the wrong one wins.

**R8 — Default to the simplest role.** *(superseded — see the Round 2 addendum below.)*
Fresh installs default to Everyday (progressive disclosure's baseline). This matches today's behaviour (`developer_mode` defaults to `false`, `mod.rs:313`), so no existing user regresses.

> **Round 2 addendum — what shipped instead (supersedes R6's `DEVELOPER_MODE` clause and R8).**
>
> - **`DEVELOPER_MODE` was dropped from role resolution entirely**, not migrated. The `.env` key no longer feeds the role on any path; `AppSettings.user_role` is the sole persisted home (R7 as written, with one home rather than a migrated second). The remaining `DEVELOPER_MODE` parser in `database/initialization.rs` serves the v34 schema migration and is unrelated to roles.
> - **`UserRole::WHEN_UNSET` resolves to `Power`, uniformly** — for fresh installs *and* for legacy blobs that never recorded a role, superseding R8's "fresh installs default to Everyday". A deliberate later product decision: accounts arriving from builds that exposed the power surface unconditionally would read a lower start as lost functionality, and Everyday is a choice the user opts into. Canonical statements of the behaviour live in `src/model/user_role.rs` (`WHEN_UNSET` docs), `CHANGELOG.md`, and `docs/user-roles.md`.
> - A role that cannot be **read** is a separate case, and resolves *down* to `UserRole::LEAST_PRIVILEGED` (Everyday), so a transient settings-store failure can never over-grant capability.
> - R3's shielded capability check keeps its behaviour (unmet on every released protocol version) but no longer infers activation from `FeatureVersionBounds::max_version > 0`, which cannot distinguish a v0 feature from an absent one; it compares the network's protocol version against a named activation constant instead.

**R9 — Role may drive layout, not only visibility (`[USER rev4]`).**
Role-based UI is *not* limited to hiding/showing the same widgets. A screen may present a **different arrangement** per role — e.g. a Power user's account screen laid out differently, not merely "the same screen with more rows revealed." This does not change the gating *mechanism* (`FeatureGate`/`Check` remain an availability predicate); it means a second, legitimate consumption pattern exists: screens read `ctx.user_role()` and branch on it for **layout selection**, not just conditional inclusion. The design must not imply "gating = hiding rows" is the only pattern. (See §6.)

---

## 3. Proposed type design

Three pure/near-pure inputs feed the gate: the user's chosen **role** (a persisted preference), the live **capabilities** of the connected platform (a runtime, per-network fact), and an **experimental-feature flag** (a build/config decision). Each is one *kind* of check.

### 3.1 `UserRole` — the role axis

**Name: `UserRole`** (`[USER rev4]` — renamed from the working title `DisclosureLevel`). Rationale: the type is named for **what the user is/occupies**, not for the UI mechanism it drives. "Disclosure level" centres the mechanism (how much the UI reveals); "role" centres the user (the capacity in which they are operating — everyday holder, power operator, platform developer). The earlier reasoning against `Persona` still holds and still applies to `UserRole`: a *persona* is a research artefact (an archetype with goals and pain points, in `docs/personas/`), whereas a **role is something the user actively holds or selects** and switches between — Priya *is* a Power user when operating her masternode; she has not become the research archetype "Priya." So `UserRole` keeps the type honest (a selectable capacity, persisted as a user choice) while reading more naturally at callsites than `DisclosureLevel`. Doc comments bind each variant back to its persona.

**Placement:** `src/model/user_role.rs` (new module). Per the `DET Module Placement Policy` in `CLAUDE.md`, this is a stateless data type with pure ordering logic — no `AppContext`, `Sdk`, DB, or `BackendTask` — so it belongs in `model/`, alongside `UserMode`'s current home in `model/settings.rs`. It may be re-exported from `settings.rs` if that reads more naturally at persistence callsites.

```rust
/// The role the user is operating in. Ordered: each role is a strict
/// superset of the one below (see docs/personas/README.md and Invariant
/// I1). "At least Power" is `role >= Power`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum UserRole {
    /// Default view — Everyday User (Alex). Balance, send/receive, DPNS,
    /// history. Account internals and address tables hidden.
    #[default]
    Everyday = 0,
    /// Detailed view — Power User (Priya). Full account breakdown, address
    /// tables + derivation paths, asset-lock management, refresh controls,
    /// key export, masternode key paths.
    Power = 1,
    /// Developer tools — Platform Developer (Jordan). Everything in Power,
    /// plus raw credits, state-transition context, Devnet config, faucet,
    /// bulk operations, signing overrides.
    Developer = 2,
}
```

**Invariant I1 — strict-superset total order (`[USER rev4]`, a first-class design commitment).**
`UserRole` is a **strict total order**, `Everyday < Power < Developer`, and **feature availability is monotonic in it**: `Developer ⊇ Power ⊇ Everyday`. Anything available to a lower role is available to every higher role. This is now a **deliberate invariant the design commits to**, part of the user's explicit ask — not merely inferred from the README. It is what justifies deriving `PartialOrd`/`Ord` and expressing every role check as "at least X." Two independent supports:
- **Documentary**: the README's cumulative wording — "Everything in detailed view, plus …" (`README.md:25`).
- **Empirical**: fable's review confirmed all 43 current callsites are monotone in the role (nothing visible at a lower role is hidden at a higher one).

A bitflag / capability-set model was considered and rejected: it would add expressive power the requirements do not need and would make "≥ X" awkward, while breaking I1's simple mental model. **The one thing that would break I1** is a future Developer-only feature that must be *hidden from* Power users (subset, not superset). None exists today; if one is ever proposed, it is a signal to revisit I1 (§8/OQ2), not to quietly violate it.

**Explicit discriminants (`= 0/1/2`)** pin the on-disk/wire encoding so reordering variants later cannot silently corrupt persisted values — the discipline `RootScreenType::to_int` already applies (`settings.rs:97`).

Pure helpers on the type (all in `model/`, no IO):

```rust
impl UserRole {
    pub fn as_str(self) -> &'static str;             // "Everyday" | "Power" | "Developer"
    pub fn from_persisted(s: &str) -> Option<Self>;  // Some(_) only for canonical strings, §5.2
    pub fn at_least(self, min: UserRole) -> bool { self >= min }
}
```

### 3.2 `Capability` — the platform axis (per-network by construction)

Capabilities are runtime facts about the connected platform — protocol version and (later) other server/network state. They are *not* user preferences. Modelled as a small enum so each predicate lives in exactly one place:

```rust
/// A runtime capability of the connected platform, evaluated against the
/// live context. Independent of the user's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// The connected platform protocol version defines all shielded
    /// state transitions. (Current `FeatureGate::Shielded` predicate.)
    ShieldedProtocol,
}

impl Capability {
    fn is_met(self, ctx: &AppContext) -> bool { /* moved verbatim from feature_gate.rs:34-51 */ }
}
```

**Capabilities are inherently network-scoped (`[USER rev4]`, verified against source).** The user notes capabilities differ per network (mainnet typically lags testnet). This is already structurally correct and needs no special machinery: `Capability::is_met` reads live protocol state off the **active network's** `AppContext`. Concretely, the `Shielded` predicate calls `ctx.platform_protocol_version()` (`feature_gate.rs:39`), which loads the `platform_protocol_version` atomic **cached per-network on that `AppContext`** (`mod.rs:520`), and `AppContext::set_platform_protocol_version` re-evaluates `FeatureGate::Shielded.is_available(self)` against the *same* per-network `self` when the version updates (`mod.rs:530`). DET runs **one `AppContext` per network** (per-network instances are pervasive — e.g. `mod.rs:100`, `:123`, `:211`, `:323`, `:413`, `:452`; mainnet always present, others on demand, per `CLAUDE.md`). So a capability check against mainnet's `ctx` sees mainnet's protocol version, and against testnet's `ctx` sees testnet's — automatically. **This is exactly why `Shielded` already gates correctly across networks today**, and the refactor preserves that property by keeping `is_met(ctx)` reading off the passed-in context rather than any global. New capability variants must follow the same rule: read live state from `ctx`, never a network-agnostic constant.

`Capability` lives in `context/feature_gate.rs` (it needs `AppContext` to evaluate) rather than `model/`, since — unlike `UserRole` — it is not pure. **Only `ShieldedProtocol` exists today**, because it is the only *platform* capability the current code checks (§1.2).

---

## 4. The generic gate: a feature-id resolving to a conjunction of checks

This is the heart of the refined brief. One mechanism, called with a feature identifier, evaluating the **AND of whichever checks apply**.

### 4.1 Composition primitive — `Check`

A single heterogeneous predicate type. The gate is available iff **all** of a feature's checks pass; a feature with **no** checks is available to everyone (empty conjunction = `true`).

```rust
/// One predicate contributing to a feature's availability. A feature is
/// available iff ALL of its checks are met (AND semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Check {
    /// The user is operating in at least this role (role axis).
    MinRole(UserRole),
    /// The connected platform provides this capability (platform axis,
    /// per-network by construction — §3.2).
    Capability(Capability),
    /// An experimental feature that has not stabilised yet (stability axis).
    /// Distinct from role: when the feature stabilises this check is removed
    /// and it unlocks for EVERY role — see §1.3 bucket 4 / PROJ-004.
    /// Introduced in Phase 2 when its callsites are reclassified.
    Experimental(ExperimentalFeature),
    // FUTURE — not built now (R5/YAGNI). A context predicate would be added
    // here as one variant, e.g. `Context(ContextPredicate)`, evaluated in
    // `is_met`. No such variant is introduced today because no current
    // callsite gates availability on such a predicate (see §6).
}

impl Check {
    fn is_met(&self, ctx: &AppContext) -> bool {
        match self {
            Check::MinRole(min)      => ctx.user_role().at_least(*min),
            Check::Capability(cap)   => cap.is_met(ctx),
            Check::Experimental(f)   => ctx.experimental_enabled(*f), // §4.5
        }
    }
}
```

`Check` is the *single* extension point. This is where the brief's generality lives: adding "wallet loaded," "identity exists," a feature flag, or any other predicate later is one new variant + one match arm — the gate, every callsite, and persistence are untouched.

### 4.2 The feature identifier — `FeatureGate` enum resolving to a check list

**Feature-id shape — options evaluated (R4 asked to weigh them):**

| Option | Verdict |
|---|---|
| **Enum variant per feature** (extends today's `FeatureGate`) | **Chosen.** Compile-checked exhaustiveness, discoverable, no typos, zero runtime lookup, natural home for the static check list. Matches the existing pattern. |
| **String / stringly-typed key** | Rejected. No compile safety, invites drift between callsite and definition — a poor fit where the feature set is closed and known at compile time. |
| **Trait object / registry** | Rejected as over-engineering (R5): a registry buys dynamic feature sets the app does not have. |

Each variant maps to its conjunction of checks. Because `Check` is built from `Copy` const-constructible values, the lists are `const` and promote to `'static` — no allocation:

```rust
impl FeatureGate {
    /// The checks that must ALL pass for this feature. Empty = always available.
    fn checks(self) -> &'static [Check] {
        match self {
            // Capability-only (unchanged): any user, if the protocol supports it.
            FeatureGate::Shielded =>
                &[Check::Capability(Capability::ShieldedProtocol)],
            // Role-only: Power User and up. Masternode operation (R2).
            FeatureGate::Masternodes =>
                &[Check::MinRole(UserRole::Power)],
            // Role-only: the Developer tier itself (successor to DeveloperMode).
            FeatureGate::DeveloperTools =>
                &[Check::MinRole(UserRole::Developer)],
            // No checks — always available (placeholder, as today).
            FeatureGate::DashPay => &[],
        }
    }

    pub fn is_available(self, ctx: &AppContext) -> bool {
        self.checks().iter().all(|c| c.is_met(ctx))
    }
}
```

The earlier fixed two-field struct (`{min_level, capability}`) dissolves into the general list: `Shielded` is a one-check list with no role; `Masternodes` is a one-check list with no capability; a future feature needing both is `&[Check::MinRole(Developer), Check::Capability(ShieldedProtocol)]`; an experimental feature is `&[Check::Experimental(...)]`; `DashPay` is the empty list. Nothing in the mechanism is specialised to "exactly role + capability" — that was the brief's core ask.

- **`Shielded` is preserved exactly** (R3), including its per-network scoping (§3.2).
- **`Masternodes`** is the gate PR #876's nav entry should use instead of `DeveloperMode`.
- **`DeveloperMode` → `DeveloperTools`**, renamed to stop implying the old semantics. Most of today's callsites are **Power-role disclosure** (§1.3 bucket 1) and reclassify to `Power`; buckets 2–3 stay `Developer`; bucket 4 moves to `Experimental` (§7 Phase 2).

### 4.3 Granularity guardrail — not every toggle earns a variant

If every one of the ~43 toggles became its own `FeatureGate` variant we would have ~43 variants each `&[Check::MinRole(Power)]` — noise, not structure. Rule of thumb:

- **Yes, a variant** — the feature is referenced from **more than one callsite**, or composes **more than one check**, or is a nameable product capability (Masternodes, Shielded, DeveloperTools).
- **No variant** — a one-off, single-callsite UI toggle, or a *layout* branch (R9). Those read the role directly: `if ctx.user_role().at_least(UserRole::Power) { … }` / `match ctx.user_role() { … }`. Still the same mechanism — a bare role check is `is_available`'s predicate inlined — just without minting a variant for a thing used once.

### 4.4 AND-only today; OR is a future `Check`, not a mechanism change

All current and foreseeable requirements are conjunctive. The mechanism is AND-only. A disjunction, if ever needed, is expressed *inside* a single `Check` variant (e.g. a future `Check::AnyOf(&[…])`), not by changing `is_available`. Not built now (R5).

### 4.5 The `Experimental` axis (`[REVIEW PROJ-004]`)

Bucket-4 gates (§1.3) are stability flags, not role gates. Model them as `Check::Experimental(ExperimentalFeature)`, where `ExperimentalFeature` is a small enum (e.g. `ShieldedSend`, `DashPayPay`) and `AppContext::experimental_enabled(f)` returns whether that feature is currently exposed. For **Phase-1 behaviour preservation** it returns `is_developer_mode()`-equivalent (`>= Power`), matching today's dev-gating; the semantic win is that stabilising a feature later is a **one-line flip to `true`** that unlocks it for every role — impossible under the current disclosure-costume gating. This is deliberately minimal: no new persisted state, one enum + one accessor.

---

## 5. Persistence & migration

### 5.1 Consolidate onto `AppSettings`, retire the split

Decision: **`AppSettings` becomes the single persisted home** for the user role (R7). The `.env DEVELOPER_MODE` key is demoted to a one-time seed (§5.3); `AppContext`'s runtime field is re-typed from `AtomicBool` to an atomic role (e.g. `AtomicU8` of the discriminant, or `ArcSwap<UserRole>`).

### 5.2 Reuse the orphaned `user_mode` wire slot — with the sentinel fix (`[REVIEW PROJ-001]`)

The positional bincode wire form (`AppSettingsWire`, `settings.rs:267`) already has a `user_mode: String` field (`settings.rs:278`) that is **written and read but never consumed** (§1.1B). Repurpose this existing slot to carry the user role: it is already a length-prefixed `String` (bincode `config::standard()` length-prefixes every string), so **changing the stored value is safe in both directions** — no wire-offset shift, no layout risk.

**The blocker fable caught:** an earlier draft mapped `"Advanced" → Power`. That is wrong. Because `user_mode` defaults to `Advanced` (`settings.rs:246`) and **no code path ever persists any other value**, *every* existing blob already contains `"Advanced"` — including pure Everyday users. Mapping that literal to `Power` would silently promote the **entire user base** to expert mode, flatly contradicting R6 and the "zero behavioural change" claim.

**Correct mapping — only the new canonical strings count as an explicit choice; every legacy value is a sentinel that defers to the `.env` seed:**

```
"Everyday"            -> Some(Everyday)    explicit user choice (new)
"Power"               -> Some(Power)       explicit user choice (new)
"Developer"           -> Some(Developer)   explicit user choice (new)
"Advanced" (legacy)   -> None  ─┐
"Beginner"  (legacy)  -> None   ├─ legacy sentinel: no role intent recorded;
empty / unknown       -> None  ─┘  fall through to the .env seed (§5.3)
```

`UserRole::from_persisted` returns `Option`: `Some` short-circuits to the stored role; `None` triggers seeding. After the first save through the new path, the slot holds a canonical string and the seed is never consulted again. The domain-side `AppSettings.user_mode: UserMode` field is replaced by `user_role: UserRole`; `UserMode` is deleted.

*Alternative (fallback):* freeze `user_mode` as reserved (mirroring `_reserved_core_backend_mode`, `settings.rs:275`) and **append** `user_role` at the end of `AppSettingsWire`. Safe but leaves a second dead field. The reuse path is recommended (§8/OQ1 — fable concurs it is safe both directions).

### 5.3 Seeding from the legacy `.env` boolean

For any blob whose slot decodes to `None` (§5.2) — i.e. every pre-migration user — derive the initial role **once** from `.env`:

- `DEVELOPER_MODE=true → Power`, else `Everyday`.
- `true → Power` (not `Developer`) because `README.md:15` establishes today's "developer mode" *is* power-user mode. This is the crux: nobody loses access, they are reclassified to the role they were actually using.
- Everyday users (the vast majority, `DEVELOPER_MODE` unset/false) seed to `Everyday` — **exactly today's behaviour**, so R6 holds.

This one-time reconciliation runs at the settings-load call site (`AppContext::get_app_settings`, where impure fallbacks already run per `settings.rs:321`), not as a schema migration.

### 5.4 `RootScreenType` precedent

The repo already demonstrates the discipline this migration follows: stable integer discriminants (`settings.rs:97`), an explicit round-trip test asserting the canonical encoding stays fixed (`settings.rs:168`–`179`), and reserved fields to preserve positional layout (`settings.rs:275`). `UserRole` adopts the same: fixed discriminants, a round-trip test, and a legacy-sentinel test asserting `"Advanced"` decodes to `None` (not `Power`) and then seeds correctly — mirroring `legacy_dash_network_string_decodes_to_mainnet` (`settings.rs:532`).

---

## 6. UI implications

### 6.1 Two surfaces set the role

**Surface 1 — Settings (existing).** Where the role is set today: the "Expert mode" checkbox in the Settings/Network-chooser screen (`src/ui/network_chooser_screen.rs:607`). A binary checkbox cannot express three roles, so it becomes a **three-way selector** (segmented control or radio group: *Everyday · Power · Developer*, with one-line descriptions from the persona summaries). It writes `AppSettings.user_role` via the existing `update_app_settings` path (`context/settings_db.rs`) instead of writing `.env`.

**Surface 2 — Onboarding Welcome screen (new, `[USER rev4]`).** First app start presents the Welcome screen with three rows — "Create Wallet" (`src/ui/welcome_screen.rs:107`), "Import Wallet" (`:115`), "Just Explore" (`:123`). Add an **expertise-level selector row on top** of these, letting the user pick their `UserRole` before proceeding. This makes onboarding a **second role-setting surface**, not only Settings — the first-run default (Everyday, R8) becomes an explicit, changeable choice at the moment of setup. Both surfaces write the same `AppSettings.user_role`; there is one persisted value, two entry points. Detailed layout of the row is out of scope here; the requirement is that onboarding can set the role.

### 6.2 Consumption is not only visibility — layout too (R9, `[USER rev4]`)

Screens consume the role in **two** legitimate patterns, and the design must not imply only the first:
1. **Conditional inclusion** — gate a widget/section in or out (`FeatureGate::is_available` or a direct `at_least`). This is the bulk of today's callsites.
2. **Layout selection** — branch on `ctx.user_role()` to arrange a screen *differently* per role (e.g. a Power user's account screen with a distinct layout, not just extra rows on the Everyday layout). This reads the role but is **not** a `FeatureGate` (there is no single feature being gated); it is a `match ctx.user_role() { … }` choosing among layouts.

The gating mechanism (`FeatureGate`/`Check`) remains purely an *availability predicate* — R9 does not change it. It simply acknowledges that `ctx.user_role()` is also a first-class input to layout, so "gating = hiding rows" is not the whole story. Per the granularity guardrail (§4.3), layout branches read the role directly and do not mint `FeatureGate` variants.

### 6.3 Divergence from README's "per-section" activation (`[REVIEW PROJ-005]`)

`personas/README.md:23` describes Detailed-level features activating *per-section* (expand/collapse) rather than by a global toggle. The global 3-way selector does not contradict that — the two are orthogonal. The selector sets the **baseline availability** (which sections/features *exist* for this user); per-section expanders remain an independent in-screen affordance controlling **momentary visibility within an already-available section**. A Power user still expands/collapses the address table; the selector just decides whether that table is offered at all. Keeping one global role as the availability/layout axis is the right call: it is what the persisted state, the gate, and layout selection all need to reason about, and it avoids scattering N independent per-section preferences.

### 6.4 Change-set (minimum)

- **`src/model/settings.rs`** — replace `UserMode`/`user_mode` with `UserRole`/`user_role`; update `AppSettingsWire` mapping (§5).
- **`src/model/user_role.rs`** — new module: the `UserRole` type, helpers, tests.
- **`src/context/mod.rs`** — re-type `developer_mode: AtomicBool` (`:65`) to an atomic role; replace `is_developer_mode()`/`enable_developer_mode()` (`:398`, `:738`) with `user_role()` / `set_user_role()` + a `#[deprecated]` `is_developer_mode()` shim = `>= Power` for the migration window; add `experimental_enabled()`; **re-point the animation gate at `:313`–`321` (`[REVIEW PROJ-002]`) at `>= Power`.**
- **`src/context/feature_gate.rs`** — the `Check` primitive, `Capability`, `ExperimentalFeature`, and the `FeatureGate::checks()` table (§4).
- **`src/ui/network_chooser_screen.rs`** — the three-way selector; the developer-tools sub-panel (`:635`) keys off `== Developer`.
- **`src/ui/welcome_screen.rs`** — the onboarding role-selector row (§6.1, surface 2).
- **`src/ui/components/left_panel.rs`** — the dev-label reservation and rendering (`:119`, `:307`); add the `Masternodes` gate entry (PR #876) at `Power`.
- **The ~43 `is_developer_mode()` callsites + the direct-config animation consumer** — reclassified in Phase 2 (§7). Until then the deprecated shim keeps them compiling and behaving as "≥ Power."
- **`.env.example` (`[REVIEW PROJ-003]`)** — `DEVELOPER_MODE` is **undocumented** there today. Document it (and the new `USER_ROLE` override, §8/OQ3) as part of this work.

**Source review of context predicates (R5 evidence).** No current callsite gates *availability* on "wallet loaded" or "identity exists" through the dev-mode mechanism — those are handled by ordinary data-presence branching in screens, not by a gate. Hence no `Check::Context(…)` variant now; the door is left open (§4.1) at the cost of one comment.

---

## 7. Rollout plan

Phased, so the tree stays green at every step and the risky reclassification is isolated.

**Phase 1 — Introduce the types + compat shim (single PR, no behaviour change).**
- Add `UserRole` (`model/user_role.rs`); add `Capability`, `Check`, `ExperimentalFeature`, and the `FeatureGate::checks()` table (`feature_gate.rs`); rewrite `is_available()` as `checks().all(...)`.
- Re-type the `AppContext` field; add `user_role()` / `set_user_role()` / `experimental_enabled()`; keep `is_developer_mode()` as a `#[deprecated]` shim (`>= Power`) and `enable_developer_mode(bool)` mapping `true→Power / false→Everyday`. Re-point the animation gate (`mod.rs:313`–`321`).
- Persistence via the reused `user_mode` slot with the **sentinel-safe** mapping (§5.2) + `.env` seeding (§5.3). Add the round-trip and legacy-sentinel (`"Advanced" → None → seed`) tests.
- Keep `FeatureGate::DeveloperMode` temporarily mapped to `&[Check::MinRole(Power)]` so no existing callsite changes semantics.
- **Net effect: zero behavioural change** (the sentinel fix is what guarantees this).

**Phase 2 — Migrate callsites + add `Masternodes` (one PR, or a few grouped by domain).**
- Introduce `FeatureGate::Masternodes` (`&[Check::MinRole(Power)]`); repoint PR #876's nav entry to it.
- Walk the ~43 callsites (+ the animation consumer) and reclassify each by the **four-bucket rubric** (§1.3):
  - **Bucket 1** (disclosure: address tables, fees, history, account breakdown, masternode paths) → `Power` (variant if reused, else a direct `at_least(Power)`, §4.3).
  - **Bucket 2** (signing override `mod.rs:757`) **and Bucket 3** (`has_keys` proceed-without-key) → `Developer`, decided **together as one call** (§8/OQ4).
  - **Bucket 4** (experimental/stability: DashPay Pay, shielded send, shielded tab) → `Check::Experimental(…)`, **not** a role (`[REVIEW PROJ-004]`).
- Rename `DeveloperMode → DeveloperTools`; delete the deprecated shim once the last caller is gone.
- **Own ticket, not part of this rubric** (`[REVIEW PROJ-007]`): `token_action_screen.rs:563` and `update_token_config.rs:669` branch on `button_text.contains("Test")` — string-matching a UI label, which breaks i18n; file separately. `AddressInput::with_developer_mode` / `set_developer_mode` (`address_input.rs:477`, `:528`) have **no external callers** — dead code to remove.

**Phase 3 — New role-setting UI on both surfaces + retire `.env` as persisted source.**
- Replace the binary checkbox with the three-role selector in Settings (§6.1 surface 1).
- Add the onboarding expertise-level row on the Welcome screen (§6.1 surface 2, `[USER rev4]`). Both surfaces land here because they share the same persisted `user_role` and depend on Phase 1's type + persistence; splitting them across phases would ship a half-wired role picker.
- Surface Developer-role features as they are built (the Developer role starts nearly empty, which is correct).
- Retire `.env DEVELOPER_MODE` as a *live* control but **keep its parser forever** (§8/OQ3, `[REVIEW PROJ-003]`).

---

## 8. Recommendations on the open questions (fable-reviewed)

Positions below; those still wanting explicit user sign-off are marked **(confirm)**.

1. **Wire-slot reuse vs append → reuse (recommended).** Safe in both directions: bincode `config::standard()` length-prefixes strings, so changing the slot's value cannot shift following fields. The sentinel fix (§5.2) removes the only correctness hazard. Append remains the conservative fallback.
2. **Keep `Ord` / Invariant I1.** Empirically validated — all 43 callsites monotone in the role (fable) — and now an explicit design commitment (§3.1, `[USER rev4]`). Revisit only if a Developer-only-and-Power-hidden feature is ever proposed.
3. **`.env DEVELOPER_MODE` → one-time seed only, never a live pin.** Do **not** overload it as a persistent override. Its *parser must survive indefinitely* regardless of live semantics, because `database/initialization.rs:283`–`300` (`read_env_file_for_v34_migration`) still consumes `DEVELOPER_MODE` for the one-shot v34 SPV-migration decision (`[REVIEW PROJ-003]`). If a headless/CI role pin is wanted, add a **separate `USER_ROLE=` key** (`Everyday|Power|Developer`) and document both in `.env.example`. **(confirm)** whether the CI pin is actually wanted.
4. **Bucket-3 (`has_keys`) → `Developer`, decided together with the `mod.rs:757` signing override as one decision** (both are "let an expert do a normally-guarded thing"). Not a separate ticket. The only thing that earns its own ticket is the `contains("Test")` string-matching from item (7)/PROJ-007. **(confirm)** the Developer classification.
5. **UI labels** — use the README's user-facing phrasing ("Default view / Detailed view / Developer tools", or plain role names on the onboarding row) for the selectors; keep `Everyday / Power / Developer` as code identifiers. **(confirm)** copy.
6. **`UserRole` home** — `model/user_role.rs` (recommended) over folding into `model/settings.rs`.

---

## Findings tally

Counts architecture issues **in the codebase** (not corrections to earlier drafts of this doc). Review-surfaced items credited to fable.

| Severity | Count | Items |
|---|---|---|
| High | 2 | Persona↔code mismatch: masternode gating conflicts with the documented Power-User persona (§1.4, R2); `FeatureGate` cannot compose multiple checks for one feature (§1.2, §4). |
| Medium | 3 | Orphaned dual-mode state: `AppSettings.user_mode`/`show_evonode_tools` persisted but never consumed while the `.env` bool does the real work — and its `Advanced` default is a universal sentinel that would mis-migrate (§1.1, §1.5, §5.2); `is_developer_mode()` overloaded across ~43 callsites in four distinct concerns, one of them (animations, `mod.rs:315`) reading `Config` directly and invisible to an accessor grep (§1.3, PROJ-002); experimental/stability flags wearing a disclosure costume that would permanently hide stabilised features if misfiled (§1.3 bucket 4, §4.5, PROJ-004). |
| Low | 2 | Role source of truth lives in `.env` config rather than user settings (§1.1, R7); i18n-fragile `button_text.contains("Test")` gating plus dead `AddressInput::*developer_mode` methods (§7, PROJ-007). |

**Total: 7 findings (2 High · 3 Medium · 2 Low).**

---

## Changelog

- **rev1** (`4dc66c31`) — initial design: `DisclosureLevel` + two-axis `FeatureRequirement`.
- **rev2** (`35243b16`) — refined brief: generalised to the N-check `Check` conjunction; feature-id options weighed; binary-`developer_mode` verification; granularity guardrail.
- **rev3** (`dafa3d12`) — fable review folded in (PROJ-001…007): sentinel-safe legacy mapping (blocker); direct-config animation consumer; `.env` parser longevity + `.env.example`; experimental 4th bucket + `Check::Experimental`; per-section vs global-selector divergence; bucket-3 count; `contains("Test")` ticket + dead-code; open questions adopted as recommendations.
- **rev4** (this revision) — user refinements: **(1)** renamed `DisclosureLevel → UserRole` throughout, with re-derived naming rationale (§3.1); **(2)** strict-superset ordering promoted to a named, first-class **Invariant I1** (Developer ⊃ Power ⊃ Everyday), a deliberate commitment not just an empirical observation (§3.1, §8/OQ2); **(3)** new **R9** — role may drive *layout*, not only visibility; added the two-pattern consumption note (§6.2); **(4)** made per-network capability scoping explicit and **verified against source** (`feature_gate.rs:39` → `mod.rs:520`/`:530`, per-network `AppContext`) (§3.2); **(5)** new onboarding role-setting surface on the Welcome screen (`welcome_screen.rs:107/:115/:123`), added as surface 2 in §6.1 and Phase 3 in §7. Env override key renamed `DISCLOSURE_LEVEL → USER_ROLE`.
