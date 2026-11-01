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
            FmError::InvalidParams(
                "kind must be files, terminal, preview, sftp or network".to_string(),
            )
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
        params: "{kind: \"files\"|\"terminal\"|\"sftp\"|\"smb\"|\"trash\"|\"network\", pane?: number, confirm?: bool}",
        enabled: |_s| true,
        run: |s, p: &Params| {
            let (kind, pane, confirm) = parse(p)?;
            let pane = pane.unwrap_or_else(|| s.layout.focused());
            if s.layout.state(pane).is_none() {
                return Err(FmError::NotFound(format!("pane {pane}")));
            }
            s.pending_kind = None;
            let current = s.layout.state(pane).map(|st| st.kind);
            if current == Some(PaneKind::Terminal) && kind != PaneKind::Terminal {
                if s.terminal_busy(pane) && !confirm {
                    s.pending_kind = Some((pane, kind));
                    return Ok(());
                }
                s.drop_terminal(pane);
            }
            let root = s.layout.state(pane).map(|st| st.tree.root.scheme());
            let virtual_root = matches!(
                root,
                Some(fm_vfs::Scheme::Network | fm_vfs::Scheme::Servers)
            );
            let leaving_network = kind == PaneKind::Files && virtual_root;
            if leaving_network && let Some(state) = s.layout.state_mut(pane) {
                // The Network tree has nothing to show for a local pane: back to home.
                state.tree = fm_layout::TreeState::new(fm_vfs::VfsUri::root_file());
                state.navigate(crate::workspace::home_location());
            }
            match kind {
                PaneKind::Files | PaneKind::Preview | PaneKind::Smb => {}
                PaneKind::Sftp => {
                    let live = s.layout.state(pane).is_some_and(|st| {
                        st.tree.root.scheme() != fm_vfs::Scheme::Servers
                            && s.backends.get_for(&st.location).is_some()
                            && st.location.scheme() == fm_vfs::Scheme::Sftp
                    });
                    if !live {
                        s.open_servers(pane)?;
                    }
                }
                PaneKind::Trash => {
                    if !s.trash.busy {
                        s.trash_list()?;
                    }
                }
                PaneKind::Diff => {
                    if s.layout.state(pane).is_none_or(|st| st.diff.is_none()) {
                        return Err(FmError::InvalidParams(
                            "open a comparison with diff.open first".to_string(),
                        ));
                    }
                }
                PaneKind::Terminal => s.ensure_terminal(pane)?,
                PaneKind::Network => s.open_network(pane)?,
            }
            if let Some(state) = s.layout.state_mut(pane) {
                state.kind = kind;
            }
            if s.path_edit.as_ref().is_some_and(|e| e.pane == pane) {
                s.path_edit = None;
            }
            if leaving_network {
                let home = crate::workspace::home_location();
                s.ensure_read(home)?;
            }
            s.layout.focus_pane(pane)
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![set_kind()]
}
