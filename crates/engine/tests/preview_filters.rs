#![allow(clippy::expect_used)]

use fm_engine::{
    Prefs, Session,
    preview_types::{FilterGroup, PreviewFilters, PreviewType},
};
use fm_preview::{Format, Kind};
use serde_json::json;

#[test]
fn defaults_cover_existing_types_logs_and_rotations() {
    let filters = PreviewFilters::default();
    let rules = filters.compile().expect("bounded defaults");
    for name in [
        "a.log",
        "A.LOG",
        "a.log.1",
        "a.log.2.gz",
        "a.log-20261028.gz",
        "syslog",
        "syslog.1",
        "syslog.2.gz",
        "messages-20261028.gz",
        "secure",
        "maillog",
        "cron",
        "debug",
        "dmesg",
    ] {
        assert_eq!(rules.kind(name), Some(Kind::Log), "{name}");
    }
    for name in [
        "lastlog",
        "wtmp",
        "btmp",
        "syslog.conf",
        "cron.sh",
        "catalog",
        "a.log.bak",
    ] {
        assert_eq!(rules.kind(name), None, "{name}");
    }
    for kind in PreviewType::ALL {
        assert!(!filters.patterns(kind).is_empty(), "{kind:?}");
    }
    assert_eq!(
        filters.0.iter().map(|g| g.kind).collect::<Vec<_>>(),
        PreviewType::ALL
    );
    assert_eq!(rules.kind("photo.JPG"), Some(Kind::Image("image")));
    assert_eq!(rules.kind("page.htm"), Some(Kind::Text("html".into())));
    assert_eq!(rules.kind("data.csv"), Some(Kind::Text("csv".into())));
    assert_eq!(
        rules.kind("data.json"),
        Some(Kind::Structured(Format::Json))
    );
    assert_eq!(rules.kind("data.yml"), Some(Kind::Structured(Format::Yaml)));
    assert_eq!(
        rules.kind("data.toml"),
        Some(Kind::Structured(Format::Toml))
    );
    assert_eq!(rules.kind("data.svg"), Some(Kind::Structured(Format::Xml)));
}

fn set(filters: &mut PreviewFilters, kind: PreviewType, patterns: &[&str]) {
    let group = filters
        .0
        .iter_mut()
        .find(|g| g.kind == kind)
        .expect("group");
    group.patterns = patterns.iter().map(|p| p.to_string()).collect();
}

#[test]
fn custom_patterns_priority_clear_and_validation() {
    let mut filters = PreviewFilters::default();
    set(
        &mut filters,
        PreviewType::Logs,
        &["*.trace", "special", "*.txt"],
    );
    set(&mut filters, PreviewType::Csv, &["*.txt", "*.data"]);
    let rules = filters.compile().expect("custom");
    assert_eq!(rules.kind("SERVER.TRACE"), Some(Kind::Log));
    assert_eq!(rules.kind("special"), Some(Kind::Log));
    assert_eq!(rules.kind("a.txt"), Some(Kind::Log));
    assert_eq!(rules.kind("a.log"), None);
    assert_eq!(rules.kind("table.data"), Some(Kind::Text("csv".into())));
    set(&mut filters, PreviewType::Logs, &[]);
    assert_eq!(
        filters.compile().expect("empty").kind("a.txt"),
        Some(Kind::Text("csv".into()))
    );
    // Reordering groups changes priority.
    set(&mut filters, PreviewType::Logs, &["*.txt"]);
    let logs = filters.0.remove(0);
    filters.0.push(logs);
    assert_eq!(
        filters.compile().expect("reordered").kind("a.txt"),
        Some(Kind::Text("csv".into()))
    );
    for bad in [
        vec![" ".to_string()],
        vec![String::new()],
        vec!["x".repeat(257)],
        vec!["../*.log".into()],
        vec!["a\\b".into()],
        vec!["x\ny".into()],
        vec!["x\0".into()],
        vec!["a;b".into()],
        vec!["a,b".into()],
        vec!["a|b".into()],
        vec!["x".to_string(); 65],
        vec!["x".repeat(200); 6],
    ] {
        let mut bad_filters = PreviewFilters::default();
        bad_filters.0[1].patterns = bad.clone();
        assert!(bad_filters.compile().is_err(), "{bad:?}");
    }
    let mut duplicate = PreviewFilters::default();
    duplicate.0.push(FilterGroup {
        kind: PreviewType::Logs,
        patterns: vec!["*.dup".into()],
    });
    assert!(duplicate.compile().is_err());
}

