//! Wallet alias (display name) rules — the single source of truth for HD
//! wallet names and imported single-key names.
//!
//! Every entry point that saves a user-typed alias (create, import, rename,
//! MCP import) routes the raw text through [`resolve_alias`]:
//!
//! 1. [`clean_alias`] removes Unicode control (`Cc`) and format (`Cf`)
//!    characters and Unicode default-ignorables (including variation selectors
//!    and Hangul fillers) using ICU property data,
//!    and then trims surrounding whitespace, so a name that is visually blank
//!    or uses bidi tricks to mimic another wallet cannot be saved as-is.
//! 2. A blank result means "use the default name" (see [`next_default_alias`]).
//! 3. The result must fit [`MAX_WALLET_ALIAS_CHARS`] characters.
//!
//! Uniqueness within one wallet kind is checked by [`ensure_alias_unique`];
//! the backend runs it under a lock together with the write so two wallets can
//! never end up sharing a name through the app.
//!
//! Known tradeoff: U+200D (zero-width joiner) and emoji tag characters are
//! `Cf`, so ZWJ emoji sequences and subdivision flags decompose into their
//! component glyphs once saved. Spoofing resistance wins over emoji fidelity in
//! a field that identifies where funds are sent from.

use std::collections::HashSet;

use icu_properties::{CodePointSetData, props::DefaultIgnorableCodePoint};
use thiserror::Error;
use unicode_general_category::{GeneralCategory, get_general_category};

use crate::model::validation::{TextLengthError, validate_char_count};

/// Maximum number of characters in a wallet alias, counted after cleaning.
pub const MAX_WALLET_ALIAS_CHARS: usize = 64;

/// Why an alias was rejected.
///
/// Model-local so this pure validator carries no dependency on the
/// backend-task layer; `TaskError` provides a `From` conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AliasError {
    /// The cleaned alias exceeds [`MAX_WALLET_ALIAS_CHARS`].
    #[error("The wallet name is too long. Use 64 characters or fewer and try again.")]
    TooLong {
        #[source]
        length: TextLengthError,
    },
    /// Another wallet of the same kind already uses the cleaned alias.
    #[error("Another wallet already uses this name. Choose a different name and try again.")]
    AlreadyUsed,
}

/// Wallet kind whose namespace a default alias is generated in. HD wallets and
/// imported single keys keep separate name namespaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultAliasKind {
    /// HD wallets — "Wallet 1", "Wallet 2", …
    HdWallet,
    /// Imported single keys — "Key 1", "Key 2", …
    SingleKey,
}

impl DefaultAliasKind {
    fn format(self, number: usize) -> String {
        match self {
            DefaultAliasKind::HdWallet => format!("Wallet {number}"),
            DefaultAliasKind::SingleKey => format!("Key {number}"),
        }
    }
}

/// Where an alias handed to a persistence entry point came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasSource {
    /// Typed by the user (UI or MCP). Cleaned, blank replaced by the default
    /// name, and checked for uniqueness within its wallet kind.
    UserEntered(String),
    /// Carried over from existing storage (legacy migration or restore).
    /// Never rejected for length — legacy aliases predate the limit, so an
    /// overlong one is logged and kept; `None` stays unnamed. A legacy duplicate
    /// is never rejected, but it is not kept as an exact duplicate either —
    /// [`dedupe_preserved_alias`] suffixes it `_1`, `_2`, … with the smallest
    /// free number.
    Preserved(Option<String>),
}

fn is_stripped_char(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::Control | GeneralCategory::Format
    ) || CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(c)
}

/// Remove control, format, and default-ignorable characters, then trim whitespace.
///
/// Stripping runs first so invisible characters wrapped around whitespace
/// (e.g. `"\u{FEFF} "`) cannot keep an otherwise blank name alive.
pub fn clean_alias(raw: &str) -> String {
    let stripped: String = raw.chars().filter(|c| !is_stripped_char(*c)).collect();
    stripped.trim().to_owned()
}

/// Number of characters [`resolve_alias`] measures for `raw` — the cleaned
/// length. UI counters must display this, never the raw buffer length.
pub fn alias_char_count(raw: &str) -> usize {
    clean_alias(raw).chars().count()
}

