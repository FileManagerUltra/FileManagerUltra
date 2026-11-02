//! Read-only shortcut reference, including pane-local keys and binding aliases.

use egui::{Id, RichText, Ui};
use fm_engine::{CommandSpec, Session};

use crate::dialog_style::{hint, section_heading};
use crate::theme::Tokens;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Application,
    Workspaces,
    Panes,
    Navigation,
    Selection,
    Operations,
    Preview,
    LogsAndDiff,
    Terminal,
    TextAndDialogs,
}

impl Group {
    fn label(self) -> &'static str {
        match self {
            Self::Application => "Application and appearance",
            Self::Workspaces => "Workspace tabs",
            Self::Panes => "Pane layout",
            Self::Navigation => "Files and folder navigation",
            Self::Selection => "File selection and clipboard",
            Self::Operations => "File operations",
            Self::Preview => "Preview and thumbnails",
            Self::LogsAndDiff => "Logs and diffs",
            Self::Terminal => "Terminal pane",
            Self::TextAndDialogs => "Text fields and dialogs",
        }
    }
}

struct Row {
    group: Group,
    action: &'static str,
    keys: String,
    context: String,
}

fn group(spec: &CommandSpec) -> Group {
    match spec.id {
        "view.preview.toggle" => Group::Panes,
        "view.filter" | "view.hidden.toggle" | "bookmark.add" => Group::Navigation,
        "edit.undo" => Group::Operations,
        id if id.starts_with("workspace.") => Group::Workspaces,
        id if id.starts_with("pane.") => Group::Panes,
        id if id.starts_with("nav.") => Group::Navigation,
        id if id.starts_with("edit.") || id.starts_with("file.clip_") => Group::Selection,
        id if id.starts_with("file.") || id.starts_with("search.") => Group::Operations,
        id if id.starts_with("preview.log.") || id.starts_with("preview.diff.") => {
            Group::LogsAndDiff
        }
        id if id.starts_with("preview.") => Group::Preview,
        id if id.starts_with("term.") => Group::Terminal,
        _ => Group::Application,
    }
}

fn context(spec: &CommandSpec) -> &'static str {
    match group(spec) {
        Group::Application if spec.id.starts_with("view.zoom.") => {
            "Interface zoom; in Preview, scales the preview instead."
        }
        Group::Application => "Application shortcut.",
        Group::Workspaces => "Outside terminal input and text fields.",
        Group::Panes if spec.id == "pane.swap" => {
            "Outside text fields; use Ctrl+Shift+Tab to leave a terminal."
        }
        Group::Panes => "Pane layout; dialogs and text fields may own the keyboard.",
        Group::Navigation | Group::Selection => "Focused Files pane, outside text fields.",
        Group::Operations if spec.id == "file.mkdir" || spec.id == "file.newfile" => {
            "Files pane; in a diff, F7 and Shift+F7 navigate changes."
        }
        Group::Operations => "Files pane, outside dialogs and text fields.",
        Group::Preview if spec.id == "preview.find.open" => {
            "Focused Preview; logs open their existing Filter search."
        }
        Group::Preview if spec.id == "preview.scale.reset" => {
            "Ctrl+0 also targets workspace 10; use Ctrl+Shift+0 to reset Preview without that conflict."
        }
        Group::Preview if spec.id == "preview.find.close" => {
            "Close Preview Find or log filters; retain applied log filters."
        }
        Group::Preview => "Focused Preview; text fields keep their editing shortcuts.",
        Group::LogsAndDiff if spec.id.starts_with("preview.diff.") => "An active diff in Preview.",
        Group::LogsAndDiff => "Log preview.",
        Group::Terminal => "Focused terminal; other control keys belong to the shell.",
        Group::TextAndDialogs => "Focused field or dialog.",
    }
}

fn command_keys(spec: &CommandSpec) -> String {
    match spec.id {
        "file.delete" => "F8 / Delete / Ctrl+Backspace",
        "preview.scale.in" => "Ctrl+= / Ctrl++",
        "preview.find.next" => "F3 / Enter (in Find)",
        "preview.find.previous" => "Shift+F3 / Shift+Enter (in Find)",
        "view.zoom.in" => "Ctrl+Shift++ / Ctrl+Shift+=",
        _ => spec.shortcut.unwrap_or_default(),
    }
    .to_owned()
}

