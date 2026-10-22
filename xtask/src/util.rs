//! Shared helpers: workspace paths, source walking, subprocesses.

use std::fs;

use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository root (this crate lives at `<root>/xtask`).
pub fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot resolve workspace root".to_string())
}

/// Directories never scanned (build output, VCS, OpenSpec and dev logs, vendored trees).
pub const SKIP_DIRS: &[&str] = &[
    "target",
    ".git",
    "openspec",
    "log",
    "node_modules",
    ".opencode",
    "corpus",
];

/// Walk `root` recursively, skipping [`SKIP_DIRS`] and `skip_extra`, collecting files
/// whose extension is in `exts` (`"rs"`, `"toml"`, …). Sorted for stable output.
pub fn walk_files(root: &Path, skip_extra: &[&str], exts: &[&str]) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy().into_owned();
            if path.is_dir() {
                if SKIP_DIRS.contains(&name.as_str()) || skip_extra.contains(&name.as_str()) {
                    continue;
                }
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.contains(&e))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Path relative to the workspace root, for readable messages.
pub fn rel<'a>(root: &'a Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(root).unwrap_or(path)
}

/// Run `cargo <args>` in the workspace root with inherited stdio.
pub fn cargo(args: &[&str], root: &Path, hint: &str) -> Result<(), String> {
    let status = Command::new("cargo")
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|e| format!("cannot start cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("failed step — reproduce with: {hint}"))
    }
}

/// Read a UTF-8 file with a path-aware error.
pub fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Write a file with a path-aware error.
pub fn write(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|e| format!("cannot write {}: {e}", path.display()))
}
