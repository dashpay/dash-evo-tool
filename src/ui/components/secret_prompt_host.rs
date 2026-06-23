//! The egui implementation of the just-in-time secret prompt seam.
//!
//! [`EguiSecretPromptHost`] is the GUI's [`SecretPrompt`]. A backend operation
//! that needs a passphrase calls `request()` from a tokio task; the host
//! enqueues the request onto an mpsc channel, repaints the egui context (so
//! the modal surfaces even when the UI was idle), and awaits a one-shot reply.
//!
//! [`AppState`](crate::app::AppState) owns the receiving end: each frame it
//! drains one request into an [`ActivePrompt`], renders the shared
//! [`passphrase_modal`], and answers the one-shot on submit or cancel. Exactly
//! one prompt is active at a time; further requests wait in the channel (FIFO),
//! so two concurrent operations never stack modals.
//!
//! M-DONT-LEAK-TYPES: this host lives in the UI layer but speaks only the
//! `secret_prompt` seam — no `key_wallet`/`WalletId`/`SecretAccess` internals
//! cross into egui. The lone secret type on the wire is `SecretString`, the
//! `platform_wallet_storage` zeroizing wrapper the project already depends on.

use async_trait::async_trait;
use platform_wallet_storage::secrets::SecretString;
use tokio::sync::{mpsc, oneshot};

use crate::ui::components::passphrase_modal::{
    KEEP_UNLOCKED_LABEL, PassphraseModalConfig, PassphraseModalOutcome, passphrase_modal,
};
use crate::wallet_backend::secret_prompt::{
    RememberPolicy, SecretPrompt, SecretPromptCancelled, SecretPromptReply, SecretPromptRequest,
    SecretPromptRetry, SecretScope,
};

/// A request plus its reply channel, carried from the host to `AppState`.
type ReplySender = oneshot::Sender<Result<SecretPromptReply, SecretPromptCancelled>>;

/// One queued prompt: the request and the one-shot the host is awaiting.
pub struct QueuedPrompt {
    request: SecretPromptRequest,
    reply: ReplySender,
}

/// The GUI [`SecretPrompt`]. Clone is cheap (channel sender + egui context
/// handle are both `Arc`-backed), so it can be handed to every per-network
/// `SecretAccess`.
#[derive(Clone)]
pub struct EguiSecretPromptHost {
    queue: mpsc::UnboundedSender<QueuedPrompt>,
    egui_ctx: egui::Context,
}

impl EguiSecretPromptHost {
    /// Build a host paired with the receiver `AppState` drains each frame.
    pub fn new(egui_ctx: egui::Context) -> (Self, mpsc::UnboundedReceiver<QueuedPrompt>) {
        let (queue, receiver) = mpsc::unbounded_channel();
        (Self { queue, egui_ctx }, receiver)
    }
}

#[async_trait]
impl SecretPrompt for EguiSecretPromptHost {
    async fn request(
        &self,
        request: SecretPromptRequest,
    ) -> Result<SecretPromptReply, SecretPromptCancelled> {
        let (reply, reply_rx) = oneshot::channel();
        // If the UI side is gone (app shutting down), there is no one to ask:
        // treat it as a cancel rather than hanging.
        if self.queue.send(QueuedPrompt { request, reply }).is_err() {
            return Err(SecretPromptCancelled);
        }
        // Surface the modal immediately even if egui was idle.
        self.egui_ctx.request_repaint();
        // A dropped reply sender (modal dismissed, or AppState torn down)
        // resolves as a clean cancel.
        match reply_rx.await {
            Ok(result) => result,
            Err(_) => Err(SecretPromptCancelled),
        }
    }
}