#[test]
fn command_validates_atomically_persists_and_resets() {
    let mut s = Session::new().expect("session");
    let initial = s.prefs.clone();
    for bad in [
        json!(null),
        json!([]),
        json!({}),
        json!({"reset": false}),
        json!({"groups": []}),
        json!({"groups": null}),
        json!({"groups": [{"type": "secret", "patterns": ["x"]}]}),
        json!({"groups": [{"type": "logs", "patterns": "x"}]}),
        json!({"groups": [{"type": "logs", "patterns": [false]}]}),
        json!({"groups": [{"type": "logs", "patterns": ["*.trace"], "extra": 1}]}),
        json!({"groups": [{"type": "logs", "patterns": ["*.trace"]}, {"type": "csv", "patterns": ["../bad"]}]}),
        json!({"groups": [{"type": "logs", "patterns": ["a;b"]}]}),
        json!({"groups": [{"type": "logs", "patterns": [""]}]}),
        json!({"groups": [{"type": "logs", "patterns": ["a"]}, {"type": "logs", "patterns": ["b"]}]}),
        json!({"groups": [{"type": "logs", "patterns": ["*.trace"]}], "reset": true}),
        json!({"filters": {"logs": "*.log"}}),
    ] {
        assert!(s.dispatch("prefs.preview.filters", &bad).is_err(), "{bad}");
        assert_eq!(s.prefs, initial);
    }
    s.dispatch(
        "prefs.preview.filters",
        &json!({"groups": [
            {"type": "csv", "patterns": ["*.data"]},
            {"type": "logs", "patterns": ["*.trace", "syslog"]},
            {"type": "html", "patterns": []}
        ]}),
    )
    .expect("set");
    let filters = &s.prefs.preview_filters;
    assert_eq!(filters.patterns(PreviewType::Logs), ["*.trace", "syslog"]);
    assert_eq!(filters.patterns(PreviewType::Html), &[] as &[String]);
    assert_eq!(
        filters.patterns(PreviewType::Images),
        PreviewType::Images.default_patterns()
    );
    assert_eq!(filters.0.len(), PreviewType::ALL.len());
    assert_eq!(filters.0[0].kind, PreviewType::Csv, "order is the priority");
    let dir = tempfile::tempdir().expect("prefs dir");
    fm_engine::prefs::save(dir.path(), &s.prefs).expect("save");
    assert_eq!(
        fm_engine::prefs::load(&dir.path().join(fm_engine::prefs::PREFS_FILE)),
        s.prefs
    );
    assert_eq!(
        toml::from_str::<Prefs>("")
            .expect("old prefs")
            .preview_filters,
        PreviewFilters::default()
    );
    s.dispatch("prefs.preview.filters", &json!({"reset": true}))
        .expect("reset");
    assert_eq!(s.prefs, initial);
}

#[test]
fn legacy_string_filters_migrate_to_pattern_rows() {
    let prefs: Prefs = toml::from_str(
        "[preview_filters]\nlogs = '*.trace; special'\ncsv = ''\nimages = '*.photo|*.pic'",
    )
    .expect("legacy prefs");
    assert_eq!(
        prefs.preview_filters.patterns(PreviewType::Logs),
        ["*.trace", "special"]
    );
    assert_eq!(
        prefs.preview_filters.patterns(PreviewType::Images),
        ["*.photo", "*.pic"]
    );
    assert!(prefs.preview_filters.patterns(PreviewType::Csv).is_empty());
    assert_eq!(prefs.preview_filters.patterns(PreviewType::Pdf), ["*.pdf"]);
}

#[test]
fn invalid_hand_edited_filters_are_reported_and_defaulted() {
    let dir = tempfile::tempdir().expect("prefs dir");
    let path = dir.path().join(fm_engine::prefs::PREFS_FILE);
    std::fs::write(
        &path,
        "log_poll_ms = 1000\n[[preview_filters]]\ntype = 'logs'\npatterns = ['../*.log']",
    )
    .expect("prefs");
    let prefs = fm_engine::prefs::load(&path);
    assert_eq!(prefs.log_poll_ms, 1000);
    assert_eq!(prefs.preview_filters, PreviewFilters::default());
}

#[test]
fn unknown_saved_groups_are_skipped_without_losing_other_prefs() {
    let prefs: Prefs = toml::from_str(
        "log_poll_ms = 1000\n[[preview_filters]]\ntype = 'bogus'\npatterns = ['x']\n[[preview_filters]]\ntype = 'pdf'\npatterns = ['*.doc']",
    )
    .expect("tolerant prefs");
    assert_eq!(prefs.log_poll_ms, 1000);
    assert_eq!(prefs.preview_filters.patterns(PreviewType::Pdf), ["*.doc"]);
    assert_eq!(prefs.preview_filters.0[0].kind, PreviewType::Pdf);
}
