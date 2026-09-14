# fix(wallet): single source of truth for wallet and key names

Supersedes #624.

## Prior state

A 64-character limit on wallet aliases already existed and was already enforced by rejection at the backend (`validate_wallet_alias` in `register_wallet`, `SingleKeyView::import_wif_with_passphrase` / `set_alias`, and `WalletMetaView::set`). The actual gaps were:

- **No trimming at the authoritative layer.** Create/import screens and the rename dialog saved names verbatim; only the single-key import dialog and the MCP import tool trimmed, each in its own way.
- **No invisible / bidi character handling.** A name made of zero-width characters looked blank but was saved; bidirectional overrides (U+202A–U+202E, U+2066–U+2069) could make one wallet's displayed name mimic another's.
- **No uniqueness enforcement.** Default names were `count + 1` (`"Wallet {n}"` / `"Key {n}"`), so deleting "Wallet 2" and creating a new unnamed wallet produced a second "Wallet 2"; `.unwrap_or(0)` also masked a poisoned lock by restarting the count. `mcp::resolve::wallet()` returned the first alias match, so fund-moving MCP tools (`core_funds_send`, `identity_credits_*`, shielded tools) could silently act on the wrong one of two same-named wallets.
- **Four divergent implementations** of the rule: `model::wallet::validate_wallet_alias`, `import_single_key.rs`'s own `ALIAS_MAX_CHARS` + trim, the MCP tool's bespoke trim/filter, and the screens' ad hoc `count + 1` defaults.

## What changes

- **One model API** — `src/model/wallet/alias.rs`:
  - `resolve_alias(raw, default)` strips Unicode `Cc`/`Cf` characters, trims, substitutes the caller's default when blank, and enforces `MAX_WALLET_ALIAS_CHARS` (64) on the cleaned text.
  - `next_default_alias` picks the smallest unused "Wallet N" / "Key N".
  - `ensure_alias_unique` compares cleaned names within one wallet kind.
  - `AliasError` (`TooLong` / `AlreadyUsed`) is a `thiserror` type wired into `TaskError::InvalidWalletAliasLength` and the new `TaskError::WalletAliasAlreadyUsed` as a typed `#[source]`.
  - `AliasSource::{UserEntered, Preserved}` tells persistence entry points whether a name was typed or carried over from legacy data.
- **Atomic enforcement at the backend.** Resolution (cleaning, default name, uniqueness) runs inside the write path, so it cannot race with the save:
  - **HD wallets:** `register_wallet` and `rename_hd_wallet` hold a new `AppContext` alias-writer lock from resolution until the name is persisted and visible in `wallets`. Lock order: alias lock → per-wallet rename lock → `wallets` → inner wallet. `rename_hd_wallet` now also mirrors the saved name into the in-memory wallet under that lock.
  - **Single keys:** `SingleKeyView` holds the index write guard from resolution until insert.
  - A wallet never collides with itself: renaming to the current name and re-importing are allowed, and a re-import is still reported as `WalletAlreadyImported`.
- **Blank always means "default name"**, including when renaming. Clearing a name in the rename dialog is dispatched as-is and resets the wallet/key to the first unused "Wallet N" / "Key N"; the rename result carries the saved name. This keeps the "empty alias clears the name" capability that #624 regressed.
- **Ambiguity rejection in MCP.** `mcp::resolve::wallet()` matches cleaned aliases and returns `InvalidParam` when more than one loaded wallet matches, asking for the hex seed hash. This closes the path for wallets that already share a name on disk.
- **One UI widget.** `ui/components/alias_input.rs` (`AliasInput`) shows a counter of the cleaned length (the same count the validator uses) and truncates live input on a grapheme-cluster boundary (`unicode-segmentation`).
  - Used by: Create Wallet, Import Wallet (recovery phrase and private key), the rename dialog (HD and single key), and the Import private key dialog. The MCP `core_wallet_import` tool passes the raw alias to `register_wallet`, so it goes through the same resolution.
  - `ALIAS_MAX_CHARS` and `validate_wallet_alias` are removed.
