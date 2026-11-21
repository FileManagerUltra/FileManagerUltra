//! Git hooks (capability `build-guardrails`).
//!
//! `cargo xtask hooks` points git at the versioned `.githooks/` folder. The hook scripts
//! there only call back into `cargo xtask hook <name>`, so the checks live here, in Rust,
//! and behave the same on every OS. Skip a hook once with `--no-verify`.

use std::path::Path;
use std::process::Command;

use crate::util;

/// The folder git runs hooks from once installed (relative to the workspace root).
pub const HOOKS_DIR: &str = ".githooks";

/// Hook names with a script in [`HOOKS_DIR`].
pub const HOOKS: &[&str] = &["pre-commit", "pre-push"];

/// `cargo xtask hooks [install|uninstall]`.
pub fn manage(action: Option<&str>) -> Result<String, String> {
    let root = util::workspace_root()?;
    match action {
        None | Some("install") => {
            for hook in HOOKS {
                let script = root.join(HOOKS_DIR).join(hook);
                if !script.is_file() {
                    return Err(format!("missing hook script {}", script.display()));
                }
            }
            git(&root, &["config", "core.hooksPath", HOOKS_DIR])?;
            Ok(format!(
                "git hooks installed from {HOOKS_DIR}/:\n  \
                 pre-commit: format, layers, branding\n  \
                 pre-push:   clippy, dependency policy, unused dependencies\n\
                 Skip once with --no-verify; remove with `cargo xtask hooks uninstall`."
            ))
        }
        Some("uninstall") => {
            // Exit code 5 means the key was not set: already uninstalled.
            let status = Command::new("git")
                .args(["config", "--unset", "core.hooksPath"])
                .current_dir(&root)
                .status()
                .map_err(|e| format!("cannot start git: {e}"))?;
            match status.code() {
                Some(0 | 5) => Ok("git hooks removed (core.hooksPath unset)".to_string()),
                _ => Err("git config --unset core.hooksPath failed".to_string()),
            }
        }
        Some(other) => Err(format!(
            "unknown hooks action {other:?}; usage: cargo xtask hooks [install|uninstall]"
        )),
    }
}

/// `cargo xtask hook <name>`: the body of one git hook.
pub fn run(name: Option<&str>) -> Result<String, String> {
    let root = util::workspace_root()?;
    match name {
        Some("pre-commit") => pre_commit(&root),
        Some("pre-push") => pre_push(&root),
        other => Err(format!(
            "unknown hook {other:?}; expected one of: {}",
            HOOKS.join(", ")
        )),
    }
}

/// Fast checks only (seconds): formatting, layering, branding.
fn pre_commit(root: &Path) -> Result<String, String> {
    println!("pre-commit: format, layers, branding (skip once with --no-verify)");
    util::cargo(
        &["fmt", "--all", "--", "--check"],
        root,
        "cargo fmt --all   (then stage the result)",
    )?;
    let layers = crate::layers::run()?;
    let brand = crate::brand::check()?;
    Ok(format!("pre-commit OK\n  format ✓\n  {layers}\n  {brand}"))
}

/// Slower checks (a minute or more on a warm cache): clippy and the dependency policy.
/// Tests stay in `cargo xtask ci` and CI: they are too heavy for every push.
fn pre_push(root: &Path) -> Result<String, String> {
    println!("pre-push: clippy, dependency policy (skip once with --no-verify)");
    let mut clippy = Command::new("cargo");
    clippy.args(CLIPPY_ARGS).current_dir(root);
    // Clippy across the workspace can exhaust memory on a desktop (docs/development.md);
    // keep it modest unless the developer chose otherwise.
    if std::env::var_os("CARGO_BUILD_JOBS").is_none() {
        clippy.env("CARGO_BUILD_JOBS", "2");
    }
    let status = clippy
        .status()
        .map_err(|e| format!("cannot start cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "failed step — reproduce with: cargo {}",
            CLIPPY_ARGS.join(" ")
        ));
    }
    let mut report = vec!["pre-push OK".to_string(), "  clippy ✓".to_string()];
    for tool in DEP_TOOLS {
        if !tool.installed(root) {
            report.push(format!(
                "  {} skipped (not installed: {}; CI still runs it)",
                tool.name, tool.install
            ));
            continue;
        }
        util::cargo(tool.args, root, &format!("cargo {}", tool.args.join(" ")))?;
        report.push(format!("  {} ✓", tool.name));
    }
    Ok(report.join("\n"))
}

/// The clippy invocation shared by the pre-push hook, `cargo xtask ci` and CI.
pub const CLIPPY_ARGS: &[&str] = &[
    "clippy",
    "--workspace",
    "--all-targets",
    "--all-features",
    "--locked",
    "--",
    "-D",
    "warnings",
];

/// A cargo subcommand installed separately (not part of rustup).
pub struct DepTool {
    pub name: &'static str,
    pub args: &'static [&'static str],
    pub install: &'static str,
}

impl DepTool {
    /// Whether `cargo <subcommand> --version` runs.
    pub fn installed(&self, root: &Path) -> bool {
        let Some(sub) = self.args.first() else {
            return false;
        };
        Command::new("cargo")
            .args([sub, "--version"])
            .current_dir(root)
            .output()
            .is_ok_and(|o| o.status.success())
    }
}

/// Dependency checks: licences/advisories/sources (`deny.toml`) and unused dependencies.
/// Vendored crates are excluded from the unused-dependency scan: they are upstream code.
pub const DEP_TOOLS: &[DepTool] = &[
    DepTool {
        name: "dependency policy",
        args: &["deny", "--locked", "check"],
        install: "cargo install cargo-deny --locked",
    },
    DepTool {
        name: "unused dependencies",
        args: &["machete", "crates", "apps", "xtask"],
        install: "cargo install cargo-machete --locked",
    },
];

fn git(root: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|e| format!("cannot start git: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {} failed", args.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hook_script_calls_back_into_xtask() {
        let root = util::workspace_root().unwrap();
        for hook in HOOKS {
            let script = util::read(&root.join(HOOKS_DIR).join(hook)).unwrap();
            assert!(script.starts_with("#!/bin/sh\n"), "{hook} needs a POSIX shebang");
            assert!(
                script.contains(&format!("exec cargo xtask hook {hook}")),
                "{hook} must delegate to `cargo xtask hook {hook}`"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn hook_scripts_are_executable() {
        use std::os::unix::fs::PermissionsExt;
        let root = util::workspace_root().unwrap();
        for hook in HOOKS {
            let mode = std::fs::metadata(root.join(HOOKS_DIR).join(hook))
                .unwrap()
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "{hook} is not executable");
        }
    }

    #[test]
    fn unknown_hook_and_action_are_errors() {
        assert!(run(Some("post-merge")).is_err());
        assert!(run(None).is_err());
        assert!(manage(Some("reinstall")).is_err());
    }

    #[test]
    fn clippy_denies_warnings_on_every_target() {
        for flag in ["--workspace", "--all-targets", "--all-features", "--locked"] {
            assert!(CLIPPY_ARGS.contains(&flag), "missing {flag}");
        }
        assert!(CLIPPY_ARGS.ends_with(&["-D", "warnings"]));
    }
}
