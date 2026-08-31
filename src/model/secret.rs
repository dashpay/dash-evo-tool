use std::alloc::Layout;
use std::any::TypeId;
use std::fmt;
use std::ops::Range;
use std::ptr::NonNull;

use egui::TextBuffer;
use egui::text::CharIndex;
use zeroize::Zeroize;

/// Pre-allocation capacity for `Secret` buffers.
///
/// `memsec` prefixes every allocation with a 16-byte canary and rounds the
/// total up to whole pages, so 4080 bytes is the largest payload that still
/// fits one 4 KiB data page — ample for any passphrase, WIF key, or 24-word
/// recovery phrase. Growing past it is safe (the outgrown buffer is wiped
/// before it is freed), just wasteful of a second page.
const DEFAULT_CAPACITY: usize = 4096 - 16;

/// A page-guarded, `mlock`ed byte buffer allocated by [`memsec`].
///
/// The allocation is page-aligned, fenced by inaccessible guard pages, and
/// locked into RAM on a best-effort basis (`memsec` ignores an `mlock` refusal,
/// matching this module's best-effort contract). Because the data pages belong
/// to one buffer outright, two live buffers can never share a page — so freeing
/// one can never unlock memory another still holds, the failure mode that makes
/// page-granular locking hazardous over ordinary allocations.
struct GuardedBuf {
    ptr: NonNull<u8>,
    cap: usize,
}

// SAFETY: GuardedBuf uniquely owns its allocation and has no interior
// mutability, so it is as safe to send and share as a `Box<[u8]>`. Wrapping the
// raw pointer here keeps `Secret` itself free of manual unsafe trait impls.
unsafe impl Send for GuardedBuf {}
unsafe impl Sync for GuardedBuf {}

impl GuardedBuf {
    /// Allocate `cap` zeroed bytes in guarded memory.
    fn new(cap: usize) -> Self {
        // SAFETY: `malloc_sized` takes a plain size and returns a pointer to
        // that many writable bytes, or `None` when the allocation fails.
        let ptr = unsafe { memsec::malloc_sized(cap) }.unwrap_or_else(|| alloc_failed(cap));
        let mut buf = Self {
            ptr: ptr.cast(),
            cap,
        };
        // memsec hands back a garbage-filled block; zero it so "every byte past
        // the secret's length is zero" holds from the first write onwards.
        buf.zeroize_all();
        buf
    }

    fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// Volatile-zero every byte, including capacity past the current length.
    fn zeroize_all(&mut self) {
        // SAFETY: the allocation is valid and uniquely owned for `cap` bytes.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.cap) }.zeroize();
    }
}

impl Drop for GuardedBuf {
    fn drop(&mut self) {
        // `memsec::free` wipes the region as well, but the wipe is this
        // module's guarantee, not a dependency's implementation detail.
        self.zeroize_all();
        // SAFETY: `ptr` came from `memsec::malloc_sized` and is freed once.
        unsafe { memsec::free(self.ptr) };
    }
}

/// Report an unrecoverable guarded-allocation failure.
///
/// `Secret`'s constructors are infallible by contract — callers throughout the
/// UI and task layers build one inline with no fallback path — so exhaustion of
/// guarded memory is handled the way the global allocator handles exhaustion of
/// ordinary memory.
fn alloc_failed(cap: usize) -> ! {
    match Layout::from_size_align(cap, 1) {
        Ok(layout) => std::alloc::handle_alloc_error(layout),
        Err(_) => panic!("secret capacity {cap} exceeds the maximum allocation size"),
    }
}

