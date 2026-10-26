//! `cargo xtask brand-check` and `cargo xtask rename` — the rename story
//! (design D8, capability `branding`).

use std::fs;
use std::path::Path;

use regex::Regex;

use crate::util;

/// Identity as written in `crates/brand/src/lib.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brand {
    pub name: String,
    pub slug: String,
}

const BRAND_LIB: &str = "crates/brand/src/lib.rs";
const APP_MANIFEST: &str = "apps/fm-app/Cargo.toml";
const HISTORY: &str = "xtask/rename_history.txt";

// ---------------------------------------------------------------- brand-check

/// Fail when a stale product name survives, when the current name is hard-coded
/// outside `brand`, or when a `Color32::` literal appears outside `theme.rs`.
pub fn check() -> Result<String, String> {
    let root = util::workspace_root()?;
    let brand = read_brand(&root)?;
    let stale: Vec<String> = read_history(&root)?
        .into_iter()
        .filter(|token| token != &brand.name && token != &brand.slug)
        .collect();
    let all = util::walk_files(&root, &[], &["rs", "toml", "desktop", "plist", "xml"])?;

    let brand_dir = root.join("crates/brand");
    let mut problems: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    for path in &all {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue, // binary or unreadable: nothing to scan
        };
        scanned += 1;
        let rel = util::rel(&root, path).display().to_string();
        let in_brand = path.starts_with(&brand_dir);

        for (line_no, line) in text.lines().enumerate() {
            for token in &stale {
                if line.contains(token.as_str()) {
                    problems.push(format!(
                        "{rel}:{}: stale product name {token:?} (rename history) — remove it or finish the rename",
                        line_no + 1
                    ));
                }
            }
            if !in_brand && !brand.name.is_empty() && line.contains(&brand.name) {
                problems.push(format!(
                    "{rel}:{}: hard-coded product name {:?} — read fm_brand::APP_NAME instead",
                    line_no + 1,
                    brand.name
                ));
            }
        }
    }

    // Colour literals live in `fm-ui/src/theme.rs` only (capability `theming`).
    for path in &all {
        let rel_str = path.to_string_lossy().into_owned();
        let in_ui_scope = rel_str.contains("crates/ui/") || rel_str.contains("apps/");
        if !in_ui_scope || path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("theme.rs") {
            continue;
        }
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for (line_no, line) in text.lines().enumerate() {
            if line.contains("Color32::") || line.contains("Color32 {") {
                problems.push(format!(
                    "{}:{}: colour literal outside theme.rs — use a semantic token from fm_ui::theme",
                    util::rel(&root, path).display(),
                    line_no + 1
                ));
            }
        }
    }

    // Menu/shortcut references must exist in the engine registry (capability
    // `command-registry`): a renamed or removed command cannot leave a dead id behind.
    let registry_count = check_command_ids(&root, &mut problems)?;

    if problems.is_empty() {
        Ok(format!(
            "brand-check OK: {scanned} files scanned (name {:?}, {} stale tokens watched, {registry_count} commands)",
            brand.name,
            stale.len()
        ))
    } else {
        let mut msg = format!("brand-check failed ({} problems):\n", problems.len());
        for p in &problems {
            msg.push_str("  - ");
            msg.push_str(p);
            msg.push('\n');
        }
        Err(msg)
    }
}

/// Write `target/Packager.toml` for `cargo-packager` from the brand constants, so installer
/// labels and ids never hard-code the product name.
pub fn packager_config() -> Result<String, String> {
    let root = util::workspace_root()?;
    let brand = read_brand(&root)?;
    let lib = util::read(&root.join(BRAND_LIB))?;
    let id = Regex::new(r#"APP_ID: &str = "([^"]+)""#)
        .map_err(|e| e.to_string())?
        .captures(&lib)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or("APP_ID not found in the brand crate")?;
    let toml = format!(
        "product-name = \"{name}\"\nidentifier = \"{id}\"\npublisher = \"{name}\"\n\
description = \"A cross-platform multi-pane file manager\"\ncategory = \"Utility\"\n\
icons = [\"{root}/packaging/icon.png\", \"{root}/packaging/icon.ico\", \"{root}/packaging/icon.icns\"]\nout-dir = \"{root}/target/packager\"\n\
binaries-dir = \"{root}/target/release\"\nbinaries = [{{ path = \"{slug}\", main = true }}]\n",
        name = brand.name,
        slug = brand.slug,
        root = root.display().to_string().replace('\\', "/"),
    );
    let out = root.join("target/Packager.toml");
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&out, toml).map_err(|e| e.to_string())?;
    Ok(format!("wrote {}", util::rel(&root, &out).display()))
}

