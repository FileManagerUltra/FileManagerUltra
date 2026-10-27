//! Background, bounded GitHub release checks. Installation stays in the user's browser.

use std::io::Read;
use std::time::Duration;

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use fm_vfs::FmError;
use semver::Version;
use serde::Deserialize;

const MAX_RELEASE_BYTES: usize = 1_048_576;
type CheckResult = Result<Option<(String, String)>, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    Available { version: String, url: String },
    UpToDate,
    Failed(String),
}

#[derive(Debug)]
pub struct Updates {
    pub status: UpdateStatus,
    pub visible: bool,
    pending: Option<Receiver<CheckResult>>,
    manual: bool,
    pub(crate) browser_job: Option<crate::JobId>,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            status: UpdateStatus::Idle,
            visible: false,
            pending: None,
            manual: false,
            browser_job: None,
        }
    }
}

impl Updates {
    pub(crate) fn finish_browser(&mut self, event: &crate::JobEvent) -> bool {
        if self.browser_job != Some(event.job_id()) || !event.is_terminal() {
            return false;
        }
        self.browser_job = None;
        true
    }

    pub fn checking(&self) -> bool {
        self.pending.is_some()
    }

    pub fn check(&mut self, manual: bool) -> Result<(), FmError> {
        self.start(manual, fetch_latest)
    }

    fn start(
        &mut self,
        manual: bool,
        work: impl FnOnce() -> CheckResult + Send + 'static,
    ) -> Result<(), FmError> {
        if self.checking() {
            self.manual |= manual;
            return Ok(());
        }
        let (tx, rx) = bounded(1);
        std::thread::Builder::new()
            .name("release-check".into())
            .spawn(move || {
                let _ = tx.send(work());
            })
            .map_err(|e| FmError::backend("updates", format!("Cannot start update check: {e}")))?;
        self.pending = Some(rx);
        self.status = UpdateStatus::Checking;
        self.manual = manual;
        self.visible = false;
        Ok(())
    }

    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("Update check stopped unexpectedly".into()),
        };
        self.pending = None;
        self.status = match result {
            Ok(Some((version, url))) => UpdateStatus::Available { version, url },
            Ok(None) => UpdateStatus::UpToDate,
            Err(message) => UpdateStatus::Failed(message),
        };
        self.visible = matches!(self.status, UpdateStatus::Available { .. });
        true
    }
}

impl crate::Session {
    pub(crate) fn pump_updates(&mut self) -> bool {
        if !self.updates.poll() {
            return false;
        }
        match &self.updates.status {
            UpdateStatus::Failed(message) => self.notice = Some(message.clone()),
            UpdateStatus::UpToDate if self.updates.manual => {
                self.info = Some((
                    "No newer published release is available.".into(),
                    std::time::Instant::now(),
                ));
            }
            _ => {}
        }
        true
    }
}

