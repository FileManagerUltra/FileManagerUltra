use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use fm_vfs::{FmError, VfsUri};

use super::{Device, Removal, display_text};

fn error(message: impl Into<String>) -> FmError {
    FmError::backend("devices", message)
}

fn read_limited(reader: impl Read, cap: usize) -> Result<Vec<u8>, FmError> {
    let mut bytes = Vec::new();
    reader.take(cap as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > cap {
        return Err(error(
            "Volume metadata or tool output exceeds the safety limit",
        ));
    }
    Ok(bytes)
}

/// Drain both pipes concurrently and bound runtime/output. No shell, no elevation, no force.
fn output(command: &mut Command) -> Result<Vec<u8>, FmError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            error(format!(
                "Cannot start volume tool (install it or use the OS): {e}"
            ))
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| error("Missing tool output pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| error("Missing tool error pipe"))?;
    let read = std::thread::Builder::new()
        .name("devices-output".into())
        .spawn(move || read_limited(stdout, 2 * 1024 * 1024));
    let read_error = std::thread::Builder::new()
        .name("devices-error".into())
        .spawn(move || read_limited(stderr, 8192));
    let (read, read_error) = match (read, read_error) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error("Cannot start volume tool output readers"));
        }
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            other => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(match other {
                    Err(e) => error(format!("Cannot wait for volume tool: {e}")),
                    _ => error("Volume tool timed out; check the mount in the OS or VeraCrypt"),
                });
            }
        }
    };
    let stdout = read
        .join()
        .map_err(|_| error("Volume output reader stopped"))?;
    let stderr = read_error
        .join()
        .map_err(|_| error("Volume error reader stopped"))?;
    let status = status?;
    let stdout = stdout?;
    let stderr = stderr?;
    if !status.success() {
        let detail = display_text(&String::from_utf8_lossy(&stderr));
        return Err(error(format!(
            "Cannot remove/read volume ({status}): {detail}. Close open files or use the OS/VeraCrypt."
        )));
    }
    Ok(stdout)
}

fn unsupported() -> Removal {
    Removal::Unsupported(
        "Dismount this volume in VeraCrypt or the OS. Its owner/safe removal method could not be verified.".into(),
    )
}

