use eframe::egui;
use std::sync::mpsc as std_mpsc;
use tokio::sync::mpsc;

/// A wrapper around tokio::sync::mpsc::Sender that optionally triggers egui
/// repaints every time a message is sent.
///
/// When used with `new()`, an `egui::Context` is provided and repaints are
/// triggered on every send (the traditional egui behavior). When used with
/// `new_headless()`, no egui context is needed — suitable for Tauri or other
/// non-egui consumers of the backend task system.
pub struct SenderAsync<T> {
    sender: mpsc::Sender<T>,
    ctx: Option<egui::Context>,
}

impl<T> SenderAsync<T> {
    /// Create a new SenderAsync wrapper that triggers egui repaints on send.
    pub fn new(sender: mpsc::Sender<T>, ctx: egui::Context) -> Self {
        Self {
            sender,
            ctx: Some(ctx),
        }
    }

    /// Create a headless SenderAsync without egui repaint integration.
    ///
    /// Used by the Tauri backend where egui is not available. Messages are
    /// still delivered through the channel; the caller is responsible for
    /// any UI update signaling (e.g. via Tauri events).
    pub fn new_headless(sender: mpsc::Sender<T>) -> Self {
        Self { sender, ctx: None }
    }

    /// Send a message and trigger a repaint (if an egui context is present).
    pub async fn send(&self, value: T) -> Result<(), mpsc::error::SendError<T>> {
        let result = self.sender.send(value).await;
        if result.is_ok()
            && let Some(ctx) = &self.ctx
        {
            ctx.request_repaint();
        }
        result
    }

    /// Try to send a message without blocking and trigger a repaint (if an
    /// egui context is present).
    pub fn try_send(&self, value: T) -> Result<(), mpsc::error::TrySendError<T>> {
        let result = self.sender.try_send(value);
        if result.is_ok()
            && let Some(ctx) = &self.ctx
        {
            ctx.request_repaint();
        }
        result
    }

    /// Get a reference to the underlying sender
    pub fn sender(&self) -> &mpsc::Sender<T> {
        &self.sender
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
        }
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
