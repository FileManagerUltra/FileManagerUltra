//! Per-format record header parsing and timestamp handling. Pure and allocation-light:
//! every function takes one line and returns what it found.

use std::sync::OnceLock;

use regex::Regex;

use super::{Level, LogFormat};

/// What a header line yielded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Header {
    pub ts: Option<i64>,
    pub level: Option<Level>,
    pub status: Option<u16>,
    pub fields: Vec<String>,
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> Option<&'static Regex> {
    if cell.get().is_none() {
        let r = Regex::new(pat).ok()?;
        let _ = cell.set(r);
    }
    cell.get()
}

pub fn civil_to_unix(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64, offset: i64) -> i64 {
    // Howard Hinnant's days_from_civil.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + h * 3600 + mi * 60 + s - offset
}

fn month(name: &str) -> Option<i64> {
    const M: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let l = name.get(..3)?.to_ascii_lowercase();
    M.iter().position(|m| *m == l).map(|i| i as i64 + 1)
}

fn num(s: &str) -> Option<i64> {
    s.parse().ok()
}

/// `+0200`, `-07:00`, `Z` → seconds east of UTC.
fn tz(s: &str) -> i64 {
    let s = s.trim();
    let (sign, rest) = match s.as_bytes().first() {
        Some(b'+') => (1, &s[1..]),
        Some(b'-') => (-1, &s[1..]),
        _ => return 0,
    };
    let digits: String = rest.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 4 {
        return 0;
    }
    let (h, m) = (
        num(&digits[..2]).unwrap_or(0),
        num(&digits[2..4]).unwrap_or(0),
    );
    sign * (h * 3600 + m * 60)
}

/// `10/Oct/2000:13:55:36 -0700`
pub fn parse_clf(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once(':')?;
    let mut d = date.split('/');
    let (day, mon, year) = (num(d.next()?)?, month(d.next()?)?, num(d.next()?)?);
    let (time, zone) = rest.split_once(' ').unwrap_or((rest, ""));
    let mut t = time.split(':');
    let (h, mi, sec) = (num(t.next()?)?, num(t.next()?)?, num(t.next()?)?);
    Some(civil_to_unix(year, mon, day, h, mi, sec, tz(zone)))
}

/// `2024-05-01 12:00:00`, `2024-05-01T12:00:00.123+02:00`, `2024/05/01 12:00:00`.
pub fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || !matches!(b[4], b'-' | b'/') {
        return None;
    }
    let (y, mo, d) = (num(s.get(0..4)?)?, num(s.get(5..7)?)?, num(s.get(8..10)?)?);
    let (h, mi, sec) = (
        num(s.get(11..13)?)?,
        num(s.get(14..16)?)?,
        num(s.get(17..19)?)?,
    );
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 24 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = s.get(19..)?;
    if rest.starts_with(['.', ',']) {
        rest = rest
            .trim_start_matches(['.', ','])
            .trim_start_matches(|c: char| c.is_ascii_digit());
    }
    Some(civil_to_unix(y, mo, d, h, mi, sec, tz(rest)))
}

/// Apache 2.4 `Fri Sep 09 10:42:29.902022 2011`.
fn parse_apache_err(s: &str) -> Option<i64> {
    let mut p = s.split_whitespace();
    p.next()?;
    let mon = month(p.next()?)?;
    let day = num(p.next()?)?;
    let time = p.next()?;
    let year = num(p.next()?)?;
    let mut t = time.split(['.', ':']);
    Some(civil_to_unix(
        year,
        mon,
        day,
        num(t.next()?)?,
        num(t.next()?)?,
        num(t.next()?)?,
        0,
    ))
}

/// PHP `09-Sep-2011 10:42:29 UTC`.
fn parse_php(s: &str) -> Option<i64> {
    let (date, time) = s.split_once(' ')?;
    let mut d = date.split('-');
    let (day, mon, year) = (num(d.next()?)?, month(d.next()?)?, num(d.next()?)?);
    let mut t = time.split(' ').next()?.split(':');
    Some(civil_to_unix(
        year,
        mon,
        day,
        num(t.next()?)?,
        num(t.next()?)?,
        num(t.next()?)?,
        0,
    ))
}

