use std::fmt;

use zeroize::Zeroizing;

/// Zeroize-on-drop wrapper for sensitive strings (passwords, WIF keys, private key inputs).
///
/// Debug output is redacted. Best-effort `mlock` prevents the backing memory from
/// being swapped to disk. The lock is released automatically when the value is dropped.
///
/// `Display`, `Deref`, and `DerefMut` are intentionally **not** implemented to
/// prevent accidental leakage. Use [`expose_secret`](Self::expose_secret) or
/// [`expose_secret_mut`](Self::expose_secret_mut) for explicit access.
pub struct Secret {
    /// Dropped first -- zeroes the bytes.
    inner: Zeroizing<String>,
    /// Dropped second -- unlocks the page.
    _lock: Option<region::LockGuard>,
}

impl Secret {
    /// Wrap a string in a `Secret`, locking its backing memory on a best-effort basis.
    pub fn new(s: impl Into<String>) -> Self {
        let mut s: String = s.into();
        // Pre-allocate so later pushes are less likely to reallocate (which would
        // leave copies of the secret in freed pages).
        let target_cap = s.len().max(128);
        if s.capacity() < target_cap {
            s.reserve(target_cap - s.capacity());
        }
        let lock = region::lock(s.as_ptr(), s.capacity()).ok();
        Self {
            inner: Zeroizing::new(s),
            _lock: lock,
        }
    }

    /// Create an empty `Secret` with a pre-allocated, locked buffer.
    pub fn with_capacity(cap: usize) -> Self {
        let s = String::with_capacity(cap);
        let lock = region::lock(s.as_ptr(), s.capacity()).ok();
        Self {
            inner: Zeroizing::new(s),
            _lock: lock,
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

    /// Mutably borrow the backing `String` (needed for egui `TextEdit` binding).
    pub fn expose_secret_mut(&mut self) -> &mut String {
        &mut self.inner
    }

    /// Whether the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

// -- Trait impls -------------------------------------------------------------

impl Clone for Secret {
    fn clone(&self) -> Self {
        Self::new(self.inner.as_str().to_string())
    }
}

impl Default for Secret {
    fn default() -> Self {
        Self::with_capacity(128)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl PartialEq for Secret {
    /// Constant-time comparison to prevent timing side-channel attacks (CWE-208).
    fn eq(&self, other: &Self) -> bool {
        let a = self.expose_secret().as_bytes();
        let b = other.expose_secret().as_bytes();
        if a.len() != b.len() {
            return false;
        }
        // XOR all bytes; any difference sets bits in `diff`.
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
    fn test_expose_secret_mut() {
        let mut secret = Secret::new("hello");
        secret.expose_secret_mut().push_str(" world");
        assert_eq!(secret.expose_secret(), "hello world");
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
    }
}
