# Devices section for removable media and user-mounted volumes

Show removable drives (USB sticks, SD cards, external HDDs/SSDs) and user-mounted
volumes (including mounted VeraCrypt volumes, data disks and network mounts) in a
**Devices** section of Places, only while at least one eligible volume is mounted.
The list follows mount/unmount and plug/unplug, shows the label, mount location,
size and free space, and offers Eject for removable hardware or Dismount for
supported mounted volumes. Pure Rust orchestration, platform-specific discovery
and tools where needed, no new `unsafe`.

Out of scope: unlocking or mounting an unmounted volume, password management,
partitioning/formatting, and OS-event based detection (udev, DiskArbitration,
WM_DEVICECHANGE). Locked VeraCrypt containers do not appear.
