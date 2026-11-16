//! Third-party parts built into the app whose licence asks to be shown with it (anything
//! not under the app's own Apache-2.0 or a permissive MIT/BSD-style licence). The About
//! dialog lists them, so every installer carries the notice inside the program.

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
        name: "Eclipse ELK layered layout (merman-elk-layered 0.10.0)",
        purpose: "Layout of Mermaid diagrams",
        licence: "EPL-2.0",
        source: "https://crates.io/crates/merman-elk-layered/0.10.0",
        text: include_str!("../licenses/EPL-2.0.txt"),
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

    /// The EPL part is pinned: a merman upgrade must revisit this notice.
    #[test]
    fn the_elk_notice_matches_the_locked_version() {
        let lock = include_str!("../../../Cargo.lock");
        assert!(
            lock.contains("name = \"merman-elk-layered\"\nversion = \"0.10.0\""),
            "update the notice and licence text when merman-elk-layered changes"
        );
    }
}
