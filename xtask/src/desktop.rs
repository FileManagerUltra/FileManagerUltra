//! Per-user Linux desktop registration for an uninstalled development binary.

use std::path::{Path, PathBuf};

fn identity() -> Result<crate::brand::Brand, String> {
    crate::brand::read_brand(&crate::util::workspace_root()?)
}

pub fn install() -> Result<String, String> {
    if !cfg!(target_os = "linux") {
        return Err("desktop-install is only supported on Linux".into());
    }
    let data = data_dir(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))?;
    let brand = identity()?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let binary = executable
        .parent()
        .ok_or("cannot locate the Cargo output directory")?
        .join(&brand.slug);
    write_entry(&data, &binary)?;
    Ok(format!(
        "Registered {} in {}. Restart the app; re-pin any old dock shortcut.\n\
         The launcher uses {} (build it with cargo build -p fm-app first).",
        brand.name,
        data.display(),
        binary.display()
    ))
}

fn data_dir(
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf, String> {
    if let Some(path) = xdg.map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Ok(path);
    }
    home.map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join(".local/share"))
        .ok_or_else(|| "HOME or an absolute XDG_DATA_HOME is required".into())
}

fn write_entry(data: &Path, binary: &Path) -> Result<(), String> {
    let brand = identity()?;
    let icon =
        std::fs::read(crate::util::workspace_root()?.join("crates/brand/assets/icon-256.png"))
            .map_err(|e| format!("Cannot read branded icon: {e}"))?;
    let binary = binary
        .to_str()
        .filter(|s| !s.chars().any(char::is_control))
        .ok_or("binary path must be UTF-8 without control characters")?;
    // Exec is not a shell command: quote its argument, then escape desktop-file
    // backslashes and literal percent field codes.
    let escaped = binary
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('\\', "\\\\")
        .replace('%', "%%");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{escaped}\"\n\
         StartupWMClass={slug}\nIcon={slug}\nCategories=Utility;FileManager;\nTerminal=false\n",
        name = brand.name,
        slug = brand.slug,
    );
    let applications = data.join("applications");
    let icons = data.join("icons/hicolor/256x256/apps");
    std::fs::create_dir_all(&applications)
        .map_err(|e| format!("Cannot create applications directory: {e}"))?;
    std::fs::create_dir_all(&icons).map_err(|e| format!("Cannot create icons directory: {e}"))?;
    std::fs::write(icons.join(format!("{}.png", brand.slug)), icon)
        .map_err(|e| format!("Cannot install desktop icon: {e}"))?;
    std::fs::write(applications.join(format!("{}.desktop", brand.slug)), entry)
        .map_err(|e| format!("Cannot install desktop entry: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)] // XDG desktop entries are a Linux install step; `/data` is not absolute on Windows.
    fn xdg_paths_follow_the_standard() {
        assert_eq!(
            data_dir(Some("/data".into()), None).expect("xdg"),
            Path::new("/data")
        );
        assert_eq!(
            data_dir(Some("relative".into()), Some("/home/test".into())).expect("fallback"),
            Path::new("/home/test/.local/share"),
        );
        assert!(data_dir(None, None).is_err());
    }

    #[test]
    fn entry_and_icon_match_the_window_identity() {
        let brand = identity().expect("brand");
        let dir = tempfile::tempdir().expect("tempdir");
        write_entry(dir.path(), Path::new("/work/a b/target/debug/app")).expect("install");
        let entry = std::fs::read_to_string(
            dir.path()
                .join("applications")
                .join(format!("{}.desktop", brand.slug)),
        )
        .expect("entry");
        assert!(entry.contains(&format!("StartupWMClass={}\n", brand.slug)));
        assert!(entry.contains(&format!("Icon={}\n", brand.slug)));
        assert!(entry.contains("Exec=\"/work/a b/target/debug/app\"\n"));
        let icon = std::fs::read(
            dir.path()
                .join("icons/hicolor/256x256/apps")
                .join(format!("{}.png", brand.slug)),
        )
        .expect("icon");
        let source = std::fs::read(
            crate::util::workspace_root()
                .expect("root")
                .join("crates/brand/assets/icon-256.png"),
        )
        .expect("source");
        assert_eq!(icon, source);
    }

    #[test]
    fn exec_escapes_special_characters_and_rejects_line_injection() {
        let brand = identity().expect("brand");
        let dir = tempfile::tempdir().expect("tempdir");
        write_entry(dir.path(), Path::new("/work/$cash`quote\"%/app")).expect("install");
        let entry = std::fs::read_to_string(
            dir.path()
                .join("applications")
                .join(format!("{}.desktop", brand.slug)),
        )
        .expect("entry");
        assert!(
            entry.contains(r#"Exec="/work/\\$cash\\`quote\\"%%/app""#),
            "{entry}"
        );
        assert!(write_entry(dir.path(), Path::new("/work/\nExec=bad")).is_err());
    }
}
