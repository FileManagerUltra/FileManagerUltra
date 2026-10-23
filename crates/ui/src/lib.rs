//! The egui widget layer: design tokens ([`theme`]), the file tree
//! ([`tree`]), virtualized listing ([`listing`]) and pane chrome ([`pane`]).
//! Widgets take tokens; no widget ever hard-codes a colour.

pub mod arrange;
pub mod dialogs;
pub mod dnd;
pub mod jobs;
pub mod listing;
pub mod locationbar;
pub mod pane;
pub mod preferences;
pub mod preview;
pub mod terminal;
pub mod theme;
pub mod tree;

pub use theme::{Tokens, apply_to};
