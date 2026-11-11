//! Helper-tool commands: the app-managed ffmpeg download.

use crate::registry::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "tools.ffmpeg.install",
        label: "Download ffmpeg",
        menu: &["Help", "Download ffmpeg"],
        shortcut: None,
        params: "none: download, verify and install (or update) the pinned ffmpeg and ffprobe in the background; only where a build is pinned",
        enabled: |s| s.ffmpeg.downloadable() && !s.ffmpeg.installing(),
        run: |s, _| s.ffmpeg.start(),
    }]
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use serde_json::json;

    #[test]
    fn unsupported_platforms_fail_gracefully_and_junk_params_are_harmless() {
        let mut s = Session::default();
        if !s.ffmpeg.downloadable() {
            assert!(!s.command_enabled("tools.ffmpeg.install"));
            assert!(s.dispatch("tools.ffmpeg.install", &json!(null)).is_err());
        }
        assert!(!s.ffmpeg.installing() || s.ffmpeg.downloadable());
    }
}
