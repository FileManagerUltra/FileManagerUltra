//! `app.diagnostics.folder` — show the diagnostic log folder (capability `diagnostics`).
//! The switch itself is the `prefs.set` key `diagnostics_log`.

use fm_vfs::{FmError, VfsUri};

use crate::registry::CommandSpec;
use crate::search_cmds::show_in_files;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "app.diagnostics.folder",
        label: "Open Diagnostic Log Folder",
        menu: &["Help", "Open Diagnostic Log Folder"],
        shortcut: None,
        params: "none",
        enabled: |_| crate::diag::has_logs(),
        run: |s, params| {
            if !params.is_null() {
                return Err(FmError::InvalidParams(
                    "This command takes no parameters".into(),
                ));
            }
            let dir = crate::diag::default_dir().ok_or_else(|| {
                FmError::backend("diagnostics", "No data folder for the diagnostic log")
            })?;
            if !dir.is_dir() {
                return Err(FmError::backend(
                    "diagnostics",
                    "No diagnostic log has been written yet. Turn on \"Write diagnostic log\" in Preferences first.",
                ));
            }
            let folder = dir
                .to_str()
                .and_then(|text| VfsUri::parse(text).ok())
                .ok_or_else(|| {
                    FmError::backend("diagnostics", "The log folder path cannot be shown")
                })?;
            let current = crate::diag::current_path()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
            show_in_files(s, folder, current)
        },
    }]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::registry::Params;

    #[test]
    fn parameters_are_rejected() {
        let mut s = crate::Session::new().expect("session");
        for params in [json!({"x": 1}), json!(1), json!("folder")] {
            assert!(s.dispatch("app.diagnostics.folder", &params).is_err());
        }
        // Enabled once a log folder exists; without one it is an error, never a crash.
        let result = s.dispatch("app.diagnostics.folder", &Params::Null);
        if !crate::diag::has_logs() {
            assert!(!s.command_enabled("app.diagnostics.folder"));
            assert!(result.is_err());
        }
    }
}
