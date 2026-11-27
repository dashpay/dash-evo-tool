use eframe::egui;
use std::sync::mpsc as std_mpsc;
use tokio::sync::mpsc;

/// A wrapper around tokio::sync::mpsc::Sender that triggers egui repaints
/// every time a message is sent.
pub struct SenderAsync<T> {
    sender: mpsc::Sender<T>,
    ctx: egui::Context,
    /// The messages this sender passes on; `None` passes every one.
    keep: Option<fn(&T) -> bool>,
}

impl<T> SenderAsync<T> {
    /// Create a new SenderAsync wrapper
    pub fn new(sender: mpsc::Sender<T>, ctx: egui::Context) -> Self {
        Self {
            sender,
            ctx,
            keep: None,
        }
    }

    /// This sender, passing on only the messages `keep` accepts. The others
    /// are accepted and dropped, so a caller sees no failure.
    pub fn keeping_only(self, keep: fn(&T) -> bool) -> Self {
        Self {
            keep: Some(keep),
            ..self
        }
    }

    /// This sender without a filter: for a holder that keeps it for good,
    /// such as a context's frame loop or its wallet backend, which must pass
    /// every report whoever handed it the sender.
    pub fn unfiltered(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            ctx: self.ctx.clone(),
            keep: None,
        }
    }

    fn drops(&self, value: &T) -> bool {
        self.keep.is_some_and(|keep| !keep(value))
    }

    /// Send a message and trigger a repaint
    pub async fn send(&self, value: T) -> Result<(), mpsc::error::SendError<T>> {
        if self.drops(&value) {
            return Ok(());
        }
        let result = self.sender.send(value).await;
        if result.is_ok() {
            self.ctx.request_repaint();
        }
        result
    }

    /// Try to send a message without blocking and trigger a repaint
    pub fn try_send(&self, value: T) -> Result<(), mpsc::error::TrySendError<T>> {
        if self.drops(&value) {
            return Ok(());
        }
        let result = self.sender.try_send(value);
        if result.is_ok() {
            self.ctx.request_repaint();
        }
        result
    }

    /// Check if the sender is closed
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
}

impl<T> Clone for SenderAsync<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            ctx: self.ctx.clone(),
            keep: self.keep,
        }
    }
}

impl<T> std::fmt::Debug for SenderAsync<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SenderAsync")
            .field("closed", &self.sender.is_closed())
            .finish_non_exhaustive()
    }
}
pub trait EguiMpscAsync<T> {
    fn with_egui_ctx(self, ctx: egui::Context) -> (SenderAsync<T>, mpsc::Receiver<T>);
}

impl<T> EguiMpscAsync<T> for (mpsc::Sender<T>, mpsc::Receiver<T>) {
    fn with_egui_ctx(self, ctx: egui::Context) -> (SenderAsync<T>, mpsc::Receiver<T>) {
        let sender = SenderAsync::new(self.0, ctx);
        (sender, self.1)
    }
}

pub trait EguiMpscSync<T> {
    fn with_egui_ctx(self, ctx: egui::Context) -> (SenderSync<T>, std_mpsc::Receiver<T>);
}

impl<T> EguiMpscSync<T> for (std_mpsc::Sender<T>, std_mpsc::Receiver<T>) {
    fn with_egui_ctx(self, ctx: egui::Context) -> (SenderSync<T>, std_mpsc::Receiver<T>) {
        let sender = SenderSync::new(self.0, ctx);
        (sender, self.1)
    }
}

/// A wrapper around std::sync::mpsc::Sender that triggers egui repaints
/// every time a message is sent.
pub struct SenderSync<T> {
    sender: std_mpsc::Sender<T>,
    ctx: egui::Context,
}

impl<T> SenderSync<T> {
    /// Create a new SenderSync wrapper
    pub fn new(sender: std_mpsc::Sender<T>, ctx: egui::Context) -> Self {
        Self { sender, ctx }
    }

    /// Send a message and trigger a repaint
    pub fn send(&self, value: T) -> Result<(), std_mpsc::SendError<T>> {
        let result = self.sender.send(value);
        if result.is_ok() {
            self.ctx.request_repaint();
        }
        result
    }

    /// Get a reference to the underlying sender
    pub fn sender(&self) -> &std_mpsc::Sender<T> {
        &self.sender
    }
}

impl<T> Clone for SenderSync<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            ctx: self.ctx.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn even(value: &u32) -> bool {
        value.is_multiple_of(2)
    }

    /// A filtered sender drops what it does not keep, reporting success so
    /// the caller sees no failure, and passes on the rest; its clones filter
    /// alike, and its unfiltered copy passes everything.
    #[tokio::test]
    async fn a_filtered_sender_passes_only_what_it_keeps() {
        let (tx, mut rx) = mpsc::channel(8);
        let filtered = SenderAsync::new(tx, egui::Context::default()).keeping_only(even);

        assert!(filtered.send(1).await.is_ok());
        assert!(filtered.try_send(3).is_ok());
        filtered.send(2).await.expect("kept");
        let clone = filtered.clone();
        assert!(clone.try_send(5).is_ok());
        clone.try_send(4).expect("kept");
        filtered.unfiltered().try_send(7).expect("unfiltered");

        drop((filtered, clone));
        let mut received = Vec::new();
        while let Some(value) = rx.recv().await {
            received.push(value);
        }
        assert_eq!(received, vec![2, 4, 7]);
    }
}
