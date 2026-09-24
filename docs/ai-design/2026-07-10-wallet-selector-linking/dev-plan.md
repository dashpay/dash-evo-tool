# Development Plan — Link Top-Nav Wallet Selector to Wallets Tab (WAL-028)

Phase 1c. For `developer-bilby`. Two defects, one shared selection-resolution
concern. Test IDs reference `test-cases.md` (TC-WALLETLINK-01..13).
`AppContext::selected_wallet_hash`/`selected_single_key_hash` stay the single
source of truth (mirrors IDH-003).

## Scope guardrails

- **No `AppContext` public API *shape* change** (item 4). `set_selected_hd_wallet`
  keeps its `Option<WalletSeedHash>` signature; only its body changes. No new
  public method, no new plumbing.
- **One Bilby pass** (item 5): D1 + D2 are tiny and both touch selection
  resolution. Confirmed single pass.
- Verified: `add_top_panel_with_global_nav` (`top_panel.rs:416`) *already* calls
  `apply_global_nav_effect(app_context, effect)` unconditionally →
  `GlobalNavEffect::SwitchWallet(hash)` → `set_selected_hd_wallet`. The pill is
  inert on Wallets **only** because the spec builds `PillConsumption::Unwired`,
  which emits no `SwitchWallet`. So D1 needs no wiring — just the spec variant.

## Task 1 — D1: interactive pill on the Wallets page

1. Add a `Consumed` wallet-only spec constructor in `top_panel.rs` beside
   `subdued_wallet_only_spec` (`:360`), e.g.
   `wallet_only_spec(label, target)` →
   `PageNavSpec::new(label, target).with_wallet_pill(PillConsumption::Consumed)`.
   Exact reference already in-tree: `masternodes_view.rs:28`. (TC-WALLETLINK-02.)
2. `wallets_screen/mod.rs:2430` — swap `subdued_wallet_only_spec(...)` for the
   new consumed constructor; delete the un-actioned
   `// TODO: wire wallet selection consumption for the Wallets page.` at `:2426`
   (no tombstone comment). Update the import at `:24`. (TC-WALLETLINK-01/02.)

Result: pill click on the Wallets tab flows through the existing
`apply_global_nav_effect` → `set_selected_hd_wallet` with no navigation
(TC-WALLETLINK-01). Single-wallet Alex: nothing to switch to, no spurious effect
(TC-WALLETLINK-10).

## Task 2 — D2: re-sync on arrival + fix the dual-hash asymmetry

### 2a. Restore setter symmetry (the TC-WALLETLINK-07 trap — do this first, TDD RED)

Root cause: `set_selected_single_key_wallet` clears the HD hash
(`persist_selected_wallet_hash(None)`, `:400`), but `set_selected_hd_wallet`
(`context/mod.rs:1168`) **preserves** the single-key hash
(`persist_selected_wallet_kv(hash, self.current_single_key_hash())`). Combined
with single-key-first construction resolution (`WalletsBalancesScreen::new`,
`:257-300`: SK hash wins when both set), a pill→HD switch leaves the store
`(HD=Some(new), SK=Some(stale))` and re-resolves to the **stale SK** wallet.

**Decision — clear the SK hash inside `set_selected_hd_wallet`.** Change the
persist to `persist_selected_wallet_kv(hash, None)` (only when `hash.is_some()`;
an explicit HD pick means the active wallet is now that HD wallet). This makes
`(Some, Some)` unrepresentable via the public setters and restores symmetry with
the SK setter, so BOTH construction and the new re-sync resolve unambiguously —
no need to touch resolution order or add a write-generation counter.

**Risk flag (item 2, mandatory review point):** this changes a public setter
used by 11 call sites (`top_panel.rs:388`, `hub_screen.rs:139`,
`wallets_screen/mod.rs:354/387/415/440/862/3126`, self). Grep assessment: every
caller means "make this HD wallet active"; none semantically relies on a *stale
single-key* selection surviving an HD switch (the app models one active wallet;
the screen caches already clear the sibling on select — `select_hd_wallet` nulls
`selected_single_key_wallet` at `:369`). So clearing is the correct invariant,
not a regression. Bilby must confirm via the RED-first TC-WALLETLINK-07 (both
legs: SK-select survives navigation; then pill→HD supersedes it) before shipping.
TC-WALLETLINK-12 (identity reconciliation keep-if-owned→first User→None, FR-6
MN/evonode exclusion) stays intact — we keep routing through the setter, not
around it.

### 2b. Add the re-sync read in `refresh_on_arrival` (`:3088`)

Extract construction's store→wallet resolution (`:257-300`) into a shared helper
`fn resolve_selection_from_store(&Arc<AppContext>) -> (Option<HD>, Option<SK>)`
so `new()` and `refresh_on_arrival` share one tested code path (unit-testable,
TC-WALLETLINK-03/04/07). In `refresh_on_arrival`, ordered:

1. `refreshing = false` (unchanged — TC-WALLETLINK-13).
2. `pending_wallet_selection` handling (unchanged, runs first — TC-WALLETLINK-11).
3. **New:** re-read the store via the helper; if it yields a valid selection,
   adopt it into `self.selected_wallet`/`self.selected_single_key_wallet`
   (replacing a stale cache — TC-WALLETLINK-03). Idempotent when the cache
   already agrees (TC-WALLETLINK-04).
4. Existing "nothing selected → pick first HD/SK" fallback runs **only** when the
   store also yielded nothing — guards TC-WALLETLINK-04 against clobbering a
   valid selection with first-wallet.

Network-switch correctness (`update_selected_wallet_for_network`, `:403`) is
unaffected — the helper reads the per-network `selected_wallet_hash`, so an
invalid cross-network handle is never adopted (TC-WALLETLINK-06).

### 2c. In-tab `ComboBox` single-key path (item 3)

**No change needed.** It already clears the HD hash on single-key select
(`:400`) — that is the symmetric side of 2a. After 2a, HD-select clears SK and
SK-select clears HD: consistent both ways. Confirm it stays correct; do not
touch it (TC-WALLETLINK-05 pre-existing direction stays green). The in-tab
ComboBox remains a second surface, not removed.

## Task 3 — docs (item 6)

`docs/user-stories.md`: flip `WAL-028` placeholder → `[Implemented]`. Draft:
*"As a multi-wallet user, I can switch the active wallet from the top-nav pill
while on the Wallets tab, and arriving at the Wallets tab always shows the wallet
I last selected on any surface, so the pill and the in-tab picker never
disagree."* Add an acceptance line to `WAL-004`: *"The top-nav wallet pill is
interactive on the Wallets tab and stays consistent with the in-tab picker."*
Non-code; own commit in the same PR.

## Ordering / traceability

2a (RED-first, TC-WALLETLINK-07) → 2b (re-sync helper) → Task 1 (spec swap) →
Task 3. Unit: 02, 03, 04, 06, 07, 09-invariant, 11, 12, 13. kittest: 01, 05, 08,
10. Manual GUI: 06, 08, 09. Extract `resolve_selection_from_store` so 03/04/07
are unit-reachable without a live screen.
