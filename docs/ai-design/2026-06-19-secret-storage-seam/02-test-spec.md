# Test Case Specification — Wallet Secret Storage Raw-`SecretBytes` Seam

Phase 1c (Test Case Specification) for the security feature unifying all wallet
secret storage onto a no-serialization raw-`SecretBytes` seam, dropping DET's
AES-GCM envelopes, with `InVault` per-use JIT identity signing and a dual-format
migration.

This document is the **TDD contract** Phase 2 (`developer-bilby`, T1–T11)
implements against. It is **specifications, not code**. Tests are written first
(must fail before implementation), then made to pass.

## Source-of-truth references

- Execution plan: `~/.claude/plans/snazzy-marinating-sun.md`
- Full design (T1–T11, T10 list, blast radius): `~/.claude/plans/snazzy-marinating-sun-agent-ae6181c0dc23bdba8.md`
- In-scope findings: `bee9c055` (HIGH — identity keys plaintext at rest),
  `6a2818cd` (MED — `ClosedSingleKey` Debug leak), `f0d946ed` (LOW — zeroize
  transient plaintext).

> Marvin's note. Brain the size of a planet, and I am asked to enumerate the
> ways cryptographic plumbing might betray its own spec. I have done it
> thoroughly, because at least someone should. Every case below fails first by
> construction — that is the point.

---

## Conventions

### Test tiers

| Tag | Meaning | Where it lives | Runs in CI? |
|---|---|---|---|
| **unit** | `#[test]` / `#[tokio::test]` inline in the module under test | source `#[cfg(test)] mod tests` | yes |
| **integration (lib)** | exercises `AppContext` / wallet-backend wiring without GUI, offline | source `#[cfg(test)]` (e.g. `wallet_lifecycle.rs`) or a lib integration test | yes |
| **kittest** | egui UI surface via `egui_kittest::Harness` | `tests/kittest/` | yes |
| **backend-e2e(network)** | live testnet via SPV, `#[ignore]` | `tests/backend-e2e/` | **no** (manual / funded) |
| **compile-fail** | a `compile_fail` doctest or `trybuild` case asserting a type does NOT compile | source doctest (preferred) or `tests/trybuild/` | yes |

### Funded-wallet flag

Cases tagged **[FUNDED-TESTNET — OUT OF CI]** require `E2E_WALLET_MNEMONIC` (a
pre-funded testnet wallet ≥ 10 tDASH) and live DAPI/SPV. They are `#[ignore]`
and must never be forced into a no-network run (see `tests/backend-e2e/README.md`).

### Shared test fixtures (already exist — reuse, do not reinvent)

- `open_secret_store(path)` → `Arc<SecretStore>` over a file vault at `secrets.pwsvault` (empty global passphrase, 0700 parent). `wallet_seed_store.rs::tests::fresh_store`, `single_key.rs::tests::fresh_view*`.
- `secret_prompt::test_support::{TestPrompt, ScriptedAnswer}` — scripted prompt double; `TestPrompt::never()` panics if asked (proves no-prompt); `ask_count()` / `requests()` assertions.
- `NullSecretPrompt` — headless host; `is_interactive() == false`, every request resolves `SecretPromptCancelled` → `TaskError::SecretPromptUnavailable`.
- `assert_no_leak(rendered, secret, context)` (in `encrypted_key_storage.rs::tests`) — asserts a secret appears in **neither** lowercase-hex **nor** decimal-array (`[160, 167, …]`) form. **Promote this to a shared test util** so the seam/sidecar/QI on-disk-leak cases can call it. The decimal-array check is load-bearing: a `#[derive(Debug)]` on `[u8; N]` leaks the decimal form, and the original `6a2818cd` bug leaked exactly that.
- Offline `AppContext`: `offline_testnet_context()` and `seed_legacy_protected_hd_wallet_row(...)` (in `wallet_lifecycle.rs` tests / `database::test_helpers`) — the staging used by `protected_wallet_registers_upstream_on_unlock_without_restart`, the template for the lazy-migration integration case.
- Deterministic key material: `known_wif()` / `known_testnet_wif()` (single-key tests); fixed seed bytes (`[0x42u8; 64]`) and a sentinel passphrase pattern (`SENTINEL_*`) for leak/confinement assertions.

### Leak-assertion discipline (applies to every no-leak case)

Always assert the **plaintext** secret (raw 32/64 bytes), in BOTH hex and
decimal-array form, is absent. Never assert only on a derived/ciphertext value
(that would pass against the very bug we guard). For passphrases, assert the
literal passphrase string is absent.

---

## Traceability matrix (case → T-task → finding)

