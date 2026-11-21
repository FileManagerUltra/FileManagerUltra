//! `cargo xtask ci` — the full gate, in order, stopping at the first failure
//! (capability `build-guardrails`). `.github/workflows/ci.yml` runs the same checks.

use std::path::Path;

use crate::hooks::{CLIPPY_ARGS, DEP_TOOLS};
use crate::util;

pub fn run() -> Result<String, String> {
    let root = util::workspace_root()?;

    // Fail early, before the long steps, when a required tool is missing.
    let missing: Vec<&str> = DEP_TOOLS
        .iter()
        .filter(|t| !t.installed(&root))
        .map(|t| t.install)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "the gate needs these cargo tools; install them with:\n  {}",
            missing.join("\n  ")
        ));
    }

    step(
        &root,
        "format",
        &["fmt", "--all", "--", "--check"],
        "cargo fmt --all -- --check",
    )?;
    step(
        &root,
        "clippy",
        CLIPPY_ARGS,
        &format!("cargo {}", CLIPPY_ARGS.join(" ")),
    )?;
    step(
        &root,
        "tests",
        &["test", "--workspace", "--locked"],
        "cargo test --workspace --locked",
    )?;

    let layers = crate::layers::run()?;
    let brand = crate::brand::check()?;

    let mut done = Vec::new();
    for tool in DEP_TOOLS {
        step(
            &root,
            tool.name,
            tool.args,
            &format!("cargo {}", tool.args.join(" ")),
        )?;
        done.push(format!("  {} ✓", tool.name));
    }

    Ok(format!(
        "ci OK\n  format ✓\n  clippy ✓\n  tests ✓\n  {layers}\n  {brand}\n{}",
        done.join("\n")
    ))
}

fn step(root: &Path, label: &str, args: &[&str], hint: &str) -> Result<(), String> {
    println!("\n=== {label}: cargo {} ===", args.join(" "));
    util::cargo(args, root, hint)
}
