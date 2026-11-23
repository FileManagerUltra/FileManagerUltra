//! Disk images opened from the app (an ISO handed to the OS, which mounts it): the pane
//! goes into the mount as soon as it appears in Devices, and an image that is already
//! mounted is shown instead of being mounted a second time.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fm_vfs::VfsUri;

use super::{Device, display_text};
use crate::registry::Params;

/// How long after opening an image its mount is waited for.
pub(super) const AWAIT_FOR: Duration = Duration::from_secs(30);
/// Devices scan interval while an image mount is awaited.
pub(super) const AWAIT_SCAN_INTERVAL: Duration = Duration::from_millis(1000);

/// File types the OS mounts when they are opened.
const IMAGE_SUFFIXES: [&str; 5] = [".iso", ".img", ".dmg", ".vhd", ".vhdx"];

/// An image handed to the OS whose mount is not listed yet.
#[derive(Clone, Debug)]
pub(crate) struct AwaitImage {
    path: PathBuf,
    name: String,
    /// Mounts listed when the image was opened.
    known: Vec<VfsUri>,
    until: Instant,
}

pub fn is_disk_image_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    IMAGE_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

/// One spelling for comparing image paths: no `\\?\` prefix, and case-insensitive where
/// the file system usually is.
fn normalized(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if cfg!(any(windows, target_os = "macos")) {
        text.to_lowercase()
    } else {
        text.to_string()
    }
}

pub(crate) fn same_image(a: &Path, b: &Path) -> bool {
    normalized(a) == normalized(b)
}

/// The listed device backed by `path`.
pub(crate) fn mount_of<'a>(devices: &'a [Device], path: &Path) -> Option<&'a Device> {
    devices.iter().find(|d| {
        d.image
            .as_deref()
            .is_some_and(|image| same_image(image, path))
    })
}

/// The device an awaited image became: the one backed by it, or, where the OS does not
/// report backing files (macOS), the only device that is new since it was opened.
fn arrived<'a>(wait: &AwaitImage, devices: &'a [Device]) -> Option<&'a Device> {
    if let Some(device) = mount_of(devices, &wait.path) {
        return Some(device);
    }
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut new = devices.iter().filter(|d| !wait.known.contains(&d.mount));
    match (new.next(), new.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

impl crate::Session {
    /// The mount of a local disk image that is already listed in Devices.
    pub(crate) fn mounted_image(&self, path: &Path) -> Option<VfsUri> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        mount_of(&self.volumes.entries, &path).map(|d| d.mount.clone())
    }

    /// Wait for the mount of an image just handed to the OS, scanning Devices sooner.
    pub(crate) fn await_image(&mut self, path: &Path, name: &str) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.volumes.awaiting = Some(AwaitImage {
            path,
            name: display_text(name),
            known: self
                .volumes
                .entries
                .iter()
                .map(|d| d.mount.clone())
                .collect(),
            until: Instant::now() + AWAIT_FOR,
        });
        self.volumes.next_scan = None;
    }

    /// After a Devices scan: open the awaited image's mount in the focused pane.
    pub(super) fn check_awaited_image(&mut self) {
        let Some(wait) = self.volumes.awaiting.clone() else {
            return;
        };
        if Instant::now() > wait.until {
            self.volumes.awaiting = None;
            return;
        }
        let Some(device) = arrived(&wait, &self.volumes.entries) else {
            return;
        };
        let (mount, label) = (device.mount.clone(), display_text(&device.name));
        self.volumes.awaiting = None;
        self.show_image_mount(&mount, &wait.name, &label);
    }

    /// Go into an image's mount and say so.
    pub(crate) fn show_image_mount(&mut self, mount: &VfsUri, image: &str, label: &str) {
        match self.dispatch("nav.open", &Params::String(mount.to_string())) {
            Ok(()) => self.info = Some((format!("{image} is mounted as {label}"), Instant::now())),
            Err(error) => self.notice = Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_device(mount: &str, image: Option<&str>) -> Device {
        let mut d = super::super::tests::device();
        d.mount = VfsUri::parse(mount).expect("uri");
        d.name = "DISC".into();
        d.image = image.map(PathBuf::from);
        d
    }

    #[test]
    fn disk_image_names() {
        assert!(is_disk_image_name("Disc.ISO"));
        assert!(is_disk_image_name("win.vhdx"));
        assert!(!is_disk_image_name("photo.jpg"));
        assert!(!is_disk_image_name("iso"));
    }

    #[test]
    fn an_awaited_image_opens_its_mount_in_the_focused_pane() {
        let tmp = tempfile::tempdir().expect("tmp");
        let iso = tmp.path().join("disc.iso");
        std::fs::write(&iso, b"x").expect("iso");
        let mount = tmp.path().join("mnt");
        std::fs::create_dir(&mount).expect("mnt");
        let iso = std::fs::canonicalize(&iso).expect("canon");
        let mount_uri = mount.to_str().expect("utf8");

        let mut s = crate::Session::new().expect("session");
        s.volumes.replace(vec![image_device("/media/other", None)]);
        s.await_image(&iso, "disc.iso");
        assert!(s.mounted_image(&iso).is_none());
        s.volumes.replace(vec![
            image_device("/media/other", None),
            image_device(mount_uri, iso.to_str()),
        ]);
        s.check_awaited_image();
        assert!(s.volumes.awaiting.is_none());
        let pane = s.layout.state(s.layout.focused()).expect("pane");
        assert_eq!(pane.location, VfsUri::parse(mount_uri).expect("uri"));
        assert!(
            s.info
                .as_ref()
                .is_some_and(|(m, _)| m == "disc.iso is mounted as DISC")
        );
        assert_eq!(
            s.mounted_image(&iso),
            Some(VfsUri::parse(mount_uri).expect("uri"))
        );
    }

    #[test]
    fn an_unrelated_new_device_is_not_taken_for_the_image() {
        let mut s = crate::Session::new().expect("session");
        s.volumes.replace(Vec::new());
        s.await_image(Path::new("/nowhere/disc.iso"), "disc.iso");
        s.volumes.replace(vec![image_device("/media/usb", None)]);
        s.check_awaited_image();
        if cfg!(target_os = "macos") {
            assert!(s.volumes.awaiting.is_none());
        } else {
            assert!(s.volumes.awaiting.is_some(), "still waiting");
        }
    }
}
