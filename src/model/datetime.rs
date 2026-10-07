//! The one place that turns an instant into text and a typed date into an instant.
//!
//! Stored instants are UTC (Unix seconds or milliseconds) and stay that way.
//! Whatever a person reads or types is wall-clock time in their own time zone,
//! written in locale-neutral ISO style. Functions ending in `_in` take the zone
//! as an argument, so conversions are tested without the host's zone; the
//! `local_*` functions apply the host's zone.

use std::ops::RangeInclusive;

use chrono::{
    DateTime, Datelike, FixedOffset, Local, MappedLocalTime, NaiveDate, TimeZone, Timelike, Utc,
};

const DATE_FORMAT: &str = "%Y-%m-%d";
const DATE_TIME_FORMAT: &str = "%Y-%m-%d %H:%M";
const DATE_TIME_SECONDS_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
const UTC_OFFSET_FORMAT: &str = "UTC%:z";

/// Years a zone is asked about: Unix time starts in 1970 and ISO dates have
/// four digits. `chrono::Local` unwraps the platform's zone lookup, so a
/// far-off year in a timestamp from the network must not reach it.
const ZONED_YEARS: RangeInclusive<i32> = 1970..=9999;

/// The calendar date and time-of-day fields a person edits, in some time zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallClock {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// `0..=23`.
    pub hour: u32,
    /// `0..=59`.
    pub minute: u32,
}

/// The instant `ms` Unix milliseconds after the epoch, or `None` past the
/// calendar's range.
pub fn instant_from_unix_millis(ms: u64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(i64::try_from(ms).ok()?)
}

/// The instant `secs` Unix seconds after the epoch, or `None` past the
/// calendar's range.
pub fn instant_from_unix_secs(secs: u64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(i64::try_from(secs).ok()?, 0)
}

/// Unix milliseconds of `instant`, or `None` before 1970.
pub fn unix_millis(instant: DateTime<Utc>) -> Option<u64> {
    u64::try_from(instant.timestamp_millis()).ok()
}

/// `instant` on the clocks of `tz`; UTC clocks stand in outside [`ZONED_YEARS`].
fn on_clocks_of<Tz: TimeZone>(instant: DateTime<Utc>, tz: &Tz) -> DateTime<FixedOffset> {
    if ZONED_YEARS.contains(&instant.year()) {
        instant.with_timezone(tz).fixed_offset()
    } else {
        instant.fixed_offset()
    }
}

/// `instant` as a calendar date in `tz`: `2026-10-05`.
pub fn date_in<Tz: TimeZone>(instant: DateTime<Utc>, tz: &Tz) -> String {
    on_clocks_of(instant, tz).format(DATE_FORMAT).to_string()
}

/// `instant` to the minute in `tz`: `2026-10-05 14:00`.
pub fn date_time_in<Tz: TimeZone>(instant: DateTime<Utc>, tz: &Tz) -> String {
    on_clocks_of(instant, tz)
        .format(DATE_TIME_FORMAT)
        .to_string()
}

/// `instant` to the second in `tz`: `2026-10-05 14:00:59`.
pub fn date_time_seconds_in<Tz: TimeZone>(instant: DateTime<Utc>, tz: &Tz) -> String {
    on_clocks_of(instant, tz)
        .format(DATE_TIME_SECONDS_FORMAT)
        .to_string()
}

/// How far `tz` is from UTC at `instant`: `UTC+02:00`.
pub fn utc_offset_in<Tz: TimeZone>(instant: DateTime<Utc>, tz: &Tz) -> String {
    on_clocks_of(instant, tz)
        .format(UTC_OFFSET_FORMAT)
        .to_string()
}

/// The fields that show `instant` in `tz`. Seconds are dropped.
pub fn wall_clock_in<Tz: TimeZone>(instant: DateTime<Utc>, tz: &Tz) -> WallClock {
    let shown = on_clocks_of(instant, tz);
    WallClock {
        date: shown.format(DATE_FORMAT).to_string(),
        hour: shown.hour(),
        minute: shown.minute(),
    }
}

/// The instant at which a clock in `tz` shows `date` (`YYYY-MM-DD`), `hour`
/// and `minute`.
///
/// `None` when the fields are not a real date and time between 1970 and 9999,
/// or name a time the zone's clocks skip when they go forward. A time the
/// clocks show twice, when they go back, is read as the earlier of the two
/// instants.
pub fn instant_from_wall_clock_in<Tz: TimeZone>(
    date: &str,
    hour: u32,
    minute: u32,
    tz: &Tz,
) -> Option<DateTime<Utc>> {
    let shown = NaiveDate::parse_from_str(date, DATE_FORMAT)
        .ok()
        .filter(|date| ZONED_YEARS.contains(&date.year()))?
        .and_hms_opt(hour, minute, 0)?;
    // Compared as instants: `chrono` zones differ on which of the two they list first.
    let instant = match tz.from_local_datetime(&shown) {
        MappedLocalTime::Single(instant) => instant,
        MappedLocalTime::Ambiguous(one, other) => one.min(other),
        MappedLocalTime::None => return None,
    };
    Some(instant.with_timezone(&Utc))
}

