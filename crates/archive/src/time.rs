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

/// `SystemTime` to UTC `(year, month, day, hour, minute, second)`.
pub fn to_civil(t: SystemTime) -> (i64, u8, u8, u8, u8, u8) {
    let secs = match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX),
    };
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let narrow = |v: i64| u8::try_from(v).unwrap_or(0);
    (
        year,
        narrow(month),
        narrow(day),
        narrow(rem / 3600),
        narrow(rem % 3600 / 60),
        narrow(rem % 60),
    )
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
        assert_eq!(to_civil(t), (2026, 11, 23, 12, 0, 0));
        assert_eq!(to_civil(SystemTime::UNIX_EPOCH), (1970, 1, 1, 0, 0, 0));
    }
}
