//! The just-in-time secret chokepoint.
//!
//! [`SecretAccess`] is the single doorway through which every plaintext
//! secret is obtained. A consumer that needs a seed or imported key calls
//! [`SecretAccess::with_secret`] (one-shot) or
//! [`SecretAccess::with_secret_session`] (one prompt, many signs in one
//! operation), and receives the plaintext **by borrow inside a closure**.
//! The plaintext never crosses the closure boundary and zeroizes when the
//! closure returns.
//!
//! Resolution order for each call:
//!   1. session cache (only populated when the user opted in; TTL honored);
//!   2. else, an **unprotected** scope (a migrated raw secret, or a no-password
//!      HD wallet / no-passphrase imported key) resolves **without prompting** —
//!      the chokepoint reads it directly with no passphrase;
//!   3. else prompt via [`SecretPrompt`] for the passphrase, decrypt the
//!      stored secret just-in-time, optionally promote to the session cache,
//!      run the closure, then zeroize.
//!
//! Secret hygiene:
//! - **Closure form, no storable guard.** [`SecretPlaintext`] and
//!   [`SecretSession`] are bound to the closure's lifetime; they cannot be
//!   parked across awaits outside the chokepoint.
//! - **Borrow-only.** The closure borrows `&Zeroizing<…>`; neither helper
//!   type is `Clone`, and there is no `Deref` to the raw bytes — access is
//!   via explicit `expose_*` returning a borrow.
//! - **Boxed session secrets.** Cached plaintext lives behind `Box` so a
//!   `HashMap` rehash never leaves an un-wiped inline copy; eviction and
//!   `forget*` drop the `Box`, zeroizing it.
//!
//! M-DONT-LEAK-TYPES: this type and its borrowed handles stay inside the
//! `wallet_backend` seam. The UI sees only the
//! [`secret_prompt`](crate::wallet_backend::secret_prompt) contract.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use dash_sdk::dpp::dashcore::Network;
use platform_wallet_storage::secrets::{
    SecretStore, SecretStoreError, SecretString, WalletId as SecretWalletId,
};
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::model::single_key::ImportedKey;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::encryption::derive_password_key;
use crate::model::wallet::seed_envelope::StoredSeedEnvelope;
use crate::wallet_backend::secret_prompt::{
    RememberPolicy, SecretPrompt, SecretPromptRequest, SecretPromptRetry, SecretScope,
};
use crate::wallet_backend::secret_seam::{SecretScheme, SecretSeam};
use crate::wallet_backend::single_key::{label_for_address, single_key_namespace_id};
use crate::wallet_backend::single_key_entry::SingleKeyEntry;
use crate::wallet_backend::wallet_seed_store::WalletSeedView;

/// Length of an HD BIP-39 seed.
const HD_SEED_LEN: usize = 64;
/// Length of an imported single-key secret.
const SINGLE_KEY_LEN: usize = 32;
/// Vault label for a raw (migrated) HD seed, distinct from the legacy
/// `envelope.v1` so the loader can tell raw from legacy by label presence.
pub(crate) const SEED_RAW_LABEL: &str = "seed.raw.v1";

/// Borrowed, kind-tagged plaintext handed to a [`SecretAccess::with_secret`]
/// closure. Lives only for the closure call. No `Clone`, no `Deref` to raw
/// bytes — read via [`SecretPlaintext::expose_hd_seed`] /
/// [`SecretPlaintext::expose_single_key`], which return a borrow tied to
/// the closure's lifetime.
pub enum SecretPlaintext<'a> {
    /// A 64-byte HD wallet seed.
    HdSeed(&'a Zeroizing<[u8; HD_SEED_LEN]>),
    /// A 32-byte imported single-key secret.
    SingleKey(&'a Zeroizing<[u8; SINGLE_KEY_LEN]>),
    /// A 32-byte identity private key, read raw from the vault per-use.
    IdentityKey(&'a Zeroizing<[u8; SINGLE_KEY_LEN]>),
}

impl SecretPlaintext<'_> {
    /// Borrow the 64-byte HD seed, or `None` if this is a single-key
    /// plaintext.
    pub fn expose_hd_seed(&self) -> Option<&[u8; HD_SEED_LEN]> {
        match self {
            // Deref through `Zeroizing` explicitly: `[u8; N]` also
            // implements `AsRef<PushBytes>` (dashcore), which makes a bare
            // `.as_ref()` ambiguous.
            SecretPlaintext::HdSeed(s) => Some(&***s),
            _ => None,
        }
    }

    /// Borrow the 32-byte single-key secret, or `None` if this is an HD
    /// seed plaintext.
    pub fn expose_single_key(&self) -> Option<&[u8; SINGLE_KEY_LEN]> {
        match self {
            SecretPlaintext::SingleKey(k) => Some(&***k),
            _ => None,
        }
    }

    /// Borrow the 32-byte identity private key, or `None` for the other
    /// kinds. The plaintext is borrowed for the closure only and zeroizes
    /// on return — it is never resident.
    pub fn expose_identity_key(&self) -> Option<&[u8; SINGLE_KEY_LEN]> {
        match self {
            SecretPlaintext::IdentityKey(k) => Some(&***k),
            _ => None,
        }
    }
}

/// Within-operation secret handle for [`SecretAccess::with_secret_session`].
/// Holds one decrypted secret for the whole closure so a multi-sign
/// operation prompts at most once. Borrowed-only; dropped (zeroized) when
/// the closure returns.
pub struct SecretSession<'a> {
    plaintext: &'a Plaintext,
}

impl SecretSession<'_> {
    /// Borrow the held plaintext as a [`SecretPlaintext`] for a single
    /// derive/sign step. May be called many times within the operation
    /// without re-prompting.
    pub fn plaintext(&self) -> SecretPlaintext<'_> {
        self.plaintext.borrow()
    }
}

/// Owned decrypted plaintext, kept on the chokepoint's stack for the
/// duration of one operation (or boxed in the session cache). Zeroizes on
/// drop. Never leaves `wallet_backend`.
enum Plaintext {
    HdSeed(Zeroizing<[u8; HD_SEED_LEN]>),
    SingleKey(Zeroizing<[u8; SINGLE_KEY_LEN]>),
    IdentityKey(Zeroizing<[u8; SINGLE_KEY_LEN]>),
}

impl Plaintext {
    fn borrow(&self) -> SecretPlaintext<'_> {
        match self {
            Plaintext::HdSeed(s) => SecretPlaintext::HdSeed(s),
            Plaintext::SingleKey(k) => SecretPlaintext::SingleKey(k),
            Plaintext::IdentityKey(k) => SecretPlaintext::IdentityKey(k),
        }
    }

    /// An owned, op-scoped `Zeroizing` copy of this plaintext. Used only to
    /// lift a cached secret off the cache lock so the consuming closure can
    /// run without holding it. The copy zeroizes on drop.
    fn to_op_copy(&self) -> Plaintext {
        match self {
            Plaintext::HdSeed(s) => Plaintext::HdSeed(Zeroizing::new(**s)),
            Plaintext::SingleKey(k) => Plaintext::SingleKey(Zeroizing::new(**k)),
            Plaintext::IdentityKey(k) => Plaintext::IdentityKey(Zeroizing::new(**k)),
        }
    }
}

/// A session-cache entry: the boxed plaintext plus its expiry policy.
///
/// The plaintext is boxed so a `HashMap` rehash moves only the `Box` pointer,
/// never the secret bytes — no un-wiped inline copy is left behind.
/// `expires_at = None` means "until app close".
struct SessionEntry {
    plaintext: Box<Plaintext>,
    expires_at: Option<Instant>,
}

impl SessionEntry {
    fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|deadline| now >= deadline)
    }
}

/// O(1)-clone handle to the JIT secret chokepoint (M-SERVICES-CLONE).
#[derive(Clone)]
pub struct SecretAccess {
    inner: Arc<SecretAccessInner>,
}

impl std::fmt::Debug for SecretAccess {
    /// Redacts every field — the vault, prompt, caches, and meta could all
    /// surface secret material. Prints only the network so embedding types
    /// (e.g. `QualifiedIdentity`) can derive `Debug` without leaking
    /// (M-PUBLIC-DEBUG, M-DONT-LEAK-TYPES).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretAccess")
            .field("network", &self.inner.network)
            .finish_non_exhaustive()
    }
}