pub fn level_from_word(w: &str) -> Option<Level> {
    let w = w
        .trim_matches(|c: char| !c.is_ascii_alphabetic())
        .to_ascii_lowercase();
    Some(match w.as_str() {
        "debug" | "trace" | "dbg" => Level::Debug,
        "info" | "information" | "informational" => Level::Info,
        "notice" | "deprecated" | "strict" => Level::Notice,
        "warn" | "warning" => Level::Warning,
        "error" | "err" | "fatal" | "parse" => Level::Error,
        "crit" | "critical" | "alert" | "emerg" | "emergency" | "severe" => Level::Critical,
        _ => return None,
    })
}

fn status_level(status: u16) -> Level {
    match status {
        500.. => Level::Error,
        400..=499 => Level::Warning,
        _ => Level::Info,
    }
}

/// Parse `line` as a header of `format`; `None` means it continues the previous record.
pub fn header(format: LogFormat, line: &str) -> Option<Header> {
    static ACCESS: OnceLock<Regex> = OnceLock::new();
    static NGX_ERR: OnceLock<Regex> = OnceLock::new();
    static APA_ERR: OnceLock<Regex> = OnceLock::new();
    static MONO: OnceLock<Regex> = OnceLock::new();
    static PHP: OnceLock<Regex> = OnceLock::new();
    static GENERIC_TIME: OnceLock<Regex> = OnceLock::new();
    match format {
        LogFormat::NginxAccess | LogFormat::ApacheAccess => {
            let r = re(
                &ACCESS,
                r#"^(\S+) \S+ \S+ \[([^\]]+)\] "([A-Za-z]+) (\S*)[^"]*" (\d{3}) (\d+|-)"#,
            )?;
            let c = r.captures(line)?;
            let status: u16 = c[5].parse().ok()?;
            Some(Header {
                ts: parse_clf(&c[2]),
                level: Some(status_level(status)),
                status: Some(status),
                fields: vec![
                    c[2].into(),
                    c[1].into(),
                    c[3].into(),
                    c[4].into(),
                    c[5].into(),
                    c[6].into(),
                ],
            })
        }
        LogFormat::NginxError => {
            let r = re(&NGX_ERR, r"^(\d{4}/\d\d/\d\d \d\d:\d\d:\d\d) \[(\w+)\] ")?;
            let c = r.captures(line)?;
            Some(Header {
                ts: parse_iso(&c[1]),
                level: level_from_word(&c[2]),
                ..Header::default()
            })
        }
        LogFormat::ApacheError => {
            let r = re(
                &APA_ERR,
                r"^\[([A-Za-z]{3} [A-Za-z]{3} +\d+ [\d:.]+ \d{4})\] \[([^\]]+)\]",
            )?;
            let c = r.captures(line)?;
            let lvl = c[2].rsplit(':').next().unwrap_or("");
            Some(Header {
                ts: parse_apache_err(&c[1]),
                level: level_from_word(lvl),
                ..Header::default()
            })
        }
        LogFormat::Monolog => {
            let r = re(
                &MONO,
                r"^\[?(\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(?:[.,]\d+)?(?:Z|[+-]\d\d:?\d\d)?)\]? +[\w.\-]+\.(DEBUG|INFO|NOTICE|WARNING|ERROR|CRITICAL|ALERT|EMERGENCY)\b",
            )?;
            let c = r.captures(line)?;
            Some(Header {
                ts: parse_iso(&c[1]),
                level: level_from_word(&c[2]),
                ..Header::default()
            })
        }
        LogFormat::Php => {
            let r = re(
                &PHP,
                r"^\[(\d\d-[A-Za-z]{3}-\d{4} \d\d:\d\d:\d\d)(?: [^\]]+)?\] (.*)",
            )?;
            let c = r.captures(line)?;
            let msg = c[2].trim_start_matches("PHP ");
            Some(Header {
                ts: parse_php(&c[1]),
                level: msg.split_whitespace().next().and_then(level_from_word),
                ..Header::default()
            })
        }
        LogFormat::MagentoReport => None,
        LogFormat::Generic => {
            let first = line.chars().next()?;
            if first.is_whitespace() || line.starts_with("Stack trace") || line.starts_with('#') {
                return None;
            }
            let level = line
                .split(|c: char| !c.is_ascii_alphabetic())
                .filter(|word| !word.is_empty())
                .take(8)
                .find_map(|w| {
                    (w.len() >= 3 && w.chars().all(|c| c.is_ascii_uppercase()))
                        .then(|| level_from_word(w))
                        .flatten()
                });
            Some(Header {
                ts: parse_iso(line).or_else(|| {
                    let timestamp =
                        re(&GENERIC_TIME, r"\b\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d")?.find(line)?;
                    parse_iso(line.get(timestamp.start()..)?)
                }),
                level,
                ..Header::default()
            })
        }
    }
}