/// Resolve a raw alias into the value to persist: clean it, substitute
/// `default()` when the cleaned text is blank, and enforce the length limit.
///
/// A blank input is never an error — it always means "use the default name",
/// including when renaming.
pub fn resolve_alias(raw: &str, default: impl FnOnce() -> String) -> Result<String, AliasError> {
    let cleaned = clean_alias(raw);
    let alias = if cleaned.is_empty() {
        default()
    } else {
        cleaned
    };
    validate_char_count(&alias, 0, MAX_WALLET_ALIAS_CHARS)
        .map_err(|length| AliasError::TooLong { length })?;
    Ok(alias)
}

/// Length guard for aliases persisted verbatim (legacy data that bypasses
/// [`resolve_alias`]). Counts the stored characters as-is.
pub fn validate_stored_alias(alias: &str) -> Result<(), AliasError> {
    validate_char_count(alias, 0, MAX_WALLET_ALIAS_CHARS)
        .map_err(|length| AliasError::TooLong { length })
}

/// The smallest-numbered default alias of `kind` not used by any of
/// `existing` (compared after cleaning). Filling gaps keeps default names
/// collision-free after deletions, unlike `count + 1`.
pub fn next_default_alias<'a>(
    kind: DefaultAliasKind,
    existing: impl IntoIterator<Item = &'a str>,
) -> String {
    let taken: HashSet<String> = existing.into_iter().map(clean_alias).collect();
    // At most `taken.len()` candidates can be occupied, so the search ends by
    // `taken.len() + 1`.
    (1..=taken.len() + 1)
        .map(|number| kind.format(number))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| kind.format(taken.len() + 1))
}

/// Reject `alias` when any of `existing` has the same cleaned form.
///
/// `existing` must exclude the wallet being named, so re-saving a wallet under
/// its own current name succeeds. A blank alias is "unnamed", not a name, and
/// never collides.
pub fn ensure_alias_unique<'a>(
    alias: &str,
    existing: impl IntoIterator<Item = &'a str>,
) -> Result<(), AliasError> {
    let cleaned = clean_alias(alias);
    if cleaned.is_empty() {
        return Ok(());
    }
    if existing
        .into_iter()
        .any(|other| clean_alias(other) == cleaned)
    {
        return Err(AliasError::AlreadyUsed);
    }
    Ok(())
}