| Case ID | Tier | T-task | Finding |
|---|---|---|---|
| TS-INV-01 / 02 / 03 | compile-fail / unit | T2, T10 | R-INVARIANT |
| TS-RT-01 (HD) | unit | T2, T6, T10 | bee9c055 |
| TS-RT-02 (single key) | unit | T2, T6, T10 | bee9c055 |
| TS-RT-03 (identity key) | unit | T2, T6, T10 | bee9c055 |
| TS-EAGER-01 (no-pw seed) | integration (lib) | T7, T10 | bee9c055 |
| TS-EAGER-02 (unprotected single key) | unit | T7, T10 | bee9c055 |
| TS-EAGER-03 (identity key) | integration (lib) | T7, T10 | bee9c055 |
| TS-EAGER-04 (idempotent) | unit | T7, T10 | R-MIGRATION-CRASH |
| TS-CRASH-01 / 02 | unit | T7, T10 | R-MIGRATION-CRASH |
| TS-LAZY-01 (unlock migrates) | integration (lib) | T7, T10 | bee9c055 / R-PROMPT-BOUNDARY |
| TS-LAZY-02 (second unlock prompt-free) | integration (lib) | T7, T10 | R-PROMPT-BOUNDARY |
| TS-LAZY-03 (single-key protected) | unit | T7, T10 | bee9c055 |
| TS-LAZY-KIT-01 (modal once) | kittest | T7 | R-PROMPT-BOUNDARY / R-SEC-201 |
| TS-LEGACY-01 (HD legacy read) | unit | T3, T6, T10 | R-MIGRATION-CRASH |
| TS-LEGACY-02 (single-key legacy read) | unit | T3, T6, T10 | R-MIGRATION-CRASH |
| TS-HEADLESS-01 (pw wallet served) | integration (lib) | T7, T10 | R-HEADLESS-SPLIT |
| TS-HEADLESS-02 (no migration headless) | integration (lib) | T7, T10 | R-HEADLESS-SPLIT |
| TS-RESID-01 (InVault only) | unit | T1, T7, T10 | bee9c055 |
| TS-RESID-02 (old blob decodes) | unit | T1, T10 | bee9c055 |
| TS-NOLEAK-01 (seam blob) | unit | T2, T10 | bee9c055 |
| TS-NOLEAK-02 (sidecar) | unit | T5, T10 | bee9c055 |
| TS-NOLEAK-03 (QI blob InVault) | unit | T1, T10 | bee9c055 |
| TS-FAST-01 (headless identity resolve) | unit | T3, T7, T10 | bee9c055 / R-HEADLESS-SPLIT |
| TS-DEL-01 (identity delete) | unit | T7, T10 | bee9c055 |
| TS-DEL-02 (wallet/single-key delete) | unit | T6, T10 | bee9c055 |
| TS-DBG-01 (ClosedSingleKey Debug) | unit | T9 | 6a2818cd |
| TS-MISS-01 (SecretSeamMissing) | unit | T4, T7, T10 | R-MIGRATION-CRASH |
| TS-MISS-02 (loud not silent) | unit | T4, T7, T10 | R-MIGRATION-CRASH |
| TS-META-01 / 02 (WalletMeta schema gate) | unit | T5 | R-SCHEMA |
| TS-ZERO-01 (transient plaintext zeroized) | unit | T6, T9 | f0d946ed |
| TS-SIGN-E2E-01 (testnet ST) | backend-e2e(network) | T7, T8, T11 | bee9c055 |

---

## 1. No-serialization invariant guard (R-INVARIANT)

The whole architecture rests on `SecretBytes` having **no** `Serialize` (verified
in pinned platform `b4506492`). The guard is the canary if upstream ever adds it.

### TS-INV-01 — `SecretBytes` is not `Serialize`/`Encode` (compile-fail)

- **Tier:** compile-fail (preferred: a `compile_fail` doctest on the seam module; alternative: `trybuild` case — note `trybuild` is **not** a current dependency, adding it is a Phase-2 decision).
- **T-task / finding:** T2, T10 / R-INVARIANT.
- **Preconditions:** seam module exists; no test-only `Serialize` shim for `SecretBytes`.
- **Steps:**
  1. A doctest fragment attempts to derive `Serialize` (and separately `bincode::Encode`) on a newtype `struct Leaky(SecretBytes);`.
  2. A second fragment attempts `serde_json::to_string(&secret_bytes_value)`.
- **Expected outcome:** each fragment **fails to compile** (`SecretBytes: !Serialize`, `!Encode`). The test asserts the failure, not a runtime value.
- **Why it bites:** if a future upstream adds `Serialize` to `SecretBytes`, this case starts compiling — and FAILS — flagging that the invariant has silently weakened.

### TS-INV-02 — seam accepts/returns `SecretBytes`, never a serde struct (unit)

