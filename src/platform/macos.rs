use objc2::MainThreadMarker;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};

/// Queries `accessibilityChildren` on the key window's content view to force
/// macOS to call the AccessKit adapter's subclassed method, which transitions
/// the adapter from `Inactive` to `Active`. Without this, tools like Peekaboo
/// that use `AXUIElement` queries won't trigger the lazy activation that
/// VoiceOver normally provides.
///
/// # Thread safety
///
/// This function calls AppKit APIs that must run on the main thread.
/// The `MainThreadMarker` parameter enforces this at compile time.
///
/// # Why raw `msg_send!` instead of `objc2-app-kit`
///
/// We intentionally use raw `msg_send!` here to avoid pulling in `objc2-app-kit`
/// (and its transitive dependencies) for just four well-known, stable AppKit
/// selectors: `sharedApplication`, `keyWindow`/`mainWindow`, `contentView`,
/// and `accessibilityChildren`. These selectors have been stable since macOS
/// 10.0+ and are unlikely to change.
///
/// Returns `true` if activation was triggered, `false` if the window or view
/// was not yet available (caller should retry on the next frame).
pub fn force_accessibility_activation(_mtm: MainThreadMarker) -> bool {
    // SAFETY: All msg_send! calls below target well-known AppKit selectors
    // on NSApplication, NSWindow, and NSView. The return types (nullable
    // object pointers) match the Objective-C declarations. Null checks
    // guard every intermediate result before use.
    unsafe {
        let Some(cls) = AnyClass::get(c"NSApplication") else {
            return false;
        };
        let app: *mut AnyObject = msg_send![cls, sharedApplication];
        if app.is_null() {
            return false;
        }
        let mut window: *mut AnyObject = msg_send![app, keyWindow];
        if window.is_null() {
            // keyWindow is only set for the focused window; fall back to
            // mainWindow which is available even when another dialog has focus.
            window = msg_send![app, mainWindow];
            if window.is_null() {
                return false;
            }
        }
        let view: *mut AnyObject = msg_send![window, contentView];
        if view.is_null() {
            return false;
        }
        // Querying accessibilityChildren triggers the AccessKit adapter to
        // populate the accessibility tree for this view hierarchy.
        let _: *mut AnyObject = msg_send![view, accessibilityChildren];
        true
    }
}
