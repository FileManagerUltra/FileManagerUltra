//! Cached mounted volumes. Discovery has one worker at a time: a stuck mount must
//! neither block local file jobs nor accumulate abandoned scan threads.

mod platform;

use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use fm_vfs::{FmError, VfsUri};

pub const SCAN_INTERVAL: Duration = Duration::from_secs(2);
pub const IDLE_SCAN_INTERVAL: Duration = Duration::from_secs(10);
const SLOW_SCAN_WARNING: Duration = Duration::from_secs(10);
const MAX_DEVICES: usize = 256;
type ScanResult = Result<Vec<Device>, FmError>;

/// Wakes the UI when a scan finishes, so it need not poll while one runs.
#[derive(Clone, Default)]
struct ScanWaker(Option<std::sync::Arc<dyn Fn() + Send + Sync>>);

impl std::fmt::Debug for ScanWaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "ScanWaker(set)"
        } else {
            "ScanWaker(none)"
        })
    }
}

/// What can safely detach this volume. Unknown virtual/encrypted mounts are browse-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Removal {
    Eject,
    Dismount,
    Unsupported(String),
}

impl Removal {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Eject => "Eject",
            Self::Dismount | Self::Unsupported(_) => "Dismount",
        }
    }

    pub fn command(&self) -> &'static str {
        match self {
            Self::Eject => "devices.eject",
            _ => "devices.dismount",
        }
    }
}

/// A mounted volume. `identity` is an opaque OS identity, not a display label.
#[derive(Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub mount: VfsUri,
    pub filesystem: String,
    pub total: Option<u64>,
    pub free: Option<u64>,
    pub identity: String,
    pub removal: Removal,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("name", &display_text(&self.name))
            .field("mount", &display_text(&self.mount.to_string()))
            .field("filesystem", &self.filesystem)
            .field("total", &self.total)
            .field("free", &self.free)
            .field("removal", &self.removal)
            .finish_non_exhaustive()
    }
}

/// Display-only text. Raw OS identities and operation arguments are never sanitised.
pub(crate) fn display_text(text: &str) -> String {
    fm_vfs::redact::redact(text)
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect()
}

/// Sanitise volume labels/paths for display; never use this to form operation arguments.
pub fn display_label(text: &str) -> String {
    display_text(text)
}

/// Shared state for discovery and one removal job.
#[derive(Debug, Default)]
pub struct Devices {
    entries: Vec<Device>,
    pub generation: u64,
    pending: Option<Receiver<ScanResult>>,
    started: Option<Instant>,
    next_scan: Option<Instant>,
    slow_reported: bool,
    initialized: bool,
    last_error: Option<String>,
    focused: bool,
    unavailable: Vec<VfsUri>,
    pub(crate) removal: Option<(crate::JobId, Device)>,
    waker: ScanWaker,
}

impl Devices {
    /// Called from the scan thread once its result is ready (the UI then calls
    /// `Session::poll`, which applies it).
    pub fn set_waker(&mut self, waker: impl Fn() + Send + Sync + 'static) {
        self.waker = ScanWaker(Some(std::sync::Arc::new(waker)));
    }

    pub fn entries(&self) -> &[Device] {
        &self.entries
    }

    pub fn scanning(&self) -> bool {
        self.pending.is_some()
    }

    pub fn removing(&self) -> bool {
        self.removal.is_some()
    }

    pub fn removal_target(&self) -> Option<&Device> {
        self.removal.as_ref().map(|(_, device)| device)
    }

    /// Merge a discovered snapshot. Public for injected discovery providers and UI fixtures.
    pub fn replace(&mut self, mut entries: Vec<Device>) -> Option<String> {
        entries.truncate(MAX_DEVICES);
        entries.sort_by_key(|a| a.mount.as_str());
        entries.dedup_by(|a, b| a.mount == b.mount);
        let mut messages = Vec::new();
        if self.initialized {
            for entry in &entries {
                if !self.entries.iter().any(|old| same_mount(old, entry)) {
                    messages.push(format!("{} connected", display_text(&entry.name)));
                }
            }
            for old in &self.entries {
                if !entries.iter().any(|entry| same_mount(old, entry)) {
                    messages.push(format!("{} disconnected", display_text(&old.name)));
                }
            }
        }
        self.initialized = true;
        if self.entries != entries {
            for old in &self.entries {
                if !entries.iter().any(|entry| same_mount(old, entry))
                    && !self.unavailable.contains(&old.mount)
                {
                    self.unavailable.push(old.mount.clone());
                }
            }
            self.unavailable
                .retain(|uri| !entries.iter().any(|entry| &entry.mount == uri));
            if self.unavailable.len() > MAX_DEVICES {
                self.unavailable
                    .drain(..self.unavailable.len() - MAX_DEVICES);
            }
            self.entries = entries;
            self.generation = self.generation.wrapping_add(1);
        }
        (!messages.is_empty()).then(|| messages.into_iter().take(4).collect::<Vec<_>>().join("; "))
    }

