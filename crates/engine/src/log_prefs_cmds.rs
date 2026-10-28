use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::prefs::{LOG_POLL_RANGE, LOG_TAIL_RANGE};
use crate::registry::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "prefs.preview.log",
        label: "Log Preview Settings",
        menu: &["File", "Preferences", "Log preview"],
        shortcut: None,
        params: "{poll_ms?: 100..60000, tail_mib?: 0..64 (0 = unlimited initial window, safety caps retained)}",
        enabled: |_| true,
        run: |s, params| {
            let map = object(params)?.ok_or_else(|| {
                FmError::InvalidParams("log preview settings object required".into())
            })?;
            if map.is_empty() || map.keys().any(|key| key != "poll_ms" && key != "tail_mib") {
                return Err(FmError::InvalidParams("use poll_ms and/or tail_mib".into()));
            }
            let number = |key: &str, range: std::ops::RangeInclusive<u64>| {
                map.get(key)
                    .map(|value| {
                        value.as_u64().filter(|n| range.contains(n)).ok_or_else(|| {
                            FmError::InvalidParams(format!(
                                "{key} must be {}..={}",
                                range.start(),
                                range.end()
                            ))
                        })
                    })
                    .transpose()
            };
            let poll = number("poll_ms", LOG_POLL_RANGE)?;
            let tail = number("tail_mib", 0..=*LOG_TAIL_RANGE.end())?;
            if let Some(value) = poll {
                s.prefs.log_poll_ms = value;
                for slot in s.previews.slots.values_mut() {
                    if let Some(view) = slot.log_mut() {
                        view.reschedule_follow();
                    }
                }
            }
            if let Some(value) = tail {
                s.prefs.log_tail_mib = value;
            }
            Ok(())
        },
    }]
}

#[cfg(test)]
mod tests {
    use crate::{Prefs, Session};
    use serde_json::json;

    #[test]
    fn log_settings_validate_atomically_and_round_trip() {
        let mut s = Session::new().expect("session");
        let defaults = s.prefs.clone();
        for bad in [
            json!(null),
            json!([]),
            json!({}),
            json!({"unknown": 1}),
            json!({"poll_ms": 99}),
            json!({"poll_ms": 60001}),
            json!({"poll_ms": "500"}),
            json!({"poll_ms": null}),
            json!({"tail_mib": -1}),
            json!({"tail_mib": 65}),
            json!({"tail_mib": 1.5}),
            json!({"tail_mib": false}),
            json!({"poll_ms": 1000, "tail_mib": 65}),
        ] {
            assert!(s.dispatch("prefs.preview.log", &bad).is_err(), "{bad}");
            assert_eq!(s.prefs, defaults);
        }
        for tail in [0, 1, 16, 64] {
            s.dispatch(
                "prefs.preview.log",
                &json!({"poll_ms": 2000, "tail_mib": tail}),
            )
            .expect("settings");
            let text = toml::to_string(&s.prefs).expect("serialize");
            assert_eq!(toml::from_str::<Prefs>(&text).expect("restore"), s.prefs);
            assert_eq!(s.prefs.log_poll().as_millis(), 2000);
            assert_eq!(
                s.prefs.log_limits().initial_tail,
                (if tail == 0 { 64 } else { tail }) << 20
            );
        }
        let old: Prefs = toml::from_str("").expect("old preferences");
        assert_eq!(old.log_poll_ms, 500);
        assert_eq!(old.log_tail_mib, 8);
        s.prefs.log_poll_ms = u64::MAX;
        s.prefs.log_tail_mib = u64::MAX;
        assert_eq!(s.prefs.log_poll().as_millis(), 60000);
        assert_eq!(s.prefs.log_limits().initial_tail, 64 << 20);
    }
}
