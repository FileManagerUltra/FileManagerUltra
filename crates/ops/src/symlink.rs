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
    fn hostile_inputs_are_total() {
        let p = SymlinkPolicy::DerefWithinRoot;
        assert!(!p.allows_deref("", ""));
        assert!(!p.allows_deref("/root", ""));
        assert!(!p.allows_deref("", "/root/x"));
        assert!(!p.allows_deref("/root", "root/x"));
    }
}