fn rows(s: &Session, mac: bool) -> Vec<Row> {
    let mut rows: Vec<Row> = s
        .registry
        .specs()
        .iter()
        .filter(|spec| spec.shortcut.is_some())
        .map(|spec| Row {
            group: group(spec),
            action: spec.label,
            keys: command_keys(spec),
            context: context(spec).into(),
        })
        .collect();
    for (group, action, keys, context) in [
        (
            Group::Workspaces,
            "Switch workspace",
            "Ctrl+1 through Ctrl+9 / Ctrl+0",
            "Tabs 1-9 and 10; outside terminals and text fields. Ctrl+0 overlaps Preview zoom reset.",
        ),
        (
            Group::Panes,
            "Switch pane from a terminal",
            "Ctrl+Shift+Tab",
            "Plain Tab stays with the shell.",
        ),
        (
            Group::Navigation,
            "Move file cursor",
            "Up / Down",
            "Focused file listing.",
        ),
        (
            Group::Navigation,
            "First or last row",
            "Home / End",
            "Focused file listing.",
        ),
        (
            Group::Navigation,
            "Move by one page",
            "PageUp / PageDown",
            "Focused file listing.",
        ),
        (
            Group::Navigation,
            "Open focused item",
            "Enter",
            "File listing or thumbnail grid.",
        ),
        (
            Group::Navigation,
            "Expand folder",
            "Right",
            "Focused folder tree.",
        ),
        (
            Group::Navigation,
            "Collapse folder or go to parent",
            "Left",
            "Folder tree; at the root, collapses all folders.",
        ),
        (
            Group::Navigation,
            "Open tree folder",
            "Enter",
            "Focused folder tree.",
        ),
        (
            Group::Navigation,
            "Move tree cursor",
            "Up / Down",
            "Focused folder tree.",
        ),
        (
            Group::Navigation,
            "Select path suggestion",
            "Up / Down",
            "Location editor.",
        ),
        (
            Group::Navigation,
            "Complete path",
            "Tab",
            "Location editor.",
        ),
        (
            Group::Navigation,
            "Navigate to typed path",
            "Enter",
            "Location editor.",
        ),
        (
            Group::Navigation,
            "Cancel path editing",
            "Escape",
            "Location editor.",
        ),
        (
            Group::Navigation,
            "Clear file filter",
            "Escape",
            "File listing or its quick-filter field.",
        ),
        (
            Group::Navigation,
            "Return from filter to listing",
            "Enter / Down",
            "Quick-filter field; keeps the filter.",
        ),
        (
            Group::Selection,
            "Extend selection",
            "Shift+Up / Shift+Down / Shift+Home / Shift+End / Shift+PageUp / Shift+PageDown",
            "File listing.",
        ),
        (
            Group::Preview,
            "Scroll preview",
            "Up / Down / Left / Right",
            "Preview body; horizontal arrows scroll horizontally.",
        ),
        (
            Group::Preview,
            "Page through preview",
            "PageUp / PageDown",
            "Preview body.",
        ),
        (
            Group::Preview,
            "Start or end of preview",
            "Home / End",
            "Preview body.",
        ),
        (
            Group::Preview,
            "Move thumbnail cursor",
            "Up / Down / Left / Right / Home / End / PageUp / PageDown",
            "Thumbnail grid.",
        ),
        (
            Group::Preview,
            "Extend thumbnail selection",
            "Shift+Arrow / Shift+Home / Shift+End / Shift+PageUp / Shift+PageDown",
            "Thumbnail grid.",
        ),
        (
            Group::Preview,
            "Select all thumbnails",
            "Ctrl+A",
            "Thumbnail grid.",
        ),
        (
            Group::Preview,
            "Clear preview text selection",
            "Escape",
            "Preview body when Find is closed.",
        ),
        (
            Group::Preview,
            "Zoom preview using interface zoom keys",
            "Ctrl+Shift++ / Ctrl+Shift+= / Ctrl+Shift+- / Ctrl+Shift+0",
            "Focused Preview; otherwise these change interface zoom.",
        ),
        (
            Group::Terminal,
            "Scroll terminal history",
            "Shift+PageUp / Shift+PageDown",
            "Terminal scrollback.",
        ),
        (
            Group::Terminal,
            "Shell and terminal-program keys",
            "Ctrl+key / Alt+key / Tab / Shift+Tab / Enter / Escape / Backspace / Insert / Delete / Arrow / Home / End / PageUp / PageDown / F1-F12",
            "Forwarded to the shell or running program; behavior depends on that program.",
        ),
        (
            Group::TextAndDialogs,
            "Select, copy, cut and paste text",
            "Ctrl+A / Ctrl+C / Ctrl+X / Ctrl+V",
            "Focused text field; terminal input follows shell rules instead.",
        ),
        (
            Group::TextAndDialogs,
            "Move between controls",
            "Tab / Shift+Tab",
            "Dialogs and text fields; outside these, Tab switches panes.",
        ),
        (
            Group::TextAndDialogs,
            "Confirm form or search",
            "Enter",
            "Where supported by the dialog; not automatic approval of every prompt.",
        ),
        (
            Group::TextAndDialogs,
            "Cancel or close",
            "Escape",
            "Dismiss a dialog or popup without approving its action.",
        ),
    ] {
        rows.push(Row {
            group,
            action,
            keys: keys.into(),
            context: context.into(),
        });
    }
    if mac {
        rows.push(Row {
            group: Group::Terminal,
            action: "Copy terminal selection",
            keys: "Cmd+C".into(),
            context: "macOS terminal clipboard shortcut; Control+C still goes to the shell.".into(),
        });
    }
    for row in &mut rows {
        if mac {
            row.keys = row.keys.replace("Ctrl+", "Cmd+");
            row.context = row.context.replace("Ctrl+", "Cmd+");
            if row.action == "Shell and terminal-program keys" {
                row.keys = row.keys.replace("Cmd+key", "Ctrl+key");
            }
        }
    }
    rows.sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.action.cmp(b.action)));
    rows
}