/// Cross-check `apps/fm-app/src/menu.rs` command-id literals against the ids
/// registered in `crates/engine` (`id: "…"` in CommandSpec literals).
fn check_command_ids(root: &Path, problems: &mut Vec<String>) -> Result<usize, String> {
    let mut registry: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let id_re = Regex::new(r#"id:\s*"([^"]+)""#).map_err(|e| e.to_string())?;
    for path in util::walk_files(&root.join("crates/engine/src"), &[], &["rs"])? {
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for cap in id_re.captures_iter(&text) {
            if let Some(m) = cap.get(1) {
                registry.insert(m.as_str().to_string());
            }
        }
    }

    // Dotted lowercase string literals in menu.rs are command ids by convention.
    let lit_re =
        Regex::new(r#""([a-z][a-z0-9]*(?:\.[a-z][a-z0-9]*)+)""#).map_err(|e| e.to_string())?;
    let menu_path = root.join("apps/fm-app/src/menu.rs");
    if menu_path.is_file() {
        let text = util::read(&menu_path)?;
        for (line_no, line) in text.lines().enumerate() {
            for cap in lit_re.captures_iter(line) {
                let Some(id) = cap.get(1) else { continue };
                if !registry.contains(id.as_str()) {
                    problems.push(format!(
                        "apps/fm-app/src/menu.rs:{}: menu/shortcut references unknown command id {:?} — not in the engine registry",
                        line_no + 1,
                        id.as_str()
                    ));
                }
            }
        }
    }
    Ok(registry.len())
}

// --------------------------------------------------------------------- rename

/// Rename the product in one commit: brand constants, binary name, desktop template
/// inputs; record the old name so `check()` keeps scanning for it; then re-run
/// `brand-check` as proof.
pub fn rename(new_name: &str) -> Result<String, String> {
    let root = util::workspace_root()?;
    validate_name(new_name)?;
    let old = read_brand(&root)?;
    let new_slug = slugify(new_name);
    validate_slug(&new_slug)?;

    if old.name == new_name && old.slug == new_slug {
        return Ok(format!("rename: already named {new_name} — nothing to do"));
    }

    // 1. brand constants
    let brand_path = root.join(BRAND_LIB);
    let mut text = util::read(&brand_path)?;
    for (constant, value) in [
        ("APP_NAME", new_name.to_string()),
        ("APP_SLUG", new_slug.clone()),
        ("APP_ID", format!("io.{new_slug}.app")),
        ("ORG_NAME", new_name.to_string()),
        ("EXECUTABLE", new_slug.clone()),
    ] {
        text = replace_const(&text, constant, &value)?;
    }
    util::write(&brand_path, &text)?;

    // 2. binary name (must equal brand::EXECUTABLE — asserted by a test in fm-app)
    let manifest_path = root.join(APP_MANIFEST);
    let manifest = util::read(&manifest_path)?;
    let bin_re =
        Regex::new(r#"(?s)(\[\[bin\]\].*?name\s*=\s*")[^"]*(")"#).map_err(|e| e.to_string())?;
    if !bin_re.is_match(&manifest) {
        return Err(format!(
            "{APP_MANIFEST}: no [[bin]] name found to rename to {new_slug:?}"
        ));
    }
    let updated = bin_re.replace(&manifest, format!("${{1}}{new_slug}${{2}}"));
    util::write(&manifest_path, &updated)?;

    // 3. rename history (so brand-check keeps watching the old identity; tokens that
    //    match the *new* identity are un-watched — renaming back stops the warning)
    let mut history = read_history(&root)?;
    let before = history.len();
    history.retain(|token| token != new_name && token != &new_slug);
    let mut changed = history.len() != before;
    for old_token in [&old.name, &old.slug] {
        if old_token != new_name && old_token.as_str() != new_slug && !history.contains(old_token) {
            history.push(old_token.clone());
            changed = true;
        }
    }
    if changed {
        let mut contents =
            String::from("# Stale product identities watched by `cargo xtask brand-check`.\n");
        for token in &history {
            contents.push_str(token);
            contents.push('\n');
        }
        util::write(&root.join(HISTORY), &contents)?;
    }

    // 4. prove it
    check()?;

    Ok(format!(
        "renamed {:?} → {:?} (slug {new_slug:?}, id io.{new_slug}.app, executable {new_slug:?}); watched stale tokens: {}",
        old.name,
        new_name,
        history.len()
    ))
}

// --------------------------------------------------------------------- helpers

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("rename: name is empty".to_string());
    }
    if name.trim() != name {
        return Err("rename: name has leading/trailing whitespace".to_string());
    }
    if name.chars().count() > 64 {
        return Err("rename: name longer than 64 characters".to_string());
    }
    if name.contains(['"', '\\', '\n', '\r', '\t']) {
        return Err("rename: name contains a quote, backslash or control character".to_string());
    }
    Ok(())
}

fn validate_slug(slug: &str) -> Result<(), String> {
    if slug.is_empty() {
        return Err(
            "rename: derived slug is empty — use a name with at least one letter or digit"
                .to_string(),
        );
    }
    if !slug.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(format!("rename: slug {slug:?} must start with a letter"));
    }
    Ok(())
}

