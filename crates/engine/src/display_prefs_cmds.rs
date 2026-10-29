//! `prefs.set` — validated single-value settings of the Preferences dialog (appearance,
//! file display, confirmations, caches). One command, one key per call, bounded values.

use fm_vfs::FmError;
use serde_json::Value;

use crate::file_cmds::object;
use crate::prefs::{
    DateFormat, Density, FONT_SIZE_RANGE, PREVIEW_TIME_RANGE, Prefs, SizeUnits, THUMB_DISK_RANGE,
    THUMB_MEMORY_RANGE, parse_hex_color,
};
use crate::registry::CommandSpec;
use crate::session::Session;

const KEYS: &str = "accent (\"default\"|\"#rrggbb\"), density (compact|normal|comfortable), \
    font_size 9-24, terminal_font_size 9-24, terminal_scrollback_rows 5000-1000000, size_units (binary|decimal|bytes), \
    date_format (iso|isoseconds|us|eu), show_hidden, single_click_open, confirm_delete, \
    confirm_run (bool), thumb_memory_mib 32-2048, thumb_disk_mib 32-8192, thumb_cache_dir (\"default\"|absolute path), preview_time_secs 2-120";

fn bad(key: &str, expect: &str) -> FmError {
    FmError::InvalidParams(format!("{key} must be {expect}"))
}

fn boolean(key: &str, value: &Value) -> Result<bool, FmError> {
    value.as_bool().ok_or_else(|| bad(key, "true or false"))
}

fn font(key: &str, value: &Value) -> Result<f32, FmError> {
    let size = value
        .as_f64()
        .filter(|v| v.is_finite() && FONT_SIZE_RANGE.contains(&(*v as f32)))
        .ok_or_else(|| bad(key, "a number from 9 to 24"))?;
    Ok(((size * 2.0).round() / 2.0) as f32)
}

fn ranged(key: &str, value: &Value, range: &std::ops::RangeInclusive<u64>) -> Result<u64, FmError> {
    value
        .as_u64()
        .filter(|n| range.contains(n))
        .ok_or_else(|| bad(key, &format!("{}-{}", range.start(), range.end())))
}

fn choice<'a>(key: &str, value: &'a Value, allowed: &[&str]) -> Result<&'a str, FmError> {
    value
        .as_str()
        .filter(|v| allowed.contains(v))
        .ok_or_else(|| bad(key, &format!("one of {}", allowed.join(", "))))
}

/// Validate and apply one `key`/`value` pair.
pub fn set(s: &mut Session, key: &str, value: &Value) -> Result<(), FmError> {
    match key {
        "accent" => {
            let text = value.as_str().ok_or_else(|| bad(key, "a string"))?;
            s.prefs.accent = if text == "default" {
                None
            } else if parse_hex_color(text).is_some() {
                Some(text.to_ascii_lowercase())
            } else {
                return Err(bad(key, "\"default\" or #rrggbb"));
            };
        }
        "density" => {
            s.prefs.density = match choice(key, value, &["compact", "normal", "comfortable"])? {
                "compact" => Density::Compact,
                "comfortable" => Density::Comfortable,
                _ => Density::Normal,
            };
        }
        "font_size" => s.prefs.font_size = font(key, value)?,
        "terminal_font_size" => s.prefs.terminal_font_size = font(key, value)?,
        "terminal_scrollback_rows" => {
            s.prefs.terminal_scrollback_rows = ranged(key, value, &(5_000..=1_000_000))?;
        }
        "size_units" => {
            s.prefs.size_units = match choice(key, value, &["binary", "decimal", "bytes"])? {
                "decimal" => SizeUnits::Decimal,
                "bytes" => SizeUnits::Bytes,
                _ => SizeUnits::Binary,
            };
        }
        "date_format" => {
            s.prefs.date_format = match choice(key, value, &["iso", "isoseconds", "us", "eu"])? {
                "isoseconds" => DateFormat::IsoSeconds,
                "us" => DateFormat::Us,
                "eu" => DateFormat::Eu,
                _ => DateFormat::Iso,
            };
        }
        "show_hidden" => {
            let on = boolean(key, value)?;
            s.prefs.show_hidden = on;
            if s.show_hidden != on {
                s.dispatch("view.hidden.toggle", &Value::Null)?;
            }
        }
        "single_click_open" => s.prefs.single_click_open = boolean(key, value)?,
        "confirm_delete" => s.prefs.confirm_delete = boolean(key, value)?,
        "confirm_run" => s.prefs.confirm_run = boolean(key, value)?,
        "thumb_memory_mib" => {
            s.prefs.thumb_memory_mib = ranged(key, value, &THUMB_MEMORY_RANGE)?;
        }
        "thumb_disk_mib" => s.prefs.thumb_disk_mib = ranged(key, value, &THUMB_DISK_RANGE)?,
        "thumb_cache_dir" => {
            let text = value.as_str().ok_or_else(|| bad(key, "a string"))?.trim();
            let dir = if text == "default" || text.is_empty() {
                None
            } else if crate::prefs::valid_cache_parent(std::path::Path::new(text)) {
                Some(text.to_string())
            } else {
                return Err(bad(key, "\"default\" or an absolute folder path"));
            };
            let previous = std::mem::replace(&mut s.prefs.thumb_cache_dir, dir);
            if !s.thumbs.set_cache_dir(s.prefs.thumb_cache_path()) {
                s.prefs.thumb_cache_dir = previous;
                return Err(FmError::InvalidParams(
                    "cannot create or use that folder for thumbnails".into(),
                ));
            }
            s.thumbs.clear_memory();
        }
        "preview_time_secs" => {
            s.prefs.preview_time_secs = ranged(key, value, &PREVIEW_TIME_RANGE)?;
        }
        _ => {
            return Err(FmError::InvalidParams(format!(
                "unknown setting; use {KEYS}"
            )));
        }
    }
    apply_runtime(&s.prefs, s);
    Ok(())
}