- **Tier:** unit. **T-task:** T2, T10.
- **Preconditions:** `SecretSeam::{put_secret,get_secret,delete_secret}` defined.
- **Steps:** assert the signatures: `put_secret(scope, label, secret: &SecretBytes)`, `get_secret(...) -> Result<Option<SecretBytes>, TaskError>`. (Encoded as a real call site that round-trips a `SecretBytes`; the compiler is the assertion.)
- **Expected outcome:** compiles and round-trips; no intermediate serializable wrapper type is constructed in the seam body.

### TS-INV-03 — audit guard over the changed secret-path modules (unit)

- **Tier:** unit. **T-task:** T10.
- **Preconditions:** the changed modules (`secret_seam`, `wallet_seed_store`, `single_key`, `identity_key_store`, `secret_access`, `encrypted_key_storage`) are listed in a const array in the test.
- **Steps:** a source-text audit test reads each module file and asserts no struct that `#[derive(Serialize)]`/`#[derive(Encode)]` also names a `SecretBytes` / `Zeroizing<[u8` / plaintext-key field. (Text-level guard — the compiler already forbids the strongest case via TS-INV-01; this catches a `Vec<u8>`-shaped plaintext field that bypasses the type guard.)
- **Expected outcome:** zero matches. A new serializable struct embedding plaintext fails the audit.
- **Note:** keep the module list in sync with the blast-radius table; a stale list is itself a finding.

---

## 2. Raw round-trip via the seam — all three classes

### TS-RT-01 — HD seed raw round-trip (unit)

- **Tier:** unit. **T-task:** T2, T6, T10. **Finding:** bee9c055.
- **Preconditions:** fresh file vault; `SecretSeam::new(&store)`.
- **Steps:**
  1. `put_secret(seed_hash_scope, "seed.raw.v1", &SecretBytes::from_slice(&seed64))` with a known 64-byte seed.
  2. `get_secret(seed_hash_scope, "seed.raw.v1")`.
- **Expected outcome:** `Some(bytes)` whose `expose_secret()` **equals the exact 64 input bytes** (assert full equality, not just length/non-empty). `get_secret` on a missing label → `Ok(None)`. A different scope (different seed_hash) → `Ok(None)` (scope partition).
- **Anti-pattern rejected:** asserting only `is_some()` or `len() == 64`.

### TS-RT-02 — single-key raw round-trip (unit)

- **Tier:** unit. **T-task:** T2, T6, T10. **Finding:** bee9c055.
- **Preconditions:** fresh vault; the fixed `SINGLE_KEY_NAMESPACE_BYTES` scope; label `single_key_priv.<addr>` (unchanged label scheme).
- **Steps:** `put_secret` raw 32 bytes under the canonical label; `get_secret` it back.
- **Expected outcome:** returned bytes **equal the exact 32 input bytes**; value length is exactly 32 (raw, NOT a `SingleKeyEntry` envelope — assert it does NOT start with `SINGLE_KEY_ENTRY_VERSION` framing). Reading under a foreign `WalletId` → `Ok(None)`.

### TS-RT-03 — identity-key raw round-trip (unit)

- **Tier:** unit. **T-task:** T2, T6, T10. **Finding:** bee9c055.
- **Preconditions:** fresh vault; scope `identity.id().to_buffer()`; label `identity_key_priv.<target>.<key_id>`.
- **Steps:** `put_secret` raw 32 bytes; `get_secret`.
- **Expected outcome:** returned bytes equal the 32 input bytes; two distinct `(target, key_id)` labels under the same identity scope do not collide; two identities (distinct scopes) with the same `key_id` do not collide.

---

## 3. Eager migration (no dialog) — no-password seed, unprotected single key, identity key

Order invariant for ALL eager paths: **vault `put_secret` → sidecar write → legacy delete.**

### TS-EAGER-01 — no-password HD seed migrates on load (integration, lib)

- **Tier:** integration (lib). **T-task:** T7, T10. **Finding:** bee9c055.
- **Preconditions:** a legacy `envelope.v1` `StoredSeedEnvelope` with `uses_password == false` (raw 64-byte seed verbatim) present under `seed_hash`; NO raw `seed.raw.v1` label; a matching `WalletMeta` sidecar absent or pre-migration shape.
- **Steps:** run the hydration/load path (`reconstruct_wallet` / seam `get_secret` miss path).
- **Expected outcome (assert ALL):**
  1. raw `seed.raw.v1` now present and `expose_secret()` equals the original 64-byte seed;
  2. `WalletMeta` sidecar written with `uses_password == false`, `xpub_encoded` carried over, hint preserved;
  3. legacy `envelope.v1` label **deleted** (`store.get(scope,"envelope.v1") == None`);
  4. a fresh reload reads via raw seam (legacy reader not consulted — assert by deleting/absence of legacy and successful resolve).
- **Anti-pattern rejected:** asserting only that the wallet "loads" without verifying the four post-conditions.

### TS-EAGER-02 — unprotected single key migrates (unit)