/// Zeroize-on-drop wrapper for sensitive strings (passwords, WIF keys, private key inputs).
///
/// Debug output is redacted. The bytes live in a [`GuardedBuf`], so they are
/// kept out of swap on a best-effort basis, sit between inaccessible guard
/// pages, and are wiped when the value is dropped.
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
    buf: GuardedBuf,
    /// Length in bytes of the UTF-8 plaintext held at the start of `buf`.
    len: usize,
}

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
        let mut source: String = s.into();
        let secret = Self::from_plaintext(&source);
        source.zeroize();
        secret
    }

    /// Copy `text` straight into a fresh guarded buffer, with no intermediate
    /// unprotected allocation.
    fn from_plaintext(text: &str) -> Self {
        let mut secret = Self::with_capacity(text.len());
        secret.splice(0..0, text.as_bytes());
        secret
    }

    /// Create an empty `Secret` with a pre-allocated guarded buffer.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: GuardedBuf::new(cap.max(DEFAULT_CAPACITY)),
            len: 0,
        }
    }

    /// Create an empty `Secret` with a pre-allocated guarded buffer.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Borrow the plaintext.
    ///
    /// # Panics
    ///
    /// Panics if the buffer does not hold valid UTF-8, which can only mean a
    /// mutation spliced somewhere other than a character boundary — a bug in
    /// this module rather than a recoverable condition.
    pub fn expose_secret(&self) -> &str {
        std::str::from_utf8(self.as_bytes()).expect("Secret buffer holds valid UTF-8")
    }

    /// The length of the secret in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the secret's trimmed value is empty.
    ///
    /// Prefer this over `expose_secret().trim().is_empty()` for presence
    /// checks — it avoids exposing the raw bytes unnecessarily.
    pub fn is_blank(&self) -> bool {
        self.expose_secret().trim().is_empty()
    }

    /// Returns a new `Secret` containing the trimmed content.
    /// Keeps the data within the secure wrapper unlike `text().trim()`
    /// which returns a borrowed `&str`.
    pub fn trimmed(&self) -> Self {
        Self::from_plaintext(self.expose_secret().trim())
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: the first `len` bytes of the buffer are always initialised.
        unsafe { std::slice::from_raw_parts(self.buf.as_ptr(), self.len) }
    }

    /// Ensure the buffer holds at least `needed` bytes, migrating the contents
    /// to a larger guarded allocation when it does not.
    fn reserve(&mut self, needed: usize) {
        if needed <= self.buf.cap {
            return;
        }
        let cap = needed
            .max(self.buf.cap.saturating_mul(2))
            .max(DEFAULT_CAPACITY);
        let mut grown = GuardedBuf::new(cap);
        // SAFETY: the source holds `len` initialised bytes and `grown` is a
        // distinct allocation of `cap >= needed >= len` bytes.
        unsafe { std::ptr::copy_nonoverlapping(self.buf.as_ptr(), grown.as_mut_ptr(), self.len) };
        // Assigning drops the outgrown buffer, which wipes it.
        self.buf = grown;
    }

    /// Replace the plaintext bytes in `byte_range` with `replacement`.
    ///
    /// The single mutation primitive: insertion, deletion, and replacement all
    /// reduce to it. `byte_range` is clamped into the current plaintext, and
    /// both ends must fall on character boundaries for the result to stay valid
    /// UTF-8.
    fn splice(&mut self, byte_range: Range<usize>, replacement: &[u8]) {
        let old_len = self.len;
        let start = byte_range.start.min(old_len);
        let end = byte_range.end.clamp(start, old_len);
        let new_len = old_len - (end - start) + replacement.len();
        self.reserve(new_len);

        let base = self.buf.as_mut_ptr();
        // SAFETY: every offset below lies inside a buffer of at least
        // `max(old_len, new_len)` bytes, and `copy` tolerates the tail overlap.
        unsafe {
            std::ptr::copy(
                base.add(end),
                base.add(start + replacement.len()),
                old_len - end,
            );
            std::ptr::copy_nonoverlapping(replacement.as_ptr(), base.add(start), replacement.len());
            if new_len < old_len {
                std::slice::from_raw_parts_mut(base.add(new_len), old_len - new_len).zeroize();
            }
        }
        self.len = new_len;
    }
}

// -- TextBuffer impl (allows `TextEdit::singleline(&mut secret)`) -----------

impl TextBuffer for Secret {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.expose_secret()
    }

    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        let at = self.byte_index_from_char_index(char_index).0;
        self.splice(at..at, text.as_bytes());
        text.chars().count()
    }

    fn delete_char_range(&mut self, char_range: Range<CharIndex>) {
        assert!(
            char_range.start <= char_range.end,
            "start must be <= end, but got {char_range:?}"
        );
        let start = self.byte_index_from_char_index(char_range.start).0;
        let end = self.byte_index_from_char_index(char_range.end).0;
        self.splice(start..end, &[]);
    }

    fn clear(&mut self) {
        let len = self.len;
        self.splice(0..len, &[]);
    }

    fn replace_with(&mut self, text: &str) {
        let len = self.len;
        self.splice(0..len, text.as_bytes());
    }

    fn take(&mut self) -> String {
        // Deliberately returns an unprotected String — required by egui TextBuffer trait.
        // The undoer is disabled in PasswordInput, limiting the call paths. Accepted as
        // inherent limitation of the egui framework for the desktop GUI threat model.
        let copy = self.expose_secret().to_owned();
        TextBuffer::clear(self);
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
        Self::from_plaintext(self.expose_secret())
    }
}