struct SecretAccessInner {
    /// The encrypted vault — decrypt-on-demand source of truth.
    secret_store: Arc<SecretStore>,
    /// HD wallet meta (seed hash → password hint / alias) for prompt copy.
    wallet_meta: RwLock<BTreeMap<WalletSeedHash, WalletPromptMeta>>,
    /// Single-key index (address → alias / hint / has_passphrase) for
    /// prompt copy and the unprotected fast-path check.
    single_key_index: RwLock<BTreeMap<String, ImportedKey>>,
    /// The UI seam. `dyn` so the host is chosen at construction.
    prompt: Arc<dyn SecretPrompt>,
    /// Opt-in session cache. Empty by default; a scope lands here only on
    /// a non-`None` [`RememberPolicy`]. Values boxed + zeroizing; cleared
    /// on app close, network switch, and manual lock.
    session: RwLock<HashMap<SecretScope, SessionEntry>>,
    /// Network used for BIP-32/derivation by consumers (carried for
    /// signer construction; not used by decryption itself).
    network: Network,
}

/// Minimal prompt-copy metadata for an HD wallet, mirrored from the
/// wallet-meta sidecar so the chokepoint can build an informative
/// [`SecretPromptRequest`] without reaching back into the wallet backend.
#[derive(Clone, Debug, Default)]
pub struct WalletPromptMeta {
    /// User-visible wallet name, if any.
    pub alias: Option<String>,
    /// User-set password hint, if any.
    pub password_hint: Option<String>,
}

impl SecretAccess {
    /// Build a chokepoint over `secret_store`, prompting through `prompt`.
    ///
    /// Prompt-copy metadata is seeded via [`SecretAccess::set_wallet_meta`]
    /// / [`SecretAccess::set_single_key_index`]; absent metadata degrades
    /// to a generic label, never an error.
    pub fn new(
        secret_store: Arc<SecretStore>,
        prompt: Arc<dyn SecretPrompt>,
        network: Network,
    ) -> Self {
        Self {
            inner: Arc::new(SecretAccessInner {
                secret_store,
                wallet_meta: RwLock::new(BTreeMap::new()),
                single_key_index: RwLock::new(BTreeMap::new()),
                prompt,
                session: RwLock::new(HashMap::new()),
                network,
            }),
        }
    }

    /// The network this chokepoint derives for.
    pub fn network(&self) -> Network {
        self.inner.network
    }

    /// Replace the HD prompt-copy metadata map. Used at hydration time so
    /// prompts can show the wallet name and password hint.
    pub fn set_wallet_meta(&self, meta: BTreeMap<WalletSeedHash, WalletPromptMeta>) {
        if let Ok(mut guard) = self.inner.wallet_meta.write() {
            *guard = meta;
        }
    }

    /// Replace the single-key prompt-copy index. Used at hydration time and
    /// after an import so prompts can show the key nickname and hint, and
    /// so the unprotected fast-path can skip the prompt.
    pub fn set_single_key_index(&self, index: BTreeMap<String, ImportedKey>) {
        if let Ok(mut guard) = self.inner.single_key_index.write() {
            *guard = index;
        }
    }

