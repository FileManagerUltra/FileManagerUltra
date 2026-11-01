//! Branch, tag and recent-checkout listing for the diff revision picker (read-only).

use std::path::Path;

use gix::bstr::ByteSlice;

use crate::GitError;

/// Caps so a repository with huge ref counts cannot flood the UI.
const MAX_REFS: usize = 5_000;
const MAX_RECENT: usize = 10;
const MAX_REFLOG: usize = 500;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Refs {
    /// The checked-out branch, when HEAD is not detached.
    pub current: Option<String>,
    /// Branches recently checked out, newest first (from the HEAD reflog).
    pub recent: Vec<String>,
    pub local: Vec<String>,
    /// `origin/main` style names.
    pub remote: Vec<String>,
    pub tags: Vec<String>,
    /// A cap was hit; the lists are partial.
    pub truncated: bool,
}

fn short(name: &gix::refs::FullNameRef) -> String {
    name.shorten().to_str_lossy().into_owned()
}

pub fn list(root: &Path) -> Result<Refs, GitError> {
    let repo = crate::open(root)?;
    let err = |e: &dyn std::fmt::Display| GitError(e.to_string());
    let mut out = Refs::default();
    let platform = repo.references().map_err(|e| err(&e))?;
    let collect = |iter: gix::reference::iter::Iter<'_, '_>,
                   into: &mut Vec<String>,
                   truncated: &mut bool,
                   skip_head: bool| {
        for r in iter.take(MAX_REFS * 2).flatten() {
            if into.len() >= MAX_REFS {
                *truncated = true;
                break;
            }
            let name = short(r.name());
            if skip_head && name.ends_with("/HEAD") {
                continue;
            }
            into.push(name);
        }
    };
    let mut truncated = false;
    collect(
        platform.local_branches().map_err(|e| err(&e))?,
        &mut out.local,
        &mut truncated,
        false,
    );
    collect(
        platform.remote_branches().map_err(|e| err(&e))?,
        &mut out.remote,
        &mut truncated,
        true,
    );
    collect(
        platform.tags().map_err(|e| err(&e))?,
        &mut out.tags,
        &mut truncated,
        false,
    );
    out.truncated = truncated;
    out.local.sort();
    out.remote.sort();
    out.tags.sort();
    out.current = repo.head_name().ok().flatten().map(|n| short(n.as_ref()));

    out.recent = recent(&repo, &out.local);
    Ok(out)
}

/// Branch names from "checkout: moving from A to B" reflog lines, newest first, unique and
/// still existing.
fn recent(repo: &gix::Repository, local: &[String]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let Ok(head) = repo.find_reference("HEAD") else {
        return found;
    };
    let mut platform = head.log_iter();
    let Ok(Some(iter)) = platform.rev() else {
        return found;
    };
    // Bound the lines read, not the good ones, so a corrupt log cannot spin forever.
    for line in iter.take(MAX_REFLOG).flatten() {
        let msg = line.message.to_str_lossy();
        let Some(rest) = msg.strip_prefix("checkout: moving from ") else {
            continue;
        };
        let Some((from, to)) = rest.split_once(" to ") else {
            continue;
        };
        for name in [to, from] {
            if local.iter().any(|b| b == name) && !found.iter().any(|f| f == name) {
                found.push(name.to_string());
            }
        }
        if found.len() >= MAX_RECENT {
            break;
        }
    }
    found.truncate(MAX_RECENT);
    found
}