/// Lowercase ASCII alphanumerics only; everything else dropped.
fn slugify(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn replace_const(text: &str, constant: &str, value: &str) -> Result<String, String> {
    let pattern = format!(r#"pub const {constant}: &str = "[^"]*";"#);
    let re = Regex::new(&pattern).map_err(|e| e.to_string())?;
    let replacement = format!("pub const {constant}: &str = \"{value}\";");
    let replaced = re.replace_all(text, replacement.as_str());
    if replaced == text && !text.contains(&format!("pub const {constant}:")) {
        return Err(format!("{BRAND_LIB}: missing `pub const {constant}`"));
    }
    if !replaced.contains(&replacement) {
        return Err(format!(
            "{BRAND_LIB}: could not rewrite `pub const {constant}`"
        ));
    }
    Ok(replaced.into_owned())
}

fn read_brand(root: &Path) -> Result<Brand, String> {
    let text = util::read(&root.join(BRAND_LIB))?;
    let name = capture_const(&text, "APP_NAME")?;
    let slug = capture_const(&text, "APP_SLUG")?;
    Ok(Brand { name, slug })
}

fn capture_const(text: &str, constant: &str) -> Result<String, String> {
    let pattern = format!(r#"pub const {constant}: &str = "([^"]*)";"#);
    let re = Regex::new(&pattern).map_err(|e| e.to_string())?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| format!("{BRAND_LIB}: cannot read `pub const {constant}`"))
}

fn read_history(root: &Path) -> Result<Vec<String>, String> {
    let path = root.join(HISTORY);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = util::read(&path)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_drops_separators() {
        assert_eq!(slugify("Some App 2"), "someapp2");
        assert_eq!(slugify("Ökofilé 2"), "kofil2");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn current_identity_is_slugify_consistent() {
        // The brand crate's APP_SLUG must be slugify(APP_NAME), or a rename leaves drift.
        let root = util::workspace_root().expect("workspace root");
        let brand = read_brand(&root).expect("read brand constants");
        assert_eq!(
            slugify(&brand.name),
            brand.slug,
            "APP_SLUG must equal slugify(APP_NAME)"
        );
    }

    #[test]
    fn name_validation_rejects_hostile_input() {
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name(" padded ").is_err());
        assert!(validate_name("quote\"name").is_err());
        assert!(validate_name("back\\slash").is_err());
        assert!(validate_name(&"x".repeat(65)).is_err());
        assert!(validate_name("NewUltra").is_ok());
    }

    #[test]
    fn slug_validation_rejects_bad_starts() {
        assert!(validate_slug("").is_err());
        assert!(validate_slug("9lives").is_err());
        assert!(validate_slug("ok").is_ok());
    }

    #[test]
    fn replace_const_rewrites_only_the_target() {
        let text = "pub const APP_NAME: &str = \"A\";\npub const APP_SLUG: &str = \"b\";\n";
        let out = replace_const(text, "APP_NAME", "New").expect("replace");
        assert!(out.contains("pub const APP_NAME: &str = \"New\";"));
        assert!(out.contains("pub const APP_SLUG: &str = \"b\";"));
    }

    #[test]
    fn replace_const_errors_when_missing() {
        let text = "pub const OTHER: &str = \"x\";\n";
        assert!(replace_const(text, "APP_NAME", "New").is_err());
    }
}
