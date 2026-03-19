use objc2::msg_send;
use objc2::runtime::AnyClass;

/// Queries `accessibilityChildren` on the key window's content view to force
/// macOS to call the AccessKit adapter's subclassed method, which transitions
/// the adapter from `Inactive` to `Active`. Without this, tools like Peekaboo
/// that use `AXUIElement` queries won't trigger the lazy activation that
/// VoiceOver normally provides.
///
/// Returns `true` if activation was triggered, `false` if the window or view
/// was not yet available (caller should retry on the next frame).
pub fn force_accessibility_activation() -> bool {
    unsafe {
        let Some(cls) = AnyClass::get(c"NSApplication") else {
            return false;
        };
        let app: *mut objc2::runtime::AnyObject = msg_send![cls, sharedApplication];
        if app.is_null() {
            return false;
        }
        let mut window: *mut objc2::runtime::AnyObject = msg_send![app, keyWindow];
        if window.is_null() {
            // keyWindow is only set for the focused window; fall back to
            // mainWindow which is available even when another dialog has focus.
            window = msg_send![app, mainWindow];
            if window.is_null() {
                return false;
            }
        }
        let view: *mut objc2::runtime::AnyObject = msg_send![window, contentView];
        if view.is_null() {
            return false;
        }
        // Querying accessibilityChildren triggers the AccessKit adapter to
        // populate the accessibility tree for this view hierarchy.
        let _: *mut objc2::runtime::AnyObject = msg_send![view, accessibilityChildren];
        true
    }
}
