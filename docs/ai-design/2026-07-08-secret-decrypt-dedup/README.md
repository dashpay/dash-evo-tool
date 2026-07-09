# Secret-decrypt dedup (audit R2, Wave 1 — CODE-003)

**Date:** 2026-07-08
**Scope:** `model/wallet/encryption.rs`, `wallet_backend/{secret_access,single_key_entry}.rs`

## Decision: local dedup (route a), NOT the upstream per-secret migration (route b)

CODE-003 flagged the AES-256-GCM decrypt sequence (derive Argon2 key → init cipher
→ checked-nonce → decrypt) as copy-pasted across three readers:

- `wallet_backend/secret_access.rs::decrypt_hd_seed` — HD-seed migration reader
- `wallet_backend/single_key_entry.rs::SingleKeyEntry::decrypt` — imported single-key reader
- `model/wallet/encryption.rs::ClosedKeyItem::decrypt_seed` — deprecated `src/database` seed store

The triage rationale proposed adopting the upstream platform per-secret encryption
(XChaCha20-Poly1305 Tier-2 via `SecretStore::set_secret`/`get_secret`, already wired
in `secret_seam.rs` as `put_secret_protected`/`get_secret_protected`) and deleting the
local code.

**We did NOT do that, deliberately.** Those three AES-GCM decrypts are the **legacy
migration reader** for wallet secrets already written to users' disks in DET's own
AES-GCM envelope format. Two authoritative module docs establish this:

- `secret_seam.rs` module doc: *"The only remaining legacy decrypt (for not-yet-migrated
  AES-GCM secrets) lives in the legacy reader."*
- `wallet_seed_store.rs` module doc: the `envelope.v1` row *"is retained DECODE-ONLY as a
  migration reader … rewritten to the raw label on the first load/unlock and then deleted."*

The Tier-2 primitive already exists and the lazy legacy→Tier-2 re-wrap already runs in
`secret_access::decrypt_jit`. Route (b) is therefore a **data migration** whose write side
is already built — the readers must stay until every user's on-disk secret has been
re-wrapped. Deleting them would lock users out of existing wallets.

## What was done (route a)

Extracted one crate-private `decrypt_message(ciphertext, salt, nonce, password, site)`
in `model/wallet/encryption.rs`, returning `Zeroizing<Vec<u8>>` and a two-variant
`DecryptError` (`WrongPassword` = AEAD auth failure; `Malformed` = structural/corrupt).
All three readers route through it, mapping the two variants to their existing domain
errors. Behavior is preserved exactly (same `TaskError` mapping for wrong-password vs
corrupt-blob; same length validation at each caller). Structural diagnostics are logged
once inside the helper with a `site` field.

## Consequence for Wave 16 CODE-087

**CODE-087 still applies, unchanged.** It targets the `(Vec<u8>, Vec<u8>, Vec<u8>)` triple
returned by `encrypt_message` (+ its `type_complexity` allows) in
`model/wallet/encryption.rs`. Route (a) keeps `encryption.rs` and `encrypt_message`
exactly as-is, so the named-struct cleanup CODE-087 asks for is neither resolved nor
mooted by this change — schedule it as originally planned.

## If route (b) is ever scheduled

It is a standalone migration task, not a dedup: extend the existing lazy re-wrap so the
`ClosedKeyItem` (deprecated `src/database`) path is also migrated, confirm all three
legacy formats have a re-wrap path, keep the readers until telemetry/versioning shows no
un-migrated secrets remain, then delete the readers and `encryption.rs` together. Route (a)
does not block route (b).