fn device(
    mount: &str,
    name: &str,
    filesystem: &str,
    identity: String,
    capacity: Option<(u64, u64)>,
    removal: Removal,
) -> Result<Device, FmError> {
    let mount = VfsUri::parse(mount)?;
    Ok(Device {
        name: display_text(name),
        mount,
        filesystem: display_text(filesystem),
        total: capacity.map(|(total, _)| total),
        free: capacity.map(|(_, free)| free),
        identity,
        removal,
    })
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    #[derive(Debug)]
    pub(super) struct Mount {
        pub id: String,
        pub dev: String,
        pub point: String,
        pub fs: String,
        pub source: String,
    }

    fn unescape(text: &str) -> String {
        text.replace("\\040", " ")
            .replace("\\011", "\t")
            .replace("\\012", "\n")
            .replace("\\134", "\\")
    }

    pub(super) fn mounts(text: &str) -> Result<Vec<Mount>, FmError> {
        let mut out = Vec::new();
        for line in text.lines() {
            let (left, right) = line
                .split_once(" - ")
                .ok_or_else(|| error("Invalid mount table"))?;
            let fields: Vec<_> = left.split_whitespace().collect();
            let tail: Vec<_> = right.split_whitespace().collect();
            let get = |i| {
                fields
                    .get(i)
                    .copied()
                    .ok_or_else(|| error("Invalid mount table"))
            };
            out.push(Mount {
                id: get(0)?.into(),
                dev: get(2)?.into(),
                point: unescape(get(4)?),
                fs: tail
                    .first()
                    .ok_or_else(|| error("Missing filesystem"))?
                    .to_string(),
                source: unescape(tail.get(1).ok_or_else(|| error("Missing volume source"))?),
            });
        }
        Ok(out)
    }

    fn read_mounts() -> Result<Vec<Mount>, FmError> {
        let bytes = read_limited(
            std::fs::File::open("/proc/self/mountinfo")?,
            2 * 1024 * 1024,
        )?;
        let text = std::str::from_utf8(&bytes).map_err(|_| error("Non-UTF-8 mount table"))?;
        mounts(text)
    }

    fn network(fs: &str) -> bool {
        matches!(
            fs,
            "cifs" | "smb3" | "nfs" | "nfs4" | "fuse.sshfs" | "davfs"
        )
    }

    pub(super) fn eligible(mount: &Mount, root_dev: Option<&str>) -> bool {
        let path = Path::new(&mount.point);
        if !path.is_absolute()
            || matches!(mount.fs.as_str(), "fuse.portal" | "fuse.gvfsd-fuse")
            || matches!(mount.point.as_str(), "/" | "/home" | "/tmp" | "/var")
            || [
                "/boot", "/dev", "/proc", "/sys", "/etc", "/usr", "/var", "/snap",
            ]
            .iter()
            .any(|prefix| path.starts_with(prefix))
            || (path.starts_with("/run")
                && !path.starts_with("/run/media")
                && !mount.fs.starts_with("fuse"))
            || root_dev == Some(mount.dev.as_str())
            || path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".veracrypt_aux_mnt"))
        {
            return false;
        }
        network(&mount.fs)
            || mount.source.starts_with("/dev/")
                && !matches!(mount.fs.as_str(), "squashfs" | "autofs")
            || mount.fs.starts_with("fuse") && mount.fs != "fusectl"
    }

    fn identity(m: &Mount) -> String {
        format!("{}:{}:{}:{}", m.id, m.dev, m.source, m.fs)
    }

    fn drive(m: &Mount) -> Option<PathBuf> {
        let sys = std::fs::canonicalize(Path::new("/sys/dev/block").join(&m.dev)).ok()?;
        if sys.join("partition").is_file() {
            sys.parent().map(Path::to_path_buf)
        } else {
            Some(sys)
        }
    }

    fn removal(m: &Mount, all: &[Mount]) -> Removal {
        if network(&m.fs) {
            return if Path::new("/bin/umount").is_file() {
                Removal::Dismount
            } else {
                unsupported()
            };
        }
        // Device mapper and FUSE can belong to VeraCrypt or other volume managers.
        // A label/source spelling is not proof of ownership.
        if !m.source.starts_with("/dev/")
            || m.source.starts_with("/dev/mapper/")
            || m.source.starts_with("/dev/dm-")
            || m.source.starts_with("/dev/loop")
            || m.fs.starts_with("fuse")
            || !Path::new("/usr/bin/udisksctl").is_file()
        {
            return unsupported();
        }
        let Some(parent) = drive(m) else {
            return unsupported();
        };
        if parent.join("dm").exists() {
            return unsupported();
        }
        let external = parent.to_string_lossy().contains("/usb")
            || std::fs::read_to_string(parent.join("removable")).is_ok_and(|s| s.trim() == "1");
        let siblings = all
            .iter()
            .any(|other| other.id != m.id && drive(other).as_ref() == Some(&parent));
        if external && !siblings {
            Removal::Eject
        } else {
            Removal::Dismount
        }
    }

    pub(crate) fn scan() -> Result<Vec<Device>, FmError> {
        scan_impl(true)
    }

    pub(crate) fn scan_current() -> Result<Vec<Device>, FmError> {
        scan_impl(false)
    }

    fn scan_impl(capacity: bool) -> Result<Vec<Device>, FmError> {
        let all = read_mounts()?;
        let root = all.iter().find(|m| m.point == "/").map(|m| m.dev.as_str());
        // sysinfo deliberately skips hard NFS/CIFS mounts to avoid statvfs hangs;
        // mountinfo still exposes them below, with unavailable capacity.
        // An unreadable disk list leaves capacity unavailable, as before sysinfo 0.40.
        let disks: Vec<sysinfo::Disk> = if capacity {
            sysinfo::Disks::new_with_refreshed_list_specifics(
                sysinfo::DiskRefreshKind::nothing().with_storage(),
            )
        } else {
            sysinfo::Disks::new()
        }
        .map(Vec::from)
        .unwrap_or_default();
        let labels: HashMap<_, _> = match std::fs::read_dir("/dev/disk/by-label") {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let target = std::fs::canonicalize(entry.path()).ok()?;
                    Some((target, entry.file_name().to_string_lossy().into_owned()))
                })
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for m in all.iter().filter(|m| eligible(m, root)) {
            if out.len() >= super::super::MAX_DEVICES {
                return Err(error("Too many mounted volumes (limit 256)"));
            }
            let capacity = disks
                .iter()
                .find(|d| d.mount_point() == Path::new(&m.point) && d.total_space() > 0)
                .map(|d| (d.total_space(), d.available_space()));
            let name = labels
                .get(Path::new(&m.source))
                .cloned()
                .unwrap_or_else(|| {
                    Path::new(&m.point)
                        .file_name()
                        .map_or_else(|| m.point.clone(), |n| n.to_string_lossy().into_owned())
                });
            out.push(device(
                &m.point,
                &name,
                &m.fs,
                identity(m),
                capacity,
                removal(m, &all),
            )?);
        }
        Ok(out)
    }

    pub(crate) fn remove(d: &Device) -> Result<(), FmError> {
        let all = read_mounts()?;
        let mount = all
            .iter()
            .find(|m| identity(m) == d.identity)
            .ok_or_else(|| error("Mount identity changed; refresh Devices"))?;
        if removal(mount, &all) != d.removal {
            return Err(error("Mount removal capability changed; refresh Devices"));
        }
        if network(&mount.fs) {
            output(Command::new("/bin/umount").args(["--", &mount.point]))?;
        } else {
            output(Command::new("/usr/bin/udisksctl").args([
                "unmount",
                "--no-user-interaction",
                "-b",
                &mount.source,
            ]))?;
            if d.removal == Removal::Eject {
                // Refuse to power off if any other filesystem on this disk is now mounted.
                let remaining = read_mounts()?;
                let parent =
                    drive(mount).ok_or_else(|| error("Cannot identify the parent drive"))?;
                if remaining.iter().any(|m| drive(m).as_ref() == Some(&parent)) {
                    return Err(error(
                        "Volume unmounted, but another volume uses this disk; power-off skipped",
                    ));
                }
                output(Command::new("/usr/bin/udisksctl").args([
                    "power-off",
                    "--no-user-interaction",
                    "-b",
                    &mount.source,
                ]))?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn desktop_bridges_are_hidden_but_user_fuse_mounts_remain_visible() {
            let entries = mounts(concat!(
                "1 0 0:50 / /run/user/1000/doc rw - fuse.portal portal rw\n",
                "2 0 0:51 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n",
                "3 0 0:52 / /mnt/portal rw - fuse.portal portal rw\n",
                "4 0 0:53 / /mnt/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n",
                "5 0 0:54 / /run/user/1000/remote rw - fuse.sshfs user@host:/data rw\n",
                "6 0 0:55 / /mnt/encrypted rw - fuse.gocryptfs gocryptfs rw\n",
            ))
            .expect("mounts");
            let visible: Vec<_> = entries
                .iter()
                .filter(|m| eligible(m, None))
                .map(|m| m.point.as_str())
                .collect();
            assert_eq!(visible, ["/run/user/1000/remote", "/mnt/encrypted"]);
        }

        #[test]
        fn mount_table_includes_virtual_network_and_escaped_paths_not_system() {
            let entries = mounts(concat!(
                "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n",
                "2 1 8:1 / /mnt/bind rw - ext4 /dev/sda1 rw\n",
                "3 1 0:7 / /proc rw - proc proc rw\n",
                "4 1 7:0 / /snap/pkg rw - squashfs /dev/loop0 ro\n",
                "5 1 253:1 / /media/veracrypt1 rw - ext4 /dev/mapper/veracrypt1 rw\n",
                "6 1 0:50 / /mnt/share rw - cifs //server/share rw\n",
                "7 1 8:17 / /media/My\\040USB rw - exfat /dev/sdb1 rw\n",
            ))
            .expect("mounts");
            let visible: Vec<_> = entries
                .iter()
                .filter(|m| eligible(m, Some("8:1")))
                .collect();
            assert_eq!(visible.len(), 3);
            assert_eq!(visible.last().expect("usb").point, "/media/My USB");
            assert!(matches!(
                removal(visible[0], &entries),
                Removal::Unsupported(_)
            ));
            assert!(mounts("broken").is_err());
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) use linux::{remove, scan, scan_current};

#[cfg(any(target_os = "windows", test))]
mod windows {
    use super::*;

    const SCAN: &str = r#"
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
@(Get-CimInstance Win32_LogicalDisk | Where-Object {
    $_.DeviceID -ne $env:SystemDrive -and $_.DriveType -in 2,3,4 -and
    ($_.DriveType -ne 2 -or $_.Size -gt 0 -or -not [string]::IsNullOrEmpty($_.VolumeSerialNumber))
} | ForEach-Object {
    $volume=$_
    $userMounted=$volume.DriveType -in 2,4
    if ($volume.DriveType -eq 3) {
        $partitions=@(Get-CimAssociatedInstance -InputObject $volume -Association Win32_LogicalDiskToPartition)
        $userMounted=$partitions.Count -eq 0
        foreach ($partition in $partitions) {
            foreach ($disk in @(Get-CimAssociatedInstance -InputObject $partition -Association Win32_DiskDriveToDiskPartition)) {
                if ($disk.InterfaceType -eq 'USB' -or $disk.PNPDeviceID -match '^(USBSTOR|USB)\\') {
                    $userMounted=$true
                }
            }
        }
    }
    $volume | Select-Object DeviceID,VolumeName,VolumeSerialNumber,ProviderName,FileSystem,Size,FreeSpace,DriveType,@{Name='UserMounted';Expression={$userMounted}}
}) | ConvertTo-Json -Compress
"#;
    const EJECT: &str = r#"
$ErrorActionPreference='Stop'
$d=$env:FM_EJECT_DRIVE
if ($d -notmatch '^[A-Za-z]:$') { throw 'Invalid drive' }
$volume=Get-CimInstance Win32_LogicalDisk | Where-Object { $_.DeviceID -eq $d }
if ($null -eq $volume -or $volume.DriveType -ne 2 -or [string]::IsNullOrEmpty($volume.VolumeSerialNumber)) {
    throw 'The selected removable volume is no longer available'
}
$identity='{0}:{1}:{2}' -f $volume.DeviceID,$volume.VolumeSerialNumber,$volume.ProviderName
if ($identity -cne $env:FM_EJECT_IDENTITY) { throw 'Mount identity changed; refresh Devices' }
$shell=New-Object -ComObject Shell.Application
$folder=$shell.Namespace(17)
$item=$folder.ParseName($d + '\')
if ($null -eq $item) { throw 'Drive not found in Windows Shell' }
$item.InvokeVerb('Eject')
$deadline=[DateTime]::UtcNow.AddSeconds(5)
do {
    Start-Sleep -Milliseconds 250
    $current=Get-CimInstance Win32_LogicalDisk | Where-Object { $_.DeviceID -eq $d }
    if ($null -eq $current -or ($current.Size -le 0 -and [string]::IsNullOrEmpty($current.VolumeSerialNumber))) { exit 0 }
} while ([DateTime]::UtcNow -lt $deadline)
throw 'Windows did not eject the selected drive. Close open files and use Safely Remove Hardware if it remains busy'
"#;

    fn powershell(script: &str) -> Command {
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-STA", "-Command", script]);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }

    pub(crate) fn scan() -> Result<Vec<Device>, FmError> {
        let bytes = output(&mut powershell(SCAN))?;
        parse(&bytes)
    }

    fn parse(bytes: &[u8]) -> Result<Vec<Device>, FmError> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| error(format!("Cannot read Windows volumes: {e}")))?;
        let entries = value.as_array().cloned().unwrap_or_else(|| {
            if value.is_null() {
                Vec::new()
            } else {
                vec![value]
            }
        });
        entries
            .iter()
            .filter(|v| match v.get("DriveType").and_then(|v| v.as_u64()) {
                Some(2) => {
                    v.get("Size")
                        .and_then(|v| v.as_u64())
                        .is_some_and(|size| size > 0)
                        || v.get("VolumeSerialNumber")
                            .and_then(|v| v.as_str())
                            .is_some_and(|serial| !serial.is_empty())
                }
                Some(4) => true,
                Some(3) => v.get("UserMounted").and_then(|v| v.as_bool()) == Some(true),
                _ => false,
            })
            .map(|v| {
                let text = |key| v.get(key).and_then(|v| v.as_str()).unwrap_or("");
                let drive = text("DeviceID");
                if drive.len() != 2
                    || !drive
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphabetic)
                    || !drive.ends_with(':')
                {
                    return Err(error("Invalid Windows volume identity"));
                }
                let label = if text("VolumeName").is_empty() {
                    drive
                } else {
                    text("VolumeName")
                };
                let capacity = v
                    .get("Size")
                    .and_then(|v| v.as_u64())
                    .zip(v.get("FreeSpace").and_then(|v| v.as_u64()));
                let removal = if !text("VolumeSerialNumber").is_empty()
                    && v.get("DriveType").and_then(|v| v.as_u64()) == Some(2)
                {
                    Removal::Eject
                } else {
                    unsupported()
                };
                let identity = format!(
                    "{drive}:{}:{}",
                    text("VolumeSerialNumber"),
                    text("ProviderName")
                );
                device(
                    &format!("{drive}\\"),
                    label,
                    text("FileSystem"),
                    identity,
                    capacity,
                    removal,
                )
            })
            .collect()
    }

    pub(crate) fn remove(d: &Device) -> Result<(), FmError> {
        if d.removal != Removal::Eject {
            return Err(error("Dismount this volume in VeraCrypt or Windows"));
        }
        let path = d.mount.local_path().ok_or_else(|| error("Invalid drive"))?;
        let drive = path.to_string_lossy();
        output(
            powershell(EJECT)
                .env("FM_EJECT_DRIVE", drive.trim_end_matches('\\'))
                .env("FM_EJECT_IDENTITY", &d.identity),
        )?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn unplugged_media_slots_disappear_without_hiding_other_volumes() {
            let plugged = parse(br#"[
                {"DeviceID":"E:","VolumeName":"First","VolumeSerialNumber":"111","DriveType":2,"Size":4096},
                {"DeviceID":"F:","VolumeName":"Second","VolumeSerialNumber":"222","DriveType":2,"Size":8192},
                {"DeviceID":"Z:","ProviderName":"\\\\server\\share","DriveType":4}
            ]"#).expect("plugged");
            let unplugged = parse(br#"[
                {"DeviceID":"E:","VolumeName":"","VolumeSerialNumber":null,"DriveType":2,"Size":0},
                {"DeviceID":"F:","VolumeName":"Second","VolumeSerialNumber":"222","DriveType":2,"Size":8192},
                {"DeviceID":"G:","DriveType":2},
                {"DeviceID":"Z:","ProviderName":"\\\\server\\share","DriveType":4}
            ]"#).expect("unplugged");
            assert_eq!(unplugged.len(), 2);
            assert_eq!(unplugged[0].name, "Second");
            let mut session = crate::Session::default();
            let removed = plugged[0].mount.clone();
            session.layout.state_mut(1).expect("pane").location = removed.clone();
            session.volumes.replace(plugged);
            session.volumes.start(move || Ok(unplugged)).expect("scan");
            let deadline = Instant::now() + Duration::from_secs(2);
            while session.volumes.scanning() {
                session.pump_devices();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(session.volumes.entries().len(), 2);
            assert!(session.volumes.unavailable(&removed));
            assert!(matches!(
                session.dirs.state(&removed),
                crate::DirState::Failed { .. }
            ));
            assert!(
                session
                    .info
                    .as_ref()
                    .is_some_and(|(message, _)| message.contains("First disconnected"))
            );
        }

        #[test]
        fn windows_metadata_includes_virtual_fixed_drives_and_network_mappings() {
            // Compile the complete Windows adapter on other hosts too, but never execute
            // PowerShell or a removal tool in a fixture test.
            let _scan: fn() -> Result<Vec<Device>, FmError> = scan;
            let _remove: fn(&Device) -> Result<(), FmError> = remove;
            let entries = parse(br#"[
                {"DeviceID":"E:","VolumeName":"USB","VolumeSerialNumber":"123","DriveType":2,"Size":4096,"FreeSpace":2048},
                {"DeviceID":"V:","VolumeName":"Private","VolumeSerialNumber":"456","DriveType":3,"UserMounted":true},
                {"DeviceID":"Z:","VolumeName":"","ProviderName":"\\\\server\\share","DriveType":4},
                {"DeviceID":"D:","VolumeName":"Internal data","DriveType":3,"UserMounted":false},
                {"DeviceID":"F:","VolumeName":"Unclassified fixed disk","DriveType":3},
                {"DeviceID":"G:","VolumeName":"External USB HDD","DriveType":3,"UserMounted":true}
            ]"#).expect("volumes");
            assert_eq!(entries.len(), 4);
            assert_eq!(entries[0].removal, Removal::Eject);
            assert_eq!(entries[0].total, Some(4096));
            assert!(matches!(entries[1].removal, Removal::Unsupported(_)));
            assert_eq!(entries[2].name, "Z:");
            assert_eq!(entries[2].free, None);
            assert_eq!(entries[3].name, "External USB HDD");
            assert!(matches!(entries[3].removal, Removal::Unsupported(_)));
            assert!(parse(br#"{"DeviceID":"E:;bad","DriveType":2,"Size":4096}"#).is_err());
            assert!(parse(b"null").expect("no volumes").is_empty());
        }
    }
}

#[cfg(target_os = "windows")]
pub(super) use windows::scan as scan_current;
#[cfg(target_os = "windows")]
pub(super) use windows::{remove, scan};

#[cfg(any(target_os = "macos", test))]
mod macos {
    use super::*;
    use std::path::Path;

    pub(crate) fn scan() -> Result<Vec<Device>, FmError> {
        scan_impl(true)
    }

    pub(crate) fn scan_current() -> Result<Vec<Device>, FmError> {
        scan_impl(false)
    }

    fn scan_impl(capacity: bool) -> Result<Vec<Device>, FmError> {
        // An unreadable disk list means no devices, as before sysinfo 0.40.
        let disks: Vec<sysinfo::Disk> =
            sysinfo::Disks::new_with_refreshed_list_specifics(if capacity {
                sysinfo::DiskRefreshKind::nothing().with_storage()
            } else {
                sysinfo::DiskRefreshKind::nothing()
            })
            .map(Vec::from)
            .unwrap_or_default();
        let mut out = Vec::new();
        for disk in disks.iter().filter(|d| {
            let path = d.mount_point();
            path != Path::new("/")
                && path != Path::new("/Volumes")
                && !["/System", "/private", "/dev"]
                    .iter()
                    .any(|prefix| path.starts_with(prefix))
                && !matches!(d.file_system().to_str(), Some("autofs" | "devfs"))
        }) {
            if disk.mount_point() == Path::new("/Volumes") {
                continue;
            }
            let mount = disk
                .mount_point()
                .to_str()
                .ok_or_else(|| error("Non-UTF-8 mount point"))?;
            let result = output(Command::new("/usr/sbin/diskutil").args(["info", "-plist", mount]))
                .and_then(|bytes| {
                    plist::Value::from_reader(std::io::Cursor::new(bytes))
                        .map_err(|e| error(format!("Cannot parse volume identity: {e}")))
                });
            let (info, diagnostic) = match result {
                Ok(info) => (Some(info), None),
                Err(error) => (None, Some(error.to_string())),
            };
            let text = |key| {
                info.as_ref()
                    .and_then(|v| v.as_dictionary())
                    .and_then(|v| v.get(key))
                    .and_then(|v| v.as_string())
            };
            let physical = text("VirtualOrPhysical") == Some("Physical");
            let id = text("VolumeUUID").or_else(|| text("DeviceIdentifier"));
            let identity = id
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{}:{mount}", disk.name().to_string_lossy()));
            // diskutil unmount is volume-scoped; never eject a whole APFS/physical disk
            // based on one mount, since it may contain sibling volumes.
            let removal = if physical && id.is_some() {
                Removal::Dismount
            } else {
                match diagnostic {
                    Some(message) => Removal::Unsupported(format!(
                        "Cannot verify removal: {message}. Use the OS/VeraCrypt."
                    )),
                    None => unsupported(),
                }
            };
            out.push(device(
                mount,
                &disk.name().to_string_lossy(),
                &disk.file_system().to_string_lossy(),
                identity,
                (disk.total_space() > 0).then(|| (disk.total_space(), disk.available_space())),
                removal,
            )?);
        }
        Ok(out)
    }

    pub(crate) fn remove(d: &Device) -> Result<(), FmError> {
        if d.removal != Removal::Dismount {
            return Err(error("Dismount this volume in VeraCrypt"));
        }
        let path = d.mount.local_path().ok_or_else(|| error("Invalid mount"))?;
        output(Command::new("/usr/sbin/diskutil").arg("unmount").arg(path))?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn macos_adapter_compiles_and_plist_preserves_volume_identity() {
            let _scan: fn() -> Result<Vec<Device>, FmError> = scan;
            let _scan_current: fn() -> Result<Vec<Device>, FmError> = scan_current;
            let _remove: fn(&Device) -> Result<(), FmError> = remove;
            let bytes = br#"<?xml version="1.0" encoding="UTF-8"?>
                <plist version="1.0"><dict>
                    <key>VolumeUUID</key><string>ABC-123</string>
                    <key>VirtualOrPhysical</key><string>Virtual</string>
                </dict></plist>"#;
            let info = plist::Value::from_reader(std::io::Cursor::new(bytes)).expect("plist");
            assert_eq!(
                info.as_dictionary()
                    .expect("dict")
                    .get("VolumeUUID")
                    .and_then(plist::Value::as_string),
                Some("ABC-123")
            );
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) use macos::{remove, scan, scan_current};

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) fn scan() -> Result<Vec<Device>, FmError> {
    Err(FmError::Unsupported(
        "mounted volume discovery on this platform",
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) fn remove(_: &Device) -> Result<(), FmError> {
    Err(FmError::Unsupported("device removal on this platform"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use scan as scan_current;
