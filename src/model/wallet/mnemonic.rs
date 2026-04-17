//! BIP-39 mnemonic input sanitization.
//!
//! Users paste mnemonics from many places — numbered lists, word processors,
//! terminals with trailing whitespace, IMEs that emit fullwidth or combining
//! characters, and lookalike homoglyphs. This module normalizes that input into
//! a clean lowercase ASCII form that `Mnemonic::parse_normalized` can consume.
//!
//! The pipeline is deliberately strict: drop anything that is not an ASCII
//! letter or ASCII whitespace. A silent "almost matches" is worse than a clear
//! failure — a wrong word in a seed phrase loses funds.

use unicode_normalization::UnicodeNormalization;

/// Clean up user-supplied mnemonic input for BIP-39 parsing.
///
/// Pipeline (order matters):
/// 1. Apply NFKD Unicode normalization. This decomposes compatibility forms
///    like fullwidth Latin letters (`ａ` → `a`) and separates combining marks
///    (`á` → `a` + U+0301). Doing this **before** filtering is essential —
///    filtering first would drop the fullwidth glyph entirely.
/// 2. Keep only ASCII alphabetic bytes and ASCII whitespace. Cyrillic
///    lookalikes, digits, punctuation, emoji, combining marks, and anything
///    exotic are all discarded. `is_ascii_alphabetic` is used deliberately
///    instead of `char::is_alphabetic`, which would accept combining marks
///    and non-ASCII letters.
/// 3. Collapse runs of whitespace (spaces, tabs, CR, LF) into single spaces.
/// 4. Lowercase via `to_ascii_lowercase` — BIP-39 word lists are lowercase
///    ASCII.
///
/// The output is always lowercase ASCII words separated by single spaces, or
/// the empty string if the input contained no usable letters.
pub fn sanitize_mnemonic_input(raw: &str) -> String {
    // 1. NFKD normalization.
    let nfkd: String = raw.nfkd().collect();

    // 2. Retain only ASCII letters and ASCII whitespace.
    let filtered: String = nfkd
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || (c.is_ascii() && c.is_whitespace()))
        .collect();

    // 3. Collapse whitespace into single spaces.
    let joined = filtered.split_whitespace().collect::<Vec<_>>().join(" ");

    // 4. Lowercase (ASCII only at this point).
    joined.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_numbered_list_prefixes() {
        assert_eq!(
            sanitize_mnemonic_input("1. abandon 2. ability 3. able"),
            "abandon ability able",
        );
    }

    #[test]
    fn lowercases_uppercase_input() {
        assert_eq!(
            sanitize_mnemonic_input("ABANDON ABILITY"),
            "abandon ability"
        );
    }

    #[test]
    fn collapses_mixed_whitespace() {
        assert_eq!(
            sanitize_mnemonic_input("abandon\r\nability\table   legal"),
            "abandon ability able legal",
        );
    }

    #[test]
    fn fullwidth_decomposes_to_ascii() {
        // U+FF41 FULLWIDTH LATIN SMALL LETTER A etc. NFKD maps them to ASCII.
        assert_eq!(
            sanitize_mnemonic_input("\u{FF41}\u{FF42}\u{FF41}\u{FF4E}\u{FF44}\u{FF4F}\u{FF4E}"),
            "abandon"
        );
    }

    #[test]
    fn combining_marks_are_stripped() {
        // "á bility" where á = U+0061 U+0301 (a + combining acute).
        // NFKD keeps the ASCII 'a' and emits the standalone combining mark,
        // which the filter then drops.
        let input = "a\u{0301} bility";
        assert_eq!(sanitize_mnemonic_input(input), "a bility");
    }

    #[test]
    fn cyrillic_homoglyphs_are_dropped_not_substituted() {
        // U+0430 CYRILLIC SMALL LETTER A looks like ASCII 'a' but is not.
        // The sanitizer must drop it, not silently substitute — a wrong word
        // will then fail BIP-39 parsing rather than derive a different seed.
        let input = "\u{0430} bandon";
        // The Cyrillic 'a' is removed, leaving "bandon" which will fail
        // BIP-39 word-list parsing downstream. The sanitizer itself is not
        // responsible for that check — it just produces clean ASCII.
        assert_eq!(sanitize_mnemonic_input(input), "bandon");
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert_eq!(sanitize_mnemonic_input(""), "");
    }

    #[test]
    fn all_digits_input_yields_empty_output() {
        assert_eq!(sanitize_mnemonic_input("12345 6789"), "");
    }

    #[test]
    fn preserves_multi_word_phrase() {
        let phrase =
            "abandon ability able about above absent absorb abstract absurd abuse access accident";
        assert_eq!(sanitize_mnemonic_input(phrase), phrase);
    }

    #[test]
    fn strips_leading_and_trailing_whitespace() {
        assert_eq!(
            sanitize_mnemonic_input("  \n\t abandon ability  \r\n"),
            "abandon ability",
        );
    }

    #[test]
    fn drops_punctuation_and_symbols() {
        assert_eq!(
            sanitize_mnemonic_input("abandon, ability! able? legal."),
            "abandon ability able legal",
        );
    }
}