/// The prompt `AppState` is currently rendering, holding the field state.
///
/// Drains one [`QueuedPrompt`] off the host channel and owns only the domain
/// state — the remember checkbox and the reply channel — until the user submits
/// or cancels. The [`PasswordInput`] and focus-tracking state are managed
/// internally by [`passphrase_modal`] in egui's data cache. On Submit or
/// Cancel the prompt answers the one-shot and is dropped.
pub struct ActivePrompt {
    request: SecretPromptRequest,
    reply: Option<ReplySender>,
    remember: bool,
}

impl ActivePrompt {
    /// Adopt a freshly-drained request as the active prompt.
    pub fn new(queued: QueuedPrompt) -> Self {
        Self {
            request: queued.request,
            reply: Some(queued.reply),
            remember: false,
        }
    }

    /// Build a stub active prompt for tests (RQ-1): no real secret, a reply
    /// channel whose receiver is dropped immediately. Lets a test put
    /// `AppState::active_secret_prompt` into the `Some` state to exercise the
    /// overlay input-claim gate. Compiled only under the `testing` feature.
    #[cfg(feature = "testing")]
    pub fn test_stub() -> Self {
        let (reply, _rx) = oneshot::channel();
        Self {
            request: SecretPromptRequest::new(
                crate::wallet_backend::secret_prompt::SecretScope::SingleKey {
                    address: "test".to_string(),
                },
                "Test prompt",
            ),
            reply: Some(reply),
            remember: false,
        }
    }

    /// Render the modal for this frame. Returns `true` when the prompt has
    /// resolved (the caller should drop it and drain the next request).
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let retry_error = self
            .request
            .retry_reason
            .map(|SecretPromptRetry::WrongPassphrase| "That passphrase is not correct. Try again.");

        // Identity-key prompts say "key", not "wallet" (Diziet D-2).
        let remember_label = match self.request.scope {
            SecretScope::IdentityKey { .. } => "Keep this key unlocked until I close the app.",
            _ => KEEP_UNLOCKED_LABEL,
        };

        let config = PassphraseModalConfig {
            window_title: "Unlock to continue",
            body: &self.request.display_label,
            hint: self.request.hint.as_deref(),
            error: retry_error,
            submit_label: "Unlock",
            input_placeholder: "Enter passphrase",
            remember_label: Some(remember_label),
        };

        let mut remember = self.remember;
        let outcome = passphrase_modal(ctx, &config, |ui| {
            ui.checkbox(
                &mut remember,
                config.remember_label.unwrap_or(KEEP_UNLOCKED_LABEL),
            );
        });
        self.remember = remember;

        match outcome {
            PassphraseModalOutcome::Pending => false,
            PassphraseModalOutcome::Submit(text) => {
                self.submit(&text);
                true
            }
            PassphraseModalOutcome::Cancel => {
                self.cancel();
                true
            }
        }
    }

    /// Send the typed reply.
    fn submit(&mut self, text: &str) {
        let passphrase = SecretString::new(text);
        let remember = if self.remember {
            RememberPolicy::UntilAppClose
        } else {
            RememberPolicy::None
        };
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Ok(SecretPromptReply::new(passphrase, remember)));
        }
    }

    /// Answer the one-shot with a cancel.
    fn cancel(&mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Err(SecretPromptCancelled));
        }
    }
}

impl Drop for ActivePrompt {
    fn drop(&mut self) {
        // If the prompt is dropped without an explicit decision (e.g. app
        // teardown), answer the awaiting `with_secret` with a clean cancel
        // rather than leaving it parked forever. Dropping the sender alone
        // would also resolve as cancel, but sending is explicit.
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Err(SecretPromptCancelled));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::secret_prompt::{SecretPrompt, SecretPromptRequest, SecretScope};

    fn hd_scope() -> SecretScope {
        SecretScope::HdSeed {
            seed_hash: [0x33; 32],
        }
    }

