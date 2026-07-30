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

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::KeyID;
use platform_wallet_storage::secrets::{
    SecretBytes, SecretStore, SecretStoreError, SecretString, WalletId as SecretWalletId,
};
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::single_key::ImportedKey;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::encryption::{DecryptError, decrypt_message};
use crate::model::wallet::seed_envelope::StoredSeedEnvelope;
use crate::wallet_backend::identity_key_store::identity_flavored;
use crate::wallet_backend::poison::{read_recover, write_recover};
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

/// A ref-counted claim on one session-cached scope, forgotten when the last
/// holder drops.
///
/// A secret promoted for an operation usually has more than one consumer, and
/// their lifetimes overlap in an order nobody controls — the unlock gesture's
/// own reconciliation subtask and the storage update's bootstrap pass both need
/// the seed the same unlock promoted. Handing the lifetime to whichever consumer
/// happens to finish first evicts the secret from under the other, which then
/// cache-misses and raises a passphrase prompt the user did not ask for. Each
/// consumer holds a clone of this lease instead, so the scope survives exactly
/// as long as someone still needs it.
///
/// Dropping every clone is equivalent to [`SecretAccess::forget`]; a scope
/// promoted with [`RememberPolicy::UntilAppClose`] and never leased is
/// unaffected.
#[derive(Clone, Debug)]
pub struct SecretLease(Arc<SecretLeaseInner>);

impl SecretLease {
    /// The scope this lease keeps resolvable. Carries no secret material — an
    /// `HdSeed` scope names the seed's *hash*.
    pub fn scope(&self) -> &SecretScope {
        &self.0.scope
    }
}

#[derive(Debug)]
struct SecretLeaseInner {
    access: SecretAccess,
    scope: SecretScope,
}

