//! Symlink rules for write operations (`security` capability — see
//! `openspec/specs/security/spec.md` and `docs/architecture.md` §12).
//!
//! Two rules, both fixed now so `ops` tests (change 3) implement an agreed contract:
//!
//! 1. **Destination containment (not optional, no policy flag):** a write never follows a
//!    symlink that resolves outside the destination root it was handed. Copying into
//!    `dest/` when `dest/evil -> /etc` fails with an error; it never writes to `/etc`.
//! 2. **Source treatment** — [`SymlinkPolicy`] decides what a *source* link becomes:
//!    the link itself, or its contents when they resolve inside the same root.

/// How a source symlink is treated by copy/move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SymlinkPolicy {
    /// Copy/move the link itself (never dereference). The safe default: a link can point
    /// anywhere, and materializing its target would silently copy data from outside the
    /// selected tree.
    #[default]
    CopyLink,
    /// Copy the link's contents, but only while the target resolves inside the same
    /// root as the link. Outside the root: fail, never follow.
    DerefWithinRoot,
}

impl SymlinkPolicy {
    /// Whether `target` (already resolved against `root`) may be dereferenced.
    ///
    /// `root` is the tree the user selected; `target` is the fully resolved path of the
    /// link's contents. Pure path logic — no filesystem access, total on any input.
    pub fn allows_deref(self, root: &str, target: &str) -> bool {
        match self {
            Self::CopyLink => false,
            Self::DerefWithinRoot => {
                if root == "/" {
                    // A "/" root contains every absolute target.
                    return target.starts_with('/');
                }
                let root = root.strip_suffix('/').unwrap_or(root);
                if root.is_empty() {
                    // An empty root contains nothing.
                    return false;
                }
                target
                    .strip_prefix(root)
                    .is_some_and(|rest| rest.starts_with('/'))
            }
        }
    }
}

/// Lexically normalise an absolute path into `(prefix, segments)`; the prefix is `""` for a
/// rooted Unix path or `"C:"` for a drive path. `None` for relative paths and for `..` that
/// would climb above the root. Pure — nothing is resolved on disk.
fn absolute_segments(path: &str) -> Option<(String, Vec<String>)> {
    let unified = path.replace('\\', "/");
    let mut parts = unified.split('/');
    let first = parts.next()?;
    let prefix = if first.is_empty() {
        String::new()
    } else if first.len() == 2
        && first.ends_with(':')
        && first
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    {
        first.to_string()
    } else {
        return None;
    };
    let mut segments: Vec<String> = Vec::new();
    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other.to_string()),
        }
    }
    Some((prefix, segments))
}

/// Where a copied symlink should point. An **absolute** `target` that lies inside `src_root`
/// (the copied tree) is moved to the same place under `dst_root`; relative targets, targets
/// outside the tree and anything unparsable return `None` (copy the link verbatim). The
/// result can never contain `..`, so it stays inside `dst_root`. Pure path logic.
pub fn retarget(target: &str, src_root: &str, dst_root: &str) -> Option<String> {
    let (tp, ts) = absolute_segments(target)?;
    let (sp, ss) = absolute_segments(src_root)?;
    let (dp, ds) = absolute_segments(dst_root)?;
    if tp != sp || !ts.starts_with(&ss) {
        return None;
    }
    let mut out = dp;
    let rebuilt: Vec<&str> = ds
        .iter()
        .chain(ts.iter().skip(ss.len()))
        .map(String::as_str)
        .collect();
    out.push('/');
    out.push_str(&rebuilt.join("/"));
    Some(if target.contains('\\') {
        out.replace('/', "\\")
    } else {
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_never_dereferences() {
        assert_eq!(SymlinkPolicy::default(), SymlinkPolicy::CopyLink);
        assert!(!SymlinkPolicy::default().allows_deref("/home/u/docs", "/home/u/docs/a"));
    }

    #[test]
    fn deref_only_inside_the_same_root() {
        let p = SymlinkPolicy::DerefWithinRoot;
        assert!(p.allows_deref("/home/u/docs", "/home/u/docs/sub/img.png"));
        assert!(!p.allows_deref("/home/u/docs", "/home/u/other/img.png"));
        assert!(!p.allows_deref("/home/u/docs", "/etc/passwd"));
        // Sibling-prefix trap: /home/u/docs2 is NOT inside /home/u/docs
        assert!(!p.allows_deref("/home/u/docs", "/home/u/docs2/img.png"));
    }

    #[test]
    fn root_slash_contains_all_absolute_targets() {
        let p = SymlinkPolicy::DerefWithinRoot;
        assert!(p.allows_deref("/", "/etc/passwd"));
        assert!(!p.allows_deref("/", "relative/target"));
    }

    #[test]
    fn retarget_moves_inside_links_to_the_new_root() {
        assert_eq!(
            retarget("/src/proj/lib/a.js", "/src/proj", "/dst/proj2").as_deref(),
            Some("/dst/proj2/lib/a.js")
        );
        assert_eq!(
            retarget("/src/proj", "/src/proj", "/dst/p").as_deref(),
            Some("/dst/p")
        );
        assert_eq!(
            retarget("/src/proj/x/../lib", "/src/proj", "/dst/p").as_deref(),
            Some("/dst/p/lib")
        );
        assert_eq!(retarget("/x/y", "/", "/dst").as_deref(), Some("/dst/x/y"));
    }

    #[test]
    fn retarget_leaves_other_links_alone() {
        // relative, outside, sibling-prefix, escaping via `..`, broken input
        assert_eq!(retarget("../lib/a.js", "/src/proj", "/dst"), None);
        assert_eq!(retarget("/etc/passwd", "/src/proj", "/dst"), None);
        assert_eq!(retarget("/src/proj2/a", "/src/proj", "/dst"), None);
        assert_eq!(retarget("/src/proj/../etc", "/src/proj", "/dst"), None);
        assert_eq!(retarget("/../..", "/src/proj", "/dst"), None);
        assert_eq!(retarget("", "/src/proj", "/dst"), None);
        assert_eq!(retarget("/src/proj/a", "rel", "/dst"), None);
        assert_eq!(retarget("/src/proj/a", "/src/proj", "rel"), None);
    }

    #[test]
    fn retarget_handles_windows_paths() {
        assert_eq!(
            retarget("D:\\src\\proj\\a.txt", "C:/src/proj", "D:/out/proj"),
            None,
            "a target on another drive is outside the copied tree"
        );
        assert_eq!(
            retarget("C:\\src\\proj\\a.txt", "C:/src/proj", "D:/out/proj").as_deref(),
            Some("D:\\out\\proj\\a.txt")
        );
        assert_eq!(
            retarget("C:\\src\\proj\\a.txt", "C:/src/proj", "C:/out/p").as_deref(),
            Some("C:\\out\\p\\a.txt")
        );
    }

    #[test]
    fn hostile_inputs_are_total() {
        let p = SymlinkPolicy::DerefWithinRoot;
        assert!(!p.allows_deref("", ""));
        assert!(!p.allows_deref("/root", ""));
        assert!(!p.allows_deref("", "/root/x"));
        assert!(!p.allows_deref("/root", "root/x"));
    }
}
