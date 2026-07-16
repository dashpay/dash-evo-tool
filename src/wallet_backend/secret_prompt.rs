//! The UI↔async boundary for just-in-time secret access.
//!
//! This module is the *only* seam the presentation layer implements to
//! participate in JIT secret resolution. It carries no egui types and no
//! upstream `key_wallet`/`platform_wallet` types — the lone exception is
//! [`SecretString`], a `platform_wallet_storage` zeroizing wrapper the
//! project already depends on (M-PLATFORM-WALLET-FIRST-PARTY) and reuses
//! deliberately rather than rolling a new secret type.
//!
//! Flow: a backend operation that needs plaintext builds a
//! [`SecretPromptRequest`] (scope + display copy, **no secret**) and awaits
//! [`SecretPrompt::request`]. The host (egui in the app, a test double in
//! tests) collects the user's passphrase and answers with a
//! [`SecretPromptReply`] (passphrase as `SecretString` + a
//! [`RememberPolicy`]). Dismissing the prompt resolves as
//! [`SecretPromptCancelled`].
//!
//! The reply carries the *passphrase* — a key-derivation input — never a
//! derived seed/key. The backend decrypts just-in-time inside
//! [`SecretAccess`](crate::wallet_backend::SecretAccess), so the channel
//! never transports key material and cancellation is a clean sender drop.

use std::time::Duration;

use async_trait::async_trait;
use dash_sdk::dpp::identity::KeyID;
use platform_wallet_storage::secrets::SecretString;

use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::wallet::WalletSeedHash;

/// Which secret an operation needs. DET-opaque: carries no upstream type
/// and no plaintext, only opaque-but-copyable handles (CLAUDE.md rule 6)
/// suitable for prompt copy. `Eq + Hash` so it keys the caches in
/// [`SecretAccess`](crate::wallet_backend::SecretAccess).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SecretScope {
    /// HD wallet seed, identified by DET's `SHA256(seed)` hash.
    HdSeed {
        /// 32-byte DET seed hash; reused directly as the upstream
        /// `WalletId` scope in the secret vault.
        seed_hash: WalletSeedHash,
    },
    /// Imported single key, identified by its P2PKH address.
    SingleKey {
        /// Base58 P2PKH address — the stable per-key identifier.
        address: String,
    },
    /// An identity private key stored raw in the vault, resolved per-use
    /// (the `InVault` placeholder). Unprotected — resolves prompt-free, so
    /// headless/MCP identity signing keeps working.
    IdentityKey {
        /// 32-byte identity id (`Identifier::to_buffer()`), the vault scope.
        identity_id: [u8; 32],
        /// Which associated identity the key belongs to.
        target: PrivateKeyTarget,
        /// The key's `KeyID` within the identity.
        key_id: KeyID,
    },
}

impl SecretScope {
    /// The vault label for an identity-key scope:
    /// `identity_key_priv.<target_tag>.<key_id>`. The target is a stable
    /// single-char tag so the label stays inside the upstream allowlist
    /// `^[A-Za-z0-9._-]{1,64}$`.
    pub fn identity_key_label(target: &PrivateKeyTarget, key_id: KeyID) -> String {
        format!("identity_key_priv.{}.{key_id}", target_tag(target))
    }
}

/// Stable one-char tag for a [`PrivateKeyTarget`] used in vault labels.
fn target_tag(target: &PrivateKeyTarget) -> char {
    match target {
        PrivateKeyTarget::PrivateKeyOnMainIdentity => 'm',
        PrivateKeyTarget::PrivateKeyOnVoterIdentity => 'v',
        PrivateKeyTarget::PrivateKeyOnOperatorIdentity => 'o',
    }
}

/// How long a decrypted secret may be remembered after the operation that
/// obtained it completes.
///
/// The default is [`RememberPolicy::None`] — operation-scoped residency,
/// the secret zeroizes when the operation ends. A "remember" toggle in the
/// prompt promotes the secret to the session cache.
///
/// The GUI wires only `None` and `UntilAppClose`; [`RememberPolicy::For`]
/// is data-model-only for now (the session cache honors its TTL, but no UI
/// surface emits it yet).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RememberPolicy {
    /// Do not cache. The secret lives only for the operation that
    /// obtained it. This is the default residency.
    #[default]
    None,
    /// Cache for the rest of the process. Cleared on app close, network
    /// switch, and manual lock.
    UntilAppClose,
    /// Cache until the given duration elapses from the moment of insert.
    /// The session cache stamps an expiry and evicts on access past it.
    For(Duration),
}

/// Why the prompt is being re-shown. `None` on the first ask; set when the
/// chokepoint re-asks after a wrong passphrase so the host renders the
/// inline error without closing the modal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SecretPromptRetry {
    /// The previous passphrase did not decrypt the stored secret.
    WrongPassphrase,
}

