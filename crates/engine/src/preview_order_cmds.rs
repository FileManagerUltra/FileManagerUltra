use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::preview_cmds::log_cmd;
use crate::registry::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "preview.log.order",
        label: "Log: Toggle Source Order",
        menu: &["View", "Toggle Log Source Order"],
        shortcut: None,
        params: "{mode?: \"concatenate\"|\"chronological\" (omitted toggles), pane?: number}",
        enabled: |s| {
            s.previews
                .slots
                .values()
                .any(|slot| slot.log().is_some_and(|view| !view.sources.is_empty()))
        },
        run: |s, params| {
            let chronological = match object(params)?.and_then(|o| o.get("mode")) {
                None => None,
                Some(serde_json::Value::String(mode)) if mode == "concatenate" => Some(false),
                Some(serde_json::Value::String(mode)) if mode == "chronological" => Some(true),
                _ => {
                    return Err(FmError::InvalidParams(
                        "mode: concatenate or chronological required".into(),
                    ));
                }
            };
            log_cmd(params, s, |view| {
                if view.sources.is_empty() {
                    return Err(FmError::InvalidParams(
                        "Select multiple text or log files to change source order.".into(),
                    ));
                }
                view.chronological = chronological.unwrap_or(!view.chronological);
                Ok(true)
            })
        },
    }]
}
