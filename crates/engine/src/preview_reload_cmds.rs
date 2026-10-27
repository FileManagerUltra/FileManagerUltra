use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::registry::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "preview.reload",
        label: "Reload Selected Files Preview",
        menu: &["View", "Reload Selected Files Preview"],
        shortcut: None,
        params: "{pane?: number}",
        enabled: |s| {
            s.previews.slots.values().any(|slot| {
                slot.subjects.len() > 1
                    && matches!(
                        slot.body,
                        crate::preview_cmds::Body::Log(_) | crate::preview_cmds::Body::Grid(_)
                    )
            })
        },
        run: |s, params| {
            if let Some(pane) = object(params)?.and_then(|o| o.get("pane"))
                && pane.as_u64().is_none()
            {
                return Err(FmError::InvalidParams(
                    "pane: unsigned integer required".into(),
                ));
            }
            s.reload_preview(params)
        },
    }]
}