- **Tier:** unit. **T-task:** T7, T10. **Finding:** bee9c055.
- **Preconditions:** a legacy `SingleKeyEntry` (`has_passphrase == false`) OR a bare legacy 32-byte raw blob under `single_key_priv.<addr>`, with a matching `ImportedKey` sidecar (`has_passphrase == false`).
- **Steps:** run the single-key hydrate/seam-miss migration.
- **Expected outcome:** vault label now holds the **raw 32 bytes** (length 32, no `SingleKeyEntry` framing); `ImportedKey` sidecar present (pubkey-for-locked-render moved into sidecar); legacy framed entry replaced; a subsequent unprotected `sign_with` succeeds and the signature verifies against the WIF-derived pubkey.

### TS-EAGER-03 — identity key migrates from QI blob (integration, lib)

- **Tier:** integration (lib). **T-task:** T7, T10. **Finding:** bee9c055.
- **Preconditions:** a stored `QualifiedIdentity` whose `KeyStorage` contains a `PrivateKeyData::Clear` and a `PrivateKeyData::AlwaysClear` (MEDIUM) identity key.
- **Steps:** load the identity through the path that content-detects `Clear`/`AlwaysClear` and migrates.
- **Expected outcome (assert ALL):**
  1. for each migrated key, raw 32 bytes present in the vault under `identity_key_priv.<target>.<key_id>` equal to the original plaintext;
  2. the rewritten QI blob has `PrivateKeyData::InVault` (placeholder) at those slots — **zero** `Clear`/`AlwaysClear` remain (see TS-RESID-01);
  3. `AtWalletDerivationPath` keys are untouched (not migrated — they were never plaintext-at-rest).

### TS-EAGER-04 — eager migration is idempotent (unit)

- **Tier:** unit. **T-task:** T7, T10. **Finding:** R-MIGRATION-CRASH.
- **Preconditions:** as TS-EAGER-01/02.
- **Steps:** run the migration twice (second run sees raw present, legacy already gone).
- **Expected outcome:** second run is a no-op success; raw value byte-identical after both runs; no error; legacy stays absent. (`SecretStore::set` upserts identical bytes — re-running must not corrupt or duplicate.)

---

## 4. Crash-safety (R-MIGRATION-CRASH)

### TS-CRASH-01 — crash AFTER vault+sidecar, BEFORE legacy delete → recoverable (unit)

- **Tier:** unit. **T-task:** T7, T10.
- **Preconditions:** simulate a partial migration: raw `seed.raw.v1` present AND legacy `envelope.v1` STILL present (the legal mid-migration state).
- **Steps:** run the loader.
- **Expected outcome:** loader **prefers raw** (precedence raw > legacy), serves the raw seed, and the leftover legacy is treated as deletable (deleted on this pass). Resolve succeeds; no key loss; no `SecretSeamMissing`.

### TS-CRASH-02 — never reach raw-missing-legacy-deleted (unit)

- **Tier:** unit. **T-task:** T7, T10.
- **Preconditions:** assert the ordering contract structurally: a migration step that writes raw then deletes legacy must NOT delete legacy if the raw `put_secret` returned `Err`.
- **Steps:** inject a `put_secret` failure (vault error double / read-only store) and run one migration step.
- **Expected outcome:** legacy `envelope.v1` is **still present** after the failed step (delete was not reached); the step surfaces a typed error; a later retry can still recover the seed from legacy. Proves keys are never lost on a mid-write fault.

---

## 5. Lazy migration (password wallet) via the existing unlock dialog (R-PROMPT-BOUNDARY)

### TS-LAZY-01 — unlock migrates a protected HD wallet to raw (integration, lib)

- **Tier:** integration (lib). **T-task:** T7, T10. **Finding:** bee9c055 / R-PROMPT-BOUNDARY.
- **Template:** `wallet_lifecycle.rs::protected_wallet_registers_upstream_on_unlock_without_restart` (offline context + `seed_legacy_protected_hd_wallet_row` + `handle_wallet_unlocked(&wallet_arc, Some(passphrase))`).
- **Preconditions:** a legacy PROTECTED `envelope.v1` (`uses_password == true`, AES-GCM ciphertext) staged; NO raw label; `WalletMeta.uses_password == true` (or derived from legacy).
- **Steps:**
  1. hydrate (wallet locked, not migrated, `uses_password` still true);
  2. `wallet_seed.open(passphrase)` then `ctx.handle_wallet_unlocked(&wallet_arc, Some(passphrase))` — the single existing unlock gesture, routed through `promote_hd_seed_with_passphrase`.
- **Expected outcome (assert ALL):**
  1. legacy envelope decrypted with the supplied passphrase inside the borrowed `Zeroizing` scope;
  2. raw `seed.raw.v1` written, `expose_secret()` equals the true 64-byte seed;
  3. `WalletMeta.uses_password` flipped to **`false`**;
  4. legacy `envelope.v1` deleted;
  5. exactly **one** prompt's-worth of passphrase use — the unlock the user already performs (no second/out-of-band prompt).

