//! `cargo xtask` — the one tooling entry point (design D7).
//!
//! Subcommands: `layers` (layering enforcement), `brand-check` (stale names, colour
//! literals), `rename "<NewName>"`, `ci` (the full gate).

mod brand;
mod ci;
mod layers;
mod util;

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match dispatch() {
        Ok(msg) => {
            if !msg.is_empty() {
                println!("{msg}");
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch() -> Result<String, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("layers") => layers::run(),
        Some("brand-check") => brand::check(),
        Some("rename") => match args.get(1) {
            Some(name) => brand::rename(name),
            None => Err("usage: cargo xtask rename \"<NewName>\"".to_string()),
        },
        Some("packager-config") => brand::packager_config(),
        Some("ci") => ci::run(),
        Some("--version") | Some("-V") => Ok(format!("xtask {}", env!("CARGO_PKG_VERSION"))),
        _ => Err(
            "usage: cargo xtask <layers|brand-check|rename <name>|packager-config|ci|--version>"
                .to_string(),
        ),
    }
}