/// Push cache limits to the services that own them.
pub fn apply_runtime(prefs: &Prefs, s: &Session) {
    s.thumbs
        .set_memory_cap((prefs.thumb_memory_mib.clamp(32, 2048) as usize) << 20);
    s.thumbs
        .set_disk_cap(prefs.thumb_disk_mib.clamp(32, 8192) << 20);
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "prefs.set",
        label: "Change Setting",
        menu: &["File", "Preferences", "Change setting"],
        shortcut: None,
        params: "{key: string, value: any} — see Preferences for the keys",
        enabled: |_| true,
        run: |s, params| {
            let map = object(params)?
                .ok_or_else(|| FmError::InvalidParams("key and value required".into()))?;
            let key = map
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| FmError::InvalidParams("key required".into()))?;
            let value = map
                .get("value")
                .ok_or_else(|| FmError::InvalidParams("value required".into()))?;
            set(s, key, value)
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set_via(s: &mut Session, key: &str, value: Value) -> Result<(), FmError> {
        s.dispatch("prefs.set", &json!({"key": key, "value": value}))
    }

    #[test]
    fn scrollback_range_persists_and_hand_edited_values_are_bounded() {
        let mut s = Session::new().expect("session");
        assert_eq!(s.prefs.terminal_scrollback_rows(), 10_000);
        for rows in [5_000, 10_000, 100_000, 1_000_000] {
            set_via(&mut s, "terminal_scrollback_rows", json!(rows)).expect("set");
            let encoded = toml::to_string(&s.prefs).expect("serialize");
            let decoded: Prefs = toml::from_str(&encoded).expect("deserialize");
            assert_eq!(decoded.terminal_scrollback_rows(), rows as usize);
        }
        for bad in [
            json!(4999),
            json!(1_000_001),
            json!(-1),
            json!(5000.5),
            json!("10000"),
            json!(null),
        ] {
            let before = s.prefs.clone();
            assert!(set_via(&mut s, "terminal_scrollback_rows", bad).is_err());
            assert_eq!(s.prefs, before);
        }
        s.prefs.terminal_scrollback_rows = u64::MAX;
        assert_eq!(s.prefs.terminal_scrollback_rows(), 1_000_000);
        s.prefs.terminal_scrollback_rows = 0;
        assert_eq!(s.prefs.terminal_scrollback_rows(), 5_000);
        assert_eq!(
            toml::from_str::<Prefs>("")
                .expect("old preferences")
                .terminal_scrollback_rows(),
            10_000
        );
    }

    #[test]
    fn valid_values_apply() {
        let mut s = Session::new().expect("session");
        set_via(&mut s, "accent", json!("#FF8800")).expect("accent");
        assert_eq!(s.prefs.accent_rgb(), Some([255, 136, 0]));
        set_via(&mut s, "accent", json!("default")).expect("reset");
        assert!(s.prefs.accent.is_none());
        set_via(&mut s, "density", json!("compact")).expect("density");
        assert_eq!(s.prefs.density, Density::Compact);
        set_via(&mut s, "font_size", json!(15.3)).expect("font");
        assert_eq!(s.prefs.font_size, 15.5);
        set_via(&mut s, "size_units", json!("decimal")).expect("units");
        set_via(&mut s, "date_format", json!("eu")).expect("date");
        set_via(&mut s, "single_click_open", json!(true)).expect("click");
        set_via(&mut s, "confirm_delete", json!(false)).expect("confirm");
        set_via(&mut s, "thumb_memory_mib", json!(64)).expect("memory");
        set_via(&mut s, "preview_time_secs", json!(30)).expect("time");
        assert_eq!(s.prefs.preview_budget().as_secs(), 30);
    }

    #[test]
    fn cache_dir_can_move_and_bad_paths_are_refused() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut s = Session::new().expect("session");
        let dir = tmp.path().to_string_lossy().to_string();
        set_via(&mut s, "thumb_cache_dir", json!(dir)).expect("set");
        assert_eq!(s.prefs.thumb_cache_path(), Some(tmp.path().join("thumbs")));
        assert!(tmp.path().join("thumbs").is_dir());
        let before = s.prefs.clone();
        for bad in [json!("relative/path"), json!("/a\nb"), json!(3)] {
            assert!(set_via(&mut s, "thumb_cache_dir", bad).is_err());
        }
        let file = tmp.path().join("file");
        std::fs::write(&file, b"x").expect("file");
        let under_file = file.join("sub").to_string_lossy().to_string();
        assert!(set_via(&mut s, "thumb_cache_dir", json!(under_file)).is_err());
        assert_eq!(s.prefs, before, "failed changes leave the old folder");
        set_via(&mut s, "thumb_cache_dir", json!("default")).expect("reset");
        assert!(s.prefs.thumb_cache_dir.is_none());
    }

    #[test]
    fn show_hidden_follows_the_pref() {
        let mut s = Session::new().expect("session");
        set_via(&mut s, "show_hidden", json!(true)).expect("on");
        assert!(s.show_hidden && s.prefs.show_hidden);
        set_via(&mut s, "show_hidden", json!(false)).expect("off");
        assert!(!s.show_hidden);
    }

    #[test]
    fn malformed_values_are_rejected_without_change() {
        let mut s = Session::new().expect("session");
        let before = s.prefs.clone();
        for (key, value) in [
            ("accent", json!("red")),
            ("accent", json!("#12345")),
            ("accent", json!("#ééé")),
            ("accent", json!(5)),
            ("density", json!("huge")),
            ("font_size", json!(4)),
            ("font_size", json!(99)),
            ("font_size", json!("x")),
            ("terminal_font_size", json!(null)),
            ("size_units", json!("furlongs")),
            ("date_format", json!(1)),
            ("show_hidden", json!("yes")),
            ("confirm_run", json!(2)),
            ("thumb_memory_mib", json!(1)),
            ("thumb_disk_mib", json!(-1)),
            ("preview_time_secs", json!(1000)),
            ("nope", json!(true)),
        ] {
            assert!(
                set_via(&mut s, key, value.clone()).is_err(),
                "{key} {value}"
            );
        }
        assert!(s.dispatch("prefs.set", &Value::Null).is_err());
        assert!(s.dispatch("prefs.set", &json!({"key": "density"})).is_err());
        assert_eq!(s.prefs, before);
    }

    #[test]
    fn hand_edited_prefs_are_clamped() {
        let prefs = Prefs {
            font_size: f32::NAN,
            terminal_font_size: 500.0,
            preview_time_secs: 0,
            accent: Some("garbage".into()),
            ..Prefs::default()
        };
        assert_eq!(prefs.ui_font_size(), 14.0);
        assert_eq!(prefs.term_font_size(), 24.0);
        assert_eq!(prefs.preview_budget().as_secs(), 2);
        assert!(prefs.accent_rgb().is_none());
    }
}