- **Legacy data is untouched.**
  - Wallets and keys that are unnamed or share a name keep their stored names; no synthetic name is written by hydration, migration, restore, or non-rename metadata writes. `WalletMetaView::set` only length-checks the stored alias; the migration/restore imports use `AliasSource::Preserved`.
  - Only an explicit create/import/rename runs the new resolution.

## Known limitation

U+200D (zero-width joiner) and emoji tag characters are Unicode `Cf`, so they are stripped. ZWJ emoji sequences (e.g. 👨‍👩‍👧 → 👨👩👧) and subdivision flags decompose into their component glyphs once saved. This is deliberate: spoofing resistance in a field that identifies where funds are sent from outweighs emoji fidelity. Live truncation still never splits a grapheme cluster in the input buffer.

## Files touched

- `Cargo.toml`, `Cargo.lock` — direct deps on `unicode-general-category` 1.1 and `unicode-segmentation` 1.13 (both already in the dependency graph via egui/epaint)
- `benches/wallet_hydration.rs`
- `docs/user-stories.md` — WAL-001, WAL-002, WAL-003, WAL-005 acceptance criteria
- `src/backend_task/error.rs`
- `src/backend_task/migration/finish_unwire.rs`
- `src/backend_task/migration/single_key_restore.rs`
- `src/backend_task/wallet/mod.rs`
- `src/backend_task/wallet/rename_wallet.rs`
- `src/context/mod.rs`
- `src/context/wallet_lifecycle/registration.rs`
- `src/context/wallet_lifecycle/tests.rs`
- `src/mcp/resolve.rs`
- `src/mcp/tools/wallet.rs`
- `src/model/wallet/alias.rs` (new)
- `src/model/wallet/mod.rs`
- `src/ui/components/README.md`
- `src/ui/components/alias_input.rs` (new)
- `src/ui/components/mod.rs`
- `src/ui/wallets/add_new_wallet_screen.rs`
- `src/ui/wallets/import_mnemonic_screen.rs`
- `src/ui/wallets/import_single_key.rs`
- `src/ui/wallets/wallets_screen/mod.rs`
- `src/wallet_backend/det_signer.rs`
- `src/wallet_backend/secret_access.rs`
- `src/wallet_backend/single_key.rs`
- `src/wallet_backend/wallet_meta.rs`
- `tests/kittest/add_new_wallet_screen.rs` (new)
- `tests/kittest/import_mnemonic_screen.rs` (new)
- `tests/kittest/import_single_key.rs`
- `tests/kittest/main.rs`
- `tests/kittest/wallets_screen.rs`

## Test plan

- Model units (`model::wallet::alias`):
  - invisible and bidi stripping (`"\u{200B}"`, `"\u{FEFF} "`, `"\u{202E}abc"`, plain whitespace, empty string);
  - 64/65-character boundaries in ASCII and multibyte text;
  - default-name gap filling and namespaces;
  - uniqueness on cleaned names.
- Backend:
  - `register_wallet` default name, cleaning, duplicate rejection before seed write, and re-import;
  - `rename_hd_wallet` blank → default, cleaning plus in-memory mirror, own name, other wallet's name;
  - single-key import and rename equivalents, and preserved legacy imports;
  - `WalletMetaView::set` keeps an empty alias;
  - MCP resolver ambiguity handling.
- Widget units: grapheme-safe truncation, and the counter matching the validator.
- kittest:
  - Create Wallet and Import Wallet (HD and key): cleaned save, blank → default, duplicate rejected;
  - rename dialog for HD and single key: blank dispatch → default applied and dialog closed, and cleaned counter;
  - Import private key dialog: cleaned counter and raw alias in the request; blank → "Key 1", duplicate rejected.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
