//! One question before anything is lost (quit, closing a pane, closing a workspace tab):
//! unsaved workspaces, filled-in PDFs and archives with staged changes, and, on quit,
//! work still running, are listed together in a single "Save / Don't save / Cancel"
//! prompt instead of one prompt after another.

use fm_layout::PaneId;
use fm_vfs::{FmError, VfsUri};

use crate::archive_cmds::{after_save, dirty_archives};
use crate::fileops::{OpRequest, OpState};
use crate::session::Session;
use crate::workspace::{AfterPrompt, UnsavedPrompt};

/// Saves started from the prompt; the quit or close goes on once all have finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingClose {
    pub after: AfterPrompt,
    pub pdfs: Vec<PaneId>,
    pub archives: Vec<VfsUri>,
}

impl Session {
    /// The tab index of workspace uid `uid`.
    fn tab_ix(&self, uid: u64) -> Option<usize> {
        self.workspaces.tabs.iter().position(|t| t.uid == uid)
    }

    /// Panes `after` closes (of the active layout, or of the closed tab).
    fn closing_panes(&self, after: &AfterPrompt) -> Vec<PaneId> {
        match after {
            AfterPrompt::Quit => self.layout.panes(),
            AfterPrompt::ClosePane(p) => vec![*p],
            AfterPrompt::Close(uid) => self
                .tab_ix(*uid)
                .map(|ix| self.tab_locations(ix).into_iter().map(|(p, _)| p).collect())
                .unwrap_or_default(),
        }
    }

    /// Archive handles still shown once `after` is done (every tab).
    fn handles_kept(&self, after: &AfterPrompt) -> Vec<String> {
        let active = self.workspaces.active;
        let mut kept = Vec::new();
        for ix in 0..self.workspaces.tabs.len() {
            let uid = self.workspaces.tabs.get(ix).map(|t| t.uid);
            if matches!(after, AfterPrompt::Close(u) if Some(*u) == uid) {
                continue;
            }
            for (pane, uri) in self.tab_locations(ix) {
                if matches!(after, AfterPrompt::ClosePane(p) if *p == pane && ix == active) {
                    continue;
                }
                if let Some(h) = uri.archive_handle() {
                    kept.push(h.to_string());
                }
            }
        }
        kept
    }

