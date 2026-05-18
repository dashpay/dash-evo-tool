# Verification Findings

**Purpose:** All source-verified findings from the investigation phase: DIP-14/15 derivation parity, `DiskStorageManager` byte-compat, PR #3625 drift check, and the Phase-0 runtime probe specifications.

[← back to README](README.md)

---

## Verdicts Summary

| Finding | Verdict | Gate |
|---|---|---|
| E.1 DIP-14/15 DashPay derivation parity | DIVERGENT at source (large-index path); requires runtime golden-vector probe | Phase-0 mandatory before Phase 1; hard gate for Phase 4 |
| E.2 `DiskStorageManager` byte-compat | UNDETERMINABLE by inspection; strong prior toward compatible | Phase-0 mandatory; fallback UX decision needed (see [open-questions.md § #4](open-questions.md)) |
| E.3 PR #3625 drift check | No drift since prior read; still open, draft, not merged | Gate G1 unresolved |
| E.4 Phase-0 residual probes | 4 probes specified; none yet run | All mandatory before Phase 1 |

---

## E.1 — DIP-14/15 DashPay Derivation Parity

**Verdict: DIVERGENT at source level for large identity indices; provably equivalent only in the low-index path. A runtime golden-vector probe is MANDATORY in Phase 0 and is the hard gate for Phase 4.**

### What Was Compared

**dash-evo-tool** (`src/backend_task/dashpay/dip14_derivation.rs` + `hd_derivation.rs`):
- Path: `m/9'/5'/15'/account'` via standard BIP32 (`dip14_derivation.rs:176`), then `ckd_priv_256` for sender then recipient (`dip14_derivation.rs:186-191`)
- `identifier_to_256bit_index` = raw 32-byte identifier (`dip14_derivation.rs:199-203`)
- `ckd_priv_256` is hand-rolled CKDpriv256: `HmacEngine::<sha512>` over parent chain code, `0x00||ser256(k)||ser(i)` hardened / `ser_P(point)||ser(i)` non-hardened, `add_tweak` mod n (`dip14_derivation.rs:18-90`)
- Address = `Address::p2pkh` (`hd_derivation.rs:54`)

**Upstream** (`packages/rs-platform-wallet/src/wallet/identity/crypto/dip14.rs`):
- Path: `m/9'/coin'/15'/account'/(sender_id)/(recipient_id)`; first four hardened BIP32, last two DIP-14 256-bit non-hardened
- Identifier → raw 32-byte buffer (`sender_id.to_buffer()`), not hashed
- 256-bit derivation delegated to `key_wallet::bip32` via `ChildNumber::Normal256`/`Hardened256` (not hand-rolled)
- Address = P2PKH from contact-xpub child pubkey
- Functions: `derive_contact_xpub`, `derive_contact_payment_address(_es)`, `calculate_account_reference`

### Equivalence Analysis

**Same at the contract level:**
- Identical path semantics (`9'/coin(5)'/15'/account'` + two non-hardened identity levels)
- Identical identifier-to-index encoding (raw 32 bytes, not hashed — they agree)
- Identical address scheme (P2PKH)
- CKDpriv256 algorithm is the standard DIP-14 HMAC-SHA512+tweak in both

**Divergence risk — `ChildNumber` storage/round-trip:**
dash-evo-tool's `index_to_child_number` (`dip14_derivation.rs:213-240`) collapses the 256-bit index to a 31-bit value via `sha256(index)[0..4] & 0x7FFFFFFF` when storing into the legacy `ChildNumber`. Upstream uses native `ChildNumber::Normal256` (full 256-bit, no lossy collapse). The derived key/address is computed from the full 256-bit index in both (so addresses likely match), but anywhere dash-evo-tool persisted or compared the collapsed `ChildNumber` form, the two representations are not interchangeable.

Whether the on-curve derived address actually matches cannot be proven by inspection alone: the two CKD implementations (hand-rolled vs `key_wallet`) must produce byte-identical `I_L` for the same 256-bit input — plausible but not guaranteed (endianness of `ser_256(i)`, point-serialization choice).

### Phase-0 Golden-Vector Probe Specification (Mandatory)

**Inputs:**
- Fixed BIP39 seed (publish the mnemonic in the test)
- `network = Testnet` and `Mainnet`
- `account = 0`
- A fixed `sender_id` and `recipient_id`, run in two identifier classes:
  - (a) "small" — first 28 bytes are zero (`is_index_less_than_2_32` true path)
  - (b) "large" — high bytes set (full 256-bit path)
- For each: derive contact xpub then payment addresses at `index = 0..5`

**Assertions — byte-equality of:**
1. The contact `ExtendedPubKey` serialized bytes from dash-evo-tool `derive_dashpay_incoming_xpub` vs upstream `derive_contact_xpub`
2. Each `Address` string from `derive_payment_address` vs upstream `derive_contact_payment_address`
3. `calculate_account_reference` output (`hd_derivation.rs:130` vs upstream `calculate_account_reference`)

**On mismatch:** Phase 4 deletion is blocked. The divergence becomes a migration-tool problem, not a silent swap. See [open-questions.md § #3-resid](open-questions.md) for the policy decision and [open-questions.md § DIP-14/15 Mismatch Handling](open-questions.md#dip-1415-mismatch-handling-policy) for the Phase-4 startup sanity-check design.

---

## E.2 — `DiskStorageManager` Byte-Compat

**Verdict: UNDETERMINABLE by inspection — runtime probe MANDATORY in Phase 0. Strong prior toward "compatible" with a defined rebuild fallback.**

### Reasoning

`DiskStorageManager` (`dash_sdk::dash_spv::storage`, rust-dashcore rev pinned via platform) persists chain/header/filter/wallet-tx state for the `WalletInfoInterface` impl. `PlatformWalletInfo` contains an unchanged `ManagedWalletInfo`; its `WalletInfoInterface`/`ManagedAccountOperations`/`WalletTransactionChecker` impls delegate to that inner `ManagedWalletInfo`. If the on-disk shape is keyed off `WalletId` + the `ManagedWalletInfo` serialization (unchanged), it is byte-compatible. However, `PlatformWalletInfo` may alter what the interface reports (e.g., extra accounts/identity-derived watch addresses) and thus what `DiskStorageManager` writes — not provable without running both and diffing the data directory. The `key_wallet`-native vs hand-rolled CKD question (E.1) compounds this.

### Probe Specification

In Phase 0: sync a throwaway wallet to a fixed height with `WalletManager<ManagedWalletInfo>`, snapshot the SPV data directory; repeat with `WalletManager<PlatformWalletInfo>` (same seed/height); diff. Byte-identical chain/header/filter files → compatible.

### Fallback UX if Not Compatible

> NOTE: The UX approach requires a decision from the user — see [open-questions.md § #4](open-questions.md).

Architect recommendation: silent re-sync with info banner. On first launch after the platform-pin bump, detect a schema/version marker mismatch, call `SpvManager::clear_data_dir()` (`src/spv/manager.rs:800`), and re-sync transparently with the existing "SPV sync in progress…" banner (`src/app.rs:879`).

Rationale (A04 fail-safe, UX): the data directory is a cache, not authoritative (wallet truth is the encrypted seed + SQLite). A modal prompt asking users about an internal cache is jargon and self-resolving anyway. Surface a one-line info banner: "Updating wallet data for the new version. This may take a few minutes." — actionable, calm, no technical detail (CLAUDE.md error-message rules). Do not wipe without the version-marker check (avoid gratuitous re-sync every launch).

---

## E.3 — Re-Confirmation at PR #3625 Head (Drift Check)

All facts verified at PR head `738091f734e05c7a1b822bb1ebff336c93b67891`. No drift found since prior read.

**`PlatformWalletPersistence` signature:** UNCHANGED. Read directly from `packages/rs-platform-wallet/src/changeset/traits.rs` at head — four methods exactly as documented in [architecture.md](architecture.md). No drift.

**`ClientStartState.wallets` `load()` gap:** STILL OPEN. Confirmed at head in `packages/rs-platform-wallet-storage/src/sqlite/persister.rs`: constant `LOAD_UNIMPLEMENTED = ["ClientStartState::wallets"]`; rustdoc "Partial reconstruction caveat" — "leaves `ClientStartState::wallets` empty — the latter requires an upstream `Wallet::from_persisted` constructor that doesn't exist yet." `load()` populates only `platform_addresses`. This is Gate G2.

**PR merge state:** OPEN, DRAFT, NOT MERGED. `state:"open"`, `draft:true`, `merged:false`, `mergeable_state:"unknown"`, base `v3.1-dev` (`54322f7a…`), head `738091f734…`, 17 commits, +6630/-24, milestone v3.1.0. Last updated 2026-05-14. No state drift; still not pinnable by dash-evo-tool. This is Gate G1.

---

## E.4 — Residual Runtime Probes: All Mandatory Before Phase 1

The following probes must run in Phase 0 before Phase 1 starts. None have been run yet.

| # | Probe | Purpose | Pass condition |
|---|---|---|---|
| 1 | DIP-14/15 golden-vector parity (E.1 spec) | Determine if hand-rolled and upstream derivation produce byte-identical output | Both identifier classes, both networks, xpub + addresses + account-reference byte-equal |
| 2 | `DiskStorageManager` data-dir diff (E.2) | Determine cache compat vs silent-rebuild | Chain/header/filter files byte-identical between `ManagedWalletInfo` and `PlatformWalletInfo` runs |
| 3 | `load()` round-trip smoke | Confirm `store`/`flush`/`load` round-trips identity+contact+UTXO state; explicitly confirm `wallets` returns empty | `wallets` field is empty; identity/contact/UTXO state survives round-trip |
| 4 | `PlatformWalletInfo` as `WalletManager<W>` drop-in | Confirm trait coverage for all ops `reconcile_spv_wallets` calls | `get_wallet_balance`, `wallet_utxos`, `wallet_transaction_history`, `accounts()` compile and return correctly against `src/context/wallet_lifecycle.rs:757-985` |