    /// The host enqueues a request and resolves with the reply the drain
    /// delivers — the UI↔async round-trip without a real egui frame.
    #[tokio::test]
    async fn request_round_trips_through_the_channel() {
        let (host, mut rx) = EguiSecretPromptHost::new(egui::Context::default());

        // Backend side: await the prompt.
        let handle = tokio::spawn(async move {
            host.request(SecretPromptRequest::new(hd_scope(), "My Wallet"))
                .await
        });

        // UI side: drain the request and answer the one-shot as the modal
        // would on submit.
        let queued = rx.recv().await.expect("request enqueued");
        assert_eq!(queued.request.display_label, "My Wallet");
        queued
            .reply
            .send(Ok(SecretPromptReply::new(
                SecretString::new("hunter2"),
                RememberPolicy::UntilAppClose,
            )))
            .expect("send reply");

        let reply = handle.await.expect("join").expect("not cancelled");
        assert_eq!(reply.passphrase.expose_secret(), "hunter2");
        assert_eq!(reply.remember, RememberPolicy::UntilAppClose);
    }

    /// Dropping the drained reply sender (modal dismissed / host torn down)
    /// resolves `request()` as a clean cancel.
    #[tokio::test]
    async fn dropping_the_reply_sender_cancels() {
        let (host, mut rx) = EguiSecretPromptHost::new(egui::Context::default());
        let handle = tokio::spawn(async move {
            host.request(SecretPromptRequest::new(hd_scope(), "My Wallet"))
                .await
        });

        let queued = rx.recv().await.expect("request enqueued");
        drop(queued); // user dismissed — sender dropped without a reply.

        let err = handle.await.expect("join").expect_err("cancelled");
        assert_eq!(err.to_string(), "the passphrase prompt was cancelled");
    }

    /// If the UI receiver is gone (app shutting down), `request()` does not
    /// hang — it cancels immediately.
    #[tokio::test]
    async fn request_cancels_when_ui_receiver_dropped() {
        let (host, rx) = EguiSecretPromptHost::new(egui::Context::default());
        drop(rx);
        let err = host
            .request(SecretPromptRequest::new(hd_scope(), "My Wallet"))
            .await
            .expect_err("no UI to ask");
        assert_eq!(err.to_string(), "the passphrase prompt was cancelled");
    }

    /// `ActivePrompt::submit` maps the unticked checkbox to `None` and a
    /// ticked one to `UntilAppClose` — the remember-policy mapping.
    #[tokio::test]
    async fn active_prompt_submit_maps_remember_policy() {
        // Unticked → None.
        let (tx, rx) = oneshot::channel();
        let mut prompt = ActivePrompt::new(QueuedPrompt {
            request: SecretPromptRequest::new(hd_scope(), "My Wallet"),
            reply: tx,
        });
        prompt.remember = false;
        prompt.submit("pw");
        let reply = rx.await.expect("reply").expect("not cancelled");
        assert_eq!(reply.remember, RememberPolicy::None);
        assert_eq!(reply.passphrase.expose_secret(), "pw");

        // Ticked → UntilAppClose.
        let (tx, rx) = oneshot::channel();
        let mut prompt = ActivePrompt::new(QueuedPrompt {
            request: SecretPromptRequest::new(hd_scope(), "My Wallet"),
            reply: tx,
        });
        prompt.remember = true;
        prompt.submit("pw");
        let reply = rx.await.expect("reply").expect("not cancelled");
        assert_eq!(reply.remember, RememberPolicy::UntilAppClose);
    }

    /// `ActivePrompt::cancel` answers the one-shot with a cancel.
    #[tokio::test]
    async fn active_prompt_cancel_resolves_cancelled() {
        let (tx, rx) = oneshot::channel();
        let mut prompt = ActivePrompt::new(QueuedPrompt {
            request: SecretPromptRequest::new(hd_scope(), "My Wallet"),
            reply: tx,
        });
        prompt.cancel();
        let err = rx.await.expect("reply").expect_err("cancelled");
        assert_eq!(err.to_string(), "the passphrase prompt was cancelled");
    }
}