/// Disambiguate a legacy ([`AliasSource::Preserved`]) alias against
/// `existing` (aliases already in use in the same wallet kind, compared
/// after cleaning): a colliding name is never kept as an exact duplicate —
/// it is suffixed `_1`, `_2`, … with the smallest free number instead. A
/// blank alias is "unnamed", not a name, and is returned unchanged; it never
/// collides, matching [`ensure_alias_unique`].
///
/// The base is truncated, if needed, so the suffixed result still fits
/// [`MAX_WALLET_ALIAS_CHARS`].
pub fn dedupe_preserved_alias<'a>(
    alias: String,
    existing: impl IntoIterator<Item = &'a str>,
) -> String {
    let taken: HashSet<String> = existing.into_iter().map(clean_alias).collect();
    let cleaned = clean_alias(&alias);
    if cleaned.is_empty() || !taken.contains(&cleaned) {
        return alias;
    }
    let base_chars: Vec<char> = cleaned.chars().collect();
    (1usize..)
        .find_map(|n| {
            let suffix = format!("_{n}");
            let max_base_chars = MAX_WALLET_ALIAS_CHARS.saturating_sub(suffix.chars().count());
            let base: String = base_chars.iter().take(max_base_chars).collect();
            let candidate = format!("{base}{suffix}");
            (!taken.contains(&candidate)).then_some(candidate)
        })
        .expect("infinite suffix sequence always finds a free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_name() -> String {
        "Wallet 1".to_owned()
    }

    #[test]
    fn default_ignorable_aliases_use_default_names() {
        for c in [
            '\u{fe00}',
            '\u{fe0f}',
            '\u{e0100}',
            '\u{e01ef}',
            '\u{115f}',
            '\u{1160}',
            '\u{3164}',
            '\u{ffa0}',
            '\u{034f}',
            '\u{180b}',
        ] {
            let raw = format!(" {c} ");
            assert_eq!(
                resolve_alias(&raw, default_name),
                Ok(default_name()),
                "{c:?}"
            );
            assert_eq!(alias_char_count(&raw), 0, "{c:?}");
            assert_eq!(
                ensure_alias_unique(&format!("Savings{c}"), ["Savings"]),
                Err(AliasError::AlreadyUsed)
            );
        }
        assert_eq!(clean_alias("Cafe\u{301} 日本語"), "Cafe\u{301} 日本語");
    }

    #[test]
    fn zero_width_space_alone_is_blank() {
        assert_eq!(
            resolve_alias("\u{200B}", default_name),
            Ok("Wallet 1".into())
        );
    }

    #[test]
    fn bom_followed_by_space_is_blank() {
        assert_eq!(
            resolve_alias("\u{FEFF} ", default_name),
            Ok("Wallet 1".into())
        );
    }

    #[test]
    fn bidi_override_is_stripped() {
        assert_eq!(resolve_alias("\u{202E}abc", default_name), Ok("abc".into()));
    }

    #[test]
    fn every_listed_invisible_and_bidi_char_is_stripped() {
        for c in [
            '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}', '\u{202A}', '\u{202B}',
            '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
            '\n', '\t', '\u{7F}',
        ] {
            let raw = format!("a{c}b");
            assert_eq!(
                clean_alias(&raw),
                "ab",
                "U+{:04X} must be stripped",
                c as u32
            );
        }
    }

    #[test]
    fn plain_whitespace_is_blank() {
        assert_eq!(resolve_alias("   \t ", default_name), Ok("Wallet 1".into()));
    }

    #[test]
    fn empty_string_is_blank() {
        assert_eq!(resolve_alias("", default_name), Ok("Wallet 1".into()));
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_interior_kept() {
        assert_eq!(
            resolve_alias("  My  savings ", default_name),
            Ok("My  savings".into())
        );
    }

    #[test]
    fn default_is_not_invoked_for_non_blank_input() {
        let resolved = resolve_alias("Named", || panic!("default must not run"));
        assert_eq!(resolved, Ok("Named".into()));
    }

    #[test]
    fn ascii_boundary_64_accepted_65_rejected() {
        assert_eq!(
            resolve_alias(&"w".repeat(64), default_name),
            Ok("w".repeat(64))
        );
        assert!(matches!(
            resolve_alias(&"w".repeat(65), default_name),
            Err(AliasError::TooLong { length }) if length.actual == 65 && length.max == 64
        ));
    }

    #[test]
    fn multibyte_boundary_64_accepted_65_rejected() {
        assert_eq!(
            resolve_alias(&"é".repeat(64), default_name),
            Ok("é".repeat(64))
        );
        assert!(matches!(
            resolve_alias(&"é".repeat(65), default_name),
            Err(AliasError::TooLong { .. })
        ));
    }

    #[test]
    fn length_is_measured_after_cleaning() {
        let padded = format!("  \u{200B}{}\u{202E}  ", "w".repeat(64));
        assert_eq!(resolve_alias(&padded, default_name), Ok("w".repeat(64)));
        assert_eq!(alias_char_count(&padded), 64);
    }

    #[test]
    fn default_alias_starts_at_one() {
        assert_eq!(
            next_default_alias(DefaultAliasKind::HdWallet, []),
            "Wallet 1"
        );
        assert_eq!(next_default_alias(DefaultAliasKind::SingleKey, []), "Key 1");
    }

    #[test]
    fn default_alias_fills_the_smallest_gap() {
        // "Wallet 2" was deleted: count + 1 would produce a duplicate "Wallet 3".
        let existing = ["Wallet 1", "Wallet 3"];
        assert_eq!(
            next_default_alias(DefaultAliasKind::HdWallet, existing),
            "Wallet 2"
        );
    }

    #[test]
    fn default_alias_skips_names_taken_after_cleaning() {
        let existing = ["Wallet 1 ", "\u{200B}Wallet 2", "Savings"];
        assert_eq!(
            next_default_alias(DefaultAliasKind::HdWallet, existing),
            "Wallet 3"
        );
    }

    #[test]
    fn default_alias_namespaces_are_independent() {
        let existing = ["Wallet 1", "Wallet 2"];
        assert_eq!(
            next_default_alias(DefaultAliasKind::SingleKey, existing),
            "Key 1"
        );
    }

    #[test]
    fn unique_alias_is_accepted() {
        assert_eq!(ensure_alias_unique("Savings", ["Wallet 1"]), Ok(()));
    }

    #[test]
    fn duplicate_alias_is_rejected() {
        assert_eq!(
            ensure_alias_unique("Savings", ["Wallet 1", "Savings"]),
            Err(AliasError::AlreadyUsed)
        );
    }

    #[test]
    fn duplicate_detection_compares_cleaned_forms() {
        assert_eq!(
            ensure_alias_unique("Savings", ["\u{202E}Savings "]),
            Err(AliasError::AlreadyUsed)
        );
    }

    #[test]
    fn duplicate_detection_is_case_sensitive() {
        assert_eq!(ensure_alias_unique("savings", ["Savings"]), Ok(()));
    }

    #[test]
    fn blank_alias_never_collides_with_unnamed_wallets() {
        assert_eq!(ensure_alias_unique("", ["", "Wallet 1"]), Ok(()));
    }

    #[test]
    fn stored_alias_length_is_counted_verbatim() {
        assert_eq!(validate_stored_alias(""), Ok(()));
        assert_eq!(validate_stored_alias(&"é".repeat(64)), Ok(()));
        assert!(matches!(
            validate_stored_alias(&"w".repeat(65)),
            Err(AliasError::TooLong { .. })
        ));
    }

    #[test]
    fn alias_error_exposes_length_source() {
        let error = resolve_alias(&"w".repeat(65), default_name).expect_err("too long");
        let source = std::error::Error::source(&error).expect("length source");
        assert!(source.downcast_ref::<TextLengthError>().is_some());
    }

    #[test]
    fn dedupe_leaves_a_unique_alias_untouched() {
        assert_eq!(
            dedupe_preserved_alias("Savings".into(), ["Checking"]),
            "Savings"
        );
    }

    #[test]
    fn dedupe_suffixes_a_single_collision() {
        assert_eq!(dedupe_preserved_alias("Dup".into(), ["Dup"]), "Dup_1");
    }

    #[test]
    fn dedupe_finds_the_smallest_free_suffix() {
        assert_eq!(
            dedupe_preserved_alias("Dup".into(), ["Dup", "Dup_1", "Dup_2"]),
            "Dup_3"
        );
    }

    #[test]
    fn dedupe_fills_a_suffix_gap() {
        // "Dup_1" was freed (renamed/removed elsewhere) — reuse it rather than
        // skipping straight to "Dup_2".
        assert_eq!(
            dedupe_preserved_alias("Dup".into(), ["Dup", "Dup_2"]),
            "Dup_1"
        );
    }

    #[test]
    fn dedupe_compares_cleaned_forms() {
        assert_eq!(dedupe_preserved_alias("Dup".into(), ["  Dup  "]), "Dup_1");
    }

    #[test]
    fn dedupe_never_touches_a_blank_alias() {
        assert_eq!(dedupe_preserved_alias("".into(), [""]), "");
        assert_eq!(dedupe_preserved_alias("   ".into(), ["   "]), "   ");
    }

    #[test]
    fn dedupe_truncates_the_base_to_keep_the_suffixed_name_within_the_limit() {
        let base = "a".repeat(MAX_WALLET_ALIAS_CHARS);
        let deduped = dedupe_preserved_alias(base.clone(), [base.as_str()]);
        assert_eq!(
            deduped,
            format!("{}_1", "a".repeat(MAX_WALLET_ALIAS_CHARS - 2))
        );
        assert_eq!(deduped.chars().count(), MAX_WALLET_ALIAS_CHARS);
    }

    #[test]
    fn dedupe_keeps_finding_room_past_nine_suffixes() {
        let taken: Vec<String> = (1..=9).map(|n| format!("Dup_{n}")).collect();
        let mut existing: Vec<&str> = vec!["Dup"];
        existing.extend(taken.iter().map(String::as_str));
        assert_eq!(dedupe_preserved_alias("Dup".into(), existing), "Dup_10");
    }
}
