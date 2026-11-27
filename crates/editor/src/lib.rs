//! Text editor core (capability `text-editor`): a rope buffer with bounds-checked,
//! line-addressed editing, undo/redo, encodings and find. No UI types, no I/O.

mod brackets;
mod buffer;
mod editing;
mod encoding;
mod find;
mod fold;
mod lang;
mod sql;
mod syntax;
mod undo;

pub use brackets::BracketSpan;
pub use buffer::{Buffer, EditError, EditRecord, Pos};
pub use editing::{
    IndentStyle, delete_lines, detect_indent, duplicate_lines, indent_lines, match_bracket,
    move_lines, newline_text, toggle_comment,
};
pub use encoding::{
    DecodeError, Decoder, Detected, EncodeError, Encoding, Eol, SNIFF_BYTES, convert_eol,
    detect_eol, encode_into, sniff,
};
pub use find::*;
pub use fold::{FoldIndex, FoldRanges, indent_range, tree_ranges as fold_tree_ranges};
pub use lang::{Lang, detect as detect_lang};
pub use syntax::{
    DEFAULT_MAX_BYTES as SYNTAX_MAX_BYTES, FULL_PARSE_BUDGET, INCREMENTAL_BUDGET, Kind, ParseCtl,
    ParseJob, Syntax, SyntaxError, TokenSpan,
};
pub use tree_sitter::Tree as SyntaxTree;
pub use undo::{
    Coalesce, DEFAULT_MAX_STEPS, MAX_BYTES as UNDO_MAX_BYTES, MAX_STEPS, MIN_STEPS, UndoLog,
};
