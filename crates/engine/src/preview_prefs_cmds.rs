use fm_vfs::FmError;

use crate::{
    file_cmds::object,
    prefs::SvgBackground,
    preview_types::{FilterGroup, PreviewFilters},
    registry::CommandSpec,
};

pub fn specs() -> Vec<CommandSpec> {
    vec![filters(), svg_background()]
}

/// `preview.svg.background` — what is drawn behind SVG pictures: `checkered` (default),
/// `white`, `black` or `grey` (50%). Without `value` it moves to the next one. Saved.
fn svg_background() -> CommandSpec {
    CommandSpec {
        id: "preview.svg.background",
        label: "Next SVG Background",
        menu: &["View", "Next SVG Background"],
        shortcut: None,
        params: "{value?: \"checkered\"|\"white\"|\"black\"|\"grey\"} (omitted: the next one)",
        enabled: |_| true,
        run: |s, params| {
            let next = match object(params)?.and_then(|m| m.get("value")) {
                None => s.prefs.svg_background.next(),
                Some(value) => {
                    value
                        .as_str()
                        .and_then(SvgBackground::from_id)
                        .ok_or_else(|| {
                            FmError::InvalidParams(
                                "value must be checkered, white, black or grey".into(),
                            )
                        })?
                }
            };
            s.prefs.svg_background = next;
            Ok(())
        },
    }
}

fn filters() -> CommandSpec {
    CommandSpec {
        id: "prefs.preview.filters",
        label: "Preview Filename Filters",
        menu: &["File", "Preferences", "Preview filename filters"],
        shortcut: None,
        params: "{groups: [{type: logs|images|video|audio|pdf|markdown|svg|json|yaml|toml|xml|html|csv|text, patterns: [string, ...]}, ...]} (array order is match priority; omitted types get defaults at the end, except svg which goes above xml and takes over *.svg) or {reset: true}",
        enabled: |_| true,
        run: |s, params| {
            let map = object(params)?
                .ok_or_else(|| FmError::InvalidParams("preview filters object required".into()))?;
            let next = if map.len() == 1 && map.get("reset") == Some(&serde_json::Value::Bool(true))
            {
                PreviewFilters::default()
            } else if let Some(value) = map.get("groups").filter(|_| map.len() == 1) {
                let groups: Vec<FilterGroup> = serde_json::from_value(value.clone()).map_err(|_| {
                    FmError::InvalidParams(
                        "groups must list {type, patterns: [string]} for supported preview types".into(),
                    )
                })?;
                if groups.is_empty() {
                    return Err(FmError::InvalidParams(
                        "at least one preview type is required".into(),
                    ));
                }
                let candidate = PreviewFilters(groups);
                candidate.compile()?;
                PreviewFilters::from_groups(candidate.0)
            } else {
                return Err(FmError::InvalidParams("use groups or reset: true".into()));
            };
            next.compile()?;
            if s.prefs.preview_filters != next {
                s.prefs.preview_filters = next;
                s.thumbs.invalidate_associations();
                for slot in s.previews.slots.values_mut() {
                    slot.stale = true;
                }
            }
            Ok(())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn svg_background_sets_cycles_and_rejects_bad_values() {
        let mut s = crate::Session::new().expect("session");
        assert_eq!(s.prefs.svg_background, SvgBackground::Checkered);
        s.dispatch("preview.svg.background", &json!({"value": "grey"}))
            .expect("set");
        assert_eq!(s.prefs.svg_background, SvgBackground::Grey);
        s.dispatch("preview.svg.background", &json!({}))
            .expect("next");
        assert_eq!(s.prefs.svg_background, SvgBackground::Checkered, "wraps");
        s.dispatch("preview.svg.background", &serde_json::Value::Null)
            .expect("next");
        assert_eq!(s.prefs.svg_background, SvgBackground::White);
        for bad in [
            json!({"value": "pink"}),
            json!({"value": 3}),
            json!({"value": null}),
            json!([1]),
            json!("white"),
        ] {
            assert!(s.dispatch("preview.svg.background", &bad).is_err(), "{bad}");
            assert_eq!(s.prefs.svg_background, SvgBackground::White);
        }
        let saved = toml::to_string(&s.prefs).expect("save");
        let back: crate::Prefs = toml::from_str(&saved).expect("load");
        assert_eq!(back.svg_background, SvgBackground::White);
    }
}