/// First format whose header matches `line`.
pub fn detect_line(line: &str) -> Option<LogFormat> {
    [
        LogFormat::NginxAccess,
        LogFormat::NginxError,
        LogFormat::ApacheError,
        LogFormat::Monolog,
        LogFormat::Php,
    ]
    .into_iter()
    .find(|f| header(*f, line).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(parse_clf("10/Oct/2000:13:55:36 -0700"), Some(971_211_336));
        assert_eq!(parse_iso("2000-10-10 20:55:36"), Some(971_211_336));
        assert_eq!(parse_iso("2000-10-10T13:55:36-07:00"), Some(971_211_336));
        assert_eq!(parse_iso("2000/10/10 20:55:36"), Some(971_211_336));
        assert_eq!(
            parse_apache_err("Tue Oct 10 20:55:36.123456 2000"),
            Some(971_211_336)
        );
        assert_eq!(parse_php("10-Oct-2000 20:55:36 UTC"), Some(971_211_336));
        assert_eq!(parse_clf("garbage"), None);
        assert_eq!(parse_iso("2000-13-40 99:99:99"), None);
    }

    #[test]
    fn access_line() {
        let l = r#"127.0.0.1 - frank [10/Oct/2000:13:55:36 -0700] "GET /a.gif HTTP/1.0" 502 2326 "-" "ua""#;
        let h = header(LogFormat::NginxAccess, l).expect("header");
        assert_eq!(h.status, Some(502));
        assert_eq!(h.level, Some(Level::Error));
        assert_eq!(h.fields[2], "GET");
        assert_eq!(h.fields[3], "/a.gif");
        assert_eq!(detect_line(l), Some(LogFormat::NginxAccess));
    }

    #[test]
    fn error_lines() {
        let n = "2024/05/01 10:00:00 [error] 12#12: *1 connect() failed";
        assert_eq!(
            header(LogFormat::NginxError, n).and_then(|h| h.level),
            Some(Level::Error)
        );
        let a = "[Fri Sep 09 10:42:29.902022 2011] [core:crit] [pid 1:tid 2] AH00 boom";
        assert_eq!(
            header(LogFormat::ApacheError, a).and_then(|h| h.level),
            Some(Level::Critical)
        );
        let m = "[2024-05-01T10:00:00.123456+00:00] main.ERROR: Oops {\"a\":1} []";
        assert_eq!(
            header(LogFormat::Monolog, m).and_then(|h| h.level),
            Some(Level::Error)
        );
        let m2 = "[2024-05-01 10:00:00] report.CRITICAL: x";
        assert_eq!(detect_line(m2), Some(LogFormat::Monolog));
        let p = "[09-Sep-2011 10:42:29 UTC] PHP Fatal error:  Uncaught x";
        assert_eq!(
            header(LogFormat::Php, p).and_then(|h| h.level),
            Some(Level::Error)
        );
        let f = "[09-Sep-2011 10:42:29] NOTICE: fpm is running";
        assert_eq!(
            header(LogFormat::Php, f).and_then(|h| h.level),
            Some(Level::Notice)
        );
    }

    #[test]
    fn continuation_lines_have_no_header() {
        assert!(header(LogFormat::Monolog, "#0 /var/www/a.php(3): foo()").is_none());
        assert!(header(LogFormat::Generic, "    at x.y(Z.java:1)").is_none());
        assert!(
            header(LogFormat::Generic, "ERROR: bad thing")
                .is_some_and(|h| h.level == Some(Level::Error))
        );
    }
}
