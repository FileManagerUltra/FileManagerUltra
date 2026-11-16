//! The bundled reading face (Atkinson Hyperlegible, SIL Open Font License in
//! `assets/fonts/OFL.txt`). The Markdown preview reads with it, and Mermaid diagrams are
//! measured and drawn with the same bytes so their labels always fit.

/// Family name inside the font files.
pub const FAMILY: &str = "Atkinson Hyperlegible";
pub const REGULAR: &[u8] = include_bytes!("../assets/fonts/AtkinsonHyperlegible-Regular.ttf");
pub const BOLD: &[u8] = include_bytes!("../assets/fonts/AtkinsonHyperlegible-Bold.ttf");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_faces_are_truetype_files() {
        for bytes in [REGULAR, BOLD] {
            assert!(bytes.len() > 10_000);
            assert_eq!(bytes.get(..4), Some(&[0, 1, 0, 0][..]), "TrueType magic");
        }
    }
}
