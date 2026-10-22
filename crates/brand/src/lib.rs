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
Categories=Utility;FileManagement;
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
