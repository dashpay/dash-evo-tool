# Test Case Specification — Link Top-Nav Wallet Selector to Wallets Tab

Phase 1b. Wires the top-nav wallet pill to the Wallets tab so both surfaces
(pill + in-tab `ComboBox`) drive the single shared selection
(`AppContext::selected_wallet_hash` / `selected_single_key_hash`). Two defects:
**D1** the Wallets-page pill is `PillConsumption::Unwired` (inert here only);
**D2** `refresh_on_arrival` never re-syncs the cached handle from the store.
Descriptions/expected only — no test code.

## Verified ground truth (corrections/additions to the Phase-1a brief)

- **Screen type is `WalletsBalancesScreen`** (brief said `WalletsScreen`);
  selection cached in `selected_wallet` (HD) **and** `selected_single_key_wallet`.
- **Dual-hash ambiguity trap — the real single-key hazard.** `set_selected_hd_wallet(Some(hd))`
  (`context/mod.rs:1168`) sets the HD hash but **preserves** the single-key hash
  (`persist_selected_wallet_kv(hash, self.current_single_key_hash())`). In-tab
  single-key selection, conversely, **clears** the HD hash
  (`persist_selected_wallet_hash(None)`, `:400`). Construction/re-sync resolves
  ambiguity by preferring **single-key first** (`:270`). So a stored `(HD=Some, SK=Some)`
  pair resolves to the SK wallet. The D2 re-sync must make the pill's fresh HD
  write authoritative (clear SK, or last-writer-wins) — otherwise pill→HD then
  arrive-at-Wallets re-shows the **stale single-key** wallet. This is the precise
  form of Diziet's single-key edge case, and the trap to guard against.
- **`set_selected_hd_wallet` has a cross-axis side effect**: it reconciles the
  app-global identity (keep-if-owned → first **User** identity → `None`), with an
  FR-6 masternode/evonode exclusion. Any pill-driven wallet switch on the Wallets
  page inherits this — regression-worthy.
- **Automatability is HIGH — much higher than QR-funding.** Pure in-memory state,
  no async/network/deposit detection. `AppContext` setters/getters and the new
  re-sync helper are directly **unit**-testable; `tests/kittest/identity_hub_switcher.rs`
  already proves wallets/identities can be seeded into a harness and the pill
  driven. Say so: kittest reaches the full flow here.

## 1. D1 — Pill interactive on Wallets page

**TC-WALLETLINK-01** — Switch via pill while ON Wallets tab. Seed ≥2 HD wallets,
mount on Wallets tab, select wallet A; activate wallet B from the top-nav pill.
Expected: displayed/cached `selected_wallet` = B immediately, no navigation; pill
spec is `Consumed` (interactive), not `Unwired`; the `SwitchWallet(hash)` effect
flows through `apply_global_nav_effect` → `set_selected_hd_wallet`. Trace: D1.
*Automatable (kittest for pill click + render; unit for the effect).* 

**TC-WALLETLINK-02** — Pill removal of the `Unwired` TODO. Assert the Wallets
spec no longer yields `PillConsumption::Unwired` and exposes no how-to tooltip.
Trace: D1. *Automatable (unit on the spec fn).* 

## 2. D2 — Re-sync on arrival

**TC-WALLETLINK-03** — Switch elsewhere → arrive → correct wallet. On another
page switch to wallet B via pill; navigate to Wallets tab. Expected:
`refresh_on_arrival` re-reads the store and shows B immediately — no stale A.
Trace: D2. *Automatable (unit: set store, call `refresh_on_arrival`, assert cache;
kittest for nav).* 

**TC-WALLETLINK-04** — Arrival never clobbers a valid current selection. With B
already correctly cached and the store agreeing, arriving must be idempotent (no
spurious reset to first-wallet). Guards against a naïve "if None pick first"
over-reach. Trace: D2. *Automatable (unit).* 

## 3. Two-way consistency (regression — already works per Diziet)

**TC-WALLETLINK-05** — In-tab `ComboBox` still updates the pill elsewhere. Select
wallet C via the in-tab picker; navigate to another page. Expected: pill reflects
C. This direction is pre-existing — regression, not new behavior. Trace: req 3.
*Automatable (unit on store + kittest render).* 

## 4. Edge cases

**TC-WALLETLINK-06** — Network switch, no stale cross-network wallet. Select B
(top-nav) on network X; switch network to Y; return to Wallets tab. Expected: the
network-Y-appropriate wallet shows (`selected_wallet_hash` is per-network);
`update_selected_wallet_for_network` drops an invalid handle. Never a wallet from
X. Trace: edge 1. *Automatable (unit) + manual GUI spot-check.* 

**TC-WALLETLINK-07** — Single-key selection survives; pill→HD supersedes it (the
dual-hash trap). (a) Select a single-key wallet in-tab → store `(HD=None, SK=Some)`;
navigate away and back → single-key still shown (re-sync must NOT auto-pick first
HD). (b) Then pick an HD wallet from the pill → arriving at Wallets shows that HD
wallet, NOT the stale SK one (the explicit HD pick wins). Expected: both hold.
Trace: edge 2. *Automatable (unit — assert store + resolved cache for both legs).*
**Highest-risk case; must go RED against a naïve SK-first re-sync.**

**TC-WALLETLINK-08** — Locked wallet: selection ≠ unlock. Switch (via pill or
ComboBox) to a locked/password-protected wallet. Expected: it becomes selected
and renders locked via the existing just-in-time gate; NO auto password prompt.
Trace: edge 3. *Automatable (kittest — secret gate not triggered) + manual.* 

**TC-WALLETLINK-09** — No "peek at another wallet". Confirm every Wallets-tab
dialog still acts on the single selected wallet post-fix; two-way single-selection
introduces no second concurrent wallet view. Trace: edge 4. *Manual GUI review +
unit (single cached handle invariant).* 

## 5. Single-wallet (Alex) regression

**TC-WALLETLINK-10** — With exactly one HD wallet, the pill stays effectively
non-interactive (nothing to switch to); no crash, no spurious effect on arrival or
click. Trace: req 5. *Automatable (kittest render + unit).* 

## 6. Regression from touching `refresh_on_arrival` / pill spec

**TC-WALLETLINK-11** — `pending_wallet_selection` still honored. After
wallet create/import sets `pending_wallet_selection`, `refresh_on_arrival` still
selects that new wallet (must run before/independent of the new re-sync). Trace:
req 6. *Automatable (unit).* 

**TC-WALLETLINK-12** — Identity reconciliation side effect intact. A pill-driven
HD switch on the Wallets page still reconciles `selected_identity_id`
(keep-if-owned → first User → None) and never resolves onto a masternode/evonode
(FR-6). Trace: req 6. *Automatable (unit — seed MN/User identities, assert
resolved id).* 

**TC-WALLETLINK-13** — Spinner/refresh flag unchanged. `refresh_on_arrival` still
clears `refreshing`. Trace: req 6. *Automatable (unit).* 

## Automation summary

- **Unit (`AppContext` setters/getters + `refresh_on_arrival` + spec fn):** 02,
  03, 04, 06, 07, 09(invariant), 11, 12, 13 — the core state-sync coverage,
  including the dual-hash trap (07).
- **kittest (pill click / nav / render / secret gate):** 01, 05, 08, 10; nav legs
  of 03/06.
- **Manual GUI only:** 06 (visual cross-network), 08 (lock UX), 09 (dialog audit).
  Far fewer than QR-funding — no async deposit/network means kittest+unit cover
  nearly everything.