    /// Run `f` with the plaintext secret for `scope`, obtaining it
    /// just-in-time.
    ///
    /// Resolution: session cache (honoring TTL) → unprotected fast-path →
    /// prompt + decrypt. On a wrong passphrase the chokepoint re-asks with
    /// [`SecretPromptRetry::WrongPassphrase`] without closing the modal; a
    /// dismissed prompt resolves [`TaskError::SecretPromptCancelled`].
    ///
    /// `f` receives the plaintext by borrow and MUST NOT copy it out — the
    /// type system enforces this (no `Clone`, no `Deref`). The plaintext
    /// zeroizes when this call returns.
    pub async fn with_secret<R>(
        &self,
        scope: &SecretScope,
        f: impl FnOnce(SecretPlaintext<'_>) -> Result<R, TaskError>,
    ) -> Result<R, TaskError> {
        self.with_secret_session(scope, async |session| f(session.plaintext()))
            .await
    }

    /// Run `f` with one decrypted secret held for the whole closure, so a
    /// multi-step operation (sign N inputs, derive then sign) prompts at
    /// most once. The held secret zeroizes when the closure returns.
    ///
    /// Semantics otherwise match [`SecretAccess::with_secret`].
    pub async fn with_secret_session<R>(
        &self,
        scope: &SecretScope,
        f: impl AsyncFnOnce(&SecretSession<'_>) -> Result<R, TaskError>,
    ) -> Result<R, TaskError> {
        // 1. Session-cache hit (opt-in, TTL-honored). Copy the entry into an
        //    op-scoped `Zeroizing` buffer and release the lock BEFORE the
        //    closure runs: it may `.await` and re-enter the cache for another
        //    scope, so holding the lock across it could deadlock.
        {
            let now = Instant::now();
            let mut needs_evict = false;
            let held = {
                let guard = self
                    .inner
                    .session
                    .read()
                    .map_err(|_| TaskError::SecretDecryptFailed)?;
                match guard.get(scope) {
                    Some(entry) if entry.is_expired(now) => {
                        needs_evict = true;
                        None
                    }
                    Some(entry) => Some(entry.plaintext.as_ref().to_op_copy()),
                    None => None,
                }
            };
            if let Some(plaintext) = held {
                let session = SecretSession {
                    plaintext: &plaintext,
                };
                return f(&session).await;
            }
            if needs_evict && let Ok(mut guard) = self.inner.session.write() {
                // Re-check expiry under the write lock to avoid racing a
                // concurrent refresh, then drop (zeroize) the entry.
                if guard.get(scope).is_some_and(|e| e.is_expired(now)) {
                    guard.remove(scope);
                }
            }
        }

        // 2. Unprotected fast-path: decrypt with no passphrase, no prompt.
        //    Nothing to remember — there is no toggle on a no-prompt path,
        //    and a re-resolve is a cheap vault read.
        if !self.scope_has_passphrase(scope)? {
            let plaintext = self.decrypt_jit(scope, None)?;
            let session = SecretSession {
                plaintext: &plaintext,
            };
            return f(&session).await;
        }

        // 3. Prompt → decrypt → run. Re-ask on a wrong passphrase.
        let mut retry: Option<SecretPromptRetry> = None;
        loop {
            let request = self.build_request(scope, retry);
            let reply = self
                .inner
                .prompt
                .request(request)
                .await
                .map_err(|_cancelled| self.cancel_error())?;

            match self.decrypt_jit(scope, Some(&reply.passphrase)) {
                Ok(plaintext) => {
                    self.maybe_remember(scope, &plaintext, reply.remember);
                    let session = SecretSession {
                        plaintext: &plaintext,
                    };
                    return f(&session).await;
                }
                Err(e) if is_wrong_passphrase(&e) => {
                    retry = Some(SecretPromptRetry::WrongPassphrase);
                    continue;
                }
                Err(other) => return Err(other),
            }
        }
    }

    /// Promote a decrypted secret to the session cache without running an
    /// operation. Used by the explicit unlock gesture when the user opts
    /// in to "keep unlocked". `RememberPolicy::None` is a no-op.
    pub fn remember_session(
        &self,
        scope: &SecretScope,
        plaintext: SecretPlaintext<'_>,
        policy: RememberPolicy,
    ) {
        let owned = match plaintext {
            SecretPlaintext::HdSeed(s) => Plaintext::HdSeed(Zeroizing::new(**s)),
            SecretPlaintext::SingleKey(k) => Plaintext::SingleKey(Zeroizing::new(**k)),
            SecretPlaintext::IdentityKey(k) => Plaintext::IdentityKey(Zeroizing::new(**k)),
        };
        self.maybe_remember(scope, &owned, policy);
    }

    /// Decrypt an HD-seed envelope with an explicitly-supplied passphrase and
    /// promote the result into the session cache — **without prompting**.
    ///
    /// This is the unlock-gesture bridge: the UI has just verified the
    /// passphrase (via [`WalletSeed::open`](crate::model::wallet::WalletSeed::open)),
    /// so the seed is re-decrypted here through the same chokepoint decrypt
    /// path every signing op uses, then cached so the rest of the session does
    /// not re-prompt. `passphrase` is `None` for unprotected wallets (the
    /// envelope decrypts verbatim). The plaintext is borrowed only to seed the
    /// cache and zeroizes on return.
    ///
    /// The lazy legacy→steady-state re-wrap happens inside [`Self::decrypt_jit`]:
    /// a protected seed re-wraps to **Tier-2 under the same password** (protection
    /// KEPT, never downgraded to a raw secret), an unprotected one to the raw
    /// label. So there is nothing for the unlock callsite to "finalize" — the
    /// wallet's `uses_password` stays accurate (`true` for a protected wallet).
    pub fn promote_hd_seed_with_passphrase(
        &self,
        seed_hash: &WalletSeedHash,
        passphrase: Option<&SecretString>,
        policy: RememberPolicy,
    ) -> Result<(), TaskError> {
        let scope = SecretScope::HdSeed {
            seed_hash: *seed_hash,
        };
        let plaintext = self.decrypt_jit(&scope, passphrase)?;
        self.maybe_remember(&scope, &plaintext, policy);
        Ok(())
    }

    /// Forget the session-cached secret for `scope`, zeroizing it.
    /// Idempotent. Poison-safe: a poisoned lock is recovered so a panicked
    /// reader can never strand a plaintext in the cache.
    pub fn forget(&self, scope: &SecretScope) {
        let mut guard = self
            .inner
            .session
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        guard.remove(scope);
    }

    /// Forget every session-cached secret, zeroizing all of them. Called on
    /// network switch and teardown. Poison-safe.
    pub fn forget_all(&self) {
        let mut guard = self
            .inner
            .session
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        guard.clear();
    }

    /// `true` when `scope` is currently held in the session cache and not
    /// expired. Test/diagnostic helper — does not extend the TTL.
    #[cfg(test)]
    pub(crate) fn is_session_cached(&self, scope: &SecretScope) -> bool {
        self.session_cache_hit(scope)
    }

    /// `true` when [`Self::with_secret`] could resolve `scope` without ever
    /// prompting — either the plaintext is already in the session cache or the
    /// secret is unprotected (decrypts with no passphrase).
    ///
    /// Lets a non-interactive caller (the background identity sweep) decide up
    /// front whether to attempt a derivation or skip the wallet, so it never
    /// triggers a passphrase modal. A `false` here is conservative: the resolve
    /// would prompt, so the caller should skip.
    pub fn can_resolve_without_prompt(&self, scope: &SecretScope) -> bool {
        self.session_cache_hit(scope) || !self.scope_has_passphrase(scope).unwrap_or(true)
    }

    /// Whether `scope`'s plaintext is in the session cache and not expired.
    fn session_cache_hit(&self, scope: &SecretScope) -> bool {
        let now = Instant::now();
        self.inner
            .session
            .read()
            .map(|g| g.get(scope).is_some_and(|e| !e.is_expired(now)))
            .unwrap_or(false)
    }

    /// Insert into the session cache iff `policy` requests it. Boxed value;
    /// expiry stamped for `For(duration)`.
    fn maybe_remember(&self, scope: &SecretScope, plaintext: &Plaintext, policy: RememberPolicy) {
        let now = Instant::now();
        let expires_at = match policy {
            RememberPolicy::None => return,
            RememberPolicy::UntilAppClose => None,
            // On the (unreachable today) overflow of `now + duration`, expire
            // immediately rather than risk over-retaining the secret — `None`
            // here would mean "never expires".
            RememberPolicy::For(duration) => Some(now.checked_add(duration).unwrap_or(now)),
        };
        let boxed = match plaintext {
            Plaintext::HdSeed(s) => Box::new(Plaintext::HdSeed(Zeroizing::new(**s))),
            Plaintext::SingleKey(k) => Box::new(Plaintext::SingleKey(Zeroizing::new(**k))),
            Plaintext::IdentityKey(k) => Box::new(Plaintext::IdentityKey(Zeroizing::new(**k))),
        };
        if let Ok(mut guard) = self.inner.session.write() {
            guard.insert(
                scope.clone(),
                SessionEntry {
                    plaintext: boxed,
                    expires_at,
                },
            );
        }
    }

    /// The typed error for a dismissed/absent prompt. A genuine user cancel
    /// on the interactive host is [`TaskError::SecretPromptCancelled`]; a
    /// cancel from a non-interactive host
    /// ([`NullSecretPrompt`](crate::wallet_backend::secret_prompt::NullSecretPrompt))
    /// means there was no window to ask in, surfaced as
    /// [`TaskError::SecretPromptUnavailable`].
    fn cancel_error(&self) -> TaskError {
        if self.inner.prompt.is_interactive() {
            TaskError::SecretPromptCancelled
        } else {
            TaskError::SecretPromptUnavailable
        }
    }

    /// Whether `scope`'s stored secret is passphrase-protected. Drives the
    /// unprotected fast-path.
    ///
    /// Seam-first: a secret already migrated to its raw label has no
    /// passphrase (the user password no longer gates it). Only a not-yet-
    /// migrated legacy entry can still be protected. Identity keys are always
    /// unprotected (prompt-free → headless/MCP signing works).
    fn scope_has_passphrase(&self, scope: &SecretScope) -> Result<bool, TaskError> {
        match scope {
            SecretScope::HdSeed { seed_hash } => {
                let view = WalletSeedView::new(&self.inner.secret_store);
                match view.scheme(seed_hash)? {
                    // Tier-2: the seed is sealed under its own object password.
                    SecretScheme::Protected => Ok(true),
                    // Tier-1 raw: unprotected — no passphrase.
                    SecretScheme::Unprotected => Ok(false),
                    // Nothing at the raw label yet ⇒ the legacy envelope's
                    // `uses_password` is the source of truth until first unlock
                    // migrates it to the raw label.
                    SecretScheme::Absent => {
                        let envelope = view.get(seed_hash)?.ok_or(TaskError::SecretSeamMissing)?;
                        Ok(envelope.uses_password)
                    }
                }
            }
            SecretScope::SingleKey { address } => {
                // Raw 32-byte key present ⇒ migrated ⇒ no passphrase.
                if self.single_key_raw(address)?.is_some() {
                    return Ok(false);
                }
                if let Ok(index) = self.inner.single_key_index.read()
                    && let Some(meta) = index.get(address)
                {
                    return Ok(meta.has_passphrase);
                }
                let entry = self.load_single_key_entry(address)?;
                Ok(entry.has_passphrase)
            }
            // Identity keys are stored raw, unprotected — always prompt-free.
            SecretScope::IdentityKey { .. } => Ok(false),
        }
    }

    /// Decrypt the stored secret for `scope` with `passphrase`
    /// (`None` for unprotected scopes). The only place the vault is read
    /// for plaintext. Returns the kind-tagged owned plaintext.
    ///
    /// Seam-first for all three classes: the raw label wins; the retained
    /// legacy reader is the migration fallback for HD seeds and single keys.
    fn decrypt_jit(
        &self,
        scope: &SecretScope,
        passphrase: Option<&SecretString>,
    ) -> Result<Plaintext, TaskError> {
        match scope {
            SecretScope::HdSeed { seed_hash } => {
                let view = WalletSeedView::new(&self.inner.secret_store);
                match view.scheme(seed_hash)? {
                    // Tier-1 raw — unprotected, no password.
                    SecretScheme::Unprotected => {
                        let seed = view
                            .get_raw(seed_hash)?
                            .ok_or(TaskError::SecretSeamMissing)?;
                        Ok(Plaintext::HdSeed(seed))
                    }
                    // Tier-2 — unseal with this seed's own object password.
                    SecretScheme::Protected => {
                        let pw = passphrase.ok_or(TaskError::HdPassphraseIncorrect)?;
                        let seed = view
                            .get_protected(seed_hash, pw)?
                            .ok_or(TaskError::SecretSeamMissing)?;
                        Ok(Plaintext::HdSeed(seed))
                    }
                    // Legacy AES-GCM envelope: decode-only reader, then LAZY
                    // re-wrap to the steady-state form and drop the legacy
                    // envelope. A protected seed re-wraps to Tier-2 under the
                    // SAME user password (protection KEPT, not downgraded to
                    // raw); an unprotected one goes to the raw label. An absent
                    // envelope ⇒ the secret is gone (loud, never a silent miss).
                    // Crash-safe: the re-store (upsert) precedes the delete, and
                    // the scheme probe prefers the new label, so a crash between
                    // leaves both forms and the next read takes the new one.
                    SecretScheme::Absent => {
                        let envelope = view.get(seed_hash)?.ok_or(TaskError::SecretSeamMissing)?;
                        let seed = decrypt_hd_seed(&envelope, passphrase)?;
                        if envelope.uses_password {
                            let pw = passphrase.ok_or(TaskError::HdPassphraseIncorrect)?;
                            view.set_protected(seed_hash, &seed, pw)?;
                        } else {
                            view.set_raw(seed_hash, &seed)?;
                        }
                        view.delete(seed_hash)?;
                        Ok(Plaintext::HdSeed(seed))
                    }
                }
            }
            SecretScope::SingleKey { address } => {
                if let Some(raw) = self.single_key_raw(address)? {
                    return Ok(Plaintext::SingleKey(raw));
                }
                // Legacy fallback (migration reader). A protected entry was just
                // decrypted with the user's passphrase — LAZY-migrate it to raw
                // here (the upsert under the SAME label replaces the AES-GCM
                // framing with the raw 32 bytes, so no separate delete is
                // needed) and flip the in-memory index so the next resolve takes
                // the prompt-free fast-path. Idempotent.
                let entry = self.load_single_key_entry(address)?;
                let raw = entry.decrypt(passphrase.map(|p| p.expose_secret()))?;
                if entry.has_passphrase {
                    self.migrate_single_key_to_raw(address, &raw);
                }
                Ok(Plaintext::SingleKey(raw))
            }
            SecretScope::IdentityKey {
                identity_id,
                target,
                key_id,
            } => {
                let label = SecretScope::identity_key_label(target, *key_id);
                let raw = self
                    .seam()
                    .get_secret(&SecretWalletId::from(*identity_id), &label)?
                    .ok_or(TaskError::IdentityKeyMissing)?;
                let key: [u8; SINGLE_KEY_LEN] = raw.expose_secret().try_into().map_err(|_| {
                    tracing::warn!(
                        target = "wallet_backend::secret_access",
                        blob_len = raw.expose_secret().len(),
                        "Raw identity key has wrong length",
                    );
                    TaskError::SecretDecryptFailed
                })?;
                Ok(Plaintext::IdentityKey(Zeroizing::new(key)))
            }
        }
    }

    /// Borrow the secret store as a [`SecretSeam`].
    fn seam(&self) -> SecretSeam<'_> {
        SecretSeam::new(&self.inner.secret_store)
    }

    /// LAZY-migrate a just-decrypted protected single key to raw bytes under
    /// the same label (the upsert replaces the AES-GCM framing) and flip the
    /// in-memory index so the next resolve takes the prompt-free fast-path.
    /// Best-effort: a vault-write failure is logged and the key keeps working
    /// via the legacy reader. The persistent `ImportedKey.has_passphrase` flip
    /// + the user notice are driven by the screen that owns the app k/v.
    fn migrate_single_key_to_raw(&self, address: &str, raw: &[u8; SINGLE_KEY_LEN]) {
        let label = label_for_address(address);
        if let Err(e) = self.seam().put_secret(
            &single_key_namespace_id(),
            &label,
            &platform_wallet_storage::secrets::SecretBytes::from_slice(raw),
        ) {
            tracing::warn!(
                target = "wallet_backend::secret_access",
                error = ?e,
                "Single-key lazy raw migration deferred (vault write failed)",
            );
            return;
        }
        if let Ok(mut index) = self.inner.single_key_index.write()
            && let Some(meta) = index.get_mut(address)
        {
            meta.has_passphrase = false;
        }
    }

    /// Read the raw 32-byte single-key secret for `address` if the entry has
    /// already been migrated to its raw label, else `None`. A legacy
    /// `SingleKeyEntry`-framed value (length != 32) is left for the legacy
    /// reader and reported as `None` here.
    fn single_key_raw(
        &self,
        address: &str,
    ) -> Result<Option<Zeroizing<[u8; SINGLE_KEY_LEN]>>, TaskError> {
        let label = label_for_address(address);
        let Some(payload) = self.seam().get_secret(&single_key_namespace_id(), &label)? else {
            return Ok(None);
        };
        match <[u8; SINGLE_KEY_LEN]>::try_from(payload.expose_secret()) {
            Ok(raw) => Ok(Some(Zeroizing::new(raw))),
            // Not 32 bytes ⇒ a legacy framed entry, not yet migrated.
            Err(_) => Ok(None),
        }
    }

    /// Load and decode the stored single-key entry for `address`.
    fn load_single_key_entry(&self, address: &str) -> Result<SingleKeyEntry, TaskError> {
        let label = label_for_address(address);
        let payload = self
            .inner
            .secret_store
            .get(&single_key_namespace_id(), &label)
            .map_err(|source| TaskError::SecretStore {
                source: Box::new(source),
            })?
            .ok_or(TaskError::ImportedKeyNotFound)?;
        SingleKeyEntry::decode(payload.expose_secret())
    }

    /// Build a prompt request for `scope`, filling display copy from the
    /// in-memory metadata where available.
    fn build_request(
        &self,
        scope: &SecretScope,
        retry: Option<SecretPromptRetry>,
    ) -> SecretPromptRequest {
        let (label, hint) = match scope {
            SecretScope::HdSeed { seed_hash } => {
                let meta = self
                    .inner
                    .wallet_meta
                    .read()
                    .ok()
                    .and_then(|g| g.get(seed_hash).cloned())
                    .unwrap_or_default();
                let label = meta.alias.unwrap_or_else(|| "your wallet".to_string());
                (label, meta.password_hint)
            }
            SecretScope::SingleKey { address } => {
                let meta = self
                    .inner
                    .single_key_index
                    .read()
                    .ok()
                    .and_then(|g| g.get(address).cloned());
                let label = meta
                    .as_ref()
                    .and_then(|m| m.alias.clone())
                    .unwrap_or_else(|| address.clone());
                let hint = meta.and_then(|m| m.passphrase_hint);
                (label, hint)
            }
            // Identity keys are prompt-free (unprotected fast-path), so this
            // request is never built for them — a generic label keeps the
            // match exhaustive without inventing copy that cannot surface.
            SecretScope::IdentityKey { .. } => ("this identity key".to_string(), None),
        };
        let mut request = SecretPromptRequest::new(scope.clone(), label).with_hint(hint);
        if let Some(reason) = retry {
            request = request.retrying(reason);
        }
        request
    }
}

/// Decrypt the HD seed envelope. `uses_password = false` means
/// `encrypted_seed` holds the raw 64 bytes verbatim (no passphrase needed);
/// otherwise Argon2id-derive the AES-GCM key from `passphrase` and decrypt.
fn decrypt_hd_seed(
    envelope: &StoredSeedEnvelope,
    passphrase: Option<&SecretString>,
) -> Result<Zeroizing<[u8; HD_SEED_LEN]>, TaskError> {
    if !envelope.uses_password {
        let raw: [u8; HD_SEED_LEN] =
            envelope.encrypted_seed.as_slice().try_into().map_err(|_| {
                tracing::warn!(
                    target = "wallet_backend::secret_access",
                    blob_len = envelope.encrypted_seed.len(),
                    "Unprotected HD seed envelope has wrong length",
                );
                TaskError::SecretDecryptFailed
            })?;
        return Ok(Zeroizing::new(raw));
    }

    let passphrase = passphrase.ok_or(TaskError::HdPassphraseIncorrect)?;
    let key = Zeroizing::new(
        derive_password_key(passphrase.expose_secret(), &envelope.salt).map_err(|detail| {
            tracing::warn!(
                target = "wallet_backend::secret_access",
                %detail,
                "Argon2 key derivation failed during HD seed decrypt",
            );
            TaskError::SecretDecryptFailed
        })?,
    );
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|detail| {
        tracing::warn!(
            target = "wallet_backend::secret_access",
            error = %detail,
            "AES-GCM init failed during HD seed decrypt",
        );
        TaskError::SecretDecryptFailed
    })?;
    // Checked nonce conversion: a stored envelope with the wrong nonce length
    // is a corrupt at-rest blob, not a panic. `Nonce::from_slice` would panic
    // on a length mismatch and poison the long-lived secret-store mutex.
    let nonce_bytes: &[u8; 12] = envelope.nonce.as_slice().try_into().map_err(|_| {
        tracing::warn!(
            target = "wallet_backend::secret_access",
            nonce_len = envelope.nonce.len(),
            "HD seed envelope nonce is not 12 bytes",
        );
        TaskError::SecretDecryptFailed
    })?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(nonce_bytes),
                envelope.encrypted_seed.as_slice(),
            )
            .map_err(|_| TaskError::HdPassphraseIncorrect)?,
    );
    let seed: [u8; HD_SEED_LEN] = plaintext.as_slice().try_into().map_err(|_| {
        tracing::warn!(
            target = "wallet_backend::secret_access",
            blob_len = plaintext.len(),
            "Decrypted HD seed is not 64 bytes",
        );
        TaskError::SecretDecryptFailed
    })?;
    Ok(Zeroizing::new(seed))
}

