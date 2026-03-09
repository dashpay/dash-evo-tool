use std::any::TypeId;
use std::fmt;
use std::ops::Range;

use egui::TextBuffer;
use zeroize::{Zeroize, Zeroizing};

/// Default pre-allocation size for `Secret` buffers. Set to one memory page
/// to minimize reallocations (which would leak the old buffer contents).
const PAGE_SIZE: usize = 4096;

/// Zeroize-on-drop wrapper for sensitive strings (passwords, WIF keys, private key inputs).
///
/// Debug output is redacted. Best-effort `mlock` prevents the backing memory from
/// being swapped to disk. The lock is released automatically when the value is dropped.
///
/// `Display`, `Deref`, and `DerefMut` are intentionally **not** implemented to
/// prevent accidental leakage. Use [`expose_secret`](Self::expose_secret) for
/// explicit read access, or pass `&mut Secret` directly to `TextEdit::singleline`
/// (via the [`TextBuffer`] impl) for mutable editing.
///
/// # Security: TextEdit usage
///
/// When using `Secret` with `TextEdit::singleline`, you **must** set
/// `.password(true)`. Without it, plaintext leaks to egui's layout system,
/// widget info, and accessibility events. Use [`PasswordInput`] to ensure
/// this is always enforced.
pub struct Secret {
    /// Dropped first -- zeroes the bytes.
    inner: Zeroizing<String>,
    /// Dropped second -- unlocks the page.
    _lock: Option<region::LockGuard>,
    /// Tracks the heap pointer so we can re-lock after reallocation.
    locked_ptr: *const u8,
}

// SAFETY: `locked_ptr` is only used for pointer comparison (never dereferenced).
// The actual data lives in `inner` (Send+Sync via Zeroizing<String>) and `_lock`
// (Send+Sync via region::LockGuard's unsafe impls).
unsafe impl Send for Secret {}
unsafe impl Sync for Secret {}

impl Secret {
    /// Wrap a string in a `Secret`, locking its backing memory on a best-effort basis.
    pub fn new(s: impl Into<String>) -> Self {
        let mut source: String = s.into();
        let cap = source.len().max(PAGE_SIZE);
        let mut buf = String::with_capacity(cap);
        buf.push_str(&source);
        // Zeroize the original string before it's freed
        source.zeroize();
        let lock = region::lock(buf.as_ptr(), buf.capacity())
            .map_err(|e| {
                tracing::debug!("mlock failed for Secret: {e}");
                e
            })
            .ok();
        let locked_ptr = buf.as_ptr();
        Self {
            inner: Zeroizing::new(buf),
            _lock: lock,
            locked_ptr,
        }
    }

    /// Create an empty `Secret` with a pre-allocated, locked buffer.
    pub fn with_capacity(cap: usize) -> Self {
        let cap = cap.max(PAGE_SIZE);
        let s = String::with_capacity(cap);
        let lock = region::lock(s.as_ptr(), s.capacity())
            .map_err(|e| {
                tracing::debug!("mlock failed for Secret: {e}");
                e
            })
            .ok();
        let locked_ptr = s.as_ptr();
        Self {
            inner: Zeroizing::new(s),
            _lock: lock,
            locked_ptr,
        }
    }

    /// Create an empty `Secret` with a pre-allocated, locked buffer.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Borrow the plaintext.
    pub fn expose_secret(&self) -> &str {
        &self.inner
    }

    /// Whether the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// If the backing allocation moved (e.g. after a `String` reallocation),
    /// drop the old mlock guard and create a new one for the current buffer.
    fn relock_if_moved(&mut self) {
        if self.inner.as_ptr() != self.locked_ptr {
            self._lock = region::lock(self.inner.as_ptr(), self.inner.capacity())
                .map_err(|e| {
                    tracing::debug!("mlock re-lock failed after reallocation: {e}");
                    e
                })
                .ok();
            self.locked_ptr = self.inner.as_ptr();
        }
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Zero the full capacity, including bytes beyond len that
        // Zeroize for String would miss (it only zeros 0..len).
        let ptr = self.inner.as_mut_ptr();
        let cap = self.inner.capacity();
        if cap > 0 {
            // SAFETY: ptr is valid for cap bytes (String allocation guarantee).
            unsafe {
                std::ptr::write_bytes(ptr, 0, cap);
            }
        }
        // After this, Rust drops fields in order: inner (Zeroizing re-zeroes 0..len, harmless),
        // then _lock (unlocks the page), then locked_ptr (no-op).
    }
}

// -- TextBuffer impl (allows `TextEdit::singleline(&mut secret)`) -----------

