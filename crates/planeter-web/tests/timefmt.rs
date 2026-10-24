//! Dates as the pages and the API's messages write them: ISO 8601, UTC, no locale tables.

use planeter_web::timefmt::{date, minute};

#[test]
fn a_unix_time_is_written_as_an_iso_date_and_minute_in_utc() {
    // Known instants, from the calendar: the epoch, a leap day, the end of a year, the year 2100 (not a leap year).
    for (unix, d, m) in [
        (0, "1970-01-01", "1970-01-01 00:00 UTC"),
        (86_399, "1970-01-01", "1970-01-01 23:59 UTC"),
        (86_400, "1970-01-02", "1970-01-02 00:00 UTC"),
        (951_782_400, "2000-02-29", "2000-02-29 00:00 UTC"),
        (
            1_709_164_800 + 3 * 3600 + 7 * 60 + 59,
            "2024-02-29",
            "2024-02-29 03:07 UTC",
        ),
        (1_767_225_599, "2025-12-31", "2025-12-31 23:59 UTC"),
        (1_767_225_600, "2026-01-01", "2026-01-01 00:00 UTC"),
        (4_107_542_400, "2100-03-01", "2100-03-01 00:00 UTC"),
    ] {
        assert_eq!(date(unix), d, "{unix}");
        assert_eq!(minute(unix), m, "{unix}");
    }
}

#[test]
fn a_time_beyond_the_calendar_does_not_panic() {
    // The largest time a record can hold is written with a very long year, not refused and not wrapped.
    assert!(date(u64::MAX).ends_with("-11-09"));
    assert!(minute(u64::MAX).ends_with(" UTC"));
}

#[test]
fn consecutive_days_are_consecutive_dates_for_four_hundred_years() {
    // Walk every day from 1970 to 2370 and compare with a count of the days in each month.
    let (mut y, mut m, mut d) = (1970u32, 1u32, 1u32);
    for day in 0..(400 * 366u64) {
        let want = format!("{y:04}-{m:02}-{d:02}");
        assert_eq!(date(day * 86_400 + 43_200), want, "day {day}");
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let last = match m {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        d += 1;
        if d > last {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
    }
}