fn fetch_latest() -> CheckResult {
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .user_agent(format!(
            "{}/{}",
            fm_brand::APP_SLUG,
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|e| format!("Cannot create update client: {e}"))?;
    let response = client
        .get(format!(
            "https://api.github.com/repos/{}/releases/latest",
            fm_brand::RELEASE_REPOSITORY
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| format!("Cannot check for updates: {e}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .map_err(|e| format!("Cannot check for updates: {e}"))?;
    let mut bytes = Vec::new();
    response
        .take((MAX_RELEASE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read release information: {e}"))?;
    parse_release(&bytes, env!("CARGO_PKG_VERSION"))
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn parse_release(bytes: &[u8], current: &str) -> CheckResult {
    if bytes.len() > MAX_RELEASE_BYTES {
        return Err("Release information exceeds the size limit".into());
    }
    let release: Release =
        serde_json::from_slice(bytes).map_err(|_| "Invalid release information".to_string())?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .map_err(|_| "Release tag is not a valid version".to_string())?;
    let current = Version::parse(current).map_err(|_| "Invalid application version".to_string())?;
    if !version.pre.is_empty() || !version.cmp_precedence(&current).is_gt() {
        return Ok(None);
    }
    // Construct the destination locally; never open a URL supplied by release metadata.
    Ok(Some((
        version.to_string(),
        format!(
            "https://github.com/{}/releases/latest",
            fm_brand::RELEASE_REPOSITORY
        ),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "tag_name": version, "draft": false, "prerelease": false
        }))
        .expect("json")
    }

    #[test]
    fn compares_versions_semantically_and_ignores_old_or_prereleases() {
        assert!(
            parse_release(&release("v0.10.0"), "0.9.0")
                .expect("parse")
                .is_some()
        );
        for tag in ["v0.1.0", "v0.0.9", "v1.0.0-beta.1", "v0.1.0+build.9"] {
            assert!(
                parse_release(&release(tag), "0.1.0")
                    .expect("parse")
                    .is_none()
            );
        }
        assert!(
            parse_release(
                br#"{"tag_name":"v9.0.0","draft":true,"prerelease":false}"#,
                "0.1.0"
            )
            .expect("parse")
            .is_none()
        );
        assert!(
            parse_release(
                br#"{"tag_name":"v9.0.0","draft":false,"prerelease":true}"#,
                "0.1.0"
            )
            .expect("parse")
            .is_none()
        );
    }

    #[test]
    fn malformed_and_oversized_metadata_are_errors() {
        assert!(parse_release(b"{}", "0.1.0").is_err());
        assert!(parse_release(&release("not-a-version"), "0.1.0").is_err());
        assert!(parse_release(&vec![0; MAX_RELEASE_BYTES + 1], "0.1.0").is_err());
    }

    #[test]
    fn remote_urls_are_ignored() {
        let (_, url) = parse_release(
            br#"{"tag_name":"v9.0.0","draft":false,"prerelease":false,
                 "html_url":"file:///tmp/installer","body":"<script>run()</script>"}"#,
            "0.1.0",
        )
        .expect("parse")
        .expect("update");
        assert_eq!(
            url,
            format!(
                "https://github.com/{}/releases/latest",
                fm_brand::RELEASE_REPOSITORY,
            )
        );
    }

    fn settle(updates: &mut Updates) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !updates.poll() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }

    #[test]
    fn startup_is_quiet_without_an_update_and_manual_checks_report_results() {
        for manual in [false, true] {
            let mut updates = Updates::default();
            updates.start(manual, || Ok(None)).expect("start");
            settle(&mut updates);
            assert_eq!(updates.status, UpdateStatus::UpToDate);
            assert!(!updates.visible);
        }
        let mut updates = Updates::default();
        updates
            .start(false, || parse_release(&release("v0.2.0"), "0.1.0"))
            .expect("start");
        settle(&mut updates);
        assert!(updates.visible);
        assert!(matches!(updates.status, UpdateStatus::Available { .. }));
    }

    #[test]
    fn browser_launch_failures_are_reported() {
        let mut s = crate::Session::default();
        s.updates.status = UpdateStatus::Available {
            version: "0.2.0".into(),
            url: String::new(),
        };
        s.updates.visible = true;
        s.updates.browser_job = Some(s.jobs.submit(crate::JobLane::Local, |_| {
            Err(FmError::backend("updates", "No browser available"))
        }));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while s.updates.browser_job.is_some() {
            s.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            s.notice
                .as_ref()
                .is_some_and(|n| n.contains("No browser available"))
        );
        assert!(s.updates.visible, "retain the CTA so it can be retried");
        assert!(matches!(s.updates.status, UpdateStatus::Available { .. }));
    }

    #[test]
    fn checks_are_background_deduplicated_and_failures_do_not_open_the_panel() {
        let mut updates = Updates::default();
        let (tx, rx) = bounded(1);
        updates
            .start(false, move || rx.recv().expect("receive"))
            .expect("start");
        assert!(updates.checking());
        assert!(!updates.visible);
        assert!(!updates.poll());
        updates
            .start(true, || panic!("duplicate check"))
            .expect("dedup");
        tx.send(Err("offline".into())).expect("send");
        settle(&mut updates);
        assert_eq!(updates.status, UpdateStatus::Failed("offline".into()));
        assert!(!updates.visible);
    }

    #[test]
    fn unsuccessful_checks_report_in_the_status_bar_only() {
        for manual in [false, true] {
            for fail in [false, true] {
                let mut s = crate::Session::default();
                s.updates
                    .start(manual, move || {
                        if fail {
                            Err("offline".into())
                        } else {
                            Ok(None)
                        }
                    })
                    .expect("start");
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while s.updates.checking() {
                    s.poll();
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::yield_now();
                }
                assert!(!s.updates.visible);
                assert_eq!(s.notice.as_deref(), fail.then_some("offline"));
                assert_eq!(s.info.is_some(), manual && !fail);
            }
        }
    }
}
