//! `cargo xtask ci` — the full gate, in order, stopping at the first failure
//! (capability `build-guardrails`).

use std::path::Path;

use crate::util;

pub fn run() -> Result<String, String> {
    let root = util::workspace_root()?;

    step(
        &root,
        "format",
        &["fmt", "--all", "--", "--check"],
        "cargo fmt --all -- --check",
    )?;
    step(
        &root,
        "clippy",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        "cargo clippy --workspace --all-targets -- -D warnings",
    )?;
    step(
        &root,
        "tests",
        &["test", "--workspace"],
        "cargo test --workspace",
    )?;

    let layers = crate::layers::run()?;
    let brand = crate::brand::check()?;

    Ok(format!(
        "ci OK\n  format ✓\n  clippy ✓\n  tests ✓\n  {layers}\n  {brand}"
    ))
}

fn step(root: &Path, label: &str, args: &[&str], hint: &str) -> Result<(), String> {
    println!("\n=== {label}: cargo {} ===", args.join(" "));
    util::cargo(args, root, hint)
}
