#[cfg(target_os = "macos")]
mod macos;

/// Force-activate the platform accessibility subsystem so that the
/// accessibility tree is populated even without VoiceOver running.
///
/// On macOS this queries the NSView's `accessibilityChildren` to trigger
/// AccessKit's lazy adapter. On other platforms this is a no-op.
///
/// Must be called from the main/UI thread (enforced at compile time on macOS
/// via `MainThreadMarker`).
///
/// Returns `true` if activation was triggered successfully, `false` if the
/// window was not yet available (caller should retry on the next frame).
pub fn force_accessibility_activation() -> bool {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: This is called from eframe's App::update(), which always
        // runs on the main thread. MainThreadMarker::new() will panic in
        // debug builds if this invariant is ever violated.
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            tracing::error!("force_accessibility_activation called from non-main thread");
            return false;
        };
        macos::force_accessibility_activation(mtm)
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Orders all windows out before shutdown so macOS can clean up
/// display-related observations. No-op on non-macOS platforms.
pub fn order_out_all_windows() {
    #[cfg(target_os = "macos")]
    {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            tracing::error!("order_out_all_windows called from non-main thread");
            return;
        };
        macos::order_out_all_windows(mtm);
    }
}