### TS-LAZY-02 — second unlock is prompt-free after migration (integration, lib)

- **Tier:** integration (lib). **T-task:** T7, T10. **Finding:** R-PROMPT-BOUNDARY.
- **Preconditions:** state left by TS-LAZY-01 (raw present, `uses_password == false`).
- **Steps:** drive a subsequent secret resolve for the same seed scope through `SecretAccess::with_secret` with a `TestPrompt::never()`.
- **Expected outcome:** resolve succeeds via the unprotected fast-path; `ask_count() == 0`; `can_resolve_without_prompt(scope) == true`; `scope_has_passphrase` now reads `false` from `WalletMeta`.

### TS-LAZY-03 — single-key protected lazy migration via chokepoint (unit)

- **Tier:** unit. **T-task:** T7, T10. **Finding:** bee9c055.
- **Template:** `single_key.rs::sec_002_protected_sign_via_chokepoint` (import protected, `SecretAccess::with_secret(SingleKey)` with `ScriptedAnswer`).
- **Preconditions:** a legacy protected `SingleKeyEntry` (`has_passphrase == true`) and matching sidecar (`has_passphrase == true`).
- **Steps:** drive `with_secret(SingleKey{addr})` with the correct passphrase (one `ScriptedAnswer::once`).
- **Expected outcome:** the legacy entry is decrypted JIT; inside that scope the raw 32 bytes are re-stored via the seam; `ImportedKey.has_passphrase` flipped to `false`; legacy framed entry deleted; a subsequent `with_secret` with `TestPrompt::never()` resolves the SAME key bytes prompt-free, and the recovered bytes equal the WIF plaintext.

### TS-LAZY-KIT-01 — the unlock modal renders once for the migration path (kittest)

- **Tier:** kittest. **T-task:** T7. **Finding:** R-PROMPT-BOUNDARY / R-SEC-201.
- **Template:** `tests/kittest/secret_prompt.rs` (`passphrase_modal` harness).
- **Preconditions:** the passphrase modal chrome unchanged.
- **Steps:** render the modal once; assert body/hint/submit/cancel render; submit a passphrase.
- **Expected outcome:** the migration reuses the existing single unlock modal (no new modal type, no second modal). This is a surface-contract check only — migration logic is covered by TS-LAZY-01/03. (Cross-reference SEC-201 Enter-consume: do NOT fix here; note migration runs the modal more often.)

---

## 6. Legacy-format read during transition

### TS-LEGACY-01 — HD legacy envelope served when raw absent (unit)

- **Tier:** unit. **T-task:** T3, T6, T10. **Finding:** R-MIGRATION-CRASH.
- **Preconditions:** ONLY a legacy `envelope.v1` (no raw label); `uses_password == false` (so no prompt needed for the read assertion).
- **Steps:** call the seam-first / legacy-fallback read path (`decrypt_jit` HdSeed, or the retained `legacy_envelope_get`).
- **Expected outcome:** the 64-byte seed is recovered from the legacy reader and equals the original; the retained legacy decode path is exercised (not an error). For a `uses_password == true` legacy entry, supplying the correct passphrase recovers the seed (the retained AES-GCM reader still functions).

### TS-LEGACY-02 — single-key legacy entry served when raw absent (unit)

- **Tier:** unit. **T-task:** T3, T6, T10. **Finding:** R-MIGRATION-CRASH.
- **Preconditions:** ONLY a legacy `SingleKeyEntry` (versioned framed form) OR bare 32-byte legacy blob; no raw migration yet.
- **Steps:** read via the seam-first / `SingleKeyEntry::decode` fallback.
- **Expected outcome:** the retained `SingleKeyEntry::decode` reader returns the entry; an unprotected legacy entry signs without a passphrase; a protected one routes through the chokepoint. Confirms the decode-only retained reader still works during transition.

---

## 7. Headless / `NullSecretPrompt`

### TS-HEADLESS-01 — password wallet served by legacy reader, no prompt, no failure (integration, lib)

- **Tier:** integration (lib). **T-task:** T7, T10. **Finding:** R-HEADLESS-SPLIT.
- **Preconditions:** a legacy PROTECTED `envelope.v1` (`uses_password == true`); `SecretAccess` built with `NullSecretPrompt`.
- **Steps:** attempt a secret resolve that requires the passphrase for that scope.
- **Expected outcome:** resolve fails with `TaskError::SecretPromptUnavailable` (NOT a panic, NOT `SecretPromptCancelled`); the wallet stays on the legacy reader; `WalletMeta.uses_password` is **still true** (no headless migration); legacy `envelope.v1` is **still present** (not deleted); raw `seed.raw.v1` is **still absent**. Matches the existing `null_prompt_on_protected_scope_yields_unavailable` shape, extended with the no-migration post-conditions.

