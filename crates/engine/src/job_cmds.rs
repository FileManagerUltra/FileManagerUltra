//! Job commands: cancel, resolve a parked conflict, dismiss a finished job.

use fm_ops::{Choice, Decision};
use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::jobs::JobId;
use crate::registry::{CommandSpec, Params};

fn job_id(params: &Params) -> Result<JobId, FmError> {
    object(params)?
        .and_then(|m| m.get("job"))
        .and_then(Params::as_u64)
        .ok_or_else(|| FmError::InvalidParams("job: integer is required".to_string()))
}

fn parse_choice(params: &Params) -> Result<Decision, FmError> {
    let map = object(params)?;
    let choice = match map.and_then(|m| m.get("choice")).and_then(Params::as_str) {
        Some("overwrite") => Choice::Overwrite,
        Some("keep_both") => Choice::KeepBoth,
        Some("skip") => Choice::Skip,
        Some("cancel") => Choice::Cancel,
        _ => {
            return Err(FmError::InvalidParams(
                "choice must be overwrite, keep_both, skip or cancel".to_string(),
            ));
        }
    };
    let apply_to_all = match map.and_then(|m| m.get("apply_to_all")) {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| FmError::InvalidParams("apply_to_all must be a boolean".to_string()))?,
    };
    Ok(Decision {
        choice,
        apply_to_all,
    })
}

/// `job.cancel` — cooperative cancel of a running file job.
pub fn cancel() -> CommandSpec {
    CommandSpec {
        id: "job.cancel",
        label: "Cancel Job",
        menu: &["Activity", "Cancel"],
        shortcut: None,
        params: "job: integer",
        enabled: |s| s.fileops.running() > 0,
        run: |s, params| {
            let id = job_id(params)?;
            s.cancel_op(id)
        },
    }
}

/// `job.resolve_conflict` — answer the conflict a parked job waits on.
pub fn resolve_conflict() -> CommandSpec {
    CommandSpec {
        id: "job.resolve_conflict",
        label: "Resolve Conflict",
        menu: &["Activity", "Resolve Conflict"],
        shortcut: None,
        params: "job: integer, choice: overwrite|keep_both|skip|cancel, apply_to_all?: bool",
        enabled: |s| s.fileops.pending_conflict().is_some(),
        run: |s, params| {
            let id = job_id(params)?;
            let decision = parse_choice(params)?;
            s.resolve_conflict(id, decision)
        },
    }
}

/// `job.dismiss` — remove a finished job (and its report) from the panel.
pub fn dismiss() -> CommandSpec {
    CommandSpec {
        id: "job.dismiss",
        label: "Dismiss Job",
        menu: &["Activity", "Dismiss"],
        shortcut: None,
        params: "job: integer",
        enabled: |s| s.fileops.jobs().iter().any(|j| !j.state.is_running()),
        run: |s, params| {
            let id = job_id(params)?;
            if s.fileops.dismiss(id) {
                Ok(())
            } else {
                Err(FmError::NotFound(format!("finished job {id}")))
            }
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![cancel(), resolve_conflict(), dismiss()]
}
