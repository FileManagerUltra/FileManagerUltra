//! Release checks and user-approved navigation to the download page.

use fm_vfs::FmError;

use crate::registry::{CommandSpec, Params};
use crate::updates::UpdateStatus;

fn no_params(params: &Params) -> Result<(), FmError> {
    if !params.is_null() {
        return Err(FmError::InvalidParams(
            "This command takes no parameters".into(),
        ));
    }
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "app.updates.check",
            label: "Check for Updates…",
            menu: &["Help", "Check for Updates"],
            shortcut: None,
            params: "none",
            enabled: |s| !s.updates.checking(),
            run: |s, params| {
                no_params(params)?;
                s.updates.check(true)
            },
        },
        CommandSpec {
            id: "app.updates.download",
            label: "Download Update…",
            menu: &["Help", "Download Update"],
            shortcut: None,
            params: "none",
            enabled: |s| {
                s.updates.browser_job.is_none()
                    && matches!(s.updates.status, UpdateStatus::Available { .. })
            },
            run: |s, params| {
                no_params(params)?;
                if !matches!(s.updates.status, UpdateStatus::Available { .. }) {
                    return Err(FmError::InvalidParams("No update is available".into()));
                }
                let url = format!(
                    "https://github.com/{}/releases/latest",
                    fm_brand::RELEASE_REPOSITORY
                );
                if s.updates.browser_job.is_some() {
                    return Ok(());
                }
                s.updates.browser_job = Some(s.jobs.submit(crate::JobLane::Local, move |_| {
                    opener::open_browser(url).map_err(|e| {
                        FmError::backend("updates", format!("Cannot open release page: {e}"))
                    })
                }));
                Ok(())
            },
        },
        CommandSpec {
            id: "app.updates.dismiss",
            label: "Dismiss Update Notification",
            menu: &["Help", "Dismiss Update Notification"],
            shortcut: None,
            params: "none",
            enabled: |s| s.updates.visible,
            run: |s, params| {
                no_params(params)?;
                s.updates.visible = false;
                Ok(())
            },
        },
        CommandSpec {
            id: "prefs.updates",
            label: "Check for Updates at Startup",
            menu: &["File", "Preferences", "Check for updates at startup"],
            shortcut: None,
            params: "value: bool",
            enabled: |_| true,
            run: |s, params| {
                let value = params
                    .as_object()
                    .and_then(|p| p.get("value"))
                    .and_then(Params::as_bool)
                    .ok_or_else(|| FmError::InvalidParams("value must be true or false".into()))?;
                s.prefs.check_updates = value;
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_commands_fail_without_starting_network_work() {
        let mut s = crate::Session::new().expect("session");
        for spec in specs() {
            assert!((spec.run)(&mut s, &serde_json::json!({"value": "bad"})).is_err());
        }
        assert!(!s.updates.checking());
        assert!(s.dispatch("app.updates.download", &Params::Null).is_err());
        s.dispatch("prefs.updates", &serde_json::json!({"value": false}))
            .expect("prefs");
        assert!(!s.prefs.check_updates);
        s.updates.visible = true;
        s.dispatch("app.updates.dismiss", &Params::Null)
            .expect("dismiss");
        assert!(!s.updates.visible);
    }
}