fn matches(row: &Row, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    [row.group.label(), row.action, &row.keys, &row.context]
        .iter()
        .any(|value| value.to_lowercase().contains(&query))
}

pub(super) fn show(ui: &mut Ui, s: &Session, tokens: &Tokens) {
    hint(
        ui,
        tokens,
        "Read-only reference. Shortcuts depend on the focused pane; text fields keep their editing keys.",
    );
    let id = Id::NULL.with("preferences-shortcut-search");
    let mut query = ui.data_mut(|d| d.get_temp::<String>(id).unwrap_or_default());
    ui.add(
        egui::TextEdit::singleline(&mut query)
            .hint_text("Search shortcuts")
            .desired_width(ui.available_width())
            .char_limit(128),
    )
    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Search shortcuts"));
    ui.data_mut(|d| d.insert_temp(id, query.clone()));
    ui.add_space(8.0);
    let rows: Vec<Row> = rows(s, cfg!(target_os = "macos"))
        .into_iter()
        .filter(|row| matches(row, &query))
        .collect();
    for group in rows.chunk_by(|a, b| a.group == b.group) {
        let Some(first) = group.first() else { continue };
        section_heading(ui, tokens, first.group.label());
        let column_width = (ui.available_width() - 16.0).max(0.0) * 0.5;
        egui::Grid::new(("shortcut-reference", first.group.label()))
            .num_columns(2)
            .spacing(egui::vec2(16.0, 12.0))
            .min_col_width(column_width)
            .max_col_width(column_width)
            .show(ui, |ui| {
                for row in group {
                    ui.vertical(|ui| {
                        ui.label(row.action);
                        hint(ui, tokens, &row.context);
                    });
                    ui.label(RichText::new(&row.keys).monospace());
                    ui.end_row();
                }
            });
        ui.add_space(12.0);
    }
    if rows.is_empty() {
        hint(ui, tokens, "No shortcuts match your search.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_includes_every_registered_shortcut_even_when_disabled() {
        let s = Session::new().expect("session");
        let rows = rows(&s, false);
        for spec in s
            .registry
            .specs()
            .iter()
            .filter(|spec| spec.shortcut.is_some())
        {
            assert!(
                rows.iter().any(|row| row.action == spec.label
                    && row.keys.contains(spec.shortcut.expect("shortcut"))),
                "{}",
                spec.id
            );
        }
        assert!(rows.iter().any(|row| row.keys.contains("Ctrl+Shift+Tab")));
        assert!(
            rows.iter()
                .any(|row| row.keys.contains("Ctrl+1 through Ctrl+9"))
        );
        assert!(
            rows.iter()
                .any(|row| row.keys.contains("Delete / Ctrl+Backspace"))
        );
        assert!(rows.iter().any(|row| row.group == Group::Terminal));
    }

    #[test]
    fn reference_uses_platform_modifiers_and_searches_actions_keys_and_context() {
        let s = Session::new().expect("session");
        let mac = rows(&s, true);
        assert!(mac.iter().any(|row| row.keys == "Cmd+Q"));
        assert!(
            mac.iter()
                .any(|row| row.keys == "Cmd+C" && row.group == Group::Terminal)
        );
        assert!(mac.iter().any(|row| row.keys.starts_with("Ctrl+key")));
        let rows = rows(&s, false);
        for query in ["  PASTE  ", "ctrl+q", "scrollback", "workspace tabs"] {
            assert!(rows.iter().any(|row| matches(row, query)), "{query}");
        }
        assert!(!rows.iter().any(|row| matches(row, "not-a-shortcut")));
    }
}
