use fm_vfs::FmError;

use crate::{
    file_cmds::object,
    preview_types::{FilterGroup, PreviewFilters},
    registry::CommandSpec,
};

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "prefs.preview.filters",
        label: "Preview Filename Filters",
        menu: &["File", "Preferences", "Preview filename filters"],
        shortcut: None,
        params: "{groups: [{type: logs|images|video|audio|pdf|markdown|json|yaml|toml|xml|html|csv|text, patterns: [string, ...]}, ...]} (array order is match priority; omitted types get defaults at the end) or {reset: true}",
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
    }]
}
