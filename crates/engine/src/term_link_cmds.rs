//! Opening a web link shown in a terminal pane (Ctrl+click).

use fm_layout::PaneKind;
use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "term.open_link",
        label: "Open Terminal Link",
        menu: &["Terminal", "Open Link"],
        shortcut: None,
        params: "{pane?: number, row: number, col: number}: opens the http/https URL at that screen cell",
        enabled: |s| s.focused_kind() == Some(PaneKind::Terminal),
        run: |s, params| {
            let map = object(params)?
                .ok_or_else(|| FmError::InvalidParams("row and col required".into()))?;
            let cell = |field: &str| {
                map.get(field)
                    .and_then(Params::as_u64)
                    .and_then(|v| u16::try_from(v).ok())
                    .ok_or_else(|| FmError::InvalidParams(format!("invalid {field}")))
            };
            let (row, col) = (cell("row")?, cell("col")?);
            let pane = match map.get("pane") {
                None => s.layout.focused(),
                Some(v) => v
                    .as_u64()
                    .ok_or_else(|| FmError::InvalidParams("invalid pane".into()))?,
            };
            let snap = s
                .terminal_snapshot(pane)
                .ok_or_else(|| FmError::NotFound(format!("terminal {pane}")))?;
            let link = fm_term::link_at(&snap, row, col)
                .ok_or_else(|| FmError::NotFound("no link at that position".into()))?;
            s.dispatch("preview.open_link", &serde_json::json!({ "url": link.url }))
        },
    }]
}

#[cfg(test)]
mod tests {
    use crate::Session;

    #[test]
    fn bad_params_and_missing_terminals_are_errors() {
        let mut s = Session::default();
        for p in [
            serde_json::json!(null),
            serde_json::json!({}),
            serde_json::json!({"row": -1, "col": 0}),
            serde_json::json!({"row": 0, "col": "x"}),
            serde_json::json!({"row": 0, "col": 0, "pane": 9999}),
            serde_json::json!({"row": 0, "col": 0, "pane": "x"}),
        ] {
            assert!(s.dispatch("term.open_link", &p).is_err(), "{p}");
        }
    }
}
