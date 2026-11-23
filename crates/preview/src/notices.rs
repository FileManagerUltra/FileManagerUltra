//! Third-party parts built into the app whose licence asks to be shown with it: anything
//! not under the app's own Apache-2.0 or a permissive MIT-style crate licence, and bundled
//! data (fonts, CMaps) whose BSD licence asks for its notice in binaries. Help ▸
//! Third-party Licences lists them, so every installer carries the notice inside the
//! program.

/// One bundled third-party part.
#[derive(Clone, Copy, Debug)]
pub struct Notice {
    pub name: &'static str,
    /// What it does in the app.
    pub purpose: &'static str,
    /// SPDX identifier.
    pub licence: &'static str,
    /// Where its source code is available.
    pub source: &'static str,
    /// The full licence text.
    pub text: &'static str,
}

/// Every part that needs a notice, in the order About shows them.
pub const NOTICES: &[Notice] = &[
    Notice {
        name: "Atkinson Hyperlegible",
        purpose: "Reading font for Markdown and diagrams",
        licence: "OFL-1.1",
        source: "https://github.com/google/fonts/tree/main/ofl/atkinsonhyperlegible",
        text: include_str!("../assets/fonts/OFL.txt"),
    },
    Notice {
        name: "Dancing Script",
        purpose: "Typed visual signatures in PDFs (drawn as outlines, never embedded)",
        licence: "OFL-1.1",
        source: "https://github.com/googlefonts/DancingScript",
        text: fm_pdf::visual::SIGNATURE_FONT_LICENSE,
    },
    Notice {
        name: "Foxit standard fonts (from PDFium)",
        purpose: "Drawing text in PDFs that do not embed their fonts",
        licence: "BSD-3-Clause",
        source: "https://pdfium.googlesource.com/pdfium/",
        text: fm_pdf::FOXIT_FONTS_LICENSE,
    },
    Notice {
        name: "Adobe CMap resources",
        purpose: "Chinese, Japanese and Korean text in PDFs",
        licence: "BSD-3-Clause",
        source: "https://github.com/adobe-type-tools/cmap-resources",
        text: fm_pdf::ADOBE_CMAPS_LICENSE,
    },
    Notice {
        name: "Eclipse ELK layout",
        purpose: "Layout of Mermaid diagrams (Rust port merman-elk-layered 0.10.0)",
        licence: "EPL-2.0",
        source: "https://crates.io/crates/merman-elk-layered/0.10.0",
        text: include_str!("../licenses/EPL-2.0.txt"),
    },
    Notice {
        name: "bzip2 / libbzip2 (Rust translation libbzip2-rs)",
        purpose: "Reading and writing bzip2 in archives (.tar.bz2, zip, 7z)",
        licence: "bzip2-1.0.6",
        source: "https://crates.io/crates/libbz2-rs-sys/0.2.7",
        text: include_str!("../licenses/bzip2-1.0.6.txt"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_notice_has_its_full_licence_text() {
        assert!(!NOTICES.is_empty());
        for n in NOTICES {
            assert!(n.text.len() > 1000, "{} text", n.name);
            assert!(n.source.starts_with("https://"), "{} source", n.name);
        }
        let epl = NOTICES
            .iter()
            .find(|n| n.licence == "EPL-2.0")
            .expect("EPL notice");
        assert!(epl.text.starts_with("Eclipse Public License - v 2.0"));
    }

    /// The bzip2 notice names the locked version of the translation.
    #[test]
    fn the_bzip2_notice_matches_the_locked_version() {
        let lock = include_str!("../../../Cargo.lock").replace("\r\n", "\n");
        assert!(
            lock.contains("name = \"libbz2-rs-sys\"\nversion = \"0.2.7\""),
            "update the notice when libbz2-rs-sys changes"
        );
    }

    /// The EPL part is pinned: a merman upgrade must revisit this notice.
    #[test]
    fn the_elk_notice_matches_the_locked_version() {
        // Windows checkouts may turn the lock file's line endings into CRLF.
        let lock = include_str!("../../../Cargo.lock").replace("\r\n", "\n");
        assert!(
            lock.contains("name = \"merman-elk-layered\"\nversion = \"0.10.0\""),
            "update the notice and licence text when merman-elk-layered changes"
        );
    }
}
