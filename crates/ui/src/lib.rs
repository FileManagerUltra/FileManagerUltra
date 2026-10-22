//! The egui widget layer: design tokens ([`theme`]), and — from change 2 on — the file
//! tree, virtualized listing, pane chrome and dialogs. Widgets take tokens; no widget
//! ever hard-codes a colour.

pub mod theme;

pub use theme::{Tokens, apply_to};
