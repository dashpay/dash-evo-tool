use std::any::TypeId;
use std::fmt;
use std::ops::Range;

use egui::TextBuffer;
use egui::text::CharIndex;
use platform_wallet_storage::secrets::SecretString;
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

/// Zeroize-on-drop wrapper for sensitive strings (passwords, WIF keys, private key inputs).
///
/// A newtype over [`SecretString`], which owns the storage: the bytes live in
/// a page-aligned allocation fenced by inaccessible guard pages, `mlock`ed out
/// of swap and excluded from core dumps on a best-effort basis, and wiped over
/// the buffer's full capacity when the value drops. Two live secrets never
/// share a page, so freeing one can never unlock memory another still holds.
///
/// This type adds only what DET needs on top: egui's [`TextBuffer`] (so a
/// `Secret` can back a `TextEdit` directly), equality, and `serde`/`schemars`
/// for MCP tool parameters.
///
/// Debug output is redacted. `Display`, `Deref`, and `DerefMut` are
/// intentionally **not** implemented to prevent accidental leakage. Use
/// [`expose_secret`](Self::expose_secret) for explicit read access, or pass
/// `&mut Secret` directly to `TextEdit::singleline` (via the [`TextBuffer`]
/// impl) for mutable editing.
///
/// # Security: TextEdit usage
///
/// When using `Secret` with `TextEdit::singleline`, you **must** set
/// `.password(true)`. Without it, plaintext leaks to egui's layout system,
/// widget info, and accessibility events. Use [`PasswordInput`] to ensure
/// this is always enforced.
#[derive(Default, serde::Deserialize)]
#[serde(transparent)]
pub struct Secret(SecretString);

// `Secret` rides inside `BackendTask` variants across threads; losing that is a
// compile error here rather than a puzzling one at a distant call site.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Secret>();
};

impl Secret {
    /// Wrap a string in a `Secret`, moving it into guarded memory.
    ///
    /// The source string is zeroized before it is dropped, so no unprotected
    /// copy outlives the call.
    pub fn new(s: impl Into<String>) -> Self {
        Self(SecretString::new(s))
    }

    /// Create an empty `Secret`, which holds no allocation at all.
    pub fn empty() -> Self {
        Self(SecretString::empty())
    }

    /// Borrow the plaintext.
    ///
    /// # Panics
    ///
    /// Panics if the buffer does not hold valid UTF-8, which can only mean a
    /// mutation landed off a character boundary — a bug rather than a
    /// recoverable condition.
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }

    /// The length of the secret in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether the secret's trimmed value is empty.
    ///
    /// Prefer this over `expose_secret().trim().is_empty()` for presence
    /// checks — it avoids exposing the raw bytes unnecessarily.
    pub fn is_blank(&self) -> bool {
        self.0.is_blank()
    }

    /// Returns a new `Secret` containing the trimmed content.
    /// Keeps the data within the secure wrapper unlike `expose_secret().trim()`
    /// which returns a borrowed `&str`.
    pub fn trimmed(&self) -> Self {
        Self(self.0.trimmed())
    }
}

// -- TextBuffer impl (allows `TextEdit::singleline(&mut secret)`) -----------
//
// Every edit reduces to `SecretString::replace_range`, which takes byte
// offsets. The char-index translation stays here: `byte_index_from_char_index`
// reads `as_str()` and always yields a character boundary at or before the end
// of the plaintext, so no edit below can trip that method's bounds assertions.

impl TextBuffer for Secret {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.0.expose_secret()
    }

    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        let at = self.byte_index_from_char_index(char_index).0;
        self.0.replace_range(at..at, text);
        text.chars().count()
    }

    fn delete_char_range(&mut self, char_range: Range<CharIndex>) {
        assert!(
            char_range.start <= char_range.end,
            "start must be <= end, but got {char_range:?}"
        );
        let start = self.byte_index_from_char_index(char_range.start).0;
        let end = self.byte_index_from_char_index(char_range.end).0;
        self.0.replace_range(start..end, "");
    }

    fn clear(&mut self) {
        self.0.zeroize();
    }

    fn replace_with(&mut self, text: &str) {
        self.0.replace_range(.., text);
    }

    fn take(&mut self) -> String {
        // Deliberately returns an unprotected String — required by egui TextBuffer trait.
        // The undoer is disabled in PasswordInput, limiting the call paths. Accepted as
        // inherent limitation of the egui framework for the desktop GUI threat model.
        let copy = self.0.expose_secret().to_owned();
        self.0.zeroize();
        copy
    }

    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
}

