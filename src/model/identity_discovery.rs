//! Pure, stateless decision logic for the rolling gap-limited identity scan.
//!
//! Identities are derived at wallet derivation indices that need not be
//! contiguous: a user may register identity 0, skip a few, then register at 8.
//! A fixed `0..=N` scan either misses high indices or wastes network round-trips
//! on a long empty tail. Instead the scan keeps a *rolling lookahead*: it keeps
//! probing while it is within [`IDENTITY_GAP_LIMIT`] indices of the highest
//! index that produced an identity, so each new discovery extends the window.
//!
//! This module owns only the stop/continue arithmetic so it can be unit-tested
//! without a wallet, an SDK, or the network. The async scan that calls it lives
//! in `backend_task/identity` (DET Module Placement Policy: pure decision logic
//! in `model/`, async business logic in `backend_task/`).

/// Number of consecutive empty trailing indices the scan keeps probing past the
/// highest index that produced an identity. A higher value finds identities
/// registered after larger gaps at the cost of more network round-trips.
pub const IDENTITY_GAP_LIMIT: u32 = 5;

/// Absolute ceiling on the number of indices a single scan will probe,
/// regardless of the rolling window. Bounds the network fan-out if a corrupt
/// cache or a hostile node keeps reporting "found" at every index, so a
/// background sweep can never issue unbounded fetches.
pub const IDENTITY_SCAN_HARD_CAP: u32 = 100;

/// Highest wallet derivation index a user may type into the By-Wallet search to
/// seed the rolling scan. The scan cannot probe past [`IDENTITY_SCAN_HARD_CAP`]
/// regardless, so seeding beyond it is always a typo; rejecting it keeps a
/// fat-finger from launching a full hard-cap-deep scan.
pub const MAX_IDENTITY_SEARCH_INDEX: u32 = IDENTITY_SCAN_HARD_CAP - 1;

/// Validate a user-typed By-Wallet search index. Pure (no `AppContext`/DB), so
/// the UI can call it for instant feedback while the single source of truth
/// lives in `model/`.
pub fn validate_search_index(index: u32) -> Result<u32, IdentitySearchIndexError> {
    if index > MAX_IDENTITY_SEARCH_INDEX {
        Err(IdentitySearchIndexError::TooLarge {
            max: MAX_IDENTITY_SEARCH_INDEX,
        })
    } else {
        Ok(index)
    }
}

/// A By-Wallet search index failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdentitySearchIndexError {
    /// The index exceeds [`MAX_IDENTITY_SEARCH_INDEX`].
    #[error("Enter an identity index between 0 and {max}, then try again.")]
    TooLarge { max: u32 },
}

/// Decide whether the rolling gap-limited scan should probe `current_index`.
///
/// `highest_found` is the highest index that has produced an identity so far in
/// this scan (`None` if nothing has been found yet).
///
/// - With no hit yet (`None`), probing continues while
///   `current_index < IDENTITY_GAP_LIMIT` — i.e. indices `0..IDENTITY_GAP_LIMIT`.
/// - With a hit at `h` (`Some(h)`), probing continues while
///   `current_index <= h + IDENTITY_GAP_LIMIT`, so each new discovery rolls the
///   window forward by extending `h`.
///
/// In every case probing stops once `current_index` reaches
/// [`IDENTITY_SCAN_HARD_CAP`], so the scan is bounded even if `highest_found`
/// keeps advancing.
pub fn should_continue_scan(current_index: u32, highest_found: Option<u32>) -> bool {
    if current_index >= IDENTITY_SCAN_HARD_CAP {
        return false;
    }
    match highest_found {
        None => current_index < IDENTITY_GAP_LIMIT,
        Some(highest) => current_index <= highest.saturating_add(IDENTITY_GAP_LIMIT),
    }
}

