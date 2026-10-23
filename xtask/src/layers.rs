//! `cargo xtask layers` — dependency-layer enforcement (design §3, capability
//! `build-guardrails`).
//!
//! The table below is the registry: every workspace member must appear in it, every
//! entry must exist, and a crate may depend only on equal-or-lower layers. `xtask`
//! itself is a tool (L9) and must not depend on any workspace crate.

use std::fs;
use std::path::{Path, PathBuf};

use crate::util;

/// package name → layer
const TABLE: &[(&str, u8)] = &[
    ("fm-brand", 0),
    ("fm-vfs", 0),
    ("fm-fs-local", 0),
    ("fm-fs-smb", 0),
    ("fm-fs-sftp", 0),
    ("fm-testkit", 0),
    ("fm-ops", 1),
    ("fm-term", 1),
    ("fm-layout", 2),
    ("fm-engine", 3),
    ("fm-ui", 4),
    ("fm-app", 5),
    ("xtask", 9),
];

/// Layer of a registered package, if any.
pub fn layer_of(package: &str) -> Option<u8> {
    TABLE
        .iter()
        .find(|(name, _)| *name == package)
        .map(|(_, layer)| *layer)
}

pub fn run() -> Result<String, String> {
    let root = util::workspace_root()?;
    let members = discover_members(&root)?;
    let mut problems: Vec<String> = Vec::new();

    // 1. Registry ⇄ members must agree.
    for (name, _) in TABLE {
        if !members.iter().any(|m| m.package == *name) {
            problems.push(format!(
                "xtask/src/layers.rs: stale entry `{name}` — no such workspace member; remove it"
            ));
        }
    }
    for member in &members {
        if layer_of(&member.package).is_none() {
            problems.push(format!(
                "{}: unregistered crate `{}` — add it to TABLE in xtask/src/layers.rs (see AGENTS.md §2)",
                member.manifest.display(),
                member.package
            ));
        }
    }

    // 2. Dependency direction.
    for member in &members {
        let own = match layer_of(&member.package) {
            Some(l) => l,
            None => continue, // already reported above
        };
        for dep in dependency_keys(&member.manifest)? {
            if dep == member.package {
                continue;
            }
            if let Some(dep_layer) = layer_of(&dep) {
                if member.package == "xtask" {
                    problems.push(format!(
                        "xtask must not depend on workspace crate `{dep}` — it is a standalone tool"
                    ));
                } else if dep_layer > own {
                    problems.push(format!(
                        "{} (L{own}) depends on `{dep}` (L{dep_layer}) — a crate may only depend on equal or lower layers",
                        member.package
                    ));
                }
            } else if dep.starts_with("fm-") {
                problems.push(format!(
                    "{}: dependency `{dep}` looks like a workspace crate but is not in the layer table",
                    member.package
                ));
            }
        }
    }

    if problems.is_empty() {
        Ok(format!(
            "layers OK: {} crates checked, L0–L5 (+xtask L9)",
            members.len()
        ))
    } else {
        let mut msg = format!("layering violations ({}):\n", problems.len());
        for p in &problems {
            msg.push_str("  - ");
            msg.push_str(p);
            msg.push('\n');
        }
        msg.push_str("see docs/architecture.md §3 and AGENTS.md §2");
        Err(msg)
    }
}

struct Member {
    package: String,
    manifest: PathBuf,
}

/// Expand `workspace.members` globs (`crates/*`, `apps/*`, `xtask`) into member dirs.
fn discover_members(root: &Path) -> Result<Vec<Member>, String> {
    let manifest_path = root.join("Cargo.toml");
    let text = util::read(&manifest_path)?;
    let value: toml::Value =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let members = value
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .ok_or_else(|| "root Cargo.toml: workspace.members missing".to_string())?;

    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in members {
        let pattern = entry.as_str().ok_or_else(|| {
            "root Cargo.toml: workspace.members entry is not a string".to_string()
        })?;
        match pattern.split_once('*') {
            Some((prefix, _)) => {
                let parent = root.join(prefix);
                let entries = fs::read_dir(&parent)
                    .map_err(|e| format!("cannot read {}: {e}", parent.display()))?;
                for dir_entry in entries {
                    let dir_entry =
                        dir_entry.map_err(|e| format!("cannot read {}: {e}", parent.display()))?;
                    let path = dir_entry.path();
                    if path.is_dir() && path.join("Cargo.toml").is_file() {
                        dirs.push(path);
                    }
                }
            }
            None => {
                let path = root.join(pattern);
                if path.join("Cargo.toml").is_file() {
                    dirs.push(path);
                }
            }
        }
    }
    dirs.sort();

    let mut out = Vec::with_capacity(dirs.len());
    for dir in dirs {
        let manifest = dir.join("Cargo.toml");
        let text = util::read(&manifest)?;
        let value: toml::Value =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
        let package = value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .ok_or_else(|| format!("{}: missing package.name", manifest.display()))?
            .to_string();
        out.push(Member { package, manifest });
    }
    Ok(out)
}

/// Keys of `[dependencies]` + `[dev-dependencies]`.
fn dependency_keys(manifest: &Path) -> Result<Vec<String>, String> {
    let text = util::read(manifest)?;
    let value: toml::Value =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut keys = Vec::new();
    for section in ["dependencies", "dev-dependencies"] {
        if let Some(table) = value.get(section).and_then(|s| s.as_table()) {
            keys.extend(table.iter().map(|(k, _)| k.clone()));
        }
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_unique_package_names() {
        for (i, (name, _)) in TABLE.iter().enumerate() {
            for (other, _) in &TABLE[i + 1..] {
                assert_ne!(name, other, "duplicate entry for {name}");
            }
        }
    }

    #[test]
    fn layers_are_ordered_and_non_decreasing_per_group() {
        // Sanity: the documented shape L0 < L1 < … < L5 < L9 holds for the crate map.
        assert_eq!(layer_of("fm-vfs"), Some(0));
        assert_eq!(layer_of("fm-ops"), Some(1));
        assert_eq!(layer_of("fm-layout"), Some(2));
        assert_eq!(layer_of("fm-engine"), Some(3));
        assert_eq!(layer_of("fm-ui"), Some(4));
        assert_eq!(layer_of("fm-app"), Some(5));
        assert_eq!(layer_of("xtask"), Some(9));
    }

    #[test]
    fn every_member_is_registered() {
        let root = util::workspace_root().expect("workspace root");
        let members = discover_members(&root).expect("members");
        assert!(!members.is_empty(), "no workspace members found");
        for member in members {
            assert!(
                layer_of(&member.package).is_some(),
                "crate `{}` is not registered in the layer table",
                member.package
            );
        }
    }
}
