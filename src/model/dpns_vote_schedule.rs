//! Absolute UTC schedule input, independent of the time the form is submitted.

use chrono::NaiveDate;

pub(crate) fn parse_utc_schedule(date: &str, hour: u32, minute: u32) -> Option<u64> {
    let timestamp = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(hour, minute, 0)?
        .and_utc()
        .timestamp_millis();
    u64::try_from(timestamp).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_input_rejects_invalid_dates_and_times() {
        assert_eq!(parse_utc_schedule("2026-02-30", 12, 0), None);
        assert_eq!(parse_utc_schedule("2026-09-08", 24, 0), None);
        assert_eq!(parse_utc_schedule("1960-01-01", 12, 0), None);
        assert!(parse_utc_schedule("2028-02-29", 23, 59).is_some());
    }
}