/// Outcome counts of one gap-limited discovery pass over a single wallet.
///
/// `found` is the number of indices that resolved to an on-chain identity;
/// `stored` is how many of those were newly inserted or refreshed in the local
/// database. A wallet skipped because it was locked reports zero of each.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiscoverySummary {
    /// Indices that resolved to an on-chain identity during this pass.
    pub found: u32,
    /// Identities newly stored or refreshed in the local database.
    pub stored: u32,
    /// Identities left unloaded because the user had unloaded them. Counted
    /// separately from the rest of `found - stored` so the user can be told a
    /// deliberate unload — not a failure — is why the scan loaded fewer
    /// identities than it found.
    pub skipped_forgotten: u32,
    /// Identities found on the network that could not be stored on this device.
    /// Without this count a scan whose every write failed is indistinguishable
    /// from one that simply found nothing new.
    pub failed: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect every index a scan would probe, given the indices at which an
    /// identity is "found". Mirrors the real loop: start at 0, advance by 1,
    /// extend `highest_found` whenever the current index is a hit.
    fn probed_indices(hits: &[u32]) -> Vec<u32> {
        let mut probed = Vec::new();
        let mut highest_found: Option<u32> = None;
        let mut current = 0u32;
        while should_continue_scan(current, highest_found) {
            probed.push(current);
            if hits.contains(&current) {
                highest_found = Some(highest_found.map_or(current, |h| h.max(current)));
            }
            current += 1;
        }
        probed
    }

    // ── Decision-table tests (the design's `should_continue_scan` checklist) ──
    // These probe the pure function directly with an established `highest_found`,
    // independent of whether a cold scan could organically reach that index.

    #[test]
    fn no_hit_window_is_zero_until_gap_limit() {
        // With nothing found, continue for 0..GAP and stop at GAP.
        for i in 0..IDENTITY_GAP_LIMIT {
            assert!(should_continue_scan(i, None), "index {i} should continue");
        }
        assert!(!should_continue_scan(IDENTITY_GAP_LIMIT, None));
    }

    #[test]
    fn hit_at_zero_extends_to_five() {
        // Found at 0 ⇒ probe through 0+5=5 inclusive, stop at 6.
        assert!(should_continue_scan(5, Some(0)));
        assert!(!should_continue_scan(6, Some(0)));
    }

    #[test]
    fn hit_at_seven_extends_to_twelve() {
        // Found at 7 ⇒ probe through 7+5=12 inclusive, stop at 13.
        assert!(should_continue_scan(12, Some(7)));
        assert!(!should_continue_scan(13, Some(7)));
    }

    #[test]
    fn hits_at_zero_and_twelve_roll_to_seventeen() {
        // Window seeded by the latest hit at 12 ⇒ probe through 12+5=17.
        assert!(should_continue_scan(17, Some(12)));
        assert!(!should_continue_scan(18, Some(12)));
    }

    // ── Rolling-simulation tests (the loop the backend actually runs) ──────────

    #[test]
    fn empty_wallet_stops_at_gap_limit() {
        // No hits: probe 0..GAP, then stop.
        assert_eq!(probed_indices(&[]), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn hit_at_zero_scans_to_gap_limit() {
        // Found at 0 ⇒ window is 0..=0+5, probe 0..=5.
        assert_eq!(probed_indices(&[0]), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn rolling_chain_extends_window_past_static_range() {
        // A chain of hits inside each rolling window reaches index 8 — proving
        // the lookahead finds identities a static `0..=5` scan would miss.
        let probed = probed_indices(&[3, 8]);
        assert!(
            probed.contains(&8),
            "rolling window must reach the hit at 8"
        );
        assert_eq!(*probed.last().unwrap(), 13, "8+5 closes the window at 13");
    }

    #[test]
    fn hard_cap_clamps_a_pathological_always_found_scan() {
        // Every index is a hit ⇒ the rolling window never closes, but the hard
        // cap stops the scan at IDENTITY_SCAN_HARD_CAP exclusive.
        let all_hits: Vec<u32> = (0..200).collect();
        let probed = probed_indices(&all_hits);
        assert_eq!(probed.len() as u32, IDENTITY_SCAN_HARD_CAP);
        assert_eq!(*probed.last().unwrap(), IDENTITY_SCAN_HARD_CAP - 1);
    }

    #[test]
    fn stops_exactly_at_hard_cap_boundary() {
        assert!(should_continue_scan(
            IDENTITY_SCAN_HARD_CAP - 1,
            Some(u32::MAX)
        ));
        assert!(!should_continue_scan(
            IDENTITY_SCAN_HARD_CAP,
            Some(u32::MAX)
        ));
    }

    #[test]
    fn high_seed_never_overflows() {
        // A pre-existing identity at u32::MAX must not panic via overflow in the
        // `highest + GAP` arithmetic; the hard cap stops it regardless.
        assert!(!should_continue_scan(
            IDENTITY_SCAN_HARD_CAP,
            Some(u32::MAX)
        ));
    }

    #[test]
    fn validate_search_index_accepts_in_range_rejects_beyond_cap() {
        assert_eq!(validate_search_index(0), Ok(0));
        assert_eq!(
            validate_search_index(MAX_IDENTITY_SEARCH_INDEX),
            Ok(MAX_IDENTITY_SEARCH_INDEX)
        );
        assert_eq!(
            validate_search_index(MAX_IDENTITY_SEARCH_INDEX + 1),
            Err(IdentitySearchIndexError::TooLarge {
                max: MAX_IDENTITY_SEARCH_INDEX
            })
        );
        assert!(validate_search_index(u32::MAX).is_err());
    }
}