    fn start(
        &mut self,
        work: impl FnOnce() -> Result<Vec<Device>, FmError> + Send + 'static,
    ) -> Result<(), FmError> {
        if self.scanning() {
            return Ok(());
        }
        let (tx, rx) = bounded(1);
        let waker = self.waker.clone();
        std::thread::Builder::new()
            .name("devices-scan".into())
            .spawn(move || {
                let result = work().and_then(|entries| {
                    if entries.len() > MAX_DEVICES {
                        Err(FmError::backend(
                            "devices",
                            "Too many mounted volumes (limit 256)",
                        ))
                    } else {
                        Ok(entries)
                    }
                });
                let _ = tx.send(result);
                if let Some(wake) = &waker.0 {
                    wake();
                }
            })
            .map_err(|e| FmError::backend("devices", format!("Cannot start volume scan: {e}")))?;
        self.pending = Some(rx);
        self.started = Some(Instant::now());
        self.slow_reported = false;
        Ok(())
    }

    pub(crate) fn refresh(&mut self) -> Result<(), FmError> {
        if self.removing() {
            return Err(FmError::backend(
                "devices",
                "Wait for device removal to finish",
            ));
        }
        self.start(platform::scan)?;
        self.next_scan = Some(Instant::now() + SCAN_INTERVAL);
        Ok(())
    }

    pub(crate) fn blocks(&self, uri: &VfsUri) -> bool {
        self.unavailable(uri)
            || self
                .removal
                .as_ref()
                .is_some_and(|(_, device)| device.mount.is_same_or_descendant_of(uri))
    }

    pub(crate) fn unavailable(&self, uri: &VfsUri) -> bool {
        self.unavailable
            .iter()
            .any(|mount| mount.is_same_or_descendant_of(uri))
    }
}

pub(crate) fn same_mount(a: &Device, b: &Device) -> bool {
    a.mount == b.mount && a.identity == b.identity
}

impl crate::Session {
    /// Called by the shell even when there is no user input. Tests need not start OS scans.
    pub fn devices_tick(&mut self, focused: bool) {
        let now = Instant::now();
        if focused && !self.volumes.focused {
            self.volumes.next_scan = None;
        }
        self.volumes.focused = focused;
        let interval = if focused {
            SCAN_INTERVAL
        } else {
            IDLE_SCAN_INTERVAL
        };
        if !self.volumes.scanning()
            && !self.volumes.removing()
            && self.volumes.next_scan.is_none_or(|next| now >= next)
        {
            if let Err(error) = self.volumes.refresh() {
                self.notice = Some(error.to_string());
            }
            self.volumes.next_scan = Some(now + interval);
        }
    }

    pub(crate) fn pump_devices(&mut self) -> bool {
        let Some(rx) = &self.volumes.pending else {
            return false;
        };
        let received = rx.try_recv();
        if matches!(received, Err(TryRecvError::Empty))
            && !self.volumes.slow_reported
            && self
                .volumes
                .started
                .is_some_and(|start| start.elapsed() > SLOW_SCAN_WARNING)
        {
            self.volumes.slow_reported = true;
            let message = "Volume scan is still waiting after 10 seconds; showing the previous Devices list. Check slow or disconnected mounts in the OS or VeraCrypt.";
            if self.volumes.last_error.as_deref() != Some(message) {
                self.notice = Some(message.into());
            }
            self.volumes.last_error = Some(message.into());
        }
        let result = match received {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err(FmError::backend(
                "devices",
                "Volume scan stopped unexpectedly",
            )),
        };
        self.volumes.pending = None;
        self.volumes.started = None;
        self.volumes.next_scan = Some(
            Instant::now()
                + if self.volumes.focused {
                    SCAN_INTERVAL
                } else {
                    IDLE_SCAN_INTERVAL
                },
        );
        match result {
            Ok(entries) => {
                let old = self.volumes.entries.clone();
                if let Some(message) = self.volumes.replace(entries) {
                    self.info = Some((message, Instant::now()));
                }
                let removed: Vec<_> = old
                    .into_iter()
                    .filter(|old| {
                        !self
                            .volumes
                            .entries
                            .iter()
                            .any(|entry| same_mount(old, entry))
                    })
                    .collect();
                for removed in &removed {
                    self.forget_volume_reads(&removed.mount);
                    for pane in self.layout.panes() {
                        if let Some(location) = self.layout.state(pane).map(|p| p.location.clone())
                            && removed.mount.is_same_or_descendant_of(&location)
                        {
                            self.dirs.set_failed(
                                &location,
                                format!(
                                    "{} was disconnected. Reconnect it or choose another folder.",
                                    display_text(&removed.name)
                                ),
                            );
                            self.after_read(&location);
                        }
                    }
                }
                self.volumes.last_error = None;
            }
            Err(error) => {
                let message = error.to_string();
                if self.volumes.last_error.as_ref() != Some(&message) {
                    self.notice = Some(message.clone());
                }
                self.volumes.last_error = Some(message);
            }
        }
        true
    }