/// Whether `e` is the "wrong passphrase" condition that the re-ask loop
/// catches and re-prompts on (rather than aborting).
fn is_wrong_passphrase(e: &TaskError) -> bool {
    match e {
        TaskError::SingleKeyPassphraseIncorrect | TaskError::HdPassphraseIncorrect => true,
        // A Tier-2 unseal that rejected the object password surfaces through the
        // seam as `WrongPassword`; the re-ask loop catches it and re-prompts
        // rather than aborting (same UX as the legacy AES-GCM wrong-pass path).
        TaskError::SecretSeam { source } => matches!(**source, SecretStoreError::WrongPassword),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::model::wallet::encryption::encrypt_message;
    use crate::wallet_backend::secret_prompt::NullSecretPrompt;
    use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
    use crate::wallet_backend::single_key::{SingleKeyView, open_secret_store};

    /// A sentinel passphrase + seed/key used by confinement tests. If any
    /// of these byte sequences appears in an error, log, or Debug output,
    /// the chokepoint leaked a secret.
    const SENTINEL_PASSPHRASE: &str = "correct-horse-battery-staple-SENTINEL";
    const SENTINEL_SEED: [u8; HD_SEED_LEN] = [0x5A; HD_SEED_LEN];

    fn fresh_store(dir: &std::path::Path) -> Arc<SecretStore> {
        let path = dir.join("secrets.pwsvault");
        Arc::new(open_secret_store(&path).expect("open vault"))
    }

    /// Write a protected HD seed envelope under `seed_hash`, encrypting
    /// `seed` with `passphrase`.
    fn store_protected_hd(
        store: &Arc<SecretStore>,
        seed_hash: &WalletSeedHash,
        seed: &[u8; 64],
        passphrase: &str,
    ) {
        let (encrypted_seed, salt, nonce) =
            encrypt_message(seed, passphrase).expect("encrypt seed");
        let envelope = StoredSeedEnvelope {
            encrypted_seed,
            salt,
            nonce,
            password_hint: Some("granny's birthday".into()),
            uses_password: true,
            xpub_encoded: vec![0xCD; 78],
        };
        WalletSeedView::new(store)
            .set(seed_hash, &envelope)
            .expect("store envelope");
    }

    /// Write an unprotected HD seed envelope (raw 64 bytes, no password).
    fn store_unprotected_hd(store: &Arc<SecretStore>, seed_hash: &WalletSeedHash, seed: &[u8; 64]) {
        let envelope = StoredSeedEnvelope {
            encrypted_seed: seed.to_vec(),
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: vec![0x22; 78],
        };
        WalletSeedView::new(store)
            .set(seed_hash, &envelope)
            .expect("store envelope");
    }

    fn access(store: Arc<SecretStore>, prompt: Arc<dyn SecretPrompt>) -> SecretAccess {
        SecretAccess::new(store, prompt, Network::Testnet)
    }

    #[test]
    fn hd_seed_wrong_length_nonce_returns_typed_error_not_panic() {
        // A protected envelope whose nonce is not 12 bytes is a corrupt
        // at-rest blob: `decrypt_hd_seed` must surface a typed error rather
        // than panic inside `Nonce::from_slice` (which would poison the
        // long-lived secret-store mutex).
        let envelope = StoredSeedEnvelope {
            encrypted_seed: vec![0u8; 80],
            salt: vec![0u8; 16],
            nonce: vec![0u8; 5], // wrong length on purpose
            password_hint: None,
            uses_password: true,
            xpub_encoded: vec![0u8; 78],
        };
        let passphrase = SecretString::new(SENTINEL_PASSPHRASE);
        match decrypt_hd_seed(&envelope, Some(&passphrase)) {
            Err(TaskError::SecretDecryptFailed) => {}
            other => panic!("expected SecretDecryptFailed, got {other:?}"),
        }
    }

    // --- HD seed scope ----------------------------------------------------

    #[tokio::test]
    async fn cache_miss_prompts_decrypts_and_borrows_seed() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x01; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = access(store, prompt.clone());

        let scope = SecretScope::HdSeed { seed_hash };
        let matched = sa
            .with_secret(&scope, |pt| {
                Ok(pt.expose_hd_seed().copied() == Some(SENTINEL_SEED))
            })
            .await
            .expect("with_secret");
        assert!(matched, "closure saw the decrypted seed");
        assert_eq!(prompt.ask_count(), 1, "exactly one prompt on cache miss");
        // None policy ⇒ nothing cached.
        assert!(!sa.is_session_cached(&scope));
    }

    #[tokio::test]
    async fn session_hit_does_not_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x02; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        // The prompt is scripted with exactly ONE answer. The first op
        // remembers for the session; the second op must hit the cache —
        // if it prompted, `TestPrompt` would panic on the empty script,
        // failing the test for the right reason.
        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::remember(
            SENTINEL_PASSPHRASE,
            RememberPolicy::UntilAppClose,
        )]));
        let sa = access(store, prompt.clone());
        let scope = SecretScope::HdSeed { seed_hash };

        sa.with_secret(&scope, |_pt| Ok(())).await.unwrap();
        assert!(sa.is_session_cached(&scope), "promoted to session cache");

        sa.with_secret(&scope, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(SENTINEL_SEED));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            prompt.ask_count(),
            1,
            "session hit reused the cache, no re-prompt"
        );
    }

    #[tokio::test]
    async fn none_policy_does_not_cache() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x03; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = access(store, prompt);
        let scope = SecretScope::HdSeed { seed_hash };
        sa.with_secret(&scope, |_pt| Ok(())).await.unwrap();
        assert!(!sa.is_session_cached(&scope), "None ⇒ no caching");
    }

    #[tokio::test]
    async fn cancel_aborts_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x04; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::Cancel]));
        let sa = access(store, prompt);
        let scope = SecretScope::HdSeed { seed_hash };

        let mut ran = false;
        let err = sa
            .with_secret(&scope, |_pt| {
                ran = true;
                Ok(())
            })
            .await
            .expect_err("cancel aborts");
        assert!(matches!(err, TaskError::SecretPromptCancelled));
        assert!(!ran, "closure never ran on cancel");
        assert!(!sa.is_session_cached(&scope), "nothing cached on cancel");
    }

    #[tokio::test]
    async fn null_prompt_on_protected_scope_yields_unavailable() {
        // Headless host: a passphrase-protected scope has no window to ask
        // in, so the chokepoint surfaces the typed "unavailable" error
        // rather than a misleading "you cancelled".
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x0C; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        let sa = access(store, Arc::new(NullSecretPrompt));
        let scope = SecretScope::HdSeed { seed_hash };
        let err = sa
            .with_secret(&scope, |_pt| Ok(()))
            .await
            .expect_err("no interactive prompt");
        assert!(matches!(err, TaskError::SecretPromptUnavailable));
    }

    #[tokio::test]
    async fn null_prompt_unprotected_scope_still_resolves() {
        // The headless host must not block no-password wallets: unprotected
        // scopes decrypt with no passphrase and never reach the prompt.
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x0D; 32];
        store_unprotected_hd(&store, &seed_hash, &SENTINEL_SEED);

        let sa = access(store, Arc::new(NullSecretPrompt));
        let scope = SecretScope::HdSeed { seed_hash };
        sa.with_secret(&scope, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(SENTINEL_SEED));
            Ok(())
        })
        .await
        .expect("unprotected resolves headless");
    }

    #[tokio::test]
    async fn unprotected_scope_does_not_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x05; 32];
        store_unprotected_hd(&store, &seed_hash, &SENTINEL_SEED);

        // never() would panic if asked — proves no prompt fired.
        let prompt = Arc::new(TestPrompt::never());
        let sa = access(store, prompt.clone());
        let scope = SecretScope::HdSeed { seed_hash };

        sa.with_secret(&scope, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(SENTINEL_SEED));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(prompt.ask_count(), 0, "unprotected ⇒ no prompt");
    }

    #[tokio::test]
    async fn can_resolve_without_prompt_tracks_protection_and_cache() {
        // The background identity sweep keys off this: an unprotected wallet or
        // a session-unlocked protected wallet resolves without a prompt; a
        // locked protected wallet does not, so the sweep skips it.
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());

        let unprotected: WalletSeedHash = [0x10; 32];
        store_unprotected_hd(&store, &unprotected, &SENTINEL_SEED);
        let protected: WalletSeedHash = [0x11; 32];
        store_protected_hd(&store, &protected, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        // The prompt is never consulted — `can_resolve_without_prompt` must
        // decide purely from at-rest protection and the session cache.
        let prompt = Arc::new(TestPrompt::never());
        let sa = access(store, prompt.clone());

        assert!(
            sa.can_resolve_without_prompt(&SecretScope::HdSeed {
                seed_hash: unprotected
            }),
            "unprotected scope resolves with no prompt"
        );
        let protected_scope = SecretScope::HdSeed {
            seed_hash: protected,
        };
        assert!(
            !sa.can_resolve_without_prompt(&protected_scope),
            "locked protected scope would prompt"
        );

        // Once the seed is session-cached (the user unlocked it), it resolves
        // without a prompt.
        sa.remember_session(
            &protected_scope,
            SecretPlaintext::HdSeed(&Zeroizing::new(SENTINEL_SEED)),
            RememberPolicy::UntilAppClose,
        );
        assert!(
            sa.can_resolve_without_prompt(&protected_scope),
            "session-unlocked protected scope resolves with no prompt"
        );
        assert_eq!(prompt.ask_count(), 0, "decision never prompts");
    }

    #[tokio::test]
    async fn wrong_passphrase_reasks_then_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x06; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::once("wrong-pass"),
            ScriptedAnswer::once(SENTINEL_PASSPHRASE),
        ]));
        let sa = access(store, prompt.clone());
        let scope = SecretScope::HdSeed { seed_hash };

        sa.with_secret(&scope, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(SENTINEL_SEED));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(prompt.ask_count(), 2, "one wrong + one right");
        let second = &prompt.requests()[1];
        assert_eq!(
            second.retry_reason,
            Some(SecretPromptRetry::WrongPassphrase),
            "re-ask carries the wrong-passphrase reason",
        );
    }

    #[tokio::test]
    async fn ttl_expiry_reprompts() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x07; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::remember(SENTINEL_PASSPHRASE, RememberPolicy::For(Duration::ZERO)),
            ScriptedAnswer::once(SENTINEL_PASSPHRASE),
        ]));
        let sa = access(store, prompt.clone());
        let scope = SecretScope::HdSeed { seed_hash };

        sa.with_secret(&scope, |_pt| Ok(())).await.unwrap();
        // For(ZERO) is already expired: not cached, and the next call
        // re-prompts rather than hitting the cache.
        assert!(
            !sa.is_session_cached(&scope),
            "zero TTL is immediately expired"
        );
        sa.with_secret(&scope, |_pt| Ok(())).await.unwrap();
        assert_eq!(prompt.ask_count(), 2, "expired entry forces a re-prompt");
    }

    #[tokio::test]
    async fn forget_clears_session_cache() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x08; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);
        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::remember(
            SENTINEL_PASSPHRASE,
            RememberPolicy::UntilAppClose,
        )]));
        let sa = access(store, prompt);
        let scope = SecretScope::HdSeed { seed_hash };
        sa.with_secret(&scope, |_pt| Ok(())).await.unwrap();
        assert!(sa.is_session_cached(&scope));
        sa.forget(&scope);
        assert!(!sa.is_session_cached(&scope));
    }

    #[tokio::test]
    async fn remember_session_promotes_and_forget_all_clears() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        // The unlock gesture promotes a verified seed without running an
        // operation. A never-prompt double proves no prompt is involved.
        let sa = access(store, Arc::new(TestPrompt::never()));
        let scope = SecretScope::HdSeed {
            seed_hash: [0x09; 32],
        };

        // None is a no-op.
        sa.remember_session(
            &scope,
            SecretPlaintext::HdSeed(&Zeroizing::new(SENTINEL_SEED)),
            RememberPolicy::None,
        );
        assert!(!sa.is_session_cached(&scope), "None ⇒ not cached");

        sa.remember_session(
            &scope,
            SecretPlaintext::HdSeed(&Zeroizing::new(SENTINEL_SEED)),
            RememberPolicy::UntilAppClose,
        );
        assert!(sa.is_session_cached(&scope), "promoted to session cache");

        sa.forget_all();
        assert!(
            !sa.is_session_cached(&scope),
            "forget_all clears everything"
        );
    }

    // --- single-key scope -------------------------------------------------

    fn import_protected_key(store: &Arc<SecretStore>, passphrase: &str) -> String {
        let index = std::sync::RwLock::new(std::collections::BTreeMap::new());
        let view = SingleKeyView::from_views(store, &index, Network::Testnet, None);
        let imported = view
            .import_wif_with_passphrase(
                &known_testnet_wif(),
                Some("My Key".into()),
                crate::wallet_backend::single_key::ImportPassphrase {
                    passphrase: Some(zeroize::Zeroizing::new(passphrase.to_string())),
                    hint: Some("the usual".into()),
                },
            )
            .expect("import");
        imported.address
    }

    fn known_testnet_wif() -> String {
        use dash_sdk::dpp::dashcore::PrivateKey;
        use dash_sdk::dpp::dashcore::secp256k1::SecretKey;
        let mut bytes = [0u8; 32];
        bytes[31] = 1;
        let sk = SecretKey::from_byte_array(&bytes).unwrap();
        PrivateKey::new(sk, Network::Testnet).to_wif()
    }

    #[tokio::test]
    async fn single_key_cache_miss_prompts_and_decrypts() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let address = import_protected_key(&store, SENTINEL_PASSPHRASE);

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = access(Arc::clone(&store), prompt.clone());
        let scope = SecretScope::SingleKey {
            address: address.clone(),
        };
        let len = sa
            .with_secret(&scope, |pt| {
                Ok(pt.expose_single_key().map(|k| k.len()).unwrap_or(0))
            })
            .await
            .unwrap();
        assert_eq!(len, 32, "decrypted single-key is 32 bytes");
        assert_eq!(prompt.ask_count(), 1);
    }

    #[tokio::test]
    async fn single_key_wrong_passphrase_reasks() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let address = import_protected_key(&store, SENTINEL_PASSPHRASE);

        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::once("bad-passphrase"),
            ScriptedAnswer::once(SENTINEL_PASSPHRASE),
        ]));
        let sa = access(Arc::clone(&store), prompt.clone());
        let scope = SecretScope::SingleKey { address };
        sa.with_secret(&scope, |_pt| Ok(())).await.unwrap();
        assert_eq!(prompt.ask_count(), 2);
    }

    /// TS-LAZY-03 — a protected single key lazy-migrates through the chokepoint:
    /// the first `with_secret` decrypts with the passphrase AND re-stores the
    /// raw 32 bytes; a second `with_secret` with a never-prompt host then
    /// resolves the SAME bytes prompt-free, and the recovered bytes equal the
    /// WIF plaintext.
    #[tokio::test]
    async fn ts_lazy_03_protected_single_key_migrates_via_chokepoint() {
        use dash_sdk::dpp::dashcore::PrivateKey;

        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let address = import_protected_key(&store, SENTINEL_PASSPHRASE);
        let expected: [u8; 32] = PrivateKey::from_wif(&known_testnet_wif()).unwrap().inner[..]
            .try_into()
            .unwrap();

        // First resolve: one passphrase, migrates to raw.
        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = access(Arc::clone(&store), prompt.clone());
        let scope = SecretScope::SingleKey {
            address: address.clone(),
        };
        let first = sa
            .with_secret(&scope, |pt| Ok(pt.expose_single_key().copied()))
            .await
            .unwrap();
        assert_eq!(first, Some(expected));
        assert_eq!(prompt.ask_count(), 1);

        // The vault now holds the raw 32 bytes (migration replaced the framing).
        let label = label_for_address(&address);
        let stored = store
            .get(&single_key_namespace_id(), &label)
            .unwrap()
            .unwrap();
        assert_eq!(stored.expose_secret().len(), 32, "migrated to raw");
        assert_eq!(stored.expose_secret(), &expected[..]);

        // Second resolve under a fresh never-prompt chokepoint is prompt-free.
        let never = Arc::new(TestPrompt::never());
        let sa2 = access(Arc::clone(&store), never.clone());
        let second = sa2
            .with_secret(&scope, |pt| Ok(pt.expose_single_key().copied()))
            .await
            .expect("prompt-free after migration");
        assert_eq!(second, Some(expected));
        assert_eq!(never.ask_count(), 0, "migrated key resolves prompt-free");
    }

    // --- secret confinement -----------------------------------------------

    #[tokio::test]
    async fn sentinel_never_appears_in_error_or_debug() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x0A; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        // Drive a wrong-passphrase-then-cancel so an error surfaces.
        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::once(SENTINEL_PASSPHRASE),
            ScriptedAnswer::Cancel,
        ]));
        let sa = access(store, prompt);
        let scope = SecretScope::HdSeed { seed_hash };

        // First: success path returns the decrypted seed by borrow, but we
        // must not be able to surface it. Build an error deliberately.
        let err = sa
            .with_secret::<()>(&scope, |_pt| Err(TaskError::SecretDecryptFailed))
            .await
            .expect_err("closure returned an error");

        let display = err.to_string();
        let debug = format!("{err:?}");
        let sentinel_seed_hex = hex::encode(SENTINEL_SEED);
        for (label, haystack) in [("display", &display), ("debug", &debug)] {
            assert!(
                !haystack.contains(SENTINEL_PASSPHRASE),
                "{label} leaked the sentinel passphrase: {haystack}"
            );
            assert!(
                !haystack.contains(&sentinel_seed_hex),
                "{label} leaked the sentinel seed bytes: {haystack}"
            );
        }

        // Second op cancels — the cancellation error must also be clean.
        let cancel = sa
            .with_secret::<()>(&scope, |_pt| Ok(()))
            .await
            .expect_err("cancel");
        let cdisplay = cancel.to_string();
        let cdebug = format!("{cancel:?}");
        assert!(!cdisplay.contains(SENTINEL_PASSPHRASE));
        assert!(!cdebug.contains(SENTINEL_PASSPHRASE));
        assert!(!cdisplay.contains(&sentinel_seed_hex));
        assert!(!cdebug.contains(&sentinel_seed_hex));
    }

    /// Cross-scope re-entrancy: a `with_secret_session` for scope A whose
    /// closure `.await`s another secret access for scope B must resolve both
    /// — the chokepoint releases the session-cache lock BEFORE running (and
    /// awaiting in) the closure (see step 1 of `with_secret_session`), so an
    /// inner call that re-takes the lock for a different scope cannot deadlock.
    /// This guards that documented lock-release-before-await property against a
    /// future cross-scope deadlock regression.
    #[tokio::test]
    async fn nested_cross_scope_access_resolves_both() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_a: WalletSeedHash = [0xA1; 32];
        let seed_b: WalletSeedHash = [0xB2; 32];
        let seed_b_bytes = [0x77u8; 64];
        store_protected_hd(&store, &seed_a, &SENTINEL_SEED, SENTINEL_PASSPHRASE);
        store_protected_hd(&store, &seed_b, &seed_b_bytes, SENTINEL_PASSPHRASE);

        // Both scopes remember for the session: scope B is promoted first so
        // the inner call hits the cache (re-taking the read lock) while the
        // outer scope-A session is live. Scope A's first access also remembers.
        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::remember(SENTINEL_PASSPHRASE, RememberPolicy::UntilAppClose),
            ScriptedAnswer::remember(SENTINEL_PASSPHRASE, RememberPolicy::UntilAppClose),
        ]));
        let sa = access(store, prompt.clone());
        let scope_a = SecretScope::HdSeed { seed_hash: seed_a };
        let scope_b = SecretScope::HdSeed { seed_hash: seed_b };

        // Seed the cache for B so the nested call is a pure cache hit.
        sa.with_secret(&scope_b, |_pt| Ok(())).await.unwrap();
        assert!(sa.is_session_cached(&scope_b));

        let sa_inner = sa.clone();
        let both = sa
            .with_secret_session(&scope_a, async move |session| {
                let outer_ok = session.plaintext().expose_hd_seed().copied() == Some(SENTINEL_SEED);
                // Re-enter the chokepoint for scope B from inside scope A's
                // live session. If the outer call still held the cache lock,
                // this would deadlock.
                let inner_ok = sa_inner
                    .with_secret(&scope_b, |pt| {
                        Ok(pt.expose_hd_seed().copied() == Some(seed_b_bytes))
                    })
                    .await?;
                Ok(outer_ok && inner_ok)
            })
            .await
            .expect("nested access must resolve, not deadlock");

        assert!(both, "both the outer and the nested inner secret resolved");
        assert_eq!(
            prompt.ask_count(),
            2,
            "one prompt for B (seeding) + one for A; the nested B hit the cache"
        );
    }

    #[tokio::test]
    async fn with_secret_session_holds_one_secret_across_steps() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x0B; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = access(store, prompt.clone());
        let scope = SecretScope::HdSeed { seed_hash };

        // Borrow the held secret three times (simulating three signs).
        let count = sa
            .with_secret_session(&scope, async |session| {
                let mut matches = 0;
                for _ in 0..3 {
                    if session.plaintext().expose_hd_seed().copied() == Some(SENTINEL_SEED) {
                        matches += 1;
                    }
                }
                Ok(matches)
            })
            .await
            .unwrap();
        assert_eq!(count, 3, "held secret borrowed N times");
        assert_eq!(prompt.ask_count(), 1, "one prompt for the whole operation");
    }

    // --- identity-key scope (raw seam, prompt-free) -----------------------

    use crate::model::qualified_identity::PrivateKeyTarget;
    use platform_wallet_storage::secrets::{SecretBytes, WalletId as SecretWalletId};

    /// Store a raw identity key in the vault under the seam label, the way the
    /// migration does.
    fn store_identity_key(
        store: &Arc<SecretStore>,
        identity_id: [u8; 32],
        target: &PrivateKeyTarget,
        key_id: u32,
        key: &[u8; 32],
    ) {
        let label = SecretScope::identity_key_label(target, key_id);
        SecretSeam::new(store)
            .put_secret(
                &SecretWalletId::from(identity_id),
                &label,
                &SecretBytes::from_slice(key),
            )
            .expect("store identity key");
    }

    /// TS-FAST-01 — an identity-key scope resolves prompt-free under a
    /// never-prompt host (the unprotected fast-path), returns the exact 32
    /// bytes, and never asks. Proves headless/MCP identity signing works.
    #[tokio::test]
    async fn ts_fast_01_identity_key_resolves_prompt_free() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let identity_id = [0x33u8; 32];
        let key = [0xC7u8; 32];
        store_identity_key(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            7,
            &key,
        );

        // never() panics if asked — proves no prompt fires.
        let prompt = Arc::new(TestPrompt::never());
        let sa = access(store, prompt.clone());
        let scope = SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 7,
        };

        let matched = sa
            .with_secret(&scope, |pt| {
                Ok(pt.expose_identity_key().copied() == Some(key))
            })
            .await
            .expect("identity key resolves prompt-free");
        assert!(matched, "closure saw the raw identity key");
        assert_eq!(prompt.ask_count(), 0, "identity key never prompts");
        assert!(
            sa.can_resolve_without_prompt(&scope),
            "identity key is always resolvable without a prompt"
        );
    }

    /// TS-MISS-01/02 — an HD seed present in NEITHER raw nor legacy form
    /// surfaces the loud typed `SecretSeamMissing` (never a silent `Ok(None)`
    /// that would drop a key on the floor), distinct from `WalletNotFound`.
    #[tokio::test]
    async fn ts_miss_01_hd_seed_in_neither_form_is_secret_seam_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let sa = access(store, Arc::new(TestPrompt::never()));
        let scope = SecretScope::HdSeed {
            seed_hash: [0x7Du8; 32],
        };
        let err = sa
            .with_secret(&scope, |_pt| Ok(()))
            .await
            .expect_err("seed gone");
        assert!(
            matches!(err, TaskError::SecretSeamMissing),
            "expected SecretSeamMissing, got {err:?}"
        );
    }

    /// A missing identity key surfaces the loud typed `IdentityKeyMissing`,
    /// never a silent miss.
    #[tokio::test]
    async fn identity_key_missing_is_loud() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let sa = access(store, Arc::new(TestPrompt::never()));
        let scope = SecretScope::IdentityKey {
            identity_id: [0x44u8; 32],
            target: PrivateKeyTarget::PrivateKeyOnVoterIdentity,
            key_id: 1,
        };
        let err = sa
            .with_secret(&scope, |_pt| Ok(()))
            .await
            .expect_err("missing identity key");
        assert!(
            matches!(err, TaskError::IdentityKeyMissing),
            "expected IdentityKeyMissing, got {err:?}"
        );
    }

    /// TS-LEGACY-01 — with only a legacy unprotected envelope present (no raw
    /// `seed.raw.v1`), the seam-first reader falls through to the retained
    /// legacy decoder and recovers the exact seed, prompt-free.
    #[tokio::test]
    async fn ts_legacy_01_hd_legacy_envelope_served_when_raw_absent() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x4E; 32];
        store_unprotected_hd(&store, &seed_hash, &SENTINEL_SEED);

        let prompt = Arc::new(TestPrompt::never());
        let sa = access(store, prompt.clone());
        let scope = SecretScope::HdSeed { seed_hash };
        sa.with_secret(&scope, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(SENTINEL_SEED));
            Ok(())
        })
        .await
        .expect("legacy envelope served via fallback");
        assert_eq!(prompt.ask_count(), 0, "unprotected legacy ⇒ no prompt");
    }

    /// Seam-first precedence: when BOTH a raw `seed.raw.v1` and a legacy
    /// envelope exist (the legal mid-migration state, TS-CRASH-01 read half),
    /// the raw value wins and the legacy is not consulted.
    #[tokio::test]
    async fn raw_seed_wins_over_legacy_when_both_present() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x5E; 32];
        // Legacy holds one seed; raw holds a DIFFERENT one — proving which won.
        let legacy_seed = [0x11u8; 64];
        store_unprotected_hd(&store, &seed_hash, &legacy_seed);
        let raw_seed = [0x99u8; 64];
        WalletSeedView::new(&store)
            .set_raw(&seed_hash, &raw_seed)
            .unwrap();

        let sa = access(store, Arc::new(TestPrompt::never()));
        let scope = SecretScope::HdSeed { seed_hash };
        sa.with_secret(&scope, |pt| {
            assert_eq!(
                pt.expose_hd_seed().copied(),
                Some(raw_seed),
                "raw seam value must win over the legacy envelope"
            );
            Ok(())
        })
        .await
        .expect("raw wins");
    }

    // --- Tier-2 per-secret object-password adoption -----------------------

    /// TS-T2-01 — lazy re-wrap KEEPS protection. A protected legacy AES-GCM
    /// envelope, on first unlock, migrates to a Tier-2 object-password envelope
    /// at the raw label (NOT downgraded to a password-free raw secret), the
    /// legacy envelope is dropped, and the seed reads back only with its
    /// password.
    #[tokio::test]
    async fn ts_t2_01_protected_seed_rewraps_to_tier2_on_first_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x71; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SENTINEL_PASSPHRASE);

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = access(store.clone(), prompt.clone());
        let scope = SecretScope::HdSeed { seed_hash };

        sa.with_secret(&scope, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(SENTINEL_SEED));
            Ok(())
        })
        .await
        .expect("first unlock");
        assert_eq!(prompt.ask_count(), 1);

        let view = WalletSeedView::new(&store);
        // Steady state is Tier-2 protected, NOT raw.
        assert_eq!(view.scheme(&seed_hash).unwrap(), SecretScheme::Protected);
        // Legacy envelope dropped.
        assert!(
            view.get(&seed_hash).unwrap().is_none(),
            "legacy envelope removed after re-wrap"
        );
        // Reads back only WITH the object password ...
        let pw = SecretString::new(SENTINEL_PASSPHRASE);
        assert_eq!(
            view.get_protected(&seed_hash, &pw).unwrap().map(|z| *z),
            Some(SENTINEL_SEED)
        );
        // ... and NOT without it (a raw read sees a protected blob).
        assert!(
            view.get_raw(&seed_hash).is_err(),
            "raw read of a protected seed must fail, never strip protection"
        );
    }

    /// TS-T2-02 — a Tier-2 seed re-asks on a wrong object password (upstream
    /// `WrongPassword` ⇒ re-prompt, not abort) and then succeeds.
    #[tokio::test]
    async fn ts_t2_02_tier2_seed_wrong_password_reasks_then_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x72; 32];
        let right = SecretString::new(SENTINEL_PASSPHRASE);
        WalletSeedView::new(&store)
            .set_protected(&seed_hash, &SENTINEL_SEED, &right)
            .expect("seal seed as Tier-2");
        assert_eq!(
            WalletSeedView::new(&store).scheme(&seed_hash).unwrap(),
            SecretScheme::Protected
        );

        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::once("not-the-password"),
            ScriptedAnswer::once(SENTINEL_PASSPHRASE),
        ]));
        let sa = access(store, prompt.clone());
        let scope = SecretScope::HdSeed { seed_hash };
        sa.with_secret(&scope, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(SENTINEL_SEED));
            Ok(())
        })
        .await
        .expect("retry succeeds");
        assert_eq!(prompt.ask_count(), 2, "one wrong-pass re-ask, then success");
    }

    /// TS-T2-03 — PER-SECRET password isolation. Two seeds protected under
    /// DIFFERENT passwords: unlocking A (and remembering it) does NOT satisfy
    /// B — B still prompts for its OWN password, each decrypts only with its
    /// own, and A's remembered entry never unlocks B.
    #[tokio::test]
    async fn ts_t2_03_per_secret_passwords_are_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let hash_a: WalletSeedHash = [0xAA; 32];
        let hash_b: WalletSeedHash = [0xBB; 32];
        let seed_a = [0xA1u8; 64];
        let seed_b = [0xB2u8; 64];
        let pw_a = SecretString::new("password-A-aaaaaaaaaa");
        let pw_b = SecretString::new("password-B-bbbbbbbbbb");
        let view = WalletSeedView::new(&store);
        view.set_protected(&hash_a, &seed_a, &pw_a).unwrap();
        view.set_protected(&hash_b, &seed_b, &pw_b).unwrap();

        // Scripted in access order: A remembers, then B.
        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::remember("password-A-aaaaaaaaaa", RememberPolicy::UntilAppClose),
            ScriptedAnswer::remember("password-B-bbbbbbbbbb", RememberPolicy::UntilAppClose),
        ]));
        let sa = access(store, prompt.clone());
        let scope_a = SecretScope::HdSeed { seed_hash: hash_a };
        let scope_b = SecretScope::HdSeed { seed_hash: hash_b };

        sa.with_secret(&scope_a, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(seed_a));
            Ok(())
        })
        .await
        .unwrap();
        assert!(sa.is_session_cached(&scope_a));
        assert!(
            !sa.is_session_cached(&scope_b),
            "A's unlock must not cache B"
        );

        // B STILL prompts (A's cache entry does not satisfy B) and decrypts to B.
        sa.with_secret(&scope_b, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(seed_b));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(prompt.ask_count(), 2, "B prompted independently of A");

        // A still resolves from its own cache entry — no third prompt.
        sa.with_secret(&scope_a, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(seed_a));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(prompt.ask_count(), 2, "A served from cache, no re-prompt");
    }
}
