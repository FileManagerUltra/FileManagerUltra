//! Git status markers shared by the listing and the tree (capability `git-status`):
//! a tint for the name and a small gutter glyph; folders show an aggregate dot.

use std::sync::Arc;

use egui::{Align2, Color32, Painter, Pos2, Rect};
use fm_engine::Session;
use fm_engine::git::{Mark, RepoStatus};
use fm_vfs::VfsUri;

use crate::theme::Tokens;

/// Width reserved at the right of a name cell for the gutter glyph.
pub(crate) const GUTTER: f32 = 16.0;

/// Status of the repository around one folder, resolved once per frame.
pub(crate) struct Marks {
    status: Option<Arc<RepoStatus>>,
    pub tint: bool,
    pub gutter: bool,
}

impl Marks {
    pub fn none() -> Self {
        Self {
            status: None,
            tint: false,
            gutter: false,
        }
    }

    /// Markers for entries of `folder`; empty outside a repository or when disabled.
    pub fn for_folder(s: &mut Session, folder: &VfsUri) -> Self {
        let status = s.git_view(folder);
        Self {
            status,
            tint: s.prefs.git_tint,
            gutter: s.prefs.git_gutter,
        }
    }

    /// Cached markers only (tree rows): never starts tracking a repository.
    pub fn peek(s: &mut Session, folder: &VfsUri) -> Self {
        let status = s.git_peek(folder);
        Self {
            status,
            tint: s.prefs.git_tint,
            gutter: s.prefs.git_gutter,
        }
    }

    pub fn mark(&self, uri: &VfsUri) -> Option<Mark> {
        let path = uri.local_path()?;
        self.status.as_ref()?.mark_abs(&path)
    }

    /// Tint for a name, if tinting applies.
    pub fn tint_of(&self, mark: Option<Mark>, tokens: &Tokens) -> Option<Color32> {
        if !self.tint {
            return None;
        }
        mark.map(|m| tokens.git(m))
    }
}

/// Paint the gutter glyph centred in the icon gutter at the left of `cell`. Folders get a dot, files a letter.
pub(crate) fn paint_gutter(
    painter: &Painter,
    cell: Rect,
    mark: Mark,
    is_dir: bool,
    tokens: &Tokens,
) {
    if mark == Mark::Ignored {
        return;
    }
    let color = tokens.git(mark);
    let center = Pos2::new(cell.left() + GUTTER * 0.5, cell.center().y);
    if is_dir {
        painter.circle_filled(center, 3.0, color);
    } else {
        painter.text(
            center,
            Align2::CENTER_CENTER,
            mark.letter(),
            egui::FontId::monospace((tokens.font_size - 2.0).max(8.0)),
            color,
        );
    }
}

/// Footer label for the repository around `folder`, e.g. `git: main · 3 changed`.
pub fn branch_label(s: &mut Session, folder: &VfsUri) -> Option<String> {
    let status = s.git_peek(folder)?;
    let branch = status.branch.as_deref()?;
    let changed = status.changed();
    let more = if status.truncated { "+" } else { "" };
    Some(if changed == 0 {
        format!("git: {branch}")
    } else {
        format!("git: {branch} · {changed}{more} changed")
    })
}

/// Short description for hover text.
pub(crate) fn describe(mark: Mark) -> &'static str {
    match mark {
        Mark::Modified => "Modified",
        Mark::Added => "Added",
        Mark::Removed => "Deleted",
        Mark::Renamed => "Renamed",
        Mark::Untracked => "Untracked",
        Mark::Ignored => "Ignored",
        Mark::Conflicted => "Conflict",
    }
}
