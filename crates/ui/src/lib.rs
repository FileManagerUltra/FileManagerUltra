//! The egui widget layer: design tokens ([`theme`]), the file tree
//! ([`tree`]), virtualized listing ([`listing`]) and pane chrome ([`pane`]).
//! Widgets take tokens; no widget ever hard-codes a colour.

pub mod arrange;
pub mod context_menu;
pub mod dialogs;
pub mod dnd;
pub mod jobs;
pub mod listing;
pub mod locationbar;
pub mod pane;
pub mod preferences;
pub mod preview;
pub mod remote;
pub mod sftp_popup;
pub mod terminal;
pub mod theme;
pub mod trash;
pub mod tree;
pub mod workspaces;

pub use theme::{Tokens, apply_to};
