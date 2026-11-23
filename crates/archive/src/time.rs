//! Calendar times from archive headers (zip's DOS dates, ISO records) without a date crate.

use std::time::{Duration, SystemTime};

/// UTC civil date and time to `SystemTime`; `None` for impossible dates or times before
/// 1970 that the platform cannot represent.
pub fn from_civil(
    year: i64,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
) -> Option<SystemTime> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs =
        days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second.min(59));
    from_unix(secs)
}

/// Seconds since 1970 (negative allowed) to `SystemTime`.
pub fn from_unix(secs: i64) -> Option<SystemTime> {
    if secs >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(secs.unsigned_abs()))
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(Duration::from_secs(secs.unsigned_abs()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates() {
        let t = from_civil(2026, 11, 23, 12, 0, 0).expect("t");
        let secs = t
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("after")
            .as_secs();
        assert_eq!(secs, 1_795_435_200);
        assert_eq!(
            from_civil(1970, 1, 1, 0, 0, 0),
            Some(SystemTime::UNIX_EPOCH)
        );
        assert!(from_civil(2026, 13, 1, 0, 0, 0).is_none());
    }
}