/// [`date_in`] the user's time zone.
pub fn local_date(instant: DateTime<Utc>) -> String {
    date_in(instant, &Local)
}

/// [`date_time_in`] the user's time zone.
pub fn local_date_time(instant: DateTime<Utc>) -> String {
    date_time_in(instant, &Local)
}

/// [`date_time_seconds_in`] the user's time zone.
pub fn local_date_time_seconds(instant: DateTime<Utc>) -> String {
    date_time_seconds_in(instant, &Local)
}

/// [`utc_offset_in`] the user's time zone.
pub fn local_utc_offset(instant: DateTime<Utc>) -> String {
    utc_offset_in(instant, &Local)
}

/// [`wall_clock_in`] the user's time zone.
pub fn local_wall_clock(instant: DateTime<Utc>) -> WallClock {
    wall_clock_in(instant, &Local)
}

/// [`instant_from_wall_clock_in`] the user's time zone.
pub fn instant_from_local_wall_clock(date: &str, hour: u32, minute: u32) -> Option<DateTime<Utc>> {
    instant_from_wall_clock_in(date, hour, minute, &Local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDateTime, NaiveTime, TimeDelta};

    fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        naive(year, month, day, hour, minute, second).and_utc()
    }

    fn naive(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, minute, second))
            .expect("valid test instant")
    }

    fn east(hours: i32) -> FixedOffset {
        FixedOffset::east_opt(hours * 3600).expect("valid test offset")
    }

    /// Central European clocks in 2031: UTC+1, and UTC+2 from the last Sunday
    /// of March to the last Sunday of October, both changes at 01:00 UTC.
    #[derive(Debug, Clone, Copy)]
    struct CentralEurope2031;

    impl CentralEurope2031 {
        fn summer_time(utc: &NaiveDateTime) -> bool {
            (naive(2031, 3, 30, 1, 0, 0)..naive(2031, 10, 26, 1, 0, 0)).contains(utc)
        }
    }

    impl TimeZone for CentralEurope2031 {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            Self
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
        }

        /// A repeated time lists its later instant first, as the host zone
        /// does on Unix for times its rule (not its table) covers.
        fn offset_from_local_datetime(
            &self,
            local: &NaiveDateTime,
        ) -> MappedLocalTime<FixedOffset> {
            let shown_at =
                |offset: FixedOffset| self.offset_from_utc_datetime(&(*local - offset)) == offset;
            match (shown_at(east(1)), shown_at(east(2))) {
                (true, true) => MappedLocalTime::Ambiguous(east(1), east(2)),
                (true, false) => MappedLocalTime::Single(east(1)),
                (false, true) => MappedLocalTime::Single(east(2)),
                (false, false) => MappedLocalTime::None,
            }
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_time(NaiveTime::MIN))
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            if Self::summer_time(utc) {
                east(2)
            } else {
                east(1)
            }
        }
    }

    #[test]
    fn an_instant_is_shown_as_the_wall_clock_of_the_zone() {
        let instant = utc(2026, 10, 5, 22, 34, 56);

        assert_eq!(date_in(instant, &east(2)), "2026-10-06");
        assert_eq!(date_time_in(instant, &east(2)), "2026-10-06 00:34");
        assert_eq!(
            date_time_seconds_in(instant, &east(2)),
            "2026-10-06 00:34:56"
        );
        assert_eq!(date_in(instant, &east(-5)), "2026-10-05");
        assert_eq!(date_time_in(instant, &east(-5)), "2026-10-05 17:34");
        assert_eq!(
            date_time_seconds_in(instant, &east(-5)),
            "2026-10-05 17:34:56"
        );
    }

    #[test]
    fn the_shown_wall_clock_follows_daylight_saving_time() {
        assert_eq!(
            date_time_in(utc(2031, 1, 15, 12, 0, 0), &CentralEurope2031),
            "2031-01-15 13:00"
        );
        assert_eq!(
            date_time_in(utc(2031, 7, 15, 12, 0, 0), &CentralEurope2031),
            "2031-07-15 14:00"
        );
    }

    #[test]
    fn the_offset_names_how_far_the_zone_is_from_utc_at_that_instant() {
        let instant = utc(2026, 10, 5, 12, 0, 0);
        assert_eq!(utc_offset_in(instant, &east(2)), "UTC+02:00");
        assert_eq!(utc_offset_in(instant, &east(0)), "UTC+00:00");
        assert_eq!(
            utc_offset_in(
                instant,
                &FixedOffset::west_opt(3 * 3600 + 1800).expect("valid test offset")
            ),
            "UTC-03:30"
        );
        assert_eq!(
            utc_offset_in(utc(2031, 1, 15, 12, 0, 0), &CentralEurope2031),
            "UTC+01:00"
        );
        assert_eq!(
            utc_offset_in(utc(2031, 7, 15, 12, 0, 0), &CentralEurope2031),
            "UTC+02:00"
        );
    }

    #[test]
    fn a_typed_time_is_read_as_the_wall_clock_of_the_zone() {
        assert_eq!(
            instant_from_wall_clock_in("2031-02-03", 4, 5, &east(2)),
            Some(utc(2031, 2, 3, 2, 5, 0))
        );
        assert_eq!(
            instant_from_wall_clock_in("2031-02-03", 4, 5, &east(-5)),
            Some(utc(2031, 2, 3, 9, 5, 0))
        );
        assert_eq!(
            instant_from_wall_clock_in("2031-07-15", 14, 0, &CentralEurope2031),
            Some(utc(2031, 7, 15, 12, 0, 0))
        );
        assert_eq!(
            instant_from_wall_clock_in("2031-01-15", 13, 0, &CentralEurope2031),
            Some(utc(2031, 1, 15, 12, 0, 0))
        );
    }

    #[test]
    fn the_fields_shown_for_an_instant_read_back_as_that_instant() {
        let instant = utc(2031, 2, 3, 23, 45, 0);
        for hours in [-11, -5, 0, 2, 14] {
            let zone = east(hours);
            let fields = wall_clock_in(instant, &zone);
            assert_eq!(
                instant_from_wall_clock_in(&fields.date, fields.hour, fields.minute, &zone),
                Some(instant),
                "UTC{hours:+}"
            );
        }
        assert_eq!(
            wall_clock_in(instant, &east(2)),
            WallClock {
                date: "2031-02-04".to_owned(),
                hour: 1,
                minute: 45,
            }
        );
    }

    #[test]
    fn the_shown_fields_drop_seconds() {
        let fields = wall_clock_in(utc(2031, 2, 3, 4, 5, 37), &east(2));

        assert_eq!(
            instant_from_wall_clock_in(&fields.date, fields.hour, fields.minute, &east(2)),
            Some(utc(2031, 2, 3, 4, 5, 0))
        );
    }

    #[test]
    fn fields_that_are_not_a_real_date_and_time_have_no_instant() {
        let zone = east(2);
        assert_eq!(instant_from_wall_clock_in("2026-02-30", 12, 0, &zone), None);
        assert_eq!(instant_from_wall_clock_in("2026-09-08", 24, 0, &zone), None);
        assert_eq!(
            instant_from_wall_clock_in("2026-09-08", 12, 60, &zone),
            None
        );
        assert_eq!(instant_from_wall_clock_in("08/09/2026", 12, 0, &zone), None);
        assert_eq!(instant_from_wall_clock_in("", 12, 0, &zone), None);
        assert!(instant_from_wall_clock_in("2028-02-29", 23, 59, &zone).is_some());
    }

    /// Clocks go from 02:00 straight to 03:00 on 2031-03-30: 02:30 never shows.
    #[test]
    fn a_time_the_clocks_skip_has_no_instant() {
        let zone = CentralEurope2031;
        assert_eq!(instant_from_wall_clock_in("2031-03-30", 2, 0, &zone), None);
        assert_eq!(instant_from_wall_clock_in("2031-03-30", 2, 30, &zone), None);
        assert_eq!(
            instant_from_wall_clock_in("2031-03-30", 1, 59, &zone),
            Some(utc(2031, 3, 30, 0, 59, 0))
        );
        assert_eq!(
            instant_from_wall_clock_in("2031-03-30", 3, 0, &zone),
            Some(utc(2031, 3, 30, 1, 0, 0))
        );
    }

    /// Clocks go from 03:00 back to 02:00 on 2031-10-26: 02:30 shows twice.
    #[test]
    fn a_time_the_clocks_show_twice_is_the_earlier_instant() {
        let zone = CentralEurope2031;
        let earlier = utc(2031, 10, 26, 0, 30, 0);
        let later = earlier + TimeDelta::hours(1);
        assert_eq!(date_time_in(earlier, &zone), "2031-10-26 02:30");
        assert_eq!(date_time_in(later, &zone), "2031-10-26 02:30");

        assert_eq!(
            instant_from_wall_clock_in("2031-10-26", 2, 30, &zone),
            Some(earlier)
        );
        assert_eq!(
            instant_from_wall_clock_in("2031-10-26", 2, 0, &zone),
            Some(utc(2031, 10, 26, 0, 0, 0))
        );
        assert_eq!(
            instant_from_wall_clock_in("2031-10-26", 3, 0, &zone),
            Some(utc(2031, 10, 26, 2, 0, 0))
        );
    }

    #[test]
    fn unix_time_converts_to_an_instant_and_back() {
        assert_eq!(
            instant_from_unix_secs(1_700_000_000),
            Some(utc(2023, 11, 14, 22, 13, 20))
        );
        assert_eq!(
            instant_from_unix_millis(1_700_000_000_123),
            Some(utc(2023, 11, 14, 22, 13, 20) + TimeDelta::milliseconds(123))
        );
        assert_eq!(instant_from_unix_millis(0), Some(DateTime::UNIX_EPOCH));
        assert_eq!(instant_from_unix_millis(u64::MAX), None);
        assert_eq!(instant_from_unix_secs(u64::MAX), None);

        assert_eq!(
            unix_millis(utc(2023, 11, 14, 22, 13, 20)),
            Some(1_700_000_000_000)
        );
        assert_eq!(unix_millis(utc(1960, 1, 1, 12, 0, 0)), None);
    }

    /// Timestamps arrive from the network, so the far ends of the calendar
    /// must format instead of panicking, in the host zone too.
    #[test]
    fn the_ends_of_the_calendar_are_shown_without_panicking() {
        for instant in [DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC] {
            for zone in [east(-12), east(14)] {
                assert!(!date_in(instant, &zone).is_empty());
                assert!(!date_time_in(instant, &zone).is_empty());
                assert!(!date_time_seconds_in(instant, &zone).is_empty());
                assert!(!wall_clock_in(instant, &zone).date.is_empty());
            }
            assert!(!local_date(instant).is_empty());
            assert!(!local_date_time(instant).is_empty());
            assert!(!local_date_time_seconds(instant).is_empty());
            assert!(!local_wall_clock(instant).date.is_empty());
            assert_eq!(local_utc_offset(instant), "UTC+00:00");
        }
    }

    #[test]
    fn years_no_zone_is_asked_about_show_utc_clocks() {
        assert_eq!(
            date_time_in(utc(12_000, 1, 1, 0, 0, 0), &east(2)),
            "+12000-01-01 00:00"
        );
        assert_eq!(
            utc_offset_in(utc(12_000, 1, 1, 0, 0, 0), &east(2)),
            "UTC+00:00"
        );
        assert_eq!(
            date_time_in(utc(9999, 12, 31, 12, 0, 0), &east(2)),
            "9999-12-31 14:00"
        );
        assert_eq!(
            date_time_in(DateTime::UNIX_EPOCH, &east(-5)),
            "1969-12-31 19:00"
        );
    }

    #[test]
    fn a_typed_year_no_zone_is_asked_about_has_no_instant() {
        let zone = east(2);
        assert_eq!(instant_from_wall_clock_in("1969-12-31", 12, 0, &zone), None);
        assert_eq!(
            instant_from_wall_clock_in("+10000-01-01", 12, 0, &zone),
            None
        );
        assert!(instant_from_wall_clock_in("1970-01-01", 12, 0, &zone).is_some());
        assert!(instant_from_wall_clock_in("9999-12-31", 12, 0, &zone).is_some());
    }

    /// Holds in every host zone: the instant is mid-winter and mid-summer,
    /// away from any clock change.
    #[test]
    fn the_host_zone_reads_back_the_fields_it_shows() {
        for instant in [utc(2031, 1, 15, 12, 0, 0), utc(2031, 7, 15, 12, 0, 0)] {
            let fields = local_wall_clock(instant);
            assert_eq!(
                instant_from_local_wall_clock(&fields.date, fields.hour, fields.minute),
                Some(instant)
            );
            assert_eq!(
                local_date_time(instant),
                format!(
                    "{date} {hour:02}:{minute:02}",
                    date = fields.date,
                    hour = fields.hour,
                    minute = fields.minute
                )
            );
            assert_eq!(local_date(instant), fields.date);
            assert!(local_date_time_seconds(instant).starts_with(&local_date_time(instant)));
            assert!(local_utc_offset(instant).starts_with("UTC"));
        }
    }
}