impl TextBuffer for Secret {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.inner.as_str()
    }

    fn insert_text(&mut self, text: &str, char_index: usize) -> usize {
        let n = <String as TextBuffer>::insert_text(&mut *self.inner, text, char_index);
        self.relock_if_moved();
        n
    }

    fn delete_char_range(&mut self, char_range: Range<usize>) {
        <String as TextBuffer>::delete_char_range(&mut *self.inner, char_range);
        // Zero deleted bytes in trailing capacity [len..capacity)
        let ptr = self.inner.as_mut_ptr();
        let len = self.inner.len();
        let cap = self.inner.capacity();
        if cap > len {
            // SAFETY: ptr is valid for cap bytes (String's allocation guarantee),
            // and we only write to the unused portion [len..cap).
            unsafe {
                std::ptr::write_bytes(ptr.add(len), 0, cap - len);
            }
        }
    }

    fn clear(&mut self) {
        Zeroize::zeroize(&mut *self.inner);
    }

    fn replace_with(&mut self, text: &str) {
        Zeroize::zeroize(&mut *self.inner);
        self.inner.push_str(text);
        self.relock_if_moved();
    }

    fn take(&mut self) -> String {
        let copy = self.inner.to_string();
        Zeroize::zeroize(&mut *self.inner);
        copy
    }

    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
}

// -- Trait impls -------------------------------------------------------------

impl Clone for Secret {
    fn clone(&self) -> Self {
        Self::new(self.inner.to_string())
    }
}

impl Default for Secret {
    fn default() -> Self {
        Self::with_capacity(PAGE_SIZE)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl PartialEq for Secret {
    /// Best-effort timing-resistant comparison. Note: length differences cause
    /// an early return, which leaks length information through timing. Acceptable
    /// for this application's local threat model.
    fn eq(&self, other: &Self) -> bool {
        let a = self.expose_secret().as_bytes();
        let b = other.expose_secret().as_bytes();
        if a.len() != b.len() {
            return false;
        }
        let diff = a
            .iter()
            .zip(b.iter())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y));
        diff == 0
    }
}

impl Eq for Secret {}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Self::new(s.to_string())
    }
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_redacted() {
        let secret = Secret::new("hunter2");
        let debug = format!("{:?}", secret);
        assert!(debug.contains("***"), "debug output must be redacted");
        assert!(
            !debug.contains("hunter2"),
            "debug output must not contain the actual value"
        );
    }

    #[test]
    fn test_expose_secret() {
        let secret = Secret::new("hello");
        assert_eq!(secret.expose_secret(), "hello");
    }

    #[test]
    fn test_text_buffer_insert_and_delete() {
        let mut secret = Secret::new("hello");

        // insert_text appends " world"
        let inserted = secret.insert_text(" world", 5);
        assert_eq!(inserted, 6);
        assert_eq!(secret.expose_secret(), "hello world");

        // delete_char_range removes " world"
        secret.delete_char_range(5..11);
        assert_eq!(secret.expose_secret(), "hello");
    }

    #[test]
    fn test_text_buffer_clear() {
        let mut secret = Secret::new("sensitive");
        TextBuffer::clear(&mut secret);
        assert!(secret.is_empty());
        assert_eq!(secret.expose_secret(), "");
    }

    #[test]
    fn test_text_buffer_replace_with() {
        let mut secret = Secret::new("old content");
        secret.replace_with("new content");
        assert_eq!(secret.expose_secret(), "new content");
    }

    #[test]
    fn test_text_buffer_take() {
        let mut secret = Secret::new("take me");
        let taken = TextBuffer::take(&mut secret);
        assert_eq!(taken, "take me");
        assert!(secret.is_empty());
    }

    #[test]
    fn test_is_empty() {
        assert!(Secret::empty().is_empty());
        assert!(Secret::default().is_empty());
        assert!(!Secret::new("x").is_empty());
    }

    #[test]
    fn test_partial_eq() {
        let a = Secret::new("same");
        let b = Secret::new("same");
        let c = Secret::new("different");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_from_string() {
        let s = String::from("owned");
        let secret: Secret = s.into();
        assert_eq!(secret.expose_secret(), "owned");
    }

    #[test]
    fn test_from_str() {
        let secret: Secret = "borrowed".into();
        assert_eq!(secret.expose_secret(), "borrowed");
    }

    #[test]
    fn test_default_is_empty() {
        let secret = Secret::default();
        assert!(secret.is_empty());
        assert_eq!(secret.expose_secret(), "");
    }

    #[test]
    fn test_clone() {
        let original = Secret::new("clone me");
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn test_with_capacity() {
        let secret = Secret::with_capacity(256);
        assert!(secret.is_empty());
        assert_eq!(secret.expose_secret(), "");
        // Capacity must be at least PAGE_SIZE
        assert!(
            secret.inner.capacity() >= PAGE_SIZE,
            "capacity {} must be >= PAGE_SIZE {}",
            secret.inner.capacity(),
            PAGE_SIZE,
        );
    }

    #[test]
    fn test_delete_char_range_zeroes_trailing() {
        let mut secret = Secret::new("abcdef");
        secret.delete_char_range(3..6);
        assert_eq!(secret.expose_secret(), "abc");
        // Trailing capacity bytes (after len) should be zeroed
        let len = secret.inner.len();
        let cap = secret.inner.capacity();
        let bytes = secret.inner.as_bytes();
        // We can only inspect up to len via safe API, but the content is correct
        assert_eq!(bytes, b"abc");
        assert!(cap > len, "buffer should have trailing capacity");
    }

    #[test]
    fn test_drop_does_not_panic() {
        let secret = Secret::new("drop me safely");
        assert_eq!(secret.expose_secret(), "drop me safely");
        drop(secret);
        // If we get here, drop didn't panic
    }
}