    /// Everything `after` would lose, or `None` when nothing needs asking.
    fn gather_unsaved(&mut self, after: &AfterPrompt) -> Option<UnsavedPrompt> {
        let ask_workspaces = self.prefs.unsaved_workspaces == crate::prefs::UnsavedWorkspaces::Ask;
        let workspaces: Vec<(u64, String)> = match after {
            _ if !ask_workspaces => Vec::new(),
            AfterPrompt::Quit => self.workspace_unsaved(),
            AfterPrompt::Close(uid) => match self.tab_ix(*uid) {
                Some(ix) if self.compute_dirty(ix) => self
                    .workspaces
                    .tabs
                    .get(ix)
                    .map(|t| vec![(t.uid, t.name.clone())])
                    .unwrap_or_default(),
                _ => Vec::new(),
            },
            AfterPrompt::ClosePane(_) => Vec::new(),
        };
        let closing = self.closing_panes(after);
        let pdfs: Vec<(PaneId, String)> = self
            .pdf_unsaved_panes()
            .into_iter()
            .filter(|(p, _)| matches!(after, AfterPrompt::Quit) || closing.contains(p))
            .collect();
        let kept = match after {
            AfterPrompt::Quit => Vec::new(),
            _ => self.handles_kept(after),
        };
        let shown: Vec<String> = match after {
            AfterPrompt::Quit => Vec::new(),
            AfterPrompt::ClosePane(p) => self
                .layout
                .state(*p)
                .and_then(|st| st.location.archive_handle().map(str::to_string))
                .into_iter()
                .collect(),
            AfterPrompt::Close(uid) => self
                .tab_ix(*uid)
                .map(|ix| {
                    self.tab_locations(ix)
                        .into_iter()
                        .filter_map(|(_, u)| u.archive_handle().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        };
        let archives: Vec<(VfsUri, String)> = dirty_archives(self)
            .into_iter()
            .filter(|(root, _, _)| {
                let h = root.archive_handle().unwrap_or_default();
                matches!(after, AfterPrompt::Quit)
                    || (shown.iter().any(|s| s == h) && !kept.iter().any(|k| k == h))
            })
            .map(|(root, container, _)| {
                (root, container.file_name().unwrap_or("archive").to_string())
            })
            .collect();
        let busy = match after {
            AfterPrompt::Quit => self.background_work(),
            _ => Vec::new(),
        };
        if workspaces.is_empty() && pdfs.is_empty() && archives.is_empty() && busy.is_empty() {
            return None;
        }
        Some(UnsavedPrompt {
            after: after.clone(),
            names: workspaces.iter().map(|(_, n)| n.clone()).collect(),
            workspaces: workspaces.into_iter().map(|(u, _)| u).collect(),
            pdfs,
            archives,
            busy,
        })
    }

    /// Quit, close a pane or close a tab: ask once when something would be lost, else do
    /// it now.
    pub(crate) fn close_or_ask(&mut self, after: AfterPrompt) -> Result<(), FmError> {
        if let Some(prompt) = self.gather_unsaved(&after) {
            self.unsaved_prompt = Some(prompt);
            return Ok(());
        }
        self.finish_close(&after)
    }

    /// Do the quit or close; workspaces with a remembered answer are saved or dropped
    /// without asking.
    pub(crate) fn finish_close(&mut self, after: &AfterPrompt) -> Result<(), FmError> {
        let remembered: Vec<u64> = match after {
            AfterPrompt::Quit => self
                .workspace_unsaved()
                .into_iter()
                .map(|(u, _)| u)
                .collect(),
            AfterPrompt::Close(uid) => match self.tab_ix(*uid) {
                Some(ix) if self.compute_dirty(ix) => vec![*uid],
                _ => Vec::new(),
            },
            AfterPrompt::ClosePane(_) => Vec::new(),
        };
        if !remembered.is_empty()
            && self.prefs.unsaved_workspaces != crate::prefs::UnsavedWorkspaces::Ask
            && !self.answer_unsaved_unasked(&remembered)
        {
            return Ok(()); // saving failed: the notice says why, nothing is lost
        }
        match after {
            AfterPrompt::Quit => {
                self.running = false;
                Ok(())
            }
            AfterPrompt::Close(uid) => match self.tab_ix(*uid) {
                Some(ix) => self.workspace_close_now(ix),
                None => Ok(()),
            },
            AfterPrompt::ClosePane(p) => {
                if self.layout.panes().len() > 1 {
                    self.layout.close(*p)?;
                    self.drop_terminal(*p);
                    self.media_visibility_changed();
                }
                Ok(())
            }
        }
    }

    /// Answer the prompt: `save` (everything listed), `discard` (don't save) or `cancel`.
    /// `remember` ("Don't ask again") keeps a save or discard answer for workspaces.
    pub fn workspace_resolve_prompt(
        &mut self,
        choice: &str,
        remember: bool,
    ) -> Result<(), FmError> {
        if !matches!(choice, "save" | "discard" | "cancel") {
            return Err(FmError::InvalidParams(format!(
                "choice must be save, discard or cancel, not {choice:?}"
            )));
        }
        let Some(prompt) = self.unsaved_prompt.take() else {
            return Ok(());
        };
        if remember && !prompt.workspaces.is_empty() {
            self.prefs.unsaved_workspaces = match choice {
                "save" => crate::prefs::UnsavedWorkspaces::Save,
                "discard" => crate::prefs::UnsavedWorkspaces::Discard,
                _ => self.prefs.unsaved_workspaces,
            };
        }
        match choice {
            "discard" => {
                for p in &prompt.pdfs {
                    if let Some(v) = self.pdf_view_mut(p.0) {
                        v.fill = None;
                    }
                }
                for (root, _) in &prompt.archives {
                    if let Some(fs) = self.archives.for_uri(root).and_then(|m| m.opened()) {
                        fs.discard();
                    }
                    after_save(self, root);
                }
                self.finish_close(&prompt.after)
            }
            "save" => {
                for uid in &prompt.workspaces {
                    if let Some(ix) = self.tab_ix(*uid) {
                        self.workspace_save(ix, None)?;
                    }
                }
                let pending = PendingClose {
                    after: prompt.after.clone(),
                    pdfs: prompt.pdfs.iter().map(|(p, _)| *p).collect(),
                    archives: prompt.archives.iter().map(|(r, _)| r.clone()).collect(),
                };
                if pending.pdfs.is_empty() && pending.archives.is_empty() {
                    return self.finish_close(&prompt.after);
                }
                self.pending_close = Some(pending.clone());
                for p in &pending.pdfs {
                    if let Err(e) =
                        self.start_save(*p, crate::pdf_form_cmds::SaveTarget::Copy, true, None)
                    {
                        self.pending_close = None;
                        return Err(e);
                    }
                }
                for root in &pending.archives {
                    let Some(mount) = self.archives.for_uri(root).cloned() else {
                        continue;
                    };
                    let request = OpRequest::ArchiveSave(crate::archive_ops::SaveTarget {
                        mount,
                        copy_to: None,
                        backup: None,
                    });
                    if let Err(e) = self.submit_op(request) {
                        self.pending_close = None;
                        return Err(e);
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// A save the prompt started finished: go on once all are done, or give up on the
    /// quit or close when one failed or was skipped (nothing is lost).
    pub(crate) fn close_save_finished(&mut self, state: &OpState) {
        let Some(pending) = self.pending_close.clone() else {
            return;
        };
        if !matches!(state, OpState::Finished) {
            self.pending_close = None;
            return;
        }
        let pdf_open = self
            .pdf_unsaved_panes()
            .iter()
            .any(|(p, _)| pending.pdfs.contains(p));
        let archive_open = pending
            .archives
            .iter()
            .any(|r| self.archives.for_uri(r).is_some_and(|m| m.is_dirty()));
        if !pdf_open && !archive_open {
            self.pending_close = None;
            if let Err(e) = self.finish_close(&pending.after) {
                self.notice = Some(e.to_string());
            }
        }
    }

    /// A save the prompt started failed or was skipped.
    pub(crate) fn close_save_failed(&mut self) {
        self.pending_close = None;
    }
}
