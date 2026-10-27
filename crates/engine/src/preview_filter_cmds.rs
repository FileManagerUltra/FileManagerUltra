use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::preview_cmds::log_cmd;
use crate::registry::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "preview.log.clear_filters",
        label: "Log: Clear All Filters",
        menu: &["View", "Clear All Preview Filters"],
        shortcut: None,
        params: "{pane?: number}",
        enabled: |s| s.previews.slots.values().any(|slot| slot.log().is_some()),
        run: |s, params| {
            if let Some(pane) = object(params)?.and_then(|o| o.get("pane"))
                && pane.as_u64().is_none()
            {
                return Err(FmError::InvalidParams(
                    "pane: unsigned integer required".into(),
                ));
            }
            log_cmd(params, s, |view| {
                view.filter = Default::default();
                view.filter_error = None;
                Ok(true)
            })
        },
    }]
}