/// A request for the user to supply a passphrase. Built by the backend,
/// answered by the host. Carries **no secret** — only the scope and the
/// copy the host needs to render an informative modal.
///
/// `#[non_exhaustive]` so future prompt-copy fields can be added without a
/// breaking change to every construction site.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SecretPromptRequest {
    /// Which secret is being requested.
    pub scope: SecretScope,
    /// Human label for the modal body — wallet alias, key nickname, or
    /// address. Already user-appropriate; the host shows it verbatim.
    pub display_label: String,
    /// Optional user-set hint (HD password hint or single-key hint).
    pub hint: Option<String>,
    /// Set on a re-ask after a wrong passphrase. `None` on first ask.
    pub retry_reason: Option<SecretPromptRetry>,
}

impl SecretPromptRequest {
    /// Build a first-ask request for `scope` with the given display copy.
    pub fn new(scope: SecretScope, display_label: impl Into<String>) -> Self {
        Self {
            scope,
            display_label: display_label.into(),
            hint: None,
            retry_reason: None,
        }
    }

    /// Attach a user-set hint shown next to the passphrase field.
    pub fn with_hint(mut self, hint: Option<String>) -> Self {
        self.hint = hint;
        self
    }

    /// Mark this request as a re-ask after a wrong passphrase.
    pub fn retrying(mut self, reason: SecretPromptRetry) -> Self {
        self.retry_reason = Some(reason);
        self
    }
}

/// The user's answer to a [`SecretPromptRequest`].
///
/// `passphrase` is a [`SecretString`] (zeroizing) — a key-derivation
/// *input*, never a derived key. `remember` carries the session-toggle
/// choice; default [`RememberPolicy::None`].
#[derive(Debug)]
pub struct SecretPromptReply {
    /// The passphrase the user typed. Decrypted just-in-time by the
    /// chokepoint and dropped (zeroized) immediately after.
    pub passphrase: SecretString,
    /// How long, if at all, to remember the decrypted secret.
    pub remember: RememberPolicy,
}

impl SecretPromptReply {
    /// Build a reply with the given passphrase and remember policy.
    pub fn new(passphrase: SecretString, remember: RememberPolicy) -> Self {
        Self {
            passphrase,
            remember,
        }
    }
}

/// The user dismissed the prompt, or no interactive host was available.
/// Carries no data — there is nothing to report beyond "no passphrase".
#[derive(Debug, thiserror::Error)]
#[error("the passphrase prompt was cancelled")]
pub struct SecretPromptCancelled;

/// The UI↔async seam: ask the user for the passphrase for a scope.
///
/// Implementations MUST NOT block the caller's executor; the egui host
/// enqueues the request, repaints, and awaits a one-shot reply. A test
/// double answers synchronously from a script. Resolving with
/// [`SecretPromptCancelled`] aborts the operation cleanly.
#[async_trait]
pub trait SecretPrompt: Send + Sync {
    /// Ask the user for the passphrase for `request.scope`. Resolves when
    /// the user submits (`Ok`) or dismisses (`Err`).
    async fn request(
        &self,
        request: SecretPromptRequest,
    ) -> Result<SecretPromptReply, SecretPromptCancelled>;

    /// Whether this host can actually ask a human. `true` for the egui host;
    /// `false` for [`NullSecretPrompt`] (headless MCP / CLI). The chokepoint
    /// uses this to distinguish a genuine user cancel from "no prompt exists
    /// here", surfacing the right typed error for each.
    fn is_interactive(&self) -> bool;
}

/// The [`SecretPrompt`] for non-interactive hosts (MCP server, CLI).
///
/// There is no window to ask for a passphrase, so every request resolves as
/// [`SecretPromptCancelled`] — the chokepoint maps that to a typed
/// [`TaskError::SecretPromptUnavailable`](crate::backend_task::error::TaskError::SecretPromptUnavailable)
/// for the caller. Per the Q-HEADLESS security ruling there is **no**
/// environment-variable or CLI-flag passphrase fallback: a passphrase-
/// protected secret simply cannot be unlocked headless. Unprotected scopes
/// never reach the prompt (the chokepoint's fast-path decrypts them with no
/// passphrase), so this host does not block read-only or no-password flows.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullSecretPrompt;

#[async_trait]
impl SecretPrompt for NullSecretPrompt {
    async fn request(
        &self,
        _request: SecretPromptRequest,
    ) -> Result<SecretPromptReply, SecretPromptCancelled> {
        Err(SecretPromptCancelled)
    }