// -- Trait impls -------------------------------------------------------------

impl Clone for Secret {
    /// Copies into a fresh guarded allocation rather than sharing one, so the
    /// clone keeps its own exclusively owned pages and is wiped independently.
    fn clone(&self) -> Self {
        Self(SecretString::from(self.0.expose_secret()))
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl PartialEq for Secret {
    /// Compares equal-length values in constant time via [`subtle`]. A length
    /// mismatch short-circuits, so timing reveals the length but never where
    /// two values of the same length diverge.
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.0.ct_eq(&other.0))
    }
}

impl Eq for Secret {}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Self(SecretString::from(s))
    }
}

impl From<&str> for Secret {
    /// Copies straight into guarded memory, with no transient `String`.
    fn from(s: &str) -> Self {
        Self(SecretString::from(s))
    }
}

// -- serde / schemars impls --------------------------------------------------
//
// `Secret` carries private-key / mnemonic material in MCP tool parameter
// structs and in the testnet-node fixture, so both impls forward to
// `SecretString`'s own: its visitor copies a borrowed `&str` straight into
// guarded memory, and refuses a value past the vault's `MAX_PASSPHRASE_LEN`
// before it is ever allocated. There is deliberately no `Serialize`.

/// Delegates to [`SecretString`]'s schema: a plain JSON string carrying no
/// length policy, pattern, or example value. The schema name stays `Secret`,
/// which is also its `schema_id`, so generated MCP tool schemas keep
/// referring to it under that name.
#[cfg(any(feature = "mcp", feature = "cli"))]
impl rmcp::schemars::JsonSchema for Secret {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Secret".into()
    }

    fn json_schema(generator: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        <SecretString as rmcp::schemars::JsonSchema>::json_schema(generator)
    }
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Comfortably past `SecretString`'s single-page default capacity, so an
    /// edit of this size is guaranteed to force a reallocation.
    const PAST_DEFAULT_CAPACITY: usize = 8192;

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
        let inserted = secret.insert_text(" world", CharIndex(5));
        assert_eq!(inserted, 6);
        assert_eq!(secret.expose_secret(), "hello world");

        // delete_char_range removes " world"
        secret.delete_char_range(CharIndex(5)..CharIndex(11));
        assert_eq!(secret.expose_secret(), "hello");
    }

    #[test]
    fn test_text_buffer_insert_in_middle() {
        let mut secret = Secret::new("held");
        let inserted = secret.insert_text("wor", CharIndex(2));
        assert_eq!(inserted, 3);
        assert_eq!(secret.expose_secret(), "heworld");
    }

    /// Character indices are not byte indices: an edit must land on character
    /// boundaries or the buffer stops being valid UTF-8.
    #[test]
    fn test_text_buffer_multibyte_utf8() {
        let mut secret = Secret::new("héllo");
        assert_eq!(secret.len(), 6, "é occupies two bytes");

        let inserted = secret.insert_text(" wörld", CharIndex(5));
        assert_eq!(inserted, 6);
        assert_eq!(secret.expose_secret(), "héllo wörld");

        // Remove the é (character 1), not just the second byte of it.
        secret.delete_char_range(CharIndex(1)..CharIndex(2));
        assert_eq!(secret.expose_secret(), "hllo wörld");

        // Insert a 4-byte character at a boundary that follows a 2-byte one.
        secret.insert_text("🦡", CharIndex(7));
        assert_eq!(secret.expose_secret(), "hllo wö🦡rld");
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

    /// A shrinking whole-buffer replacement must leave only the new value —
    /// no tail of the previous one readable past the new length. The wipe of
    /// the vacated bytes is `SecretString`'s guarantee, proven in its own
    /// suite; observable here is that nothing of the old value survives.
    #[test]
    fn test_replace_with_shorter_keeps_only_new_value() {
        let mut secret = Secret::new("a very long previous secret");
        secret.replace_with("short");
        assert_eq!(secret.expose_secret(), "short");
        assert_eq!(secret.len(), "short".len());
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
        let prefix = Secret::new("sam");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, prefix);
        assert_eq!(Secret::empty(), Secret::empty());
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
        assert_eq!(cloned.expose_secret(), "clone me");
    }

    /// A clone owns its content outright: editing one must not disturb the
    /// other, in either direction.
    #[test]
    fn test_clone_is_independent() {
        let mut original = Secret::new("original value");
        let mut cloned = original.clone();

        cloned.replace_with("clone value");
        assert_eq!(original.expose_secret(), "original value");
        assert_eq!(cloned.expose_secret(), "clone value");

        original.replace_with("second original");
        assert_eq!(cloned.expose_secret(), "clone value");
    }

    /// Deserializes from a plain JSON string, the shape every caller supplies
    /// (MCP tool parameters, the testnet-node fixture).
    #[test]
    fn test_deserialize_from_json_string() {
        let secret: Secret = serde_json::from_str(r#""correct horse battery staple""#)
            .expect("a JSON string deserializes into a Secret");
        assert_eq!(secret.expose_secret(), "correct horse battery staple");
    }

    /// A non-string JSON value is not a secret.
    #[test]
    fn test_deserialize_rejects_non_string() {
        serde_json::from_str::<Secret>("42").expect_err("a JSON number is not a Secret");
    }

    #[test]
    fn test_trimmed() {
        let secret = Secret::new("  hello world  ");
        let trimmed = secret.trimmed();
        assert_eq!(trimmed.expose_secret(), "hello world");
    }

    #[test]
    fn test_trimmed_empty() {
        let secret = Secret::new("   ");
        let trimmed = secret.trimmed();
        assert!(trimmed.is_empty());
    }

    #[test]
    fn test_delete_char_range_removes_tail() {
        let mut secret = Secret::new("abcdef");
        secret.delete_char_range(CharIndex(3)..CharIndex(6));
        assert_eq!(secret.expose_secret(), "abc");
        assert_eq!(secret.len(), 3);
    }

    #[test]
    fn test_drop_does_not_panic() {
        let secret = Secret::new("drop me safely");
        assert_eq!(secret.expose_secret(), "drop me safely");
        drop(secret);
        // If we get here, drop didn't panic
    }

    /// Dropping many `Secret`s in bulk must stay silent. Over ordinary
    /// allocations the first drop unlocked pages its neighbours still occupied,
    /// and the resulting unlock failure took the process down on Windows.
    #[test]
    fn test_dropping_many_secrets_does_not_panic() {
        let secrets: Vec<Secret> = (0..64).map(|i| Secret::new(format!("s{i}"))).collect();
        drop(secrets);
    }

    /// A `Secret` that outgrows its buffer migrates to a larger guarded
    /// allocation; the content must survive that migration intact.
    #[test]
    fn test_growth_beyond_capacity_preserves_content() {
        let mut secret = Secret::new("start");
        let long = "x".repeat(PAST_DEFAULT_CAPACITY);
        secret.replace_with(&long);
        assert_eq!(secret.expose_secret(), long);
        assert_eq!(secret.len(), long.len());
    }

    /// Growing by repeated insertion — how a `TextEdit` actually fills a buffer
    /// — must preserve every byte across the migrations it triggers.
    #[test]
    fn test_incremental_growth_preserves_content() {
        let mut secret = Secret::empty();
        let chunk = "0123456789";
        let repeats = (PAST_DEFAULT_CAPACITY / chunk.len()) + 3;
        for i in 0..repeats {
            secret.insert_text(chunk, CharIndex(i * chunk.len()));
        }
        assert_eq!(secret.len(), repeats * chunk.len());
        assert_eq!(secret.expose_secret(), chunk.repeat(repeats));
    }

    // Compile-time assertion that Secret has a Drop impl (not trivially droppable).
    const _: () = {
        assert!(std::mem::needs_drop::<Secret>());
    };
}
