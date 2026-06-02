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
//!   2. else prompt via [`SecretPrompt`] for the passphrase, decrypt the
//!      stored envelope just-in-time, optionally promote to the session
//!      cache, run the closure, then zeroize.
//!
//! Unprotected scopes (HD wallets stored without a password, imported keys
//! stored without a passphrase) resolve **without prompting** — the
//! envelope is decryptable with no passphrase, so the chokepoint reads it
//! directly (Smythe must-fix #4).
//!
//! Secret hygiene (Smythe must-fixes #1–#3):
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
use platform_wallet_storage::secrets::SecretStore;
use platform_wallet_storage::secrets::SecretString;
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::model::single_key::ImportedKey;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::encryption::derive_password_key;
use crate::model::wallet::seed_envelope::StoredSeedEnvelope;
use crate::wallet_backend::secret_prompt::{
    RememberPolicy, SecretPrompt, SecretPromptRequest, SecretPromptRetry, SecretScope,
};
use crate::wallet_backend::single_key::{label_for_address, single_key_namespace_id};
use crate::wallet_backend::single_key_entry::SingleKeyEntry;
use crate::wallet_backend::wallet_seed_store::WalletSeedView;

/// Length of an HD BIP-39 seed.
const HD_SEED_LEN: usize = 64;
/// Length of an imported single-key secret.
const SINGLE_KEY_LEN: usize = 32;

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
            SecretPlaintext::SingleKey(_) => None,
        }
    }

    /// Borrow the 32-byte single-key secret, or `None` if this is an HD
    /// seed plaintext.
    pub fn expose_single_key(&self) -> Option<&[u8; SINGLE_KEY_LEN]> {
        match self {
            SecretPlaintext::SingleKey(k) => Some(&***k),
            SecretPlaintext::HdSeed(_) => None,
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
}

impl Plaintext {
    fn borrow(&self) -> SecretPlaintext<'_> {
        match self {
            Plaintext::HdSeed(s) => SecretPlaintext::HdSeed(s),
            Plaintext::SingleKey(k) => SecretPlaintext::SingleKey(k),
        }
    }

    /// An owned, op-scoped `Zeroizing` copy of this plaintext. Used only to
    /// lift a cached secret off the cache lock so the consuming closure can
    /// run without holding it. The copy zeroizes on drop.
    fn to_op_copy(&self) -> Plaintext {
        match self {
            Plaintext::HdSeed(s) => Plaintext::HdSeed(Zeroizing::new(**s)),
            Plaintext::SingleKey(k) => Plaintext::SingleKey(Zeroizing::new(**k)),
        }
    }
}

/// A session-cache entry: the boxed plaintext plus its expiry policy.
///
/// The plaintext is boxed (Smythe must-fix #3) so a `HashMap` rehash moves
/// only the `Box` pointer, never the secret bytes — no un-wiped inline copy
/// is left behind. `expires_at = None` means "until app close".
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
        // 1. Session-cache hit (opt-in, TTL-honored). Copy the cached
        //    plaintext into an op-scoped `Zeroizing` buffer and release the
        //    lock BEFORE running the closure: the closure may `.await` and
        //    may itself reach back into the cache for a different scope, so
        //    holding the lock across it would risk a deadlock. The op copy
        //    zeroizes on scope exit; the boxed cache entry is untouched.
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
        };
        self.maybe_remember(scope, &owned, policy);
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
        let expires_at = match policy {
            RememberPolicy::None => return,
            RememberPolicy::UntilAppClose => None,
            RememberPolicy::For(duration) => Some(Instant::now() + duration),
        };
        let boxed = match plaintext {
            Plaintext::HdSeed(s) => Box::new(Plaintext::HdSeed(Zeroizing::new(**s))),
            Plaintext::SingleKey(k) => Box::new(Plaintext::SingleKey(Zeroizing::new(**k))),
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
    /// [`TaskError::SecretPromptUnavailable`] (Q-HEADLESS).
    fn cancel_error(&self) -> TaskError {
        if self.inner.prompt.is_interactive() {
            TaskError::SecretPromptCancelled
        } else {
            TaskError::SecretPromptUnavailable
        }
    }

    /// Whether `scope`'s stored secret is passphrase-protected. Drives the
    /// unprotected fast-path (Smythe must-fix #4). Reads the in-memory
    /// index/meta where possible; falls back to the stored envelope.
    fn scope_has_passphrase(&self, scope: &SecretScope) -> Result<bool, TaskError> {
        match scope {
            SecretScope::HdSeed { seed_hash } => {
                let view = WalletSeedView::new(&self.inner.secret_store);
                let envelope = view.get(seed_hash)?.ok_or(TaskError::WalletNotFound)?;
                Ok(envelope.uses_password)
            }
            SecretScope::SingleKey { address } => {
                if let Ok(index) = self.inner.single_key_index.read()
                    && let Some(meta) = index.get(address)
                {
                    return Ok(meta.has_passphrase);
                }
                let entry = self.load_single_key_entry(address)?;
                Ok(entry.has_passphrase)
            }
        }
    }

    /// Decrypt the stored secret for `scope` with `passphrase`
    /// (`None` for unprotected scopes). The only place the vault is read
    /// for plaintext. Returns the kind-tagged owned plaintext.
    fn decrypt_jit(
        &self,
        scope: &SecretScope,
        passphrase: Option<&SecretString>,
    ) -> Result<Plaintext, TaskError> {
        match scope {
            SecretScope::HdSeed { seed_hash } => {
                let view = WalletSeedView::new(&self.inner.secret_store);
                let envelope = view.get(seed_hash)?.ok_or(TaskError::WalletNotFound)?;
                let seed = decrypt_hd_seed(&envelope, passphrase)?;
                Ok(Plaintext::HdSeed(seed))
            }
            SecretScope::SingleKey { address } => {
                let entry = self.load_single_key_entry(address)?;
                let raw = entry.decrypt(passphrase.map(|p| p.expose_secret()))?;
                Ok(Plaintext::SingleKey(Zeroizing::new(raw)))
            }
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
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&envelope.nonce),
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
    matches!(
        e,
        TaskError::SingleKeyPassphraseIncorrect | TaskError::HdPassphraseIncorrect
    )
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
        // rather than a misleading "you cancelled" (Q-HEADLESS).
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
                    passphrase: Some(passphrase.to_string()),
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

    // --- secret confinement (Smythe must-fix #5) --------------------------

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
}
