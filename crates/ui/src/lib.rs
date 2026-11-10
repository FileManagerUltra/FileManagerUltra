//! The egui widget layer: design tokens ([`theme`]), the file tree
//! ([`tree`]), virtualized listing ([`listing`]) and pane chrome ([`pane`]).
//! Widgets take tokens; no widget ever hard-codes a colour.

pub mod arrange;
pub mod compare_with;
pub mod context_menu;
pub mod dialog_style;
pub mod dialogs;
pub mod diff;
pub mod dnd;
pub mod git_marks;
pub mod jobs;
pub mod listing;
pub mod locationbar;
mod markdown_fonts;
pub mod media;
pub mod media_audio;
pub mod media_keys;
pub mod open_with;
pub mod pane;
pub mod places;
pub mod preferences;
pub mod preview;
mod preview_filters;
pub mod properties;
pub mod remote;
pub mod search;
pub mod sftp_popup;
pub mod terminal;
pub mod theme;
pub mod trash;
pub mod tree;
pub mod updates;
pub mod workspaces;

pub use theme::{Tokens, apply_to};
