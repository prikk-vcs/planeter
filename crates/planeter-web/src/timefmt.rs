//! Dates and times as the pages and the API's messages write them: ISO 8601 in UTC, no locale tables and no time zone
//! database (a page that must name a zone names UTC). `civil_from_days` is Howard Hinnant's public-domain algorithm.

/// `(year, month, day)` of a count of days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `2026-10-24` for a unix time in seconds.
pub fn date(unix: u64) -> String {
    let days = i64::try_from(unix / 86_400).unwrap_or(i64::MAX / 86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `2026-10-24 14:05 UTC` for a unix time in seconds.
pub fn minute(unix: u64) -> String {
    let secs = unix % 86_400;
    format!(
        "{} {:02}:{:02} UTC",
        date(unix),
        secs / 3600,
        (secs % 3600) / 60
    )
}