    fn is_interactive(&self) -> bool {
        false
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A scripted [`SecretPrompt`] double for chokepoint unit tests.

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    /// One scripted answer the [`TestPrompt`] hands back on the next call.
    pub(crate) enum ScriptedAnswer {
        /// Reply with this passphrase and remember policy.
        Reply {
            /// Passphrase to hand back (plaintext for the script only).
            passphrase: String,
            /// Remember policy to attach to the reply.
            remember: RememberPolicy,
        },
        /// Dismiss the prompt (resolves [`SecretPromptCancelled`]).
        Cancel,
    }

    impl ScriptedAnswer {
        /// Convenience: an operation-scoped reply (no caching).
        pub(crate) fn once(passphrase: &str) -> Self {
            Self::Reply {
                passphrase: passphrase.to_string(),
                remember: RememberPolicy::None,
            }
        }

        /// Convenience: a reply promoted to the session cache.
        pub(crate) fn remember(passphrase: &str, remember: RememberPolicy) -> Self {
            Self::Reply {
                passphrase: passphrase.to_string(),
                remember,
            }
        }
    }

    /// Scripted [`SecretPrompt`] for unit tests. Pops answers FIFO and
    /// records every request it received so tests can assert prompt copy,
    /// retry reasons, and — crucially — that no prompt fired at all.
    pub(crate) struct TestPrompt {
        answers: Mutex<VecDeque<ScriptedAnswer>>,
        seen: Mutex<Vec<SecretPromptRequest>>,
    }

    impl TestPrompt {
        /// Build a prompt that will hand back `answers` in order.
        pub(crate) fn new(answers: impl IntoIterator<Item = ScriptedAnswer>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().collect()),
                seen: Mutex::new(Vec::new()),
            }
        }

        /// A prompt that fails the test if it is ever asked. Used to prove
        /// a session-cache hit or unprotected scope never prompts.
        pub(crate) fn never() -> Self {
            Self::new(std::iter::empty())
        }

        /// How many times the prompt was asked.
        pub(crate) fn ask_count(&self) -> usize {
            self.seen.lock().unwrap().len()
        }

        /// Snapshot of the requests the prompt received, in order.
        pub(crate) fn requests(&self) -> Vec<SecretPromptRequest> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SecretPrompt for TestPrompt {
        async fn request(
            &self,
            request: SecretPromptRequest,
        ) -> Result<SecretPromptReply, SecretPromptCancelled> {
            self.seen.lock().unwrap().push(request);
            let answer = self
                .answers
                .lock()
                .unwrap()
                .pop_front()
                .expect("TestPrompt asked more times than it was scripted for");
            match answer {
                ScriptedAnswer::Reply {
                    passphrase,
                    remember,
                } => Ok(SecretPromptReply::new(
                    SecretString::new(passphrase),
                    remember,
                )),
                ScriptedAnswer::Cancel => Err(SecretPromptCancelled),
            }
        }

        fn is_interactive(&self) -> bool {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{ScriptedAnswer, TestPrompt};
    use super::*;

    fn hd_scope() -> SecretScope {
        SecretScope::HdSeed {
            seed_hash: [0x11; 32],
        }
    }

    #[test]
    fn remember_policy_defaults_to_none() {
        assert_eq!(RememberPolicy::default(), RememberPolicy::None);
    }

    #[test]
    fn request_builder_starts_without_retry() {
        let req = SecretPromptRequest::new(hd_scope(), "My Wallet").with_hint(Some("hint".into()));
        assert!(req.retry_reason.is_none());
        assert_eq!(req.display_label, "My Wallet");
        assert_eq!(req.hint.as_deref(), Some("hint"));
    }

    #[test]
    fn request_builder_marks_retry() {
        let req = SecretPromptRequest::new(hd_scope(), "My Wallet")
            .retrying(SecretPromptRetry::WrongPassphrase);
        assert_eq!(req.retry_reason, Some(SecretPromptRetry::WrongPassphrase));
    }

    #[tokio::test]
    async fn test_prompt_hands_back_scripted_reply() {
        let prompt = TestPrompt::new([ScriptedAnswer::once("hunter2")]);
        let reply = prompt
            .request(SecretPromptRequest::new(hd_scope(), "My Wallet"))
            .await
            .expect("scripted reply");
        assert_eq!(reply.passphrase.expose_secret(), "hunter2");
        assert_eq!(reply.remember, RememberPolicy::None);
        assert_eq!(prompt.ask_count(), 1);
    }

    #[tokio::test]
    async fn test_prompt_cancel_resolves_cancelled() {
        let prompt = TestPrompt::new([ScriptedAnswer::Cancel]);
        let err = prompt
            .request(SecretPromptRequest::new(hd_scope(), "My Wallet"))
            .await
            .expect_err("cancel resolves Err");
        // Carries no data — the Display is generic and secret-free.
        assert_eq!(err.to_string(), "the passphrase prompt was cancelled");
    }

    #[test]
    fn cancelled_display_carries_no_secret() {
        let dbg = format!("{:?}", SecretPromptCancelled);
        assert_eq!(dbg, "SecretPromptCancelled");
    }

    #[tokio::test]
    async fn null_prompt_always_cancels_and_is_not_interactive() {
        let prompt = NullSecretPrompt;
        assert!(!prompt.is_interactive());
        let err = prompt
            .request(SecretPromptRequest::new(hd_scope(), "My Wallet"))
            .await
            .expect_err("null prompt cancels");
        assert_eq!(err.to_string(), "the passphrase prompt was cancelled");
    }
}
