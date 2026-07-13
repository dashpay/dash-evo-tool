# Masternode-Owner Edition — Design

**Status:** Implemented (PR against #885 base `feat/legacy-identity-migration`).
**Date:** 2026-07-13
**Depends on:** PR #879 (persona-capability gating: `UserRole`, `FeatureGate`, `Check`) — **verified landed** in the base branch (see §1).
**Motivating trigger:** Masternode owners performing an urgent v0.9.3 → v1.0 upgrade need a stripped-down build that surfaces only the withdrawal-capable Masternodes screen and Settings, with everything else hidden — a smaller, less error-prone surface for a one-off, time-pressured task.

This document formalises the feature and records the verification findings and the
deviations from the originally captured (chat-only, uncommitted) design, per the
instruction to *verify every claim against the actual code before implementing*.

---

## 1. Verification of the dependency (#879)

`feat/legacy-identity-migration` **already contains** the persona-capability gating:

- `src/model/user_role.rs` — `UserRole { Everyday < Power < Developer }`, `UserRoleCell`.
- `src/context/feature_gate.rs` — `Check::{MinRole, Capability, Experimental}`, `Capability`,
  `ExperimentalFeature`, `FeatureGate::{Shielded, ShieldedOperations, DashPay,
  DashPayOperations, Masternodes, DeveloperTools}`.
- `docs/ai-design/2026-07-10-persona-capability-gating/design.md`.

So this edition builds directly on it; nothing from #879 had to be pulled in.

Two facts from that landed code materially shaped this design:

1. **`FeatureGate::Masternodes = &[Check::MinRole(UserRole::Power)]`** — the Masternodes
   surface requires Power. This is the load-bearing constraint the edition must satisfy.
2. **`UserRole::WHEN_UNSET = Power`** — a fresh install *or* a legacy blob that never
   recorded a role resolves to **Power**, not Everyday (`with_default_user_role` /
   `seed_user_role_from_settings` in `context/settings_db.rs`). This **supersedes the
   originally captured "brick risk #3" premise** ("#879 defaults fresh installs to
   Everyday"): a fresh masternode-owner-edition install already lands on Power and already
   sees the Masternodes screen. The first-run force (§4) is therefore about *durability and
   explicitness*, not about rescuing a fresh install from a brick — the landed default
   already prevents that.

---

## 2. Scope

- **Reachable in the masternode-owner edition:** `RootScreenMasternodes` and
  `RootScreenNetworkChooser` (Settings) only.
- **Hidden:** every other `RootScreenType` (Identities, Wallets, Tokens, DashPay, Tools,
  Contracts, …).
- **Escape hatch:** the **Developer** role lifts the edition restriction entirely — all
  screens become reachable again. This is a first-class requirement, not a side effect, and
  it constrains the enforcement design (§3).
- **Enabling fact (verified):** masternode load is wallet-free —
  `src/ui/masternodes/load_form.rs` sets `derive_keys_from_wallets: false` and documents
  "masternode keys never live in a wallet's HD tree". A masternode owner never needs to
  create or import a wallet, which is why hiding the entire Wallets surface does not break
  the core flow.

### Non-goals / honest limitations

- **Not a security or resource boundary.** Hiding navigation does **not** stop background
  subsystems (shielded coordinator, event bridge, DashPay detection, identity-discovery
  sweeps) — they boot regardless of which nav entries are visible. This edition does not
  gate them (out of scope) and makes no attack-surface/resource claim.
- **Not dead-code elimination.** The Cargo feature does **not** remove the hidden screens
  from the binary; they stay enum-reachable and compiled. The gating is a runtime UX
  restriction only.

---

## 3. Mechanism

### 3.1 `Edition` (`src/model/edition.rs`)

```rust
pub enum Edition { Full, MasternodeOwner }
```

- `Edition::CURRENT` — selected at compile time by the `masternode-owner-edition` Cargo
  feature (`MasternodeOwner` when set, else `Full`).
- `Edition::allows(RootScreenType) -> bool` — the **single, pure** screen policy. `Full`
  allows everything; `MasternodeOwner` allows only Masternodes + NetworkChooser. No role
  logic (kept pure and unit-testable).
- `Edition::permits(RootScreenType, UserRole) -> bool` — `allows(screen) || role ≥
  Developer`. This is where the **Developer escape hatch** composes with the edition axis.
- `Edition::home_screen()` / `always_reachable_screen()` — the preferred landing and the
  guaranteed-reachable floor for the navigation clamp (§3.3).

### 3.2 Composition with the role system — deviation from `Check::Edition`

The captured design asked for a `Check::Edition(Edition)` variant in `feature_gate.rs`.
**Not added — a reasoned deviation:**

- Screen visibility is a **`RootScreenType`-level** concern. The `Check`/`FeatureGate`
  system is **feature-level** (it answers "may this role/network use feature X?"), and
  carries no screen identity. There is no natural feature-level consumer for an edition
  check.
- The Developer escape hatch reveals **all screens**, so there is likewise no *feature*
  that should be edition-restricted — an edition-gated feature would contradict the escape
  hatch.
- An enum variant that is never constructed fails the repo's `-D warnings` gate
  (`dead_code`). Adding `Check::Edition` with no consumer would not compile clean.

Instead, the edition composes with the role axis exactly where it belongs — at the
screen-reachability boundary — via `Edition::permits(screen, role)`. This honours the
"compose the edition into the existing role gating" intent without minting a dead variant,
and keeps the pure edition policy (`allows`) separate from the runtime role (`permits`).

### 3.3 Enforcement — three points, one predicate

A single private predicate in `app.rs` funnels all reachability decisions:

```rust
fn root_screen_reachable(ctx, screen) -> bool {
    Edition::CURRENT.permits(screen, ctx.user_role())          // edition + escape hatch
        && match screen {                                       // per-screen feature gate
            RootScreenMasternodes => FeatureGate::Masternodes.is_available(ctx),
            _ => true,
        }
}
fn edition_landing(ctx) -> RootScreenType {                     // clamp target, always reachable
    let home = Edition::CURRENT.home_screen();
    if root_screen_reachable(ctx, home) { home } else { Edition::CURRENT.always_reachable_screen() }
}
```

This **generalises the pre-existing Masternodes de-gate** (the old special-case that
bounced Masternodes → Identities when the role dropped below Power) into one predicate. In
the `Full` build the edition axis is always satisfied, so `root_screen_reachable` reduces
to exactly the old Masternodes gating — **no behaviour change** (locked by
`edition_nav_tests::full_edition_keeps_all_screens_reachable`).

Enforcement sites:

1. **`AppState::set_main_screen()`** — the chokepoint every `SetMainScreen*` action passes
   through (verified: all `SetMainScreen`, `…ThenPopScreen`, `…ThenGoToMainScreen` handlers
   call it, including #882's global nav-pill path `top_panel.rs` →
   `GlobalNavEffect::NavigateToRoot`). A request for an unreachable screen is clamped to
   `edition_landing`, and the **clamped** target is persisted so the next boot reopens on a
   reachable screen.
2. **`AppState::active_root_screen_mut()`** — live de-gating: if the active screen becomes
   unreachable (role dropped, or edition-hidden), clamp before the `get_mut(...).expect()`.
   Because `edition_landing` always returns a registered, reachable screen, the `expect`
   can never fire (addresses the panic risk flagged for this method).
3. **Initial selection (`AppState::new`)** — the persisted screen is honoured only if it is
   both registered *and* reachable; otherwise it clamps to `edition_landing`. Prevents the
   first frame opening on a hidden screen (e.g. a persisted Tokens tab).
4. **Left-nav table (`ui/components/left_panel.rs`)** — nav entries the current edition does
   not `permit` are skipped (defense in depth; combined with the existing per-entry
   `FeatureGate` filter). Not the sole gate — a nav-filter-only approach was rejected
   because non-nav navigation paths (buttons, global pills) would bypass it.

### 3.4 `main_screens` registration is NOT edition-filtered — deviation

The captured design asked to filter screen *registration* so hidden screens "are never
constructed at all". **Not done — a reasoned deviation**, because it is **mutually
exclusive with the Developer escape hatch**: an escaped Developer must be able to switch
into every screen, which requires the screens to exist in `main_screens`. Conditionally
constructing them by boot-time role would break the escape hatch on a runtime Power →
Developer switch (screens wouldn't exist until restart), and would reintroduce the exact
`active_root_screen_mut().expect()` panic risk (a fallback to an unregistered screen). The
captured design's own risk #2 confirms not constructing screens buys **no** dead-code or
security benefit. So all screens are constructed as before; reachability is enforced purely
at the four points in §3.3. This is strictly safer (the `expect` cannot panic) and makes
the escape hatch correct and immediate.

---

## 4. First-run role forcing

`AppContext::apply_edition_first_run_role()` (`context/settings_db.rs`), called once at boot
right after `seed_user_role_from_settings()`:

- **No-op in every edition except `MasternodeOwner`** (compile-time `Edition::CURRENT`
  check; dead-code-eliminated in the `Full` build).
- **First-run only:** it reads the *raw* persisted role (pre-`WHEN_UNSET` resolution). It
  acts **only when no role was ever recorded** (`None` on disk — a fresh or pre-role
  install), setting and persisting `Power`. An **explicit prior choice is never
  overridden** — Developer (the escape hatch) and any other recorded role are respected
  ("no migration from any prior flag/value; persists normally after that").
- **Read-failure safe:** a k/v read error is *not* mistaken for "first run"; it leaves the
  boot seed in charge (mirroring `seed_user_role_from_settings`'s caution), so a transient
  glitch can never silently rewrite the user's real role.

Relationship to `WHEN_UNSET = Power` (§1): on a truly fresh install the runtime role is
*already* Power via the seed, so this force's observable effect there is to **record** Power
durably (turning an implicit default into an explicit, single-source-of-truth value). It is
the explicit, spec'd first-run behaviour and is independently testable. The persisted-Power
value also means a subsequent `Full`-build run on the same data dir resolves deterministically.

**Corner cases and the safety net.** A user who *explicitly* picked Everyday (only possible
by choosing it in Settings), or a boot where the settings read failed, can leave the edition
showing **only Settings** (Masternodes needs Power). This is **not a hard brick**: Settings
(the network chooser) is always reachable and hosts the role selector, so the user can
restore Power/Developer themselves. The always-reachable floor (`always_reachable_screen =
NetworkChooser`) guarantees this recovery path.

---

## 5. Files touched

| File | Change |
|---|---|
| `Cargo.toml` | new `masternode-owner-edition` feature |
| `src/model/edition.rs` | **new** — `Edition`, `CURRENT`, `allows`/`permits`/`home_screen`/`always_reachable_screen` + unit tests |
| `src/model/mod.rs` | `pub mod edition;` |
| `src/context/settings_db.rs` | `apply_edition_first_run_role()` + first-run tests |
| `src/app.rs` | `root_screen_reachable` / `edition_landing`; clamp at `set_main_screen`, `active_root_screen_mut`, initial selection; boot call; nav-clamp tests |
| `src/ui/components/left_panel.rs` | edition filter in the nav loop |

---

## 6. Testing

- `model::edition::tests` — `allows`/`permits`/escape-hatch/floor/`CURRENT` (both variants
  exercised in the default build).
- `app::edition_nav_tests` — `root_screen_reachable` / `edition_landing`; `Full` build
  proves no behaviour change, `masternode-owner-edition` build proves the clamp.
- `context::settings_db::edition_first_run_tests` — first-run lands on Power & persists;
  explicit Everyday/Developer not overridden; idempotent (does not re-fire).

**Feature note for CI/reviewers:** the edition-specific assertions are behind
`#[cfg(feature = "masternode-owner-edition")]`, so `cargo test` must be run **twice** to
cover both paths:

```bash
cargo test --lib                                        # Full build (default)
cargo test --lib --features masternode-owner-edition    # edition build
```

Both were run green during implementation. Full `--all-features --all-targets` clippy and
the complete suite are deferred to the independent QA pass per the coordinator's scope
guidance; the library compiles clean under both `--features testing` (default) and
`--features masternode-owner-edition,testing`.

---

## 7. Deviations summary (for the reviewer)

1. **No `Check::Edition` variant.** Screen visibility is `RootScreenType`-level, orthogonal
   to the feature-level `Check` system; a variant with no consumer fails `-D warnings`. The
   edition composes with the role axis via `Edition::permits` at the screen boundary
   instead. (§3.2)
2. **`main_screens` registration not filtered.** Mutually exclusive with the Developer
   escape hatch (screens must exist to switch into); no dead-code/security benefit; avoids
   the `active_root_screen_mut` `expect` panic. Enforcement is at nav + `set_main_screen` +
   `active_root_screen_mut` + initial selection. (§3.4)
3. **Brick risk #3 premise superseded.** Landed `WHEN_UNSET = Power` already prevents the
   fresh-install brick; the first-run force is retained for durability/explicitness and to
   satisfy the explicit spec, keyed on "no role ever recorded". (§1, §4)