impl Drop for SecretLeaseInner {
    fn drop(&mut self) {
        self.access.forget(&self.scope);
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
    wallet_meta: RwLock<BTreeMap<WalletSeedHash, PromptMeta>>,
    /// Single-key index (address → alias / hint / has_passphrase) for
    /// prompt copy and the unprotected fast-path check.
    single_key_index: RwLock<BTreeMap<String, ImportedKey>>,
    /// Identity prompt-copy index (identity id → alias / password hint) for
    /// the sign-time prompt of an opted-in (Tier-2) identity. Display-only;
    /// the vault scheme — not this index — gates whether a prompt fires.
    identity_prompt_index: RwLock<BTreeMap<[u8; 32], PromptMeta>>,
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

/// Minimal prompt-copy metadata for a secret that may be password-protected —
/// an HD wallet (mirrored from the wallet-meta sidecar) or an identity whose
/// keys are opted-in Tier-2 (seeded from the loaded `QualifiedIdentity` alias
/// and the DET-side `IdentityMetaView` hint at hydration). The chokepoint uses
/// it to build an informative [`SecretPromptRequest`] without reaching back
/// into the wallet backend.
///
/// Display-only: it NEVER decides whether to prompt (the vault scheme does, in
/// [`SecretAccess::scope_has_passphrase`]). A missing entry degrades to a
/// generic label, never an error.
#[derive(Clone, Debug, Default)]
pub struct PromptMeta {
    /// User-visible label — wallet name, or identity DPNS name / truncated id,
    /// if any.
    pub alias: Option<String>,
    /// User-set password hint, if any.
    pub password_hint: Option<String>,
}

/// An identity object password VERIFIED against an existing protected key of
/// the identity. Produced by
/// [`SecretAccess::verify_identity_object_password`] and consumed by
/// [`SecretAccess::seal_new_identity_key_with_password`], so the add-key flow
/// can enforce the protected-identity precondition BEFORE the irreversible
/// on-chain broadcast and seal the new key AFTER it — with a single prompt.
/// Wraps a [`SecretString`], so the plaintext zeroizes on drop.
pub struct VerifiedIdentityPassword(SecretString);

impl std::fmt::Debug for VerifiedIdentityPassword {
    /// Redacts the wrapped password (M-PUBLIC-DEBUG, M-DONT-LEAK-TYPES).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("VerifiedIdentityPassword").finish()
    }
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
                identity_prompt_index: RwLock::new(BTreeMap::new()),
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
    /// prompts can show the wallet name and password hint. Poison-safe: a
    /// poisoned lock is recovered (matching `forget`/`forget_all`) so a panicked
    /// reader can never freeze prompt-copy metadata for the rest of the session.
    pub fn set_wallet_meta(&self, meta: BTreeMap<WalletSeedHash, PromptMeta>) {
        let mut guard = self
            .inner
            .wallet_meta
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        *guard = meta;
    }

    /// Replace the single-key prompt-copy index. Used at hydration time and
    /// after an import so prompts can show the key nickname and hint, and
    /// so the unprotected fast-path can skip the prompt. Poison-safe: a poisoned
    /// lock is recovered so the index can self-heal after a panicked reader.
    pub fn set_single_key_index(&self, index: BTreeMap<String, ImportedKey>) {
        let mut guard = self
            .inner
            .single_key_index
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        *guard = index;
    }

    /// Replace the identity prompt-copy index. Used at hydration time and
    /// after an opt-in migration so the sign-time prompt for a protected
    /// identity shows its label and password hint. Display-only — never
    /// gates whether a prompt fires (the vault scheme does). Poison-safe: a
    /// poisoned lock is recovered so the index can self-heal after a panicked
    /// reader.
    pub fn set_identity_prompt_index(&self, index: BTreeMap<[u8; 32], PromptMeta>) {
        let mut guard = self
            .inner
            .identity_prompt_index
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        *guard = index;
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
                let guard = read_recover(&self.inner.session);
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
            if needs_evict {
                let mut guard = write_recover(&self.inner.session);
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
                    // Cache a copy; the original is still needed for this op's
                    // session borrow below.
                    self.maybe_remember(scope, plaintext.to_op_copy(), reply.remember);
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
        // Copy the borrowed plaintext into an owned `Plaintext` exactly once,
        // then hand ownership to the cache (moved into the box, not re-copied).
        let owned = match plaintext {
            SecretPlaintext::HdSeed(s) => Plaintext::HdSeed(Zeroizing::new(**s)),
            SecretPlaintext::SingleKey(k) => Plaintext::SingleKey(Zeroizing::new(**k)),
            SecretPlaintext::IdentityKey(k) => Plaintext::IdentityKey(Zeroizing::new(**k)),
        };
        self.maybe_remember(scope, owned, policy);
    }

    /// Decrypt an HD-seed envelope with an explicitly-supplied passphrase and
    /// promote the result into the session cache — **without prompting**.
    ///
    /// This is the unlock-gesture verification boundary: the supplied
    /// passphrase is checked against the actual vault object through the same
    /// chokepoint decrypt path every signing operation uses, then the seed is
    /// cached according to `policy`. `passphrase` is `None` for unprotected
    /// wallets (the envelope decrypts verbatim). The plaintext is moved into
    /// the cache when retained and otherwise zeroizes on return.
    ///
    /// The lazy legacy→steady-state re-wrap happens inside [`Self::decrypt_jit`]:
    /// a protected seed re-wraps to **Tier-2 under the same password** (protection
    /// KEPT, never downgraded to a raw secret), an unprotected one to the raw
    /// label. A protected re-wrap rejected by the current storage policy is
    /// deferred without blocking the successfully decrypted seed; the legacy
    /// envelope remains intact for the next unlock.
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
        self.maybe_remember(&scope, plaintext, policy);
        Ok(())
    }

    /// Seal a NEW identity key Tier-2 under the identity's EXISTING object
    /// password. A protected identity must never acquire a keyless
    /// key, so when a key is added to such an identity it is sealed here rather
    /// than written raw.
    ///
    /// Prompts for the password and VERIFIES it by unsealing `verify` (an
    /// existing `Protected` key of the same identity) — so the whole identity
    /// stays under ONE password, with the standard wrong-password re-ask — then
    /// seals `new_key` at its label under that same password. Headless
    /// (`NullSecretPrompt`) yields [`TaskError::SecretPromptUnavailable`] and
    /// nothing is written (fail closed).
    ///
    /// This is the verify-then-seal composition for callers that run both
    /// halves together. The add-key flow instead calls
    /// [`Self::verify_identity_object_password`] BEFORE its on-chain broadcast
    /// and [`Self::seal_new_identity_key_with_password`] AFTER, so a headless or
    /// wrong-password attempt fails closed before any state transition is sent
    /// (O-2) — the same single prompt, split across the broadcast.
    pub async fn seal_new_identity_key(
        &self,
        identity_id: [u8; 32],
        verify: &SecretScope,
        new_target: &PrivateKeyTarget,
        new_key_id: KeyID,
        new_key: &[u8; 32],
    ) -> Result<(), TaskError> {
        let password = self.verify_identity_object_password(verify).await?;
        self.seal_new_identity_key_with_password(
            identity_id,
            new_target,
            new_key_id,
            new_key,
            &password,
        )
    }

    /// Prompt for the identity's object password and VERIFY it by unsealing
    /// `verify` (an existing `Protected` key of the same identity), returning
    /// the verified password for a later
    /// [`Self::seal_new_identity_key_with_password`].
    ///
    /// Split out of [`Self::seal_new_identity_key`] so the add-key flow can
    /// enforce the protected-identity precondition BEFORE its irreversible
    /// on-chain broadcast and seal the new key AFTER, without a second prompt.
    /// Headless ([`NullSecretPrompt`](crate::wallet_backend::secret_prompt::NullSecretPrompt))
    /// yields [`TaskError::SecretPromptUnavailable`] (fail closed); a wrong
    /// password re-asks. The verification plaintext is dropped (zeroized)
    /// immediately; the returned password zeroizes on drop.
    pub async fn verify_identity_object_password(
        &self,
        verify: &SecretScope,
    ) -> Result<VerifiedIdentityPassword, TaskError> {
        let mut retry: Option<SecretPromptRetry> = None;
        loop {
            let request = self.build_request(verify, retry);
            let reply = self
                .inner
                .prompt
                .request(request)
                .await
                .map_err(|_cancelled| self.cancel_error())?;

            // Verify the typed password against an existing protected key so the
            // new key is later sealed under the SAME password as the rest. The
            // verification plaintext is dropped (zeroized) immediately.
            match self.decrypt_jit(verify, Some(&reply.passphrase)) {
                Ok(_verified) => return Ok(VerifiedIdentityPassword(reply.passphrase)),
                Err(e) if is_wrong_passphrase(&e) => {
                    retry = Some(SecretPromptRetry::WrongPassphrase);
                    continue;
                }
                Err(other) => return Err(other),
            }
        }
    }

    /// Whether an already-verified identity object password still opens
    /// `verify`, an existing `Protected` key of the identity as the vault holds
    /// it NOW. No prompt.
    ///
    /// [`VerifiedIdentityPassword`] proves only what was true when the prompt
    /// closed. A caller that seals under it later — after an await, or after a
    /// dialog the user left open — must re-prove it against the current vault,
    /// or an identity unprotected and re-protected under a different password
    /// in between would end up needing two passwords for one identity.
    ///
    /// # Errors
    ///
    /// Only genuine vault failures. A password the vault rejects is `Ok(false)`,
    /// not an error: the caller decides whether that is a retry or a refusal.
    pub fn identity_object_password_still_opens(
        &self,
        verify: &SecretScope,
        password: &VerifiedIdentityPassword,
    ) -> Result<bool, TaskError> {
        match self.decrypt_jit(verify, Some(&password.0)) {
            Ok(_verified) => Ok(true),
            Err(e) if is_wrong_passphrase(&e) => Ok(false),
            Err(other) => Err(other),
        }
    }

    /// Seal a NEW identity key Tier-2 under an ALREADY-VERIFIED identity object
    /// password — the back half of [`Self::seal_new_identity_key`].
    /// No prompt and no re-verify: `password` came from a successful
    /// [`Self::verify_identity_object_password`], so this only writes the sealed
    /// key. The add-key flow calls this AFTER its on-chain broadcast, having
    /// verified the password up front, so the new key never lands keyless.
    pub fn seal_new_identity_key_with_password(
        &self,
        identity_id: [u8; 32],
        new_target: &PrivateKeyTarget,
        new_key_id: KeyID,
        new_key: &[u8; 32],
        password: &VerifiedIdentityPassword,
    ) -> Result<(), TaskError> {
        let scope_id = SecretWalletId::from(identity_id);
        let label = SecretScope::identity_key_label(new_target, new_key_id);
        self.seam()
            .put_secret_protected(
                &scope_id,
                &label,
                &SecretBytes::from_slice(new_key),
                &password.0,
            )
            .map_err(identity_flavored)
    }

    /// Take a ref-counted [`SecretLease`] on `scope`: the session-cached secret
    /// is forgotten once this lease and every clone of it are dropped.
    ///
    /// Give one clone to each consumer that needs the scope resolvable
    /// prompt-free, so the last one out does the forgetting. Taking a lease does
    /// not itself promote anything — the caller promotes first (e.g. via
    /// [`Self::promote_hd_seed_with_passphrase`]), then leases the lifetime.
    ///
    /// **Refcounting is per lease *object*, not per `scope`.** Each call to
    /// `lease()` mints an independent `Arc` with its own refcount — it does
    /// NOT join an existing lease on the same scope. If two unrelated
    /// consumers each call `lease(scope)` directly, the first one's lease
    /// drops and forgets the secret while the second is still relying on it.
    /// When a second consumer of an already-leased scope shows up, hand it a
    /// **clone of the existing `SecretLease`** — don't call `lease()` again.
    pub fn lease(&self, scope: SecretScope) -> SecretLease {
        SecretLease(Arc::new(SecretLeaseInner {
            access: self.clone(),
            scope,
        }))
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

    /// Insert into the session cache iff `policy` requests it; expiry stamped
    /// for `For(duration)`.
    ///
    /// Takes the plaintext **by value** and moves it straight into the boxed
    /// cache entry, so the secret is copied exactly once — at the call boundary
    /// — rather than copied to build the argument and copied again to box it. A
    /// caller that must keep the plaintext after caching (mid-operation) passes
    /// a [`Plaintext::to_op_copy`]; a caller that is done with it passes
    /// ownership.
    fn maybe_remember(&self, scope: &SecretScope, plaintext: Plaintext, policy: RememberPolicy) {
        let now = Instant::now();
        let expires_at = match policy {
            RememberPolicy::None => return,
            RememberPolicy::UntilAppClose => None,
            // On the (unreachable today) overflow of `now + duration`, expire
            // immediately rather than risk over-retaining the secret — `None`
            // here would mean "never expires".
            RememberPolicy::For(duration) => Some(now.checked_add(duration).unwrap_or(now)),
        };
        write_recover(&self.inner.session).insert(
            scope.clone(),
            SessionEntry {
                plaintext: Box::new(plaintext),
                expires_at,
            },
        );
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
                let label = label_for_address(address);
                match self.seam().scheme(&single_key_namespace_id(), &label)? {
                    // Tier-2 protected (re-wrapped) ⇒ needs the object password.
                    SecretScheme::Protected => Ok(true),
                    SecretScheme::Absent => Err(TaskError::ImportedKeyNotFound),
                    // Unprotected at the vault: either a migrated raw-32 key
                    // (no passphrase) or a not-yet-migrated legacy `SingleKeyEntry`
                    // blob whose `has_passphrase` flag decides.
                    SecretScheme::Unprotected => {
                        if self.single_key_raw(address)?.is_some() {
                            return Ok(false);
                        }
                        if let Ok(index) = self.inner.single_key_index.read()
                            && let Some(meta) = index.get(address)
                        {
                            return Ok(meta.has_passphrase);
                        }
                        Ok(self.load_single_key_entry(address)?.has_passphrase)
                    }
                }
            }
            // Identity keys default to keyless (Tier-1 raw) and resolve
            // prompt-free so headless/MCP signing keeps working. A user may
            // OPT IN per identity to seal them Tier-2; the vault scheme is the
            // single source of truth for whether to prompt — no parallel flag.
            SecretScope::IdentityKey {
                identity_id,
                target,
                key_id,
            } => {
                let label = SecretScope::identity_key_label(target, *key_id);
                match self
                    .seam()
                    .scheme(&SecretWalletId::from(*identity_id), &label)?
                {
                    // Tier-2 protected ⇒ needs the identity's object password.
                    SecretScheme::Protected => Ok(true),
                    // Tier-1 raw ⇒ keyless default, prompt-free.
                    SecretScheme::Unprotected => Ok(false),
                    // Absent ⇒ the stored identity references a key whose bytes
                    // are gone. Loud, never a silent prompt-free miss.
                    SecretScheme::Absent => Err(TaskError::IdentityKeyMissing),
                }
            }
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
                    //
                    // TODO(v1.1): `get_protected` rejects any password below
                    // `platform_wallet_storage::secrets::MIN_PASSPHRASE_LEN`
                    // (8 UTF-8 bytes after trimming) on READ, not just write (see
                    // `rs-platform-wallet-storage/src/secrets/wire/envelope.rs`,
                    // `unwrap_password_payload`, comment `(a0)` — intentional,
                    // mirrors the write floor so a backend-write attacker can't
                    // plant a weakly sealed envelope). A wallet already migrated
                    // to Tier-2 with a password below that floor by the July 17/21 weekly
                    // builds (see CHANGELOG "Compatibility blocker" entry) has no
                    // downstream escape hatch here and stays permanently
                    // unreadable until upstream adds a scoped migration-read
                    // capability. Revisit this call once that lands upstream.
                    SecretScheme::Protected => {
                        let pw = passphrase.ok_or(TaskError::HdPassphraseIncorrect)?;
                        let seed = view
                            .get_protected(seed_hash, pw)?
                            .ok_or(TaskError::SecretSeamMissing)?;
                        Ok(Plaintext::HdSeed(seed))
                    }
                    // Legacy AES-GCM envelope: decode-only reader, then LAZY
                    // re-wrap to the steady-state form and garbage-collect the
                    // redundant envelope. A protected seed re-wraps to Tier-2 under the
                    // SAME user password (protection KEPT, not downgraded to
                    // raw); an unprotected one goes to the raw label. An absent
                    // envelope ⇒ the secret is gone (loud, never a silent miss).
                    // The scheme probe prefers the new label on subsequent reads.
                    SecretScheme::Absent => {
                        let envelope = view.get(seed_hash)?.ok_or(TaskError::SecretSeamMissing)?;
                        let seed = decrypt_hd_seed(&envelope, passphrase)?;
                        if envelope.uses_password {
                            let pw = passphrase.ok_or(TaskError::HdPassphraseIncorrect)?;
                            handle_lazy_tier2_rewrap_result(
                                view.set_protected(seed_hash, &seed, pw),
                            )?;
                        } else {
                            view.set_raw(seed_hash, &seed)?;
                        }
                        Ok(Plaintext::HdSeed(seed))
                    }
                }
            }
            SecretScope::SingleKey { address } => {
                let label = label_for_address(address);
                match self.seam().scheme(&single_key_namespace_id(), &label)? {
                    // Tier-2 — unseal with this key's own object password.
                    SecretScheme::Protected => {
                        let pw = passphrase.ok_or(TaskError::SingleKeyPassphraseIncorrect)?;
                        let raw = self
                            .seam()
                            .get_secret_protected(&single_key_namespace_id(), &label, pw)?
                            .ok_or(TaskError::ImportedKeyNotFound)?;
                        let key: [u8; SINGLE_KEY_LEN] =
                            raw.expose_secret().try_into().map_err(|_| {
                                tracing::warn!(
                                    target = "wallet_backend::secret_access",
                                    blob_len = raw.expose_secret().len(),
                                    "Tier-2 single key has wrong length",
                                );
                                TaskError::SecretDecryptFailed
                            })?;
                        Ok(Plaintext::SingleKey(Zeroizing::new(key)))
                    }
                    SecretScheme::Absent => Err(TaskError::ImportedKeyNotFound),
                    SecretScheme::Unprotected => {
                        // A migrated raw-32 key wins prompt-free.
                        if let Some(raw) = self.single_key_raw(address)? {
                            return Ok(Plaintext::SingleKey(raw));
                        }
                        // Legacy `SingleKeyEntry` (decode-only reader). A
                        // protected entry was just decrypted with the user's
                        // passphrase — LAZY re-wrap it to Tier-2 under the SAME
                        // password (the upsert replaces the AES-GCM framing),
                        // KEEPING protection. Idempotent.
                        let entry = self.load_single_key_entry(address)?;
                        let raw = entry.decrypt(passphrase.map(|p| p.expose_secret()))?;
                        if entry.has_passphrase {
                            let pw = passphrase.ok_or(TaskError::SingleKeyPassphraseIncorrect)?;
                            self.migrate_single_key_to_tier2(address, &raw, pw);
                        }
                        Ok(Plaintext::SingleKey(raw))
                    }
                }
            }
            SecretScope::IdentityKey {
                identity_id,
                target,
                key_id,
            } => {
                let scope_id = SecretWalletId::from(*identity_id);
                let label = SecretScope::identity_key_label(target, *key_id);
                match self.seam().scheme(&scope_id, &label)? {
                    // Tier-2 — unseal with this identity's object password
                    // (opted-in). Symmetric to the single-key Protected arm.
                    SecretScheme::Protected => {
                        let pw = passphrase.ok_or(TaskError::IdentityKeyPassphraseIncorrect)?;
                        let raw = self
                            .seam()
                            .get_secret_protected(&scope_id, &label, pw)?
                            .ok_or(TaskError::IdentityKeyMissing)?;
                        let key = identity_key_from_bytes(raw.expose_secret())?;
                        Ok(Plaintext::IdentityKey(Zeroizing::new(key)))
                    }
                    // Tier-1 raw — keyless default, no password.
                    SecretScheme::Unprotected => {
                        let raw = self
                            .seam()
                            .get_secret(&scope_id, &label)?
                            .ok_or(TaskError::IdentityKeyMissing)?;
                        let key = identity_key_from_bytes(raw.expose_secret())?;
                        Ok(Plaintext::IdentityKey(Zeroizing::new(key)))
                    }
                    SecretScheme::Absent => Err(TaskError::IdentityKeyMissing),
                }
            }
        }
    }

    /// Borrow the secret store as a [`SecretSeam`].
    fn seam(&self) -> SecretSeam<'_> {
        SecretSeam::new(&self.inner.secret_store)
    }

    /// LAZY-re-wrap a just-decrypted protected single key to a Tier-2 envelope
    /// under the same label and object `password` (the upsert replaces the
    /// legacy AES-GCM framing), KEEPING protection. Best-effort: a vault-write
    /// failure is logged and the key keeps working via the legacy reader.
    ///
    /// `has_passphrase` is deliberately NOT flipped — the secret stays protected,
    /// so the in-memory index and the persisted flag remain accurate (the next
    /// resolve still prompts for the object password).
    fn migrate_single_key_to_tier2(
        &self,
        address: &str,
        raw: &[u8; SINGLE_KEY_LEN],
        password: &SecretString,
    ) {
        let label = label_for_address(address);
        if let Err(e) = self.seam().put_secret_protected(
            &single_key_namespace_id(),
            &label,
            &platform_wallet_storage::secrets::SecretBytes::from_slice(raw),
            password,
        ) {
            tracing::warn!(
                target = "wallet_backend::secret_access",
                error = ?e,
                "Single-key lazy Tier-2 re-wrap deferred (vault write failed)",
            );
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
            // Opted-in (Tier-2) identity keys DO prompt; read the display copy
            // from the identity prompt-index (alias + password hint). A missing
            // entry degrades to a generic label, never an error.
            SecretScope::IdentityKey { identity_id, .. } => {
                let meta = self
                    .inner
                    .identity_prompt_index
                    .read()
                    .ok()
                    .and_then(|g| g.get(identity_id).cloned());
                let label = meta
                    .as_ref()
                    .and_then(|m| m.alias.clone())
                    .unwrap_or_else(|| "this identity".to_string());
                let hint = meta.and_then(|m| m.password_hint);
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
    let plaintext = decrypt_message(
        &envelope.encrypted_seed,
        &envelope.salt,
        &envelope.nonce,
        HD_SEED_LEN,
        passphrase.expose_secret(),
        "secret_access::decrypt_hd_seed",
    )
    .map_err(|e| match e {
        DecryptError::WrongPassword => TaskError::HdPassphraseIncorrect,
        DecryptError::Malformed => TaskError::SecretDecryptFailed,
    })?;
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

/// Convert raw vault bytes into a 32-byte identity private key, mapping a
/// wrong-length blob to the typed [`TaskError::IdentityKeyMalformed`] (vault
/// corruption / truncated write) rather than a panic or a generic decrypt
/// error. Shared by the Tier-1 and Tier-2 identity-key decrypt arms.
pub(crate) fn identity_key_from_bytes(bytes: &[u8]) -> Result<[u8; SINGLE_KEY_LEN], TaskError> {
    bytes.try_into().map_err(|_| {
        tracing::warn!(
            target = "wallet_backend::secret_access",
            blob_len = bytes.len(),
            "Stored identity key has wrong length",
        );
        TaskError::IdentityKeyMalformed
    })
}

fn handle_lazy_tier2_rewrap_result(result: Result<(), TaskError>) -> Result<(), TaskError> {
    match result {
        Err(TaskError::SecretSeam { source })
            if matches!(source.as_ref(), SecretStoreError::BlankPassphrase) =>
        {
            tracing::warn!(
                target = "wallet_backend::secret_access",
                error = ?source,
                "HD seed lazy Tier-2 re-wrap deferred because the legacy password is below the storage floor",
            );
            Ok(())
        }
        other => other,
    }
}

/// Whether `e` is the "wrong passphrase" condition that the re-ask loop
/// catches and re-prompts on (rather than aborting).
fn is_wrong_passphrase(e: &TaskError) -> bool {
    match e {
        TaskError::SingleKeyPassphraseIncorrect
        | TaskError::HdPassphraseIncorrect
        | TaskError::IdentityKeyPassphraseIncorrect => true,
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
        let crate::model::wallet::encryption::EncryptedEnvelope {
            ciphertext: encrypted_seed,
            salt,
            nonce,
        } = encrypt_message(seed, passphrase).expect("encrypt seed");
        let envelope = StoredSeedEnvelope {
            encrypted_seed: Zeroizing::new(encrypted_seed),
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
            encrypted_seed: Zeroizing::new(seed.to_vec()),
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
            encrypted_seed: Zeroizing::new(vec![0u8; 80]),
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

    /// Write a legacy DET AES-GCM `SingleKeyEntry` straight to the vault Tier-1 —
    /// the pre-Tier-2 protected-import shape. Fresh imports now seal Tier-2 at
    /// import time, so this is how the legacy→Tier-2 lazy migration path stays
    /// covered.
    fn write_legacy_protected_key(store: &Arc<SecretStore>, passphrase: &str) -> String {
        use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
        use dash_sdk::dpp::dashcore::{Address, PrivateKey, PublicKey};

        let priv_key = PrivateKey::from_wif(&known_testnet_wif()).expect("wif");
        let raw: Zeroizing<[u8; 32]> =
            Zeroizing::new(priv_key.inner[..].try_into().expect("32 bytes"));
        let secp = Secp256k1::new();
        let pub_key = PublicKey {
            compressed: priv_key.compressed,
            inner: priv_key.inner.public_key(&secp),
        };
        let address = Address::p2pkh(&pub_key, Network::Testnet).to_string();
        let pub_bytes = pub_key.inner.serialize().to_vec();
        let entry =
            SingleKeyEntry::protected(&raw, passphrase, Some("the usual".into()), pub_bytes)
                .expect("build legacy protected entry");
        let payload = entry.encode().expect("encode legacy entry");
        store
            .set(
                &single_key_namespace_id(),
                &label_for_address(&address),
                &SecretBytes::from_slice(&payload),
            )
            .expect("write legacy vault entry");
        address
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

    /// TS-LAZY-03 (Tier-2) — a *legacy* protected single key lazy RE-WRAPS
    /// through the chokepoint, KEEPING protection: the first `with_secret`
    /// decrypts with the passphrase AND re-stores a Tier-2 object-password
    /// envelope (not a raw secret); a second `with_secret` therefore still
    /// requires the password. Starts from a legacy AES-GCM entry so the
    /// migration path is genuinely exercised (fresh imports already seal Tier-2).
    #[tokio::test]
    async fn ts_lazy_03_protected_single_key_rewraps_to_tier2_via_chokepoint() {
        use dash_sdk::dpp::dashcore::PrivateKey;

        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let address = write_legacy_protected_key(&store, SENTINEL_PASSPHRASE);
        let expected: [u8; 32] = PrivateKey::from_wif(&known_testnet_wif()).unwrap().inner[..]
            .try_into()
            .unwrap();

        // First resolve: one passphrase, re-wraps to Tier-2.
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

        // The vault now holds a Tier-2 envelope (kept protected) — a password-
        // free read fails, and the password read returns the 32 key bytes.
        let label = label_for_address(&address);
        assert_eq!(
            SecretSeam::new(&store)
                .scheme(&single_key_namespace_id(), &label)
                .unwrap(),
            SecretScheme::Protected,
            "the single key must re-wrap to Tier-2, never downgrade to raw"
        );
        assert!(
            store.get(&single_key_namespace_id(), &label).is_err(),
            "a password-free read of a protected single key must fail"
        );
        let pw = SecretString::new(SENTINEL_PASSPHRASE);
        let unsealed = store
            .get_secret(&single_key_namespace_id(), &label, Some(&pw))
            .unwrap()
            .unwrap();
        assert_eq!(unsealed.expose_secret(), &expected[..]);

        // Second resolve still requires the object password (Tier-2, not raw).
        let prompt2 = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa2 = access(Arc::clone(&store), prompt2.clone());
        let second = sa2
            .with_secret(&scope, |pt| Ok(pt.expose_single_key().copied()))
            .await
            .expect("resolve with the password");
        assert_eq!(second, Some(expected));
        assert_eq!(prompt2.ask_count(), 1, "protected single key prompts again");
    }

    /// TS-T2-SK-ISO — PER-SECRET isolation for imported single keys: two Tier-2
    /// keys under DIFFERENT passwords. A's password cannot open B (the negative
    /// crypto property), and remembering A never satisfies B (scope-keyed cache).
    #[tokio::test]
    async fn ts_t2_sk_iso_per_secret_passwords_are_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let addr_a = "single-key-address-A".to_string();
        let addr_b = "single-key-address-B".to_string();
        let key_a = [0xA7u8; 32];
        let key_b = [0xB8u8; 32];
        let pw_a = SecretString::new("single-key-A-pwpwpwpw");
        let pw_b = SecretString::new("single-key-B-pwpwpwpw");
        let seam = SecretSeam::new(&store);
        seam.put_secret_protected(
            &single_key_namespace_id(),
            &label_for_address(&addr_a),
            &SecretBytes::from_slice(&key_a),
            &pw_a,
        )
        .unwrap();
        seam.put_secret_protected(
            &single_key_namespace_id(),
            &label_for_address(&addr_b),
            &SecretBytes::from_slice(&key_b),
            &pw_b,
        )
        .unwrap();

        // Negative crypto property: A's password is REJECTED by B's envelope.
        match seam.get_secret_protected(
            &single_key_namespace_id(),
            &label_for_address(&addr_b),
            &pw_a,
        ) {
            Err(TaskError::SecretSeam { source })
                if matches!(*source, SecretStoreError::WrongPassword) => {}
            other => panic!("A's password must be rejected by B, got {other:?}"),
        }

        // Scope-keyed cache: remembering A does not satisfy B — B still prompts.
        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::remember("single-key-A-pwpwpwpw", RememberPolicy::UntilAppClose),
            ScriptedAnswer::remember("single-key-B-pwpwpwpw", RememberPolicy::UntilAppClose),
        ]));
        let sa = access(Arc::clone(&store), prompt.clone());
        let scope_a = SecretScope::SingleKey {
            address: addr_a.clone(),
        };
        let scope_b = SecretScope::SingleKey {
            address: addr_b.clone(),
        };

        sa.with_secret(&scope_a, |pt| {
            assert_eq!(pt.expose_single_key().copied(), Some(key_a));
            Ok(())
        })
        .await
        .unwrap();
        assert!(sa.is_session_cached(&scope_a));
        assert!(
            !sa.is_session_cached(&scope_b),
            "A's unlock must not cache B"
        );

        sa.with_secret(&scope_b, |pt| {
            assert_eq!(pt.expose_single_key().copied(), Some(key_b));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(prompt.ask_count(), 2, "B prompted independently of A");
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

    /// Seal a raw identity key Tier-2 under `password`, the way the opt-in
    /// migration does (in-place upsert at the SAME label as the Tier-1 value).
    fn store_identity_key_protected(
        store: &Arc<SecretStore>,
        identity_id: [u8; 32],
        target: &PrivateKeyTarget,
        key_id: u32,
        key: &[u8; 32],
        password: &str,
    ) {
        let label = SecretScope::identity_key_label(target, key_id);
        SecretSeam::new(store)
            .put_secret_protected(
                &SecretWalletId::from(identity_id),
                &label,
                &SecretBytes::from_slice(key),
                &SecretString::new(password),
            )
            .expect("seal identity key tier-2");
    }

    /// Opt-in seal: a Tier-2 identity key reports `Protected`
    /// (scheme-as-flag), a password-free read fails, the chokepoint prompts
    /// exactly once, decrypts the exact 32 bytes, and `can_resolve_without_prompt`
    /// is false (the background sweep skips a locked protected identity).
    #[tokio::test]
    async fn ts_t2_ik_01_protected_identity_key_prompts_and_decrypts() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let identity_id = [0x51u8; 32];
        let key = [0xD4u8; 32];
        store_identity_key_protected(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            4,
            &key,
            SENTINEL_PASSPHRASE,
        );

        // Scheme-as-flag: Protected, and a password-free read fails.
        let label = SecretScope::identity_key_label(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 4);
        assert_eq!(
            SecretSeam::new(&store)
                .scheme(&SecretWalletId::from(identity_id), &label)
                .unwrap(),
            SecretScheme::Protected,
            "opt-in seals the identity key Tier-2"
        );
        assert!(
            store
                .get(&SecretWalletId::from(identity_id), &label)
                .is_err(),
            "a password-free read of a protected identity key must fail"
        );

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = access(store, prompt.clone());
        let scope = SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 4,
        };
        assert!(
            !sa.can_resolve_without_prompt(&scope),
            "a locked protected identity key would prompt — the sweep must skip it"
        );
        let matched = sa
            .with_secret(&scope, |pt| {
                Ok(pt.expose_identity_key().copied() == Some(key))
            })
            .await
            .expect("protected identity key resolves with the password");
        assert!(matched, "closure saw the unsealed identity key");
        assert_eq!(prompt.ask_count(), 1, "exactly one prompt");
    }

    /// A Tier-2 identity key re-asks on a wrong password (no oracle) and then
    /// succeeds — the same re-ask UX as protected seeds and single keys.
    #[tokio::test]
    async fn ts_t2_ik_02_protected_identity_key_wrong_password_reasks() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let identity_id = [0x52u8; 32];
        let key = [0xE5u8; 32];
        store_identity_key_protected(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnVoterIdentity,
            2,
            &key,
            SENTINEL_PASSPHRASE,
        );

        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::once("not-the-password"),
            ScriptedAnswer::once(SENTINEL_PASSPHRASE),
        ]));
        let sa = access(store, prompt.clone());
        let scope = SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnVoterIdentity,
            key_id: 2,
        };
        let matched = sa
            .with_secret(&scope, |pt| {
                Ok(pt.expose_identity_key().copied() == Some(key))
            })
            .await
            .expect("retry succeeds");
        assert!(matched);
        assert_eq!(prompt.ask_count(), 2, "one wrong-pass re-ask, then success");
    }

    /// Headless (NullSecretPrompt): an OPTED-IN identity key has no window to
    /// ask in, so the chokepoint surfaces the typed `SecretPromptUnavailable`
    /// — the accepted trade-off. A non-opted-in identity key (default keyless)
    /// still resolves headless (covered by TS-FAST-01).
    #[tokio::test]
    async fn ts_t2_ik_03_headless_protected_identity_key_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let identity_id = [0x53u8; 32];
        store_identity_key_protected(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0xF6u8; 32],
            SENTINEL_PASSPHRASE,
        );

        let sa = access(store, Arc::new(NullSecretPrompt));
        let scope = SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 0,
        };
        let err = sa
            .with_secret(&scope, |_pt| Ok(()))
            .await
            .expect_err("no interactive prompt headless");
        assert!(
            matches!(err, TaskError::SecretPromptUnavailable),
            "expected SecretPromptUnavailable, got {err:?}"
        );
    }

    /// TS-T2-IK-ISO — PER-IDENTITY password isolation. Two identities sealed
    /// under DIFFERENT passwords: A's password is rejected by B's envelope
    /// (the negative crypto property), and remembering A never satisfies B
    /// (scope-keyed cache).
    #[tokio::test]
    async fn ts_t2_ik_iso_per_identity_passwords_are_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let id_a = [0xA1u8; 32];
        let id_b = [0xB2u8; 32];
        let key_a = [0x1Au8; 32];
        let key_b = [0x2Bu8; 32];
        store_identity_key_protected(
            &store,
            id_a,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &key_a,
            "identity-A-passwordpw",
        );
        store_identity_key_protected(
            &store,
            id_b,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &key_b,
            "identity-B-passwordpw",
        );

        // Negative crypto property: A's password is REJECTED by B's envelope.
        let label = SecretScope::identity_key_label(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0);
        match SecretSeam::new(&store).get_secret_protected(
            &SecretWalletId::from(id_b),
            &label,
            &SecretString::new("identity-A-passwordpw"),
        ) {
            Err(TaskError::SecretSeam { source })
                if matches!(*source, SecretStoreError::WrongPassword) => {}
            other => panic!("A's password must be rejected by B, got {other:?}"),
        }

        // Scope-keyed cache: remembering A does not satisfy B — B still prompts.
        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::remember("identity-A-passwordpw", RememberPolicy::UntilAppClose),
            ScriptedAnswer::remember("identity-B-passwordpw", RememberPolicy::UntilAppClose),
        ]));
        let sa = access(Arc::clone(&store), prompt.clone());
        let scope_a = SecretScope::IdentityKey {
            identity_id: id_a,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 0,
        };
        let scope_b = SecretScope::IdentityKey {
            identity_id: id_b,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 0,
        };

        sa.with_secret(&scope_a, |pt| {
            assert_eq!(pt.expose_identity_key().copied(), Some(key_a));
            Ok(())
        })
        .await
        .unwrap();
        assert!(sa.is_session_cached(&scope_a));
        assert!(
            !sa.is_session_cached(&scope_b),
            "A's unlock must not cache B"
        );

        sa.with_secret(&scope_b, |pt| {
            assert_eq!(pt.expose_identity_key().copied(), Some(key_b));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(prompt.ask_count(), 2, "B prompted independently of A");
    }

    /// The sign-time prompt for a protected identity carries the alias and
    /// password hint from the identity prompt-index (display-only). An empty
    /// index degrades to a generic label, never an error.
    #[tokio::test]
    async fn protected_identity_key_prompt_uses_identity_prompt_index() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let identity_id = [0x54u8; 32];
        store_identity_key_protected(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            1,
            &[0x77u8; 32],
            SENTINEL_PASSPHRASE,
        );

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = access(store, prompt.clone());
        sa.set_identity_prompt_index(BTreeMap::from([(
            identity_id,
            PromptMeta {
                alias: Some("alice.dash".to_string()),
                password_hint: Some("the usual".to_string()),
            },
        )]));
        let scope = SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 1,
        };
        sa.with_secret(&scope, |_pt| Ok(())).await.unwrap();
        let req = &prompt.requests()[0];
        assert_eq!(req.display_label, "alice.dash", "prompt shows the alias");
        assert_eq!(req.hint.as_deref(), Some("the usual"), "prompt shows hint");
    }

    /// A NEW key added to a protected identity is sealed
    /// Tier-2 under the identity's verified password — never written keyless.
    /// After the seal the new key reports `Protected`, a password-free read
    /// fails, and it unseals to the exact bytes under the same password.
    #[tokio::test]
    async fn seal_new_identity_key_seals_tier2_under_verified_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let identity_id = [0x61u8; 32];
        // An existing protected key of the identity (the verify anchor).
        store_identity_key_protected(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0x10u8; 32],
            SENTINEL_PASSPHRASE,
        );
        let new_key = [0x20u8; 32];

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = access(Arc::clone(&store), prompt.clone());
        let verify = SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 0,
        };
        sa.seal_new_identity_key(
            identity_id,
            &verify,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            5,
            &new_key,
        )
        .await
        .expect("seal new key under the verified password");
        assert_eq!(prompt.ask_count(), 1, "one prompt to verify + seal");

        let new_label =
            SecretScope::identity_key_label(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 5);
        let seam = SecretSeam::new(&store);
        assert_eq!(
            seam.scheme(&SecretWalletId::from(identity_id), &new_label)
                .unwrap(),
            SecretScheme::Protected,
            "the new key is sealed Tier-2, never keyless",
        );
        assert!(
            store
                .get(&SecretWalletId::from(identity_id), &new_label)
                .is_err(),
            "a password-free read of the new key must fail",
        );
        let unsealed = seam
            .get_secret_protected(
                &SecretWalletId::from(identity_id),
                &new_label,
                &SecretString::new(SENTINEL_PASSPHRASE),
            )
            .unwrap()
            .unwrap();
        assert_eq!(unsealed.expose_secret(), &new_key[..]);
    }

    /// Headless: sealing a new key onto a protected identity fails closed
    /// (`SecretPromptUnavailable`) and writes NOTHING — no keyless key lands.
    #[tokio::test]
    async fn seal_new_identity_key_headless_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let identity_id = [0x62u8; 32];
        store_identity_key_protected(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0x11u8; 32],
            SENTINEL_PASSPHRASE,
        );

        let sa = access(Arc::clone(&store), Arc::new(NullSecretPrompt));
        let verify = SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 0,
        };
        let err = sa
            .seal_new_identity_key(
                identity_id,
                &verify,
                &PrivateKeyTarget::PrivateKeyOnMainIdentity,
                5,
                &[0x20u8; 32],
            )
            .await
            .expect_err("headless cannot seal");
        assert!(
            matches!(err, TaskError::SecretPromptUnavailable),
            "expected SecretPromptUnavailable, got {err:?}"
        );
        // Nothing was written for the new key — no keyless leak.
        let new_label =
            SecretScope::identity_key_label(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 5);
        assert_eq!(
            SecretSeam::new(&store)
                .scheme(&SecretWalletId::from(identity_id), &new_label)
                .unwrap(),
            SecretScheme::Absent,
            "a failed headless seal must leave no key at all",
        );
    }

    /// A wrong password re-asks (verifying against the existing protected key),
    /// then seals the new key on the correct password.
    #[tokio::test]
    async fn seal_new_identity_key_wrong_password_reasks() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let identity_id = [0x63u8; 32];
        store_identity_key_protected(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0x12u8; 32],
            SENTINEL_PASSPHRASE,
        );

        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::once("not-the-password"),
            ScriptedAnswer::once(SENTINEL_PASSPHRASE),
        ]));
        let sa = access(Arc::clone(&store), prompt.clone());
        let verify = SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 0,
        };
        sa.seal_new_identity_key(
            identity_id,
            &verify,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            5,
            &[0x20u8; 32],
        )
        .await
        .expect("retry then seal");
        assert_eq!(prompt.ask_count(), 2, "one wrong-pass re-ask, then success");
        assert_eq!(
            SecretSeam::new(&store)
                .scheme(
                    &SecretWalletId::from(identity_id),
                    &SecretScope::identity_key_label(
                        &PrivateKeyTarget::PrivateKeyOnMainIdentity,
                        5
                    )
                )
                .unwrap(),
            SecretScheme::Protected,
        );
    }

    /// O-2: the add-key flow verifies the password UP FRONT
    /// ([`SecretAccess::verify_identity_object_password`]) and seals AFTER its
    /// broadcast ([`SecretAccess::seal_new_identity_key_with_password`]). The
    /// split prompts EXACTLY ONCE total and seals the new key Tier-2 — the same
    /// outcome as the combined `seal_new_identity_key`, with the verify and seal
    /// halves usable around an intervening on-chain broadcast.
    #[tokio::test]
    async fn verify_up_front_then_seal_after_broadcast_one_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let identity_id = [0x64u8; 32];
        store_identity_key_protected(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0x13u8; 32],
            SENTINEL_PASSPHRASE,
        );
        let new_key = [0x21u8; 32];

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(SENTINEL_PASSPHRASE)]));
        let sa = access(Arc::clone(&store), prompt.clone());
        let verify = SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 0,
        };

        // Front half: the precondition the add-key flow runs BEFORE broadcast.
        let password = sa
            .verify_identity_object_password(&verify)
            .await
            .expect("verify the object password up front");
        assert_eq!(prompt.ask_count(), 1, "one prompt at the precondition");

        // (broadcast would happen here) — back half: seal AFTER, no re-prompt.
        sa.seal_new_identity_key_with_password(
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            5,
            &new_key,
            &password,
        )
        .expect("seal the new key with the verified password");
        assert_eq!(prompt.ask_count(), 1, "sealing did not prompt again");

        let new_label =
            SecretScope::identity_key_label(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 5);
        let seam = SecretSeam::new(&store);
        assert_eq!(
            seam.scheme(&SecretWalletId::from(identity_id), &new_label)
                .unwrap(),
            SecretScheme::Protected,
            "the new key is sealed Tier-2, never keyless",
        );
        let unsealed = seam
            .get_secret_protected(
                &SecretWalletId::from(identity_id),
                &new_label,
                &SecretString::new(SENTINEL_PASSPHRASE),
            )
            .unwrap()
            .unwrap();
        assert_eq!(unsealed.expose_secret(), &new_key[..]);
    }

    /// O-2 fail-closed: headless verification of a protected identity's
    /// password yields `SecretPromptUnavailable` and writes NOTHING. Because the
    /// add-key flow runs this BEFORE its broadcast, a headless add never reaches
    /// the on-chain state transition — no on-chain/local divergence.
    #[tokio::test]
    async fn verify_identity_object_password_headless_fails_closed_before_seal() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let identity_id = [0x65u8; 32];
        store_identity_key_protected(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0x14u8; 32],
            SENTINEL_PASSPHRASE,
        );

        let sa = access(Arc::clone(&store), Arc::new(NullSecretPrompt));
        let verify = SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id: 0,
        };
        let err = sa
            .verify_identity_object_password(&verify)
            .await
            .expect_err("headless cannot verify");
        assert!(
            matches!(err, TaskError::SecretPromptUnavailable),
            "expected SecretPromptUnavailable, got {err:?}"
        );
        // The precondition failed, so the seal half never runs: no key written.
        let new_label =
            SecretScope::identity_key_label(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 5);
        assert_eq!(
            SecretSeam::new(&store)
                .scheme(&SecretWalletId::from(identity_id), &new_label)
                .unwrap(),
            SecretScheme::Absent,
            "a failed precondition must leave no key at all",
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
    /// redundant envelope is removed, and the seed reads back only with its
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
        // Exactly one current protected copy remains.
        assert!(
            view.get(&seed_hash).unwrap().is_none(),
            "legacy envelope must be collected after the Tier-2 write"
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

    /// A legacy wallet may use a password shorter than the upstream Tier-2
    /// floor. Successful legacy decryption must still release the seed for the
    /// requested operation; the unsupported re-wrap is deferred and the
    /// legacy envelope remains the source of truth.
    #[tokio::test]
    async fn short_legacy_seed_password_remains_usable_without_tier2_migration() {
        const SHORT_LEGACY_PASSWORD: &str = "short";

        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let seed_hash: WalletSeedHash = [0x73; 32];
        store_protected_hd(&store, &seed_hash, &SENTINEL_SEED, SHORT_LEGACY_PASSWORD);

        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(
            SHORT_LEGACY_PASSWORD,
        )]));
        let sa = access(store.clone(), prompt.clone());
        let scope = SecretScope::HdSeed { seed_hash };

        sa.with_secret(&scope, |pt| {
            assert_eq!(pt.expose_hd_seed().copied(), Some(SENTINEL_SEED));
            Ok(())
        })
        .await
        .expect("legacy password must keep unlocking the seed");
        assert_eq!(prompt.ask_count(), 1);

        let view = WalletSeedView::new(&store);
        assert_eq!(
            view.scheme(&seed_hash).unwrap(),
            SecretScheme::Absent,
            "a sub-floor password must not be enrolled into Tier-2"
        );
        assert!(
            view.get(&seed_hash).unwrap().is_some(),
            "the legacy envelope must remain for the next unlock"
        );
    }

    #[test]
    fn lazy_tier2_rewrap_defers_blank_passphrase() {
        let result = handle_lazy_tier2_rewrap_result(Err(TaskError::SecretSeam {
            source: Box::new(SecretStoreError::BlankPassphrase),
        }));

        assert!(result.is_ok());
    }

    #[test]
    fn lazy_tier2_rewrap_propagates_non_blank_storage_error() {
        let result = handle_lazy_tier2_rewrap_result(Err(TaskError::SecretSeam {
            source: Box::new(SecretStoreError::Corruption),
        }));

        assert!(matches!(
            result,
            Err(TaskError::SecretSeam { source })
                if matches!(*source, SecretStoreError::Corruption)
        ));
    }

    #[test]
    fn lazy_tier2_rewrap_accepts_success() {
        assert!(handle_lazy_tier2_rewrap_result(Ok(())).is_ok());
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

        // Negative crypto property: A's password CANNOT open B's envelope.
        // Upstream binds the AEAD AAD to wallet_id‖label and derives a fresh
        // per-object key, so B's envelope rejects A's password with a tag
        // failure (`WrongPassword`) rather than yielding A's — or any — bytes.
        match view.get_protected(&hash_b, &pw_a) {
            Err(TaskError::SecretSeam { source })
                if matches!(*source, SecretStoreError::WrongPassword) => {}
            other => panic!("A's password must be REJECTED by B's envelope, got {other:?}"),
        }

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
