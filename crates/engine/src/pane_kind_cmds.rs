//! `pane.kind.set` — switch what a pane shows (capability `pane-types`).

use fm_layout::{PaneId, PaneKind};
use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};

fn parse(params: &Params) -> Result<(PaneKind, Option<PaneId>, bool), FmError> {
    let map = object(params)?
        .ok_or_else(|| FmError::InvalidParams("kind: string required".to_string()))?;
    let kind = map
        .get("kind")
        .and_then(Params::as_str)
        .and_then(PaneKind::parse)
        .ok_or_else(|| {
            FmError::InvalidParams("kind must be files, terminal, sftp or network".to_string())
        })?;
    let pane = match map.get("pane") {
        None | Some(Params::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .ok_or_else(|| FmError::InvalidParams("pane must be a number".to_string()))?,
        ),
    };
    let confirm = map
        .get("confirm")
        .and_then(Params::as_bool)
        .unwrap_or(false);
    Ok((kind, pane, confirm))
}

/// `pane.kind.set {kind, pane?}`
pub fn set_kind() -> CommandSpec {
    CommandSpec {
        id: "pane.kind.set",
        label: "Pane Type",
        menu: &["View", "Pane Type"],
        shortcut: None,
        params: "{kind: \"files\"|\"terminal\"|\"sftp\"|\"network\", pane?: number, confirm?: bool}",
        enabled: |_s| true,
        run: |s, p: &Params| {
            let (kind, pane, confirm) = parse(p)?;
            let pane = pane.unwrap_or_else(|| s.layout.focused());
            if s.layout.state(pane).is_none() {
                return Err(FmError::NotFound(format!("pane {pane}")));
            }
            s.pending_kind = None;
            if matches!(kind, PaneKind::Sftp | PaneKind::Network) {
                return Err(FmError::Unsupported(
                    "this pane type arrives with the network backends",
                ));
            }
            let current = s.layout.state(pane).map(|st| st.kind);
            if current == Some(PaneKind::Terminal) && kind != PaneKind::Terminal {
                if s.terminal_busy(pane) && !confirm {
                    s.pending_kind = Some((pane, kind));
                    return Ok(());
                }
                s.drop_terminal(pane);
            }
            match kind {
                PaneKind::Files => {}
                PaneKind::Terminal => s.ensure_terminal(pane)?,
                PaneKind::Sftp | PaneKind::Network => {
                    return Err(FmError::Unsupported(
                        "this pane type arrives with the network backends",
                    ));
                }
            }
            if let Some(state) = s.layout.state_mut(pane) {
                state.kind = kind;
            }
            if s.path_edit.as_ref().is_some_and(|e| e.pane == pane) {
                s.path_edit = None;
            }
            s.layout.focus_pane(pane)
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![set_kind()]
}
