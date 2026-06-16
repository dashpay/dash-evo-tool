# Code Review Report: PR #860 session diff 44caa892..fed6bef8 — shielded subsystem retirement + upstream coordinator (A-G), WalletUnlockPopup slim, PROJ-032 docs, placement/unlock cleanups

| Field | Value |
|---|---|
| **Date** | 2026-06-16 |
| **Project** | dashpay/dash-evo-tool |
| **Branch** | docs/platform-wallet-migration-design |
| **Commit** | fed6bef887b80ab96bb66ddbf89c220868849dc4 |
| **Scope** | PR #860 session diff 44caa892..fed6bef8 — shielded subsystem retirement + upstream coordinator (A-G), WalletUnlockPopup slim, PROJ-032 docs, placement/unlock cleanups |
| **Reviewers** |  |

## Executive Summary

25 findings across security, consistency, Rust quality, and docs — ZERO critical, ZERO high. The funds-safety core of the shielded migration is sound.

The session retired DET's home-grown Orchard shielded subsystem (-4509 net lines) and routed all five shielded operations through upstream platform-wallet's coordinator, added a push balance snapshot, 5 det-cli MCP tools, slimmed WalletUnlockPopup, and reconciled the gap audit. The review found no false-success paths, an exhaustive error mapper, correct WalletId→SeedHash balance attribution, and per-network cleanup. Findings are dominated by secret-hygiene hardening on the new core_wallet_import tool (mnemonic Debug-leak vector + missing zeroization), a handful of Rust nits (a hardcoded bind guard, String-typed store errors), and documentation-accuracy drift (MCP.md/MCP_TOOL_DEVELOPMENT.md SPV-gate prose, a self-referential placement-policy inaccuracy, stale //! docs and phase-history comments).

### Findings Summary

| Severity | Security | Project | Code Quality | Call-Tree Inspection | Documentation | Dependencies | PR Comments | PR Promises | Total |
|---|---|---|---|---|---|---|---|---|---|
| CRITICAL | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| HIGH | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| MEDIUM | 0 | 0 | 4 | 0 | 0 | 0 | 0 | 0 | 4 |
| LOW | 5 | 4 | 10 | 0 | 2 | 0 | 0 | 0 | 21 |
| INFO | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

## Part I: Security Findings

### SEC-001 (LOW) *(overall=0.33, risk=0.30, impact=0.55, scope=0.15)*: core_wallet_import: BIP-39 mnemonic carried in a Debug-derived MCP param struct (log-leak vector) — A09:2021-Security-Logging-and-Monitoring-Failures, A02:2021-Cryptographic-Failures, CWE-532, CWE-312

- **Location**: `src/mcp/tools/wallet.rs:24-33`
- **Description**: ImportWalletParams holds the BIP-39 recovery phrase as a plain `mnemonic: String` and derives `Debug` (`#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]`). Any `tracing`/`log` call that formats the params with `{:?}` — or any future request-tracing middleware in the rmcp transport that dumps the raw JSON-RPC body at trace/debug level — would write the full seed phrase to logs and log sinks in cleartext. The current dispatch path (src/mcp/dispatch.rs, src/mcp/server.rs) does not log params today, so this is a latent leak rather than an active one, but the recovery phrase is the single highest-value secret in the app (full wallet compromise, irreversible fund theft) and a Debug-printable struct holding it is one stray log line away from disclosure. The HTTP transport (`mcp` feature) makes this remotely reachable if request logging is ever enabled. CWE-532 (Insertion of Sensitive Information into Log File), CWE-312.
- **Recommendation**: Do not derive `Debug` on a struct containing the mnemonic, or implement `Debug` manually to redact the field (e.g. print `mnemonic: "<redacted>"`). Wrap the phrase in a `secrecy::Secret<String>` / `Zeroizing<String>` newtype with a redacting Debug so it cannot be accidentally formatted. Confirm the rmcp transport layer does not trace request bodies; if it can, add an explicit redaction allowlist for `core_wallet_import`.

### SEC-002 (LOW) *(overall=0.32, risk=0.25, impact=0.50, scope=0.20)*: core_wallet_import: derived HD seed and mnemonic not zeroized after use — A02:2021-Cryptographic-Failures, CWE-316, CWE-226

- **Location**: `src/mcp/tools/wallet.rs:93-117`
- **Description**: In ImportWallet::invoke the mnemonic is parsed and `let seed = mnemonic.to_seed("")` produces a raw 64-byte HD seed (`[u8; 64]`). The `seed`, the `bip39::Mnemonic`, and the original `param.mnemonic` String are all dropped at end of scope without zeroization, leaving plaintext seed material in freed heap/stack memory subject to later disclosure via core dumps, swap, or heap reuse. The GUI import path that this MCP tool parallels is expected to handle the same secret; if that path uses `Zeroizing`, this headless path is an inconsistent secret-hygiene regression for the exact same value. Because `register_wallet` takes `&seed` by reference, wrapping the local in `Zeroizing` is a drop-in change. CWE-316 (Cleartext Storage of Sensitive Information in Memory), CWE-226 (Sensitive Information Uncleared Before Release).
- **Recommendation**: Bind the seed as `let seed = Zeroizing::new(mnemonic.to_seed(""));` and pass `&*seed`. Wrap `param.mnemonic` handling so the String is zeroized (e.g. move into a `Zeroizing<String>` before parse). Verify the `bip39` dependency is built with its `zeroize` feature so `Mnemonic` scrubs on drop. Mirror whatever the GUI `Wallet::new_from_seed` import flow already does so both paths agree.

### SEC-003 (LOW) *(overall=0.27, risk=0.20, impact=0.35, scope=0.25)*: Forgotten wallet's shielded balance snapshot is never evicted from AppContext::shielded_balances — A04:2021-Insecure-Design, CWE-212, CWE-459

- **Location**: `src/context/mod.rs:130; src/context/wallet_lifecycle.rs (remove_wallet)`
- **Description**: AppContext::shielded_balances is a `HashMap<WalletSeedHash, u64>` keyed by seed hash, written by the sync-completed push writer and by refresh_shielded_balance_snapshot. The wallet-removal path (remove_wallet / forget_wallet) wipes the seed-envelope vault, the wallet-meta sidecar, the in-memory wallets/id_map/snapshot registration, and detaches the upstream coordinator — but it does NOT remove the wallet's entry from shielded_balances (a grep for `shielded_balances` finds no reference in wallet_lifecycle.rs). The stale credit figure lingers for the process lifetime. Because the seed hash is deterministic from the seed, re-importing the same recovery phrase re-binds the SAME seed hash and the UI/MCP `shielded_balance_get` would surface the OLD shielded balance until the next completed sync overwrites it — showing a freshly-imported wallet a non-zero shielded balance it has not yet verified. This is a balance-display correctness / stale-attribution issue, not direct fund loss, but a user could act on a phantom figure (e.g. attempt to send shielded funds that are not actually spendable yet).
- **Recommendation**: In the wallet-removal path, evict the entry: `self.shielded_balances.lock().ok().map(|mut m| m.remove(seed_hash))`. Add a regression test asserting the snapshot entry is gone after remove_wallet, mirroring the existing seed-envelope wipe test.

### SEC-004 (LOW) *(overall=0.25, risk=0.20, impact=0.45, scope=0.10)*: map_shielded_op_error routes ambiguous ShieldedBroadcastUnconfirmed to a generic error (no 'do not re-submit' guidance) — A04:2021-Insecure-Design, A08:2021-Software-and-Data-Integrity-Failures, CWE-393, CWE-754

- **Location**: `src/wallet_backend/mod.rs:634-677`
- **Description**: map_shielded_op_error's exhaustive arm lumps `PlatformWalletError::ShieldedBroadcastUnconfirmed { .. }` together with truly-clean failures into the generic `TaskError::WalletBackend` wrapper, which renders a generic wallet-error message. Upstream documents this variant as an AMBIGUOUS post-broadcast state: 'broadcast was ACCEPTED by the relay but the SDK could not confirm its execution result ... the caller must NOT treat it as unregistered or re-submit' (rs-platform-wallet/src/error.rs:205-219; FFI result code 17). It is the identity-create sibling of ShieldedSpendUnconfirmed. In the CURRENT code none of DET's five fund-moving ops (shield/transfer/unshield/withdraw — all of which correctly emit ShieldedSpendUnconfirmed and are routed to dedicated *ConfirmationUnknown variants) construct ShieldedBroadcastUnconfirmed, so this is NOT a live double-spend path today — it is a latent defense-in-depth gap. If a future op (e.g. a shielded identity registration) is wired through this same mapper, an ambiguous broadcast would surface as a generic error and the user could re-submit, risking a double-execution. The function's own doc-comment promises the exhaustive match 'forces a review here'; the review chose the unsafe-by-omission routing for this one ambiguous variant.
- **Recommendation**: Give ShieldedBroadcastUnconfirmed its own TaskError variant carrying the same 'broadcast accepted, confirmation unknown — wait and refresh, do not re-submit' wording as the *ConfirmationUnknown family (it already may-have-executed semantics). At minimum add a comment explaining why it is deliberately bucketed as generic and assert it is unreachable from DET's current ops, so a future caller change trips a review.

### SEC-005 (LOW) *(overall=0.25, risk=0.25, impact=0.30, scope=0.20)*: 'Delete all local data' completes successfully while the shielded coordinator wipe runs fire-and-forget and only logs on failure — A04:2021-Insecure-Design, A09:2021-Security-Logging-and-Monitoring-Failures, CWE-212, CWE-459

- **Location**: `src/context/wallet_lifecycle.rs:18-37 (clear_network_database)`
- **Description**: clear_network_database synchronously unlinks the two legacy shielded files and returns Ok, but the authoritative Orchard state now lives in the upstream coordinator store (platform-wallet-shielded.sqlite), which is reset via `backend.clear_shielded()` dispatched as a detached best-effort subtask (`subtasks.spawn_sync("shielded_coordinator_clear", ...)`). On failure that subtask only emits `tracing::warn!` — the caller has already returned success. Consequences: (1) the user is told 'all local data deleted' while plaintext Orchard notes, nullifiers and viewing-key-derived material may remain in the coordinator store (incomplete secret/PII wipe, CWE-212/CWE-459); (2) the wipe is not awaited, so a subsequent re-create/re-open of the same network can race the still-pending clear. This is primarily a privacy / data-remanence defect for a shielded (privacy-focused) feature, where 'I deleted my data' must be trustworthy.
- **Recommendation**: Await the coordinator clear within clear_network_database (it is already async) and propagate its failure to the caller so the destructive action reports partial failure instead of false success. If a detached design is required, surface a persistent warning/banner to the user that shielded data removal is still pending or failed, and ensure the next backend bring-up re-attempts the clear before re-binding.

> **Positive observations:** Funds-safety core sound: no false-success, exhaustive map_shielded_op_error, correct WalletId→SeedHash, per-network cleanup. No CRITICAL/HIGH.

## Part II: Project Consistency

### PROJ-001 (LOW) *(overall=0.40, risk=0.30, impact=0.40, scope=0.50)*: gaps.md executive-summary table never updated after PROJ-032 close — contradicts its own candy tally and the JSON — consistency, tally, PROJ-032, PROJ-034

- **Location**: `docs/ai-design/2026-06-01-pr860-gap-audit/gaps.md:77-92`
- **Description**: The PROJ-032 CLOSE / PROJ-034 KEEP reconciliation was applied to the bottom-of-file candy tally and to `gaps-report.json`, but the **executive-summary table at the top of `gaps.md` was left stale**, so the document now disagrees with itself.

- Executive table (lines 77-84): `MEDIUM Open=4`, `MEDIUM Resolved=11`, `Total Open=10`, `Total Resolved=33`.
- Candy tally (lines 1024-1025, updated this PR): `34 RESOLVED ... + 9 OPEN (1 HIGH + 3 MEDIUM + 5 LOW)`.
- `gaps-report.json` (updated this PR): `open_findings=9`, `MEDIUM open=3`.

Moving PROJ-032 from open→resolved must take the table to Open=9 / Resolved=34 / MEDIUM-open=3 / MEDIUM-resolved=12. It still reads 10/33/4/11.

Second, the `Open by category ... Sum = 10 open (... net count unchanged ...)` line (86-92) is arithmetically wrong. PROJ-034 was **already** an open finding (it is one of the original 4 open MEDIUMs); listing it in the category breakdown while removing PROJ-032 keeps the *enumeration* at 10 entries but does **not** keep the *open count* at 10 — the real count drops to 9. "net count unchanged" is false.
- **Recommendation**: Update the executive-summary table to Open=9 / Resolved=34, MEDIUM 3 open / 12 resolved, and correct the `Sum = 10 open` line to `Sum = 9 open` (drop the "net count unchanged" justification). Make the top table, the bottom candy tally, and `gaps-report.json` agree on 9 open / 34 resolved before this doc is used as the merge-gate reference.

### PROJ-002 (LOW) *(overall=0.28, risk=0.15, impact=0.20, scope=0.50)*: Tombstone + phase-plan narration in code comments (describe history, not present state) — convention, tombstone, present-state

- **Location**: `src/database/initialization.rs:1347-1351`
- **Description**: The PR adds ~35 new `Phase A`–`Phase G` references inside code comments plus several outright tombstones that explain *removed* code — both against the Cross-Cutting Rules "No tombstone comments" and "Describe present state, not history" (history belongs in commit messages / the PR description, the reader has `git blame`).

Representative offenders:
- `src/database/initialization.rs:1347-1351` — "DET's shielded subsystem was retired (Phase D); the old shielded table helpers and the `database::shielded` module were deleted." Pure tombstone — it documents code that no longer exists.
- `src/backend_task/error.rs:3860` — "Ported from the deleted `backend_task::shielded::bundle` tests (Phase D)".
- `src/mcp/server.rs:156` — "Shielded read/control tools (Phase G — agent self-verification)".
- `src/mcp/tools/shielded.rs` — "(Phase E writer)"; `src/ui/wallets/send_screen.rs:2076` — `// TODO(Phase F): ...`.

"Phase D/E/F/G" are this PR's internal phasing labels; once the PR is merged they reference nothing a future reader can resolve. The *present-state* halves of these comments ("these tables are intentionally not created; the v37 migration drops any legacy copies") are fine and should stay — it is the historical narration that should go.
- **Recommendation**: Strip the "was retired / were deleted / Ported from the deleted X" tombstones and the bare `Phase X` process labels; keep only the present-tense rationale (what the code does now and why). Move the migration-phase narrative to the commit messages / PR description where it belongs.

### PROJ-003 (LOW) *(overall=0.25, risk=0.25, impact=0.20, scope=0.30)*: New TODO(PROJ-034) ephemeral review ID re-committed into source — convention, ephemeral-id

- **Location**: `src/backend_task/migration/finish_unwire.rs:56`
- **Description**: This PR edited the migration TODO block — it correctly removed `TODO(PROJ-032)` but **rewrote and re-committed** `TODO(PROJ-034): App settings, top-up history, and scheduled DPNS votes all reset/empty on upgrade`. `PROJ-NNN` is exactly the class of transient review-finding ID the coding-best-practices Cross-Cutting Rules forbid in committed code (and that `scripts/lint_ephemeral_ids.py` flags): the consolidator reassigns these IDs every run, so the reference is dead the moment this branch merges.

Context (out of session scope, not double-counted): the tree is already saturated with pre-existing `PROJ-010` / `PROJ-040` / `PROJ-007` references across `wallet_backend/`, `context/wallet_lifecycle.rs`, and `tests/backend-e2e/` — ~45 of them. They predate `44caa892`; PROJ-034 is the one this session actively touched.
- **Recommendation**: Drop the `PROJ-034` tag. Either a plain `// TODO:` describing the missing settings/top-up/scheduled-vote importer, or a durable handle (a GitHub issue ref like `#NNNN`) per the allowed-ID list. While this block was open, it was the moment to delete the ephemeral tag — not refresh it.

### PROJ-004 (LOW) *(overall=0.22, risk=0.15, impact=0.20, scope=0.30)*: Dangling policy-number references (P8, P14) in comments — defined nowhere — consistency, dangling-reference

- **Location**: `src/ui/state/mod.rs:5`
- **Description**: Two newly-added comments cite numbered policies that exist in no committed artifact:
- `src/ui/state/mod.rs:5` — "the module placement policy (P14) keeps these out of `ui/components/`".
- `src/model/single_key.rs:16` — "Used by the import dialog for instant feedback (P8)".

A grep across `CLAUDE.md`, `docs/`, and `src/` finds **no definition** of `P8` or `P14`. The actual module-placement policy this PR added to `CLAUDE.md` ("DET Module Placement Policy") is **not numbered P14**, so the reference resolves to nothing. A reader chasing "P14" has no destination.
- **Recommendation**: Either point the comments at the real anchor (`CLAUDE.md` § "DET Module Placement Policy" / "Validation placement") or drop the parenthetical numbers. If the P-numbering is meaningful, define it once in a committed doc and reference that.

> **Positive observations:** Placement policy applied; conventional commits; TaskError typing respected; PROJ-032/034 reconciliation substantively accurate.

## Part VI: Documentation

### DOC-001 (LOW) *(overall=0.37, risk=0.30, impact=0.30, scope=0.50)*: UI components catalog lists a deleted component and a relocated one — doc-drift, catalog

- **Location**: `src/ui/components/README.md:67-68`
- **Description**: `CLAUDE.md` mandates consulting `src/ui/components/README.md` before building any UI element — it is the authoritative component catalog. This PR invalidated two of its rows without updating it:
- `ScreenWithWalletUnlock | wallet_unlock.rs | Trait for screens needing wallet unlock` (line 67) — the file `src/ui/components/wallet_unlock.rs` and the trait were **deleted** this PR (commit `fed6bef8`, migrate to `WalletUnlockPopup`). The catalog now points at a non-existent module.
- `TrackedAssetLockCache | tracked_asset_lock_cache.rs | ...` (line 68) — **moved** this PR to `src/ui/state/tracked_asset_lock_cache.rs`. Per the new placement policy it is explicitly *not* a component (renders no egui), yet it is still listed in the components catalog under "Utility" with a now-wrong path.
- **Recommendation**: Remove the `ScreenWithWalletUnlock` row, and remove `TrackedAssetLockCache` from the components catalog (it belongs to `ui/state/` now — note it there, e.g. a short `src/ui/state/` README or a pointer line, rather than in the components table).

### DOC-002 (LOW) *(overall=0.22, risk=0.15, impact=0.20, scope=0.30)*: user-stories.md untouched despite five new developer-facing MCP/CLI tools — user-stories, judgment-call

- **Location**: `docs/user-stories.md`
- **Description**: This PR adds five new dev/agent-facing tools — `core_wallet_import`, `shielded_init`, `shielded_sync`, `shielded_balance_get`, `shielded_address_get` — exposed over both MCP and `det-cli`, and a documented headless shielded self-verification loop (`docs/CLI.md`). `docs/user-stories.md` was not touched.

**My call:** defensibly skippable. The bulk of the PR is a refactor (re-seating shielded state on the upstream coordinator) with no change to end-user shielded flows, and the new tools are testing/automation affordances rather than a product feature — which the repo rule ("Skip user-story updates for non-functional changes ... refactoring") permits. That said, the new headless-verification capability is a genuine Platform-Developer-persona affordance and would be a reasonable single `[Implemented]` story.
- **Recommendation**: Optional: add one Platform-Developer story (e.g. "As a Platform Developer I can drive and verify a full shielded lifecycle headlessly via det-cli") covering the new self-verification tools. Not a blocker for a refactor-dominant PR.

> **Positive observations:** New tools in CLI.md/MCP.md; policy in CLAUDE.md; gap audit reconciled. Gaps are accuracy/staleness, not omissions.

## Part III: Code Quality & Language Best Practices

### CODE-001 (MEDIUM) *(overall=0.52, risk=0.45, impact=0.50, scope=0.60)*: MCP.md network-required prose contradicts 4 new shielded tools

- **Location**: `docs/MCP.md:138`
- **Description**: The 'Network verification' section states: 'For destructive tools (those that spend funds or modify state — all identity and shielded tools), `network` is required.' The four new shielded tools (shielded_init, shielded_sync, shielded_balance_get, shielded_address_get) all use WalletIdParams with `#[serde(default)] pub network: Option<String>` — i.e., optional. The tool table on lines 90-93 correctly marks them `network?`, but the prose claim on line 138 flatly contradicts that. shielded_init is annotated `destructive: false`, shielded_sync `destructive: false`, the balance/address reads are `read_only: true` — none qualify as 'destructive' by the MCP ToolAnnotations taxonomy. A developer reading the prose will try to require `network` in client tooling and wonder why it was optional in the schema.
- **Recommendation**: Narrow the prose on line 138 to name the specific destructive shielded tools or replace 'all shielded tools' with 'shielded fund-moving tools (shielded_shield_from_core, shielded_shield_from_platform, shielded_transfer, shielded_unshield, shielded_withdraw)'. The four new control/read tools can be grouped with the read-only tools that take optional network.

### CODE-002 (MEDIUM) *(overall=0.52, risk=0.50, impact=0.55, scope=0.50)*: shielded_address_get misclassified as a no-network-call SPV-free tool

- **Location**: `docs/MCP.md:102`
- **Description**: Line 102 groups shielded_address_get with shielded_balance_get under 'shielded snapshot reads' that skip the SPV gate. This is inaccurate. shielded_balance_get is truly SPV-free: it reads from AppContext atomic fields (shielded_balance_credits / shielded_balance_duffs) with no backend call. shielded_address_get calls ctx.wallet_backend().map_err(McpToolError::TaskFailed)? and then backend.shielded_default_address(). In standalone MCP mode (det-cli serve), wallet_backend() returns Err(TaskError::WalletBackendUnavailable) if ensure_spv_synced has never run — it is the only MCP chokepoint that calls ensure_wallet_backend_and_start_spv. A cold call to shielded_address_get without a prior SPV-gated tool (e.g. shielded_init) returns a TaskFailed error in standalone mode. The tool's own description says 'Run shielded_init first if the wallet is not yet bound', which implicitly acknowledges the dependency, but the SPV gate section's categorical grouping will mislead users who rely on the prose for sequencing.
- **Recommendation**: Separate shielded_address_get from shielded_balance_get in the SPV gate prose. shielded_balance_get is a pure in-memory read and can skip SPV. shielded_address_get requires the wallet backend to be wired and should note: 'No explicit SPV wait, but requires the wallet backend to be initialized (run shielded_init or any SPV-gated tool first in standalone mode).' Alternatively, add ensure_spv_synced to shielded_address_get's invoke() to make the sequencing requirement explicit and self-enforcing.

### CODE-003 (MEDIUM) *(overall=0.45, risk=0.40, impact=0.45, scope=0.50)*: MCP_TOOL_DEVELOPMENT.md SPV gate rule stale: new SPV-skip tools not covered

- **Location**: `docs/MCP_TOOL_DEVELOPMENT.md:100`
- **Description**: The SPV gate rule says: 'Skip [ensure_spv_synced] only for metadata tools that make no network calls (core_wallets_list, network_info, tool_describe).' This PR adds three more tools that skip ensure_spv_synced: core_wallet_import (imports locally, no network call), shielded_balance_get (reads AppContext atomics, no network call), and shielded_address_get (calls wallet_backend() but no ensure_spv_synced). The updated MCP.md (line 102) already documents the broader exemption list, but MCP_TOOL_DEVELOPMENT.md — the canonical checklist for new tool authors — was not updated. A contributor following the checklist will add ensure_spv_synced to every new wallet-facing tool, including future read-only tools that legitimately should skip it.
- **Recommendation**: Update MCP_TOOL_DEVELOPMENT.md line 100 to match MCP.md's broader SPV-skip criteria: 'Skip for tools that make no network calls: metadata tools (core_wallets_list, network_info, tool_describe), local wallet import (core_wallet_import), and pure snapshot reads that read only AppContext atomics (shielded_balance_get). For tools that access wallet_backend() without network calls, document the backend-wired prerequisite instead of gating on SPV.'

### CODE-004 (MEDIUM) *(overall=0.44, risk=0.32, impact=0.50, scope=0.50)*: Typed passphrase parked in one global egui cache slot keyed only by window title — funds-safety, secret-handling, correctness, ux

- **Location**: `src/ui/components/passphrase_modal.rs:124`
- **Description**: passphrase_modal() moved all per-modal state — including the PasswordInput holding the typed (mlock-protected) passphrase — out of the caller and into egui's global data cache, keyed solely by window_title: `egui::Id::new("passphrase_modal_state").with(config.window_title)`. The slot is removed only on the Submit/Cancel resolution paths. Two consequences:

1. Cross-instance state bleed. All 34 screens that own a WalletUnlockPopup pass the constant title "Unlock Wallet", so every one of them shares the SAME cache slot. WalletUnlockPopup::open() and close() no longer touch the field (the old code called password_input.clear() and reset focus_requested on open). If a modal stops being rendered while Pending without going through Submit/Cancel — e.g. the owning screen's `if self.wallet_unlock_popup.is_open() && let Some(wallet) = &self.selected_wallet` guard drops `selected_wallet` to None from an async refresh, or a task result swaps the visible screen — the entry is never cleared. Re-opening any screen's unlock dialog then renders pre-filled with the passphrase typed in the previous context, against a possibly different wallet.

2. Secret lifetime. The abandoned-Pending entry is never zeroized until it is overwritten by the same key or the app exits (mlock keeps it off swap, but it lingers past the point the user believes the dialog was dismissed). The old design had the caller own the PasswordInput and zeroize it deterministically on open/close.

This is a regression of the open() contract: its doc says state is "Reset on open", but the password field is now outside the struct and is not reset.
- **Recommendation**: Key the cache entry by something unique to the logical prompt (e.g. fold the wallet seed_hash or a per-instance salt into the Id) instead of the human-readable title, so distinct popup instances cannot collide. Additionally restore the open()/close() reset guarantee: have WalletUnlockPopup/ActivePrompt clear the cached PassphraseModalState (a small `passphrase_modal_reset(ctx, id)` helper) on open and on programmatic close, so an abandoned-Pending secret is zeroized promptly and a reopened dialog always starts empty.

<details><summary>text</summary>

```text
let state_id = egui::Id::new("passphrase_modal_state").with(config.window_title);
let mut state: PassphraseModalState = ctx
    .data(|d| d.get_temp::<PassphraseModalState>(state_id))
    .unwrap_or_else(|| PassphraseModalState {
        password_input: PasswordInput::new().with_hint_text(config.input_placeholder),
        focus_requested: false,
    });
```

</details>

<details><summary>text</summary>

```text
pub fn open(&mut self) {
    self.is_open = true;
    self.error = None;
    self.remember = false;
}
```

</details>

### CODE-005 (LOW) *(overall=0.38, risk=0.30, impact=0.35, scope=0.50)*: Module Placement Policy claims 'one struct per file' for src/mcp/tools/ — wrong

- **Location**: `CLAUDE.md:84`
- **Description**: The newly added policy bullet reads: '`src/mcp/tools/` — MCP tool logic (one struct per file); never in `src/bin/det_cli/`.' The actual pattern, consistent across the codebase and stated explicitly in docs/MCP_TOOL_DEVELOPMENT.md ('Add a new file or extend an existing file following the domain grouping (wallet.rs, platform.rs, network.rs, etc.)'), is one file per domain with multiple tool structs per file. src/mcp/tools/wallet.rs contains 6 tool structs (GenerateReceiveAddress, WalletBalancesQuery, SendCoreFunds, FetchPlatformBalances, ImportWallet, ListWalletsTool); src/mcp/tools/shielded.rs now contains 9. The parenthetical 'one struct per file' was likely intended to echo the MCP_TOOL_DEVELOPMENT.md rule 3 ('One tool struct = one BackendTask dispatch') but conflates per-dispatch with per-file. A contributor following CLAUDE.md literally would create nine new files for the nine shielded tools.
- **Recommendation**: Replace '(one struct per file)' with '(one file per domain — e.g. wallet.rs, shielded.rs, identity.rs)' to match the actual pattern and MCP_TOOL_DEVELOPMENT.md. Optionally reference the 'one tool struct = one BackendTask dispatch' rule separately if the per-dispatch constraint is worth preserving.

### CODE-006 (LOW) *(overall=0.33, risk=0.25, impact=0.35, scope=0.40)*: Hardcoded needs_shielded_bind = true turns the JIT scope-entry guard into dead code — dead-code, correctness, ux

- **Location**: `src/context/wallet_lifecycle.rs:683`
- **Description**: bootstrap_wallet_addresses_jit computes needs_bootstrap and needs_registration, then sets `let needs_shielded_bind = true;` and guards with `if !needs_bootstrap && !needs_registration && !needs_shielded_bind { return; }`. Because the last operand is a literal `true`, the condition is always false and the early-return is unreachable; needs_bootstrap and needs_registration are still evaluated (one of them calls backend.is_wallet_registered) but their results no longer affect control flow. The whole point of that guard was to avoid entering the JIT seed scope when there is nothing to do, keeping the steady-state path prompt-free. The accompanying comment claims "the overhead of entering the scope is negligible," which is true for unprotected/session-cached wallets but NOT for an open-but-not-session-cached protected wallet (unlocked for display with 'keep unlocked' off): for those, with_secret_session resolves cache miss → unprotected fast-path → interactive prompt. init_missing_shielded_wallets (fired once when the protocol version first crosses the shielded threshold) iterates exactly those open wallets, so the always-enter behaviour can surface a surprise passphrase prompt where the early-return previously suppressed it.
- **Recommendation**: Make the guard honest. Either (a) derive needs_shielded_bind from a real, cheap, non-prompting signal (a sync 'is this wallet already shielded-bound / is the seed session-cached' check) so already-bound or non-cached wallets keep the prompt-free early-return; or (b) if the unconditional bind is deliberate, delete the vestigial needs_shielded_bind / early-return and the now-unused needs_* computations and state plainly that the scope is always entered for open wallets — and correct the 'negligible overhead' comment to acknowledge the protected-not-cached prompt case.

<details><summary>text</summary>

```text
let needs_bootstrap = wallet
    .read()
    .map(|g| Self::wallet_needs_bootstrap(&g))
    .unwrap_or(false);
let needs_registration = !backend.is_wallet_registered(&seed_hash);
let needs_shielded_bind = true;
if !needs_bootstrap && !needs_registration && !needs_shielded_bind {
    return;
}
```

</details>

### CODE-007 (LOW) *(overall=0.32, risk=0.25, impact=0.30, scope=0.40)*: MCP_TOOL_DEVELOPMENT.md Don'ts table bans direct AppContext calls but ShieldedInit/ShieldedSync do exactly that

- **Location**: `docs/MCP_TOOL_DEVELOPMENT.md:121`
- **Description**: The Don'ts table states: 'Call AppContext methods directly instead of dispatching a BackendTask | Breaks the task system contract; backend errors won't be handled uniformly.' ShieldedInit::invoke calls ctx.wallet_backend()?.ensure_shielded_bound_jit() and backend.warm_shielded_prover() without dispatching any BackendTask. ShieldedSync::invoke calls backend.sync_shielded_now(true) directly. Both tools are intentional control-plane operations (init and sync are not fund-moving state transitions that map cleanly to BackendTask variants), and they work correctly. But the documented prohibition is now violated by two shipped tools, which creates a contradictory message for the next tool author: the rulebook says 'never do this' but the reference implementation does it. McpToolError::TaskFailed wraps the wallet_backend error in both tools, so error handling is still uniform — the stated rationale for the rule does not apply to these cases.
- **Recommendation**: Refine the Don't to: 'Dispatch business logic through AppContext/wallet_backend directly when a BackendTask variant exists for the operation — prefer BackendTask for fund-moving ops. For control-plane operations with no BackendTask analog (e.g. shielded_init, shielded_sync), direct wallet_backend() calls are acceptable; map errors through McpToolError::TaskFailed.' This narrows the prohibition to the actual concern (bypassing BackendTask for ops that already have a variant) without outlawing the legitimate control-plane pattern.

### CODE-008 (LOW) *(overall=0.27, risk=0.20, impact=0.20, scope=0.40)*: CLAUDE.md smoke-test section over-excludes shielded_balance_get and misses core_wallet_import

- **Location**: `CLAUDE.md:170`
- **Description**: The smoke-test section states 'every shielded-* tool' is not a smoke test (waits on the SPV gate). This is incorrect for two of the four new shielded tools: shielded_balance_get makes no network calls and no wallet_backend() call — it reads AppContext atomics directly and returns immediately without SPV; shielded_address_get also skips ensure_spv_synced (though it does need the backend wired). Additionally, core_wallet_import is a new SPV-free tool added in this PR that exercises the full import → DB path without a live network, making it a valid smoke test candidate alongside core_wallets_list. The smoke-test list has not been updated to reflect these additions.
- **Recommendation**: Update the smoke-test 'Not smoke tests' note to carve out shielded_balance_get (add to the runnable smoke-test examples, noting it returns zeros when no wallet has synced) and note core_wallet_import as a candidate alongside core_wallets_list. Adjust the 'every shielded-* tool' blanket to 'every fund-moving shielded-* tool'.

### CODE-009 (LOW) *(overall=0.21, risk=0.15, impact=0.28, scope=0.20)*: shielded_activity/shielded_notes flatten the upstream store error to a String — error-handling

- **Location**: `src/wallet_backend/mod.rs:518`
- **Description**: Both read helpers wrap the coordinator store error via `PlatformWalletError::ShieldedStoreError(e.to_string())` before boxing into TaskError::WalletBackend. The `.to_string()` collapses the typed store error into text, discarding the source chain and structural matchability — the very anti-pattern the project's error rules call out. Both methods are currently #[allow(dead_code)] (Phase-F read path not wired yet), so the blast radius is nil today, but the pattern will ship the moment the activity/notes UI lands.
- **Recommendation**: When these are wired, route the store error through a dedicated typed TaskError variant carrying the concrete store-error type as a #[source] (or extend the upstream conversion) rather than stringifying. If upstream only exposes ShieldedStoreError(String), add a DET-side variant that preserves the original error as #[source] so Debug keeps the chain.

<details><summary>text</summary>

```text
coordinator
    .store()
    .read()
    .await
    .get_activity(subwallet, offset, limit)
    .map_err(|e| TaskError::WalletBackend {
        source: Box::new(
            platform_wallet::error::PlatformWalletError::ShieldedStoreError(e.to_string()),
        ),
    })
```

</details>

### CODE-010 (LOW) *(overall=0.20, risk=0.15, impact=0.15, scope=0.30)*: src/mcp/tools/shielded.rs //! module doc lists only 4 of 9 tools

- **Location**: `src/mcp/tools/shielded.rs:1`
- **Description**: The module-level doc reads: '//! Shielded-related MCP tools: shield, transfer, unshield, withdraw.' This enumeration reflects the four tools that existed before Phase G (ShieldedShieldFromCore, ShieldedShieldFromPlatform, ShieldedTransferTool, ShieldedWithdrawTool) but omits the four new additions: ShieldedInit, ShieldedSync, ShieldedBalanceGet, ShieldedAddressGet — and also ShieldedUnshield. The doc is both outdated (shield/transfer/unshield/withdraw were present before this PR) and incomplete (init/sync/balance_get/address_get were added in this PR). A developer opening the file to orient themselves gets a summary that covers less than half the module.
- **Recommendation**: Update the //! line to: '//! Shielded-pool MCP tools: shielding, transfers, unshielding, withdrawals, and lifecycle ops (init, sync, balance and address reads).' Or expand to two lines if the one-liner becomes unwieldy.

### CODE-011 (LOW) *(overall=0.19, risk=0.12, impact=0.20, scope=0.25)*: refresh_shielded_balance_snapshot doc frames itself as a stopgap for Phase E, which ships in the same PR — documentation

- **Location**: `src/backend_task/shielded/mod.rs:236`
- **Description**: The doc comment reads "This is the read side's producer until the Phase-E on_shielded_sync_completed push writer lands." Phase E (the push writer) landed in this very PR (commit fa0e46de), so the 'until X lands' framing describes a future that is already the present. The method is in fact a permanent, useful immediate-refresh after a confirmed spend (so the UI doesn't wait for the 60s loop) — not a temporary substitute. Per the project's 'describe present state, not history' rule, the historical/aspirational framing will mislead the next reader into thinking it is removable.
- **Recommendation**: Reword to state the present role: an immediate post-operation refresh of the frame-safe snapshot that complements (not substitutes for) the Phase-E sync-completed push writer. Drop the 'until ... lands' phrasing.

<details><summary>text</summary>

```text
/// This is the read side's producer until the Phase-E
/// `on_shielded_sync_completed` push writer lands: it keeps the UI balance
/// current immediately after a spend without waiting for the 60-second sync
/// loop. Best-effort — a failed read leaves the previous snapshot in place.
```

</details>

### CODE-012 (LOW) *(overall=0.18, risk=0.15, impact=0.15, scope=0.25)*: Phase-history narration in ShieldedTask enum and run_shielded_task rustdoc

- **Location**: `src/backend_task/shielded/mod.rs:14,61`
- **Description**: Two doc-comments contain phase-progress history rather than present-state description. Line 14: 'Phase D retired DET's home-grown Orchard subsystem: sync, nullifier scanning, key derivation and the commitment tree are all owned by the upstream coordinator now, so only the five fund-moving operations remain.' Line 61: 'shielded ops added in Phase B'. The coding-best-practices skill mandates present-state-not-history and no tombstone comments. The line-14 passage describes what the old system had and why it was removed — useful context internally but irrelevant once the system has been running for months. 'Phase B' / 'Phase D' labels have no meaning outside the PR development history. An inline comment in src/mcp/tools/shielded.rs:657 ('Phase E writer') has the same issue.
- **Recommendation**: Rewrite line 14-16 to describe what IS, not what was: e.g. 'Sync, nullifier scanning, key derivation, and the commitment tree are owned by the upstream coordinator; only the five fund-moving operations are dispatched through this task.' Remove 'Phase D retired DET's home-grown...' and 'added in Phase B'. Replace 'Phase E writer' inline comment (shielded.rs:657) with 'sync_now fires on_shielded_sync_completed synchronously, so the push snapshot is fresh by the time this returns.'

### CODE-013 (LOW) *(overall=0.15, risk=0.10, impact=0.15, scope=0.20)*: Unnecessary #[allow(clippy::too_many_arguments)] on shield_from_asset_lock — lint-hygiene, dead-code

- **Location**: `src/wallet_backend/mod.rs:296`
- **Description**: shield_from_asset_lock takes 5 non-self parameters (seed_hash, funding, recipient, dummy_outputs, settings). clippy::too_many_arguments fires at 8 (does not count the receiver), so the lint can never trigger here and the #[allow] is dead. It silently sits as a license to grow the signature, defeating the lint's purpose if a future arg pushes it over the threshold without re-review.
- **Recommendation**: Drop the attribute — the function is well under the limit. If the intent is to pre-authorise future growth, prefer the project's #[expect(...)] convention so an unfulfilled expectation surfaces in CI when the args change.

<details><summary>text</summary>

```text
#[allow(clippy::too_many_arguments)]
pub(crate) async fn shield_from_asset_lock(
    &self,
    seed_hash: &WalletSeedHash,
    funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
    recipient: dash_sdk::dpp::address_funds::OrchardAddress,
    dummy_outputs: usize,
    settings: Option<...PutSettings>,
) -> Result<(), TaskError> {
```

</details>

### CODE-014 (LOW) *(overall=0.13, risk=0.10, impact=0.10, scope=0.20)*: src/backend_task/shielded/mod.rs has no module-level //! doc

- **Location**: `src/backend_task/shielded/mod.rs:1`
- **Description**: The file begins immediately with use declarations. The ShieldedTask enum has a clear /// doc (lines 11-18) that explains what the module does, but there is no //! module-level summary. The file-level //! is the entry point for rustdoc module pages and is the first thing tools like rust-analyzer surface for the module. The ShieldedTask doc would serve well as a module doc if promoted.
- **Recommendation**: Add a module-level //! above the use declarations, e.g.: '//! Shielded-pool backend tasks: the five fund-moving operations DET dispatches\n//! into the upstream platform-wallet coordinator.' Keep it to ≤2 lines per the project comment-length convention.

> **Positive observations:** Clean upstream-adapter shape; exhaustive error mapping; no block_in_place in frame paths; -4500 net lines; 873 lib + 88 kittests green.

## Recommendations

### Before Merge (0 items)

### Before Production (4 items)
Findings: CODE-001, CODE-002, CODE-003, CODE-004

### Post Deployment (21 items)
Findings: SEC-001, SEC-002, SEC-003, SEC-004, SEC-005, PROJ-001, PROJ-002, PROJ-003, PROJ-004, DOC-001, DOC-002, CODE-005, CODE-006, CODE-007, CODE-008, CODE-009, CODE-010, CODE-011, CODE-012, CODE-013, CODE-014

## Verdict

Ship-worthy. The funds-handling change is correct and well-tested; remaining findings are LOW/MEDIUM hardening and doc fixes that should land as a follow-up commit, not blockers.

**Action:** Apply the secret-hygiene fixes (redact+zeroize the mnemonic in core_wallet_import), the dead bind-guard + String-error fixes, and the doc-accuracy corrections as a follow-up cleanup commit.
