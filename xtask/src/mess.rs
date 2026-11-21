//! `cargo xtask mess` — a report-only code-health scan (capability `build-guardrails`).
//!
//! Lists the longest and most complex functions and bool-heavy signatures, using clippy
//! lints the gate does not enforce. It never fails on findings: CI posts the report to
//! the run summary so growth shows up in review, and refactors can start at the top.

use std::collections::BTreeSet;
use std::process::Command;

use regex::Regex;

use crate::util;

/// Lints scanned, with the label used in the report.
const LINTS: &[(&str, &str)] = &[
    (
        "clippy::too_many_lines",
        "long functions (lines, limit 100)",
    ),
    (
        "clippy::cognitive_complexity",
        "complex functions (cognitive complexity, limit 25)",
    ),
    (
        "clippy::struct_excessive_bools",
        "structs with more than 3 bools",
    ),
    (
        "clippy::fn_params_excessive_bools",
        "functions with more than 3 bool parameters",
    ),
];

/// Message prefixes clippy prints for each lint in [`LINTS`] (same order).
const MESSAGES: &[&str] = &[
    "this function has too many lines",
    "the function has a cognitive complexity of",
    "more than 3 bools in a struct",
    "more than 3 bools in function parameters",
];

/// How many entries each section lists.
const TOP: usize = 15;

pub fn run() -> Result<String, String> {
    let root = util::workspace_root()?;
    let mut args = vec![
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--locked",
        "--message-format=short",
        "--",
    ];
    for (lint, _) in LINTS {
        args.extend(["-W", lint]);
    }
    println!("mess: cargo {}", args.join(" "));
    let out = Command::new("cargo")
        .args(&args)
        .current_dir(&root)
        .output()
        .map_err(|e| format!("cannot start cargo: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(format!(
            "clippy did not finish; fix the build first (cargo xtask ci)\n{}",
            tail(&stderr, 30)
        ));
    }
    Ok(report(&parse(&stderr)?))
}

/// One finding: lint index into [`LINTS`], location, and the measured size if any.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Finding {
    lint: usize,
    size: u32,
    location: String,
}

/// Findings from clippy's short output (`path:line:col: warning: message`), deduplicated
/// (a file is linted once per target that includes it).
fn parse(output: &str) -> Result<Vec<Finding>, String> {
    let line_re = Regex::new(r"^([^\s:][^:]*):(\d+):\d+: warning: (.+)$")
        .map_err(|e| format!("bad regex: {e}"))?;
    let size_re = Regex::new(r"\((\d+)/\d+\)").map_err(|e| format!("bad regex: {e}"))?;
    let mut found = BTreeSet::new();
    for line in output.lines() {
        let Some(caps) = line_re.captures(line.trim_end()) else {
            continue;
        };
        let (Some(file), Some(row), Some(message)) = (caps.get(1), caps.get(2), caps.get(3)) else {
            continue;
        };
        let message = message.as_str();
        let Some(lint) = MESSAGES.iter().position(|m| message.starts_with(m)) else {
            continue;
        };
        let size = size_re
            .captures(message)
            .and_then(|c| c.get(1))
            .and_then(|n| n.as_str().parse().ok())
            .unwrap_or(0);
        found.insert(Finding {
            lint,
            size,
            location: format!("{}:{}", file.as_str(), row.as_str()),
        });
    }
    Ok(found.into_iter().collect())
}

fn report(findings: &[Finding]) -> String {
    let mut out = vec!["Mess report (informational, never fails the build)".to_string()];
    for (lint, (_, label)) in LINTS.iter().enumerate() {
        let mut hits: Vec<&Finding> = findings.iter().filter(|f| f.lint == lint).collect();
        hits.sort_by(|a, b| {
            b.size
                .cmp(&a.size)
                .then_with(|| a.location.cmp(&b.location))
        });
        out.push(String::new());
        out.push(format!("{label}: {}", hits.len()));
        for f in hits.iter().take(TOP) {
            if f.size > 0 {
                out.push(format!("  {:>5}  {}", f.size, f.location));
            } else {
                out.push(format!("         {}", f.location));
            }
        }
        if hits.len() > TOP {
            out.push(format!("         … and {} more", hits.len() - TOP));
        }
    }
    out.join("\n")
}

/// The last `n` lines of `text`.
fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines
        .get(lines.len().saturating_sub(n)..)
        .unwrap_or_default()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
    Checking fm-ui v0.3.1
crates/ui/src/jobs.rs:220:1: warning: this function has too many lines (411/100)
crates/ui/src/preferences.rs:346:5: warning: this function has too many lines (685/100)
crates/ui/src/preferences.rs:346:5: warning: this function has too many lines (685/100)
crates/engine/src/media_cmds.rs:744:1: warning: the function has a cognitive complexity of (40/25)
crates/ui/src/state.rs:10:1: warning: more than 3 bools in a struct
crates/ui/src/other.rs:3:1: warning: unused variable: `x`
warning: `fm-ui` (lib) generated 3 warnings
";

    #[test]
    fn parses_and_deduplicates_findings() {
        let found = parse(SAMPLE).unwrap();
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(found.contains(&Finding {
            lint: 0,
            size: 685,
            location: "crates/ui/src/preferences.rs:346".into(),
        }));
        assert!(found.iter().any(|f| f.lint == 1 && f.size == 40));
        assert!(found.iter().any(|f| f.lint == 2 && f.size == 0));
    }

    #[test]
    fn report_lists_largest_first() {
        let text = report(&parse(SAMPLE).unwrap());
        let big = text.find("685").unwrap();
        let small = text.find("411").unwrap();
        assert!(big < small, "{text}");
        assert!(text.contains("long functions (lines, limit 100): 2"));
        assert!(text.contains("functions with more than 3 bool parameters: 0"));
    }

    #[test]
    fn lint_and_message_tables_line_up() {
        assert_eq!(LINTS.len(), MESSAGES.len());
    }

    #[test]
    fn hostile_output_is_ignored() {
        let junk = "::::\n:1:1: warning: x\nfoo.rs:notanumber:1: warning: this function has too many lines (9/1)\n";
        assert!(parse(junk).unwrap().is_empty());
        assert_eq!(tail("", 5), "");
    }
}