impl Default for Secret {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl PartialEq for Secret {
    /// Timing-resistant comparison via `memsec::memeq`. Note: length
    /// differences cause an early return, which leaks length information
    /// through timing. Acceptable for this application's local threat model.
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len {
            return false;
        }
        // SAFETY: both buffers hold at least `len` initialised bytes.
        unsafe { memsec::memeq(self.buf.as_ptr(), other.buf.as_ptr(), self.len) }
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
        Self::from_plaintext(s)
    }
}

// -- serde / schemars impls --------------------------------------------------
//
// `Secret` carries private-key / mnemonic material in MCP tool parameter
// structs.  Adding `Deserialize` lets the params struct derive `Deserialize`
// directly — the impl deserializes into a transient `String`, then moves it
// into the guarded buffer and zeroizes the transient, so no long-lived plain
// `String` copy persists.  The `JsonSchema` impl (gated to the features that
// bring in `rmcp`) exposes `Secret` as a JSON string in the MCP tool schema so
// clients know what format to supply.
//
// `platform_wallet_storage::SecretString` offers similar security guarantees
// but lacks both `Deserialize` and `JsonSchema`, and `IdentityInputToLoad`
// already uses this local `Secret` type — switching would require a lossy
// expose-then-rewrap at the boundary.  The local type is therefore preferred
// for MCP parameters.

impl<'de> serde::Deserialize<'de> for Secret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <String as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Secret::new(s))
    }
}