### TS-HEADLESS-02 — no eager/lazy migration of a protected wallet headless (integration, lib)

- **Tier:** integration (lib). **T-task:** T7, T10. **Finding:** R-HEADLESS-SPLIT.
- **Preconditions:** as TS-HEADLESS-01; run the full headless load/hydration path.
- **Steps:** load + (attempt) migration headlessly; then re-inspect storage.
- **Expected outcome:** storage is byte-for-byte unchanged for the protected wallet (legacy present, raw absent, `uses_password == true`). A **no-password** wallet and identity keys in the SAME headless load DO migrate eagerly (assert their raw labels appear) — proving the split is exactly "protected ⇒ deferred, unprotected ⇒ eager", not "headless ⇒ never migrate".

---

## 8. Identity residency — only `InVault` (R-INVARIANT / bee9c055)

### TS-RESID-01 — a loaded identity has only `InVault`, never Clear/AlwaysClear (unit)

- **Tier:** unit. **T-task:** T1, T7, T10. **Finding:** bee9c055.
- **Preconditions:** an identity migrated per TS-EAGER-03 (or loaded post-migration).
- **Steps:** iterate `KeyStorage.private_keys`.
- **Expected outcome:** every entry that previously carried plaintext is now `PrivateKeyData::InVault`; assert **zero** `Clear` and **zero** `AlwaysClear` variants remain anywhere in the `KeyStorage`. `AtWalletDerivationPath` (wallet-derived) entries are permitted and unchanged. Keys are never resident in memory as plaintext.

### TS-RESID-02 — old QI blob (discriminants 0–3) still decodes after appending `InVault` at index 4 (unit)

- **Tier:** unit. **T-task:** T1, T10. **Finding:** bee9c055.
- **Preconditions:** a bincode blob encoded BEFORE `InVault` was added (variants Clear=0/AlwaysClear=... per current order: `AlwaysClear, Clear, Encrypted, AtWalletDerivationPath`; `InVault` appended last as index 4).
- **Steps:** decode the legacy blob into the new `PrivateKeyData` enum.
- **Expected outcome:** decodes successfully and yields the original variant — appending `InVault` at the highest index must not shift discriminants 0–3. (Guards the bincode-discriminant trap called out as R in the design.)

---

## 9. On-disk no-leak (hex AND decimal-array)

### TS-NOLEAK-01 — seam vault blob contains no raw secret (unit)

- **Tier:** unit. **T-task:** T2, T10. **Finding:** bee9c055.
- **Preconditions:** raw secret stored via the seam for each class (seed, single key, identity key).
- **Steps:** read the on-disk vault file bytes (the `secrets.pwsvault` file), render as a string/byte search.
- **Expected outcome:** because the upstream vault encrypts at rest (Argon2id + XChaCha20-Poly1305 file backend), the plaintext appears in **neither** hex **nor** decimal-array form in the on-disk file. Use the promoted `assert_no_leak`. (This asserts the at-rest file, distinct from `get_secret` which legitimately returns plaintext in memory.)
- **Note:** the seam value in memory IS raw plaintext by design — do not assert no-leak on `get_secret`'s return; assert it on the persisted file.

### TS-NOLEAK-02 — sidecar (`WalletMeta` / `ImportedKey`) contains no secret (unit)

- **Tier:** unit. **T-task:** T5, T10. **Finding:** bee9c055.
- **Preconditions:** a migrated wallet + imported key with sidecars written.
- **Steps:** serialize each sidecar blob (bincode) and the on-disk `det-app.sqlite` k/v value; search.
- **Expected outcome:** neither sidecar's bytes contain the raw seed/key in hex or decimal-array form. The sidecar holds only non-secret metadata (alias, `uses_password`, hint, xpub, pubkey-for-locked-render). The moved single-key pubkey is the **public** key — assert it IS present (locked-render needs it) and the private key is NOT.

### TS-NOLEAK-03 — QI blob carries `InVault` markers, never plaintext (unit)

- **Tier:** unit. **T-task:** T1, T10. **Finding:** bee9c055.
- **Preconditions:** a migrated identity (TS-EAGER-03).
- **Steps:** encode the `QualifiedIdentity` / `KeyStorage` to its persisted bincode blob; search the bytes.
- **Expected outcome:** the identity-key plaintext appears in neither hex nor decimal-array form in the QI blob; the blob encodes `InVault` placeholders for those slots.

---

## 10. Headless identity-key fast-path

### TS-FAST-01 — identity-key resolve under `NullSecretPrompt` succeeds, no prompt (unit)

