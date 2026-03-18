#[cfg(target_os = "macos")]
mod macos;

/// Force-activate the platform accessibility subsystem so that the
/// accessibility tree is populated even without VoiceOver running.
///
/// On macOS this queries the NSView's `accessibilityChildren` to trigger
/// AccessKit's lazy adapter. On other platforms this is a no-op.
///
/// Returns `true` if activation was triggered successfully, `false` if the
/// window was not yet available (caller should retry on the next frame).
pub fn force_accessibility_activation() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::force_accessibility_activation()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