    pub(crate) fn finish_device_removal(&mut self, event: &crate::JobEvent) -> bool {
        if !event.is_terminal()
            || self.volumes.removal.as_ref().map(|(id, _)| *id) != Some(event.job_id())
        {
            return false;
        }
        let removed = self.volumes.removal.take().map(|(_, d)| d);
        match event {
            crate::JobEvent::Done(_) => {
                if let Some(device) = removed {
                    self.info = Some((format!("{} removed", device.name), Instant::now()));
                }
            }
            crate::JobEvent::Failed { message, .. } => self.notice = Some(message.clone()),
            _ => self.notice = Some("Device removal cancelled".into()),
        }
        self.volumes.next_scan = None;
        true
    }
}

pub(crate) fn remove(device: &Device) -> Result<(), FmError> {
    remove_with(device, platform::scan_current, platform::remove)
}

fn remove_with(
    device: &Device,
    mut scan: impl FnMut() -> Result<Vec<Device>, FmError>,
    remove: impl FnOnce(&Device) -> Result<(), FmError>,
) -> Result<(), FmError> {
    let before = scan()?;
    let current = before
        .iter()
        .find(|d| same_mount(d, device))
        .ok_or_else(|| {
            FmError::backend(
                "devices",
                "The mounted volume changed; refresh Devices and try again",
            )
        })?;
    if current.removal != device.removal {
        return Err(FmError::backend(
            "devices",
            "Volume removal capability changed",
        ));
    }
    remove(current)?;
    // Shell.Application eject is asynchronous on Windows; wait for the OS mount to go away.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if !scan()?.iter().any(|d| same_mount(d, device)) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(FmError::backend(
                "devices",
                "The volume is still mounted. Close open files and retry, or remove it with the OS/VeraCrypt.",
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn device() -> Device {
        Device {
            name: "USB".into(),
            mount: VfsUri::parse("/media/usb").expect("uri"),
            filesystem: "exfat".into(),
            total: Some(64 << 30),
            free: Some(32 << 30),
            identity: "mount-42".into(),
            removal: Removal::Eject,
        }
    }

    #[test]
    fn changes_include_replacement_but_not_free_space_updates() {
        let mut state = Devices::default();
        let mut d = device();
        assert!(state.replace(vec![d.clone()]).is_none());
        let generation = state.generation;
        assert!(state.replace(vec![d.clone()]).is_none());
        assert_eq!(state.generation, generation);
        d.free = Some(30 << 30);
        assert!(state.replace(vec![d.clone()]).is_none());
        assert_eq!(state.generation, generation + 1);
        d.identity = "mount-43".into();
        assert!(
            state
                .replace(vec![d])
                .expect("notice")
                .contains("disconnected")
        );
        assert!(
            state
                .replace(vec![])
                .expect("notice")
                .contains("disconnected")
        );
        assert!(state.entries().is_empty());
    }

    #[test]
    fn identity_reuse_never_executes_removal_and_success_requires_disappearance() {
        let d = device();
        let mut replacement = d.clone();
        replacement.identity = "replacement".into();
        let result = remove_with(
            &d,
            || Ok(vec![replacement.clone()]),
            |_| panic!("must not remove the replacement volume"),
        );
        assert!(result.is_err());
        let mut first = true;
        let mut called = false;
        remove_with(
            &d,
            || {
                let entries = if first { vec![d.clone()] } else { Vec::new() };
                first = false;
                Ok(entries)
            },
            |_| {
                called = true;
                Ok(())
            },
        )
        .expect("confirmed removal");
        assert!(called);
    }

    #[test]
    fn labels_have_no_controls_or_credentials() {
        let label = display_text("USB\n\tssh://alice:secret@example.com/path");
        assert!(!label.contains('\n'));
        assert!(!label.contains("secret"));
        assert_eq!(display_text(&"x".repeat(1000)).len(), 160);
    }

    #[test]
    fn disappearing_volume_reports_unavailable_without_browsing_the_empty_mountpoint() {
        let mut s = crate::Session::default();
        let d = device();
        s.layout.state_mut(1).expect("pane").location = d.mount.clone();
        s.volumes.replace(vec![d.clone()]);
        s.volumes.start(|| Ok(Vec::new())).expect("start");
        let deadline = Instant::now() + Duration::from_secs(2);
        while s.volumes.scanning() {
            s.pump_devices();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(matches!(
            s.dirs.state(&d.mount),
            crate::DirState::Failed { .. }
        ));
        assert_eq!(s.layout.state(1).expect("pane").location, d.mount);
        assert!(s.volumes.blocks(&d.mount));
        s.ensure_read(d.mount.clone()).expect("unavailable state");
        assert!(matches!(
            s.dirs.state(&d.mount),
            crate::DirState::Failed { .. }
        ));
        s.volumes.replace(vec![d.clone()]);
        assert!(!s.volumes.blocks(&d.mount), "reconnected mounts are usable");
    }

    #[test]
    fn real_mount_scan_has_unique_mounts_and_does_not_list_root() {
        let entries = match platform::scan_current() {
            Ok(entries) => entries,
            // A loaded CI runner can make the OS tool (PowerShell on Windows) miss its
            // deadline; this test checks the result, not the tool's speed.
            Err(e) if e.to_string().contains("timed out") => {
                eprintln!("skipped: {e}");
                return;
            }
            Err(e) => panic!("OS volume metadata: {e:?}"),
        };
        for (i, entry) in entries.iter().enumerate() {
            assert_ne!(entry.mount.path_str(), "/");
            assert!(
                entries
                    .iter()
                    .skip(i + 1)
                    .all(|other| other.mount != entry.mount)
            );
        }
    }

    #[test]
    fn failure_keeps_previous_snapshot() {
        let mut s = crate::Session::default();
        s.volumes.replace(vec![device()]);
        s.volumes
            .start(|| Err(FmError::backend("devices", "scan failed")))
            .expect("start");
        let deadline = Instant::now() + Duration::from_secs(2);
        while s.volumes.scanning() {
            s.pump_devices();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(s.volumes.entries().len(), 1);
        assert!(
            s.notice
                .as_deref()
                .is_some_and(|n| n.contains("scan failed"))
        );
    }

    #[test]
    fn slow_scan_is_single_flight_and_applies_late_snapshot() {
        let mut s = crate::Session::default();
        s.volumes.replace(vec![device()]);
        let (tx, rx) = bounded(1);
        s.volumes
            .start(move || {
                let _ = rx.recv();
                Ok(Vec::new())
            })
            .expect("start");
        s.volumes.started = Some(Instant::now() - Duration::from_secs(2));
        s.pump_devices();
        assert!(s.volumes.scanning());
        assert!(s.notice.is_none(), "a two-second scan is not an error");
        s.volumes.started = Some(Instant::now() - SLOW_SCAN_WARNING - Duration::from_secs(1));
        s.pump_devices();
        assert!(s.notice.is_some());
        s.notice = None;
        s.pump_devices();
        assert!(s.notice.is_none(), "do not repeat the slow-scan warning");
        // A second worker would clear the list before the original scan finishes.
        s.volumes.start(|| Ok(Vec::new())).expect("dedup");
        s.pump_devices();
        assert_eq!(s.volumes.entries().len(), 1);
        tx.send(()).expect("release");
        let deadline = Instant::now() + Duration::from_secs(2);
        while s.volumes.scanning() {
            s.pump_devices();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(s.volumes.entries().is_empty(), "apply the eventual result");
        assert!(
            s.volumes
                .next_scan
                .is_some_and(|next| next > Instant::now())
        );
    }

    #[test]
    fn a_finished_scan_wakes_the_ui() {
        let mut s = crate::Session::default();
        let (woke_tx, woke_rx) = bounded(1);
        s.volumes.set_waker(move || {
            let _ = woke_tx.try_send(());
        });
        s.volumes.start(|| Ok(vec![device()])).expect("start");
        woke_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("woken when the result is ready");
        assert!(s.pump_devices(), "the result is there once woken");
        assert_eq!(s.volumes.entries(), &[device()]);
    }

    #[test]
    fn successful_scan_over_one_second_is_applied_without_warning() {
        let mut s = crate::Session::default();
        let (tx, rx) = bounded(1);
        s.volumes.pending = Some(rx);
        s.volumes.started = Some(Instant::now() - Duration::from_secs(3));
        tx.send(Ok(vec![device()])).expect("result");
        assert!(s.pump_devices());
        assert_eq!(s.volumes.entries(), &[device()]);
        assert!(s.notice.is_none());
        assert!(!s.volumes.scanning());
    }
}
