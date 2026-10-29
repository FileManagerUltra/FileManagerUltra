//! Product identity — **the only place in the workspace where the product name lives.**
//!
//! Window titles, About, installers, desktop entries and file associations read these
//! constants. Renaming the product is `cargo xtask rename "<NewName>"`, which rewrites
//! this file (plus the binary name) and records the old name so `cargo xtask brand-check`
//! keeps scanning for it.

/// Product display name: window title, About box, installer labels.
pub const APP_NAME: &str = "FileManagerUltra";

/// Lowercase slug used for file names, directory names and app-id components.
pub const APP_SLUG: &str = "filemanagerultra";

/// Reverse-DNS application id (desktop entry `StartupWMClass`, bundle identifier).
pub const APP_ID: &str = "io.filemanagerultra.app";

/// Vendor/organisation name shown in About and used by installers.
pub const ORG_NAME: &str = "FileManagerUltra";

/// Executable name. Must equal the `[[bin]] name` in `apps/fm-app/Cargo.toml`
/// (asserted by a test in `fm-app`).
pub const EXECUTABLE: &str = "filemanagerultra";

/// Window class; equal to the slug so X11/Wayland matching is predictable.
pub const WINDOW_CLASS: &str = APP_SLUG;

/// Public release source; organisation and repository names need not follow a product rename.
pub const RELEASE_REPOSITORY: &str = "FileManagerUltra/FileManagerUltra";

/// Platform-specific application icon (256 px PNG); source art lives in `assets/`.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon-256.png");

#[cfg(target_os = "windows")]
/// Windows gradient split-folder icon (256 px PNG).
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon-windows-256.png");

#[cfg(target_os = "macos")]
/// macOS rounded-tile split-folder icon (256 px PNG).
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon-macos-256.png");

/// Free-desktop entry template filled from the constants above.
///
/// Packaging (change 5) writes this to `<slug>.desktop`; nothing may hard-code the
/// name instead of calling this function.
pub fn desktop_entry() -> String {
    format!(
        "\
[Desktop Entry]
Type=Application
Name={name}
Exec={exec}
StartupWMClass={wm}
Icon={slug}
Categories=Utility;FileManager;
Terminal=false
",
        name = APP_NAME,
        exec = EXECUTABLE,
        wm = WINDOW_CLASS,
        slug = APP_SLUG,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_size(bytes: &[u8]) -> (u32, u32) {
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&bytes[12..16], b"IHDR");
        (
            u32::from_be_bytes(bytes[16..20].try_into().expect("width")),
            u32::from_be_bytes(bytes[20..24].try_into().expect("height")),
        )
    }

    #[test]
    fn platform_icons_are_complete_and_packaging_matches() {
        let linux = include_bytes!("../assets/icon-256.png");
        let windows = include_bytes!("../assets/icon-windows-256.png");
        let macos = include_bytes!("../assets/icon-macos-256.png");
        for icon in [
            linux.as_slice(),
            windows.as_slice(),
            macos.as_slice(),
            ICON_PNG,
        ] {
            assert_eq!(png_size(icon), (256, 256));
        }
        assert_ne!(linux.as_slice(), windows.as_slice());
        assert_ne!(linux.as_slice(), macos.as_slice());
        #[cfg(target_os = "windows")]
        assert_eq!(ICON_PNG, windows);
        #[cfg(target_os = "macos")]
        assert_eq!(ICON_PNG, macos);
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        assert_eq!(ICON_PNG, linux);
        assert_eq!(
            png_size(include_bytes!("../../../packaging/icon.png")),
            (512, 512)
        );
        assert_eq!(
            include_bytes!("../assets/logo.svg").as_slice(),
            include_bytes!("../../../packaging/logo.svg").as_slice()
        );
    }

    #[test]
    fn windows_installer_icon_has_all_launcher_sizes() {
        let ico = include_bytes!("../../../packaging/icon.ico");
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 6, 0]);
        let mut sizes = Vec::new();
        for entry in ico[6..102].as_chunks::<16>().0 {
            let width = if entry[0] == 0 {
                256
            } else {
                u16::from(entry[0])
            };
            let height = if entry[1] == 0 {
                256
            } else {
                u16::from(entry[1])
            };
            assert_eq!(width, height);
            let length = u32::from_le_bytes(entry[8..12].try_into().expect("length")) as usize;
            let offset = u32::from_le_bytes(entry[12..16].try_into().expect("offset")) as usize;
            assert!(
                !ico.get(offset..offset + length)
                    .expect("image data")
                    .is_empty()
            );
            sizes.push(width);
        }
        sizes.sort_unstable();
        assert_eq!(sizes, [16, 32, 48, 64, 128, 256]);
    }

    #[test]
    fn macos_bundle_icon_has_standard_and_retina_pngs() {
        let icns = include_bytes!("../../../packaging/icon.icns");
        assert_eq!(&icns[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(icns[4..8].try_into().expect("length")) as usize,
            icns.len()
        );
        let mut chunks = &icns[8..];
        for (kind, size) in [
            (b"icp4", 16),
            (b"icp5", 32),
            (b"icp6", 64),
            (b"ic07", 128),
            (b"ic08", 256),
            (b"ic09", 512),
            (b"ic10", 1024),
            (b"ic11", 32),
            (b"ic12", 64),
            (b"ic13", 256),
            (b"ic14", 512),
        ] {
            assert_eq!(&chunks[..4], kind);
            let length = u32::from_be_bytes(chunks[4..8].try_into().expect("chunk")) as usize;
            assert_eq!(png_size(&chunks[8..length]), (size, size));
            chunks = &chunks[length..];
        }
        assert!(chunks.is_empty());
    }

    #[test]
    fn identity_constants_are_non_empty() {
        for value in [
            APP_NAME,
            APP_SLUG,
            APP_ID,
            ORG_NAME,
            EXECUTABLE,
            WINDOW_CLASS,
        ] {
            assert!(!value.is_empty(), "empty identity constant");
            assert!(
                value.trim() == value,
                "identity constant has leading/trailing whitespace: {value:?}"
            );
        }
    }

    #[test]
    fn slug_is_lower_case_and_filename_safe() {
        assert!(
            APP_SLUG
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "APP_SLUG must be lowercase ascii: {APP_SLUG:?}"
        );
        assert!(!APP_SLUG.starts_with('.'), "APP_SLUG must not be a dotfile");
    }

    #[test]
    fn app_id_is_reverse_dns_and_mentions_the_slug() {
        assert!(
            APP_ID.contains('.'),
            "APP_ID must look like reverse-DNS: {APP_ID:?}"
        );
        assert!(
            APP_ID.contains(APP_SLUG),
            "APP_ID should contain APP_SLUG so a rename stays coherent: {APP_ID:?}"
        );
    }

    #[test]
    fn window_class_equals_slug() {
        assert_eq!(WINDOW_CLASS, APP_SLUG);
    }

    #[test]
    fn desktop_entry_is_filled_from_constants() {
        let entry = desktop_entry();
        assert!(entry.contains(&format!("Name={APP_NAME}\n")));
        assert!(entry.contains(&format!("Exec={EXECUTABLE}\n")));
        assert!(entry.contains(&format!("StartupWMClass={WINDOW_CLASS}\n")));
        assert!(
            !entry.contains('{'),
            "unsubstituted placeholder in {entry:?}"
        );
    }
}