/// Expose as a plain JSON string schema — the secure wrapper is invisible to
/// the caller; they just supply a string value.
#[cfg(any(feature = "mcp", feature = "cli"))]
impl rmcp::schemars::JsonSchema for Secret {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Secret".into()
    }
    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        rmcp::schemars::json_schema!({ "type": "string" })
    }
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// The whole allocation, including capacity past the plaintext. Sound: the
    /// `Secret` is alive and every byte was initialised at allocation.
    fn full_buffer(secret: &Secret) -> &[u8] {
        // SAFETY: `secret` owns `cap` initialised bytes for as long as it lives.
        unsafe { std::slice::from_raw_parts(secret.buf.as_ptr(), secret.buf.cap) }
    }

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

    /// Character indices are not byte indices: splicing must land on character
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
        assert!(
            full_buffer(&secret).iter().all(|&b| b == 0),
            "clear must wipe the plaintext, not just reset the length"
        );
    }

    #[test]
    fn test_text_buffer_replace_with() {
        let mut secret = Secret::new("old content");
        secret.replace_with("new content");
        assert_eq!(secret.expose_secret(), "new content");
    }

    /// Replacing with something shorter must not leave the tail of the previous
    /// value readable past the new length.
    #[test]
    fn test_replace_with_shorter_wipes_tail() {
        let mut secret = Secret::new("a very long previous secret");
        secret.replace_with("short");
        assert_eq!(secret.expose_secret(), "short");
        assert!(
            full_buffer(&secret)[secret.len()..].iter().all(|&b| b == 0),
            "bytes past the new length must be zero"
        );
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
        assert!(
            !std::ptr::eq(original.buf.as_ptr(), cloned.buf.as_ptr()),
            "a clone must own a separate guarded allocation"
        );
    }

    #[test]
    fn test_with_capacity() {
        let secret = Secret::with_capacity(256);
        assert!(secret.is_empty());
        assert_eq!(secret.expose_secret(), "");
        // Capacity must be at least DEFAULT_CAPACITY
        assert!(
            secret.buf.cap >= DEFAULT_CAPACITY,
            "capacity {} must be >= DEFAULT_CAPACITY {}",
            secret.buf.cap,
            DEFAULT_CAPACITY,
        );
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
    fn test_delete_char_range_zeroes_trailing() {
        let mut secret = Secret::new("abcdef");
        secret.delete_char_range(CharIndex(3)..CharIndex(6));
        assert_eq!(secret.expose_secret(), "abc");
        assert!(
            full_buffer(&secret)[secret.len()..].iter().all(|&b| b == 0),
            "deleted bytes must be zeroed, not merely orphaned past the length"
        );
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

    /// Two live `Secret`s must never place their plaintext on the same page.
    /// A shared page means one secret's lifetime governs the other's anti-swap
    /// protection — the residual weakness that guarded allocation removes.
    #[test]
    fn test_secrets_never_share_a_page() {
        let page = region::page::size();
        let secrets: Vec<Secret> = (0..64)
            .map(|i| Secret::new(format!("secret-{i}")))
            .collect();

        let mut owner: HashMap<usize, usize> = HashMap::new();
        for (idx, secret) in secrets.iter().enumerate() {
            let start = secret.buf.as_ptr() as usize;
            let cap = secret.buf.cap;
            assert_eq!(
                (start + cap) % page,
                0,
                "the buffer must end on a page boundary, where memsec's guard page begins"
            );
            for page_index in (start / page)..=((start + cap - 1) / page) {
                if let Some(previous) = owner.insert(page_index, idx) {
                    panic!("secrets {previous} and {idx} share page {page_index:#x}");
                }
            }
        }
    }

    /// A `Secret` that outgrows its buffer migrates to a larger guarded
    /// allocation; the content must survive and the outgrown buffer must be
    /// released without incident.
    #[test]
    fn test_growth_beyond_capacity_preserves_content() {
        let mut secret = Secret::new("start");
        let long = "x".repeat(DEFAULT_CAPACITY * 2);
        secret.replace_with(&long);
        assert_eq!(secret.expose_secret(), long);
        assert!(secret.buf.cap >= long.len());
        drop(secret);
    }

    /// Growing by repeated insertion — how a `TextEdit` actually fills a buffer
    /// — must preserve every byte across the migrations it triggers.
    #[test]
    fn test_incremental_growth_preserves_content() {
        let mut secret = Secret::empty();
        let chunk = "0123456789";
        let repeats = (DEFAULT_CAPACITY / chunk.len()) + 3;
        for i in 0..repeats {
            secret.insert_text(chunk, CharIndex(i * chunk.len()));
        }
        assert_eq!(secret.len(), repeats * chunk.len());
        assert_eq!(secret.expose_secret(), chunk.repeat(repeats));
    }

    /// The wipe primitive behind `Drop`, exercised while the allocation is
    /// still alive so the assertion reads valid memory.
    #[test]
    fn test_zeroize_all_wipes_full_capacity() {
        let mut buf = GuardedBuf::new(DEFAULT_CAPACITY);
        // SAFETY: `buf` is alive and owns `DEFAULT_CAPACITY` writable bytes.
        let filled = unsafe {
            std::ptr::write_bytes(buf.as_mut_ptr(), 0xAB, DEFAULT_CAPACITY);
            std::slice::from_raw_parts(buf.as_ptr(), DEFAULT_CAPACITY)
        };
        assert!(filled.iter().all(|&b| b == 0xAB));

        buf.zeroize_all();

        // SAFETY: as above — the allocation is unchanged and still owned.
        let wiped = unsafe { std::slice::from_raw_parts(buf.as_ptr(), DEFAULT_CAPACITY) };
        assert!(
            wiped.iter().all(|&b| b == 0),
            "zeroize_all must clear every byte"
        );
    }

    // Compile-time assertion that Secret has a Drop impl (not trivially droppable).
    const _: () = {
        assert!(std::mem::needs_drop::<Secret>());
    };

    /// Best-effort test that Drop zeroes the full capacity.
    ///
    /// Reads freed memory after drop — technically UB. `memsec::free` restores
    /// the guard pages to read-write before handing the block back to the
    /// allocator, so the read normally succeeds, but the allocator may reuse
    /// the block between drop and inspection. Run manually with `--ignored` in
    /// a single thread for reliable results:
    ///
    /// ```sh
    /// cargo test --lib -- test_drop_zeroes_full_capacity --ignored --test-threads=1
    /// ```
    ///
    /// `test_zeroize_all_wipes_full_capacity` covers the same wipe soundly;
    /// this test additionally confirms `Drop` invokes it.
    #[test]
    #[ignore]
    fn test_drop_zeroes_full_capacity() {
        let ptr: *const u8;
        let cap: usize;
        {
            let secret = Secret::new("sensitive_data_here".to_string());
            ptr = secret.buf.as_ptr();
            cap = secret.buf.cap;
            // Verify data is present before drop
            let slice = unsafe { std::slice::from_raw_parts(ptr, cap) };
            assert!(
                slice.iter().any(|&b| b != 0),
                "Expected non-zero bytes before drop"
            );
            // secret drops here — Drop zeros 0..cap via zeroize
        }
        // SAFETY: Reading freed memory. Best-effort: the allocator is unlikely
        // to reuse this block immediately when running single-threaded.
        let post = unsafe { std::slice::from_raw_parts(ptr, cap) };
        assert!(
            post.iter().all(|&b| b == 0),
            "Memory was not zeroed after drop"
        );
    }
}
