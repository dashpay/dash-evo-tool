//! Wall-clock helpers shared across layers.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch, saturating to `0` on a pre-epoch clock.
///
/// The single spelling of "now in millis": journal timestamps, schedule
/// deadlines and proved-snapshot freshness are all compared against each other,
/// so they must not drift apart on how a bad clock is handled.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