- **Tier:** unit. **T-task:** T3, T7, T10. **Finding:** bee9c055 / R-HEADLESS-SPLIT.
- **Preconditions:** identity key stored raw via the seam (post-migration); `SecretScope::IdentityKey{...}` with `scope_has_passphrase == false`; `SecretAccess` built with `NullSecretPrompt` (or `TestPrompt::never()`).
- **Steps:** call `resolve_private_key_bytes(target, key_id)` (or `with_secret(IdentityKey)`) and sign/derive.
- **Expected outcome:** resolves the raw 32 bytes prompt-free (`ask_count() == 0`, no `SecretPromptUnavailable`); the resolved key signs and the signature verifies against the identity public key. Proves the unprotected fast-path keeps headless/MCP identity signing working and that `async Signer::sign` (verified at `mod.rs:318`) is a free rider on the resolver.

---

## 11. Delete — vault entries (raw labels) + legacy removed

### TS-DEL-01 — identity removal deletes identity-key vault entries (unit)

- **Tier:** unit. **T-task:** T7, T10. **Finding:** bee9c055.
- **Preconditions:** an identity with raw identity keys stored under `identity_key_priv.<target>.<key_id>`; `purge_identity_scope` (identity_db.rs:229, called at :621) extended to clear the identity's vault scope.
- **Steps:** delete the identity.
- **Expected outcome:** `get_secret` for every `identity_key_priv.*` label under that identity's scope → `Ok(None)`; any legacy form gone; OTHER identities' vault entries untouched (assert a second identity's key still resolves). No orphaned raw secret survives a delete.

### TS-DEL-02 — wallet / single-key removal deletes raw + legacy (unit)

- **Tier:** unit. **T-task:** T6, T10. **Finding:** bee9c055.
- **Preconditions:** a migrated HD wallet (raw `seed.raw.v1`) and a migrated imported key (raw `single_key_priv.<addr>`), each with sidecars.
- **Steps:** forget the imported key (`SingleKeyView::forget`) and delete the wallet.
- **Expected outcome:** raw vault label gone; legacy label gone (idempotent delete of both forms); sidecar entry removed; in-memory index cleared. `forget` on an already-removed address remains `Ok(())` (idempotent). A second wallet's secrets are unaffected.

---

## 12. `ClosedSingleKey` redacting Debug (6a2818cd)

### TS-DBG-01 — `ClosedSingleKey` `{:?}` exposes no raw 32 bytes (unit)

- **Tier:** unit. **T-task:** T9. **Finding:** 6a2818cd.
- **Preconditions:** a `ClosedSingleKey` populated with a distinctive 32-byte value in `encrypted_private_key` (use the `distinctive_secret()` pattern).
- **Steps:** render `format!("{:?}", closed)` and, transitively, `format!("{:?}", SingleKeyData::Closed(closed))` and a `SingleKeyWallet` holding it.
- **Expected outcome:** via the promoted `assert_no_leak`: the 32 bytes appear in **neither** hex **nor** decimal-array form at any level (the decimal-array check is the one the pre-fix derived `Debug` failed); a redaction marker (`[redacted]` / fingerprint) IS present. Mirrors `ClosedKeyItem` and `PrivateKeyData` redaction. Confirms parents `SingleKeyData`/`SingleKeyWallet` are safe by delegation.

---

## 13. `SecretSeamMissing` surfaced loudly (R-MIGRATION-CRASH)

### TS-MISS-01 — label in neither raw nor legacy → typed `SecretSeamMissing` (unit)

- **Tier:** unit. **T-task:** T4, T7, T10.
- **Preconditions:** a wallet/identity/single-key reference whose secret label is present in **neither** raw nor any legacy form (e.g. sidecar exists but both vault forms are gone).
- **Steps:** resolve the secret through the loader/seam-first path.
- **Expected outcome:** `Err(TaskError::SecretSeamMissing)` — a dedicated typed variant (no `String` field per CLAUDE.md error rules), distinct from `WalletNotFound` / `ImportedKeyNotFound` / `SecretDecryptFailed`. **Never** a silent `Ok(None)` that drops a key on the floor.

### TS-MISS-02 — `SecretSeamMissing` is loud, not silent, on the funds-safety path (unit)

- **Tier:** unit. **T-task:** T4, T7, T10.
- **Preconditions:** as TS-MISS-01, on a sign/spend path.
- **Steps:** attempt a sign with the missing secret.
- **Expected outcome:** the operation returns `SecretSeamMissing` (or a class-flavored wrapper carrying it as `#[source]`), surfaced to the banner with an actionable message; the failure is observable, not swallowed. Assert the error variant by structural match, never by parsing the message string.

---

## 14. `WalletMeta` schema-gating (R-SCHEMA)

### TS-META-01 — new `WalletMeta` shape round-trips; old blob detected and migrated (unit)

- **Tier:** unit. **T-task:** T5. **Finding:** R-SCHEMA.
- **Preconditions:** `WalletMeta` gains `uses_password` + `password_hint`; the change is format-breaking for positional bincode behind the `DetKv` schema envelope.
- **Steps:**
  1. round-trip the NEW shape through bincode (mirror `wallet_meta_round_trips_through_bincode`);
  2. write a blob in the OLD shape (no `uses_password`/`password_hint`), bump/read via the schema-version gate.
- **Expected outcome:** new shape round-trips field-for-field; the OLD blob is detected by the schema byte (NOT silently misread via `#[serde(default)]` alone — the design explicitly forbids relying on that) and content-migrated to the new shape with `uses_password` defaulted correctly. A blob read under a mismatched schema version is rejected/migrated, never positionally misparsed.

### TS-META-02 — `uses_password`/`password_hint` survive cold-boot (unit)

- **Tier:** unit. **T-task:** T5.
- **Steps:** write a `WalletMeta` with `uses_password == true` + a hint, drop the in-memory state, re-read.
- **Expected outcome:** both fields recovered exactly; `scope_has_passphrase(HdSeed)` reads them from `WalletMeta` (not the legacy envelope) post-migration.

---

## 15. Zeroize of transient decoded plaintext (f0d946ed)

### TS-ZERO-01 — legacy-reader transient plaintext is `Zeroizing`/`SecretBytes` (unit)

- **Tier:** unit. **T-task:** T6, T9. **Finding:** f0d946ed.
- **Preconditions:** the retained legacy readers (`decrypt_hd_seed`, `SingleKeyEntry::decrypt`) and the migration re-store step.
- **Steps:** assert the decoded-plaintext bindings are typed `Zeroizing<[u8; N]>` / `SecretBytes` (compile-level: the function return types already are — assert they are NOT widened to plain `Vec<u8>`/`[u8; N]` by the migration code). A confinement test (mirror `sentinel_never_appears_in_error_or_debug`) drives a migration and asserts the sentinel plaintext never appears in any error/Debug surfaced by the path.
- **Expected outcome:** transient plaintext is wrapped; no plain `Vec<u8>` copy of a secret escapes the migration scope; sentinel never leaks to error/Debug. Largely subsumed by the seam (`SecretBytes`), this case guards the legacy-reader → seam handoff specifically.

---

## 16. End-to-end signing (network) — out of CI

### TS-SIGN-E2E-01 — broadcast a testnet state transition from a migrated imported-key identity

- **Tier:** backend-e2e(network). **T-task:** T7, T8, T11. **Finding:** bee9c055.
- **[FUNDED-TESTNET — OUT OF CI]** — requires `E2E_WALLET_MNEMONIC`, live DAPI/SPV; `#[ignore]`.
- **Preconditions:** an identity whose signing key was migrated to `InVault` raw storage; a funded testnet wallet.
- **Steps:** trigger a cheap state transition (e.g. an identity update or a DPNS preorder) that signs through the async `QualifiedIdentity` `Signer` → `resolve_private_key_bytes` → `with_secret(IdentityKey)`.
- **Expected outcome:** the ST signs via the InVault per-use JIT path and broadcasts successfully; the platform accepts the proof; the key was never resident as plaintext between signs. Confirms the JIT identity-signing free-rider claim against a live network.
- **Manual fallback (if no funded wallet):** the manual checklist in the execution plan (load a pre-existing protected wallet → unlock → confirm migration + sign + neither vault nor sidecar holds raw bytes). Document the skip per CLAUDE.md when infrastructure is unavailable.

---

## Coverage self-audit (gaps the implementer must NOT silently close)

- **No-serialization guard mechanism is undecided at the dependency level.** `static_assertions` and `trybuild` are NOT in `Cargo.toml`. The preferred zero-dependency mechanism for TS-INV-01 is a `compile_fail` doctest; adding `trybuild` is a Phase-2 call. If the implementer drops the compile-fail case entirely and keeps only the text audit (TS-INV-03), the strongest leg of R-INVARIANT is lost — that is a regression, flag it.
- **The on-disk no-leak cases (TS-NOLEAK-01) depend on the upstream vault actually encrypting at rest.** The accepted interim regression is that the global vault passphrase is empty (deferred `e0a8f4b1`). The XChaCha20-Poly1305 file backend still encrypts under a derived key even with an empty passphrase, so the plaintext should not appear verbatim — but if a future change makes the at-rest format plaintext-equivalent, TS-NOLEAK-01 is the canary. Do not weaken it to "blob != exact in-memory struct".
- **`assert_no_leak` is currently private to `encrypted_key_storage.rs::tests`.** It MUST be promoted to a shared test utility for TS-NOLEAK-01/02/03 and TS-DBG-01. A copy-paste fork is a maintenance finding.
- **TS-INV-03's module list must track the blast-radius table.** A stale list silently shrinks the audit surface.
