//! The Properties window: what the selection is, how big it is (counted live on a background
//! walk) and, for one item on a backend that supports it, its permissions. Applying
//! permissions dispatches `file.chmod`; closing dispatches `file.properties.close`.

use egui::{Context, Id, RichText};
use fm_engine::Session;
use fm_engine::item_cmds::display_path;
use fm_vfs::EntryKind;
use serde_json::json;

use crate::listing::{format_mtime, human_size};
use crate::theme::Tokens;

/// The rwx grid in display order: (label, owner/group/other bit triples).
const CLASSES: [(&str, u32); 3] = [("Owner", 6), ("Group", 3), ("Others", 0)];
const SPECIAL: [(&str, u32, &str); 3] = [
    ("setuid", 0o4000, "Run as the file's owner"),
    (
        "setgid",
        0o2000,
        "Run as the file's group; new files inherit a folder's group",
    ),
    (
        "sticky",
        0o1000,
        "In a folder, only owners may delete their files",
    ),
];

/// Permission bits as `rwxr-xr-x`.
pub fn mode_string(mode: u32) -> String {
    let mut out = String::with_capacity(9);
    for (_, shift) in CLASSES {
        for (bit, ch) in [(4, 'r'), (2, 'w'), (1, 'x')] {
            out.push(if mode >> shift & bit != 0 { ch } else { '-' });
        }
    }
    out
}

fn edit_id() -> Id {
    Id::NULL.with("fm-properties-mode")
}

/// Render the window while `s.properties` is open. Call once per frame.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(props) = s.properties.as_ref() else {
        ctx.data_mut(|d| d.remove_temp::<(String, u32)>(edit_id()));
        return;
    };
    let items = props.items.clone();
    let info = props.info();
    let entry = s.properties_entry();
    let key = items
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let current = entry
        .as_ref()
        .and_then(|e| e.permissions)
        .map(|m| m & 0o7777);
    let mut edited = ctx
        .data_mut(|d| d.get_temp::<(String, u32)>(edit_id()))
        .filter(|(k, _)| *k == key)
        .map(|(_, m)| m)
        .or(current);
    if !info.done {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    let mut open = true;
    let mut close = false;
    let mut apply = None;
    egui::Window::new("Properties")
        .id(Id::NULL.with("fm-properties"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(crate::dialogs::frame(tokens))
        .show(ctx, |ui| {
            egui::Grid::new("fm-properties-grid")
                .num_columns(2)
                .spacing([tokens.padding * 3.0, tokens.padding])
                .show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, label: &str, value: String| {
                        ui.label(tokens.muted(label));
                        ui.add(egui::Label::new(RichText::new(value).color(tokens.text)).wrap());
                        ui.end_row();
                    };
                    match items.as_slice() {
                        [one] => {
                            row(ui, "Name", one.file_name().unwrap_or("/").to_string());
                            let parent = one.parent().map(|p| display_path(&p)).unwrap_or_default();
                            row(ui, "Location", parent);
                            if let Some(e) = &entry {
                                let kind = match e.kind {
                                    EntryKind::Dir => "Folder",
                                    EntryKind::File => "File",
                                    EntryKind::Symlink { .. } => "Symbolic link",
                                    _ => "Other",
                                };
                                row(ui, "Kind", kind.to_string());
                                if let Some(m) = e.modified {
                                    row(ui, "Modified", format_mtime(m));
                                }
                            }
                        }
                        many => {
                            row(ui, "Items", format!("{} selected", many.len()));
                            if let Some(parent) = many.first().and_then(|u| u.parent()) {
                                row(ui, "Location", display_path(&parent));
                            }
                        }
                    }
                    let more = if info.done { "" } else { "…" };
                    let mut size = format!("{}{more}", human_size(info.bytes));
                    if info.bytes >= 1024 {
                        size.push_str(&format!(" ({} bytes)", group_digits(info.bytes)));
                    }
                    row(ui, "Size", size);
                    let is_single_file = info.dirs == 0 && info.files == 1;
                    if !is_single_file {
                        row(
                            ui,
                            "Contains",
                            format!(
                                "{} files, {} folders{more}",
                                group_digits(info.files),
                                group_digits(info.dirs)
                            ),
                        );
                    }
                    if info.unreadable > 0 {
                        ui.label(tokens.muted("Unreadable"));
                        ui.label(
                            RichText::new(format!("{} folders could not be read", info.unreadable))
                                .color(tokens.warning),
                        );
                        ui.end_row();
                    }
                });
            if let Some(note) = &info.note {
                ui.label(RichText::new(note).color(tokens.warning));
            }
            if info.truncated {
                ui.label(RichText::new("Stopped counting: too many items").color(tokens.warning));
            }
            if let Some(mode) = edited.as_mut() {
                ui.separator();
                permissions(ui, tokens, mode);
                ui.horizontal(|ui| {
                    let changed = Some(*mode) != current;
                    if ui
                        .add_enabled(changed, egui::Button::new("Apply"))
                        .on_hover_text("Change the permissions")
                        .clicked()
                    {
                        apply = Some(*mode);
                    }
                    if ui
                        .add_enabled(changed, egui::Button::new("Revert"))
                        .on_hover_text("Undo the unapplied changes")
                        .clicked()
                    {
                        *mode = current.unwrap_or(*mode);
                    }
                });
            } else if items.len() == 1 {
                ui.label(tokens.muted("Permissions are not available here."));
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    ctx.data_mut(|d| match edited {
        Some(m) => {
            d.insert_temp(edit_id(), (key.clone(), m));
        }
        None => {
            d.remove_temp::<(String, u32)>(edit_id());
        }
    });
    if let Some(mode) = apply {
        let sources: Vec<String> = items.iter().map(|u| u.to_string()).collect();
        s.dispatch_or_notice(
            "file.chmod",
            &json!({"mode": format!("{mode:o}"), "sources": sources}),
        );
    }
    if close || !open {
        s.dispatch_or_notice("file.properties.close", &serde_json::Value::Null);
    }
}

/// The rwx checkbox grid and the special bits, editing `mode`.
fn permissions(ui: &mut egui::Ui, tokens: &Tokens, mode: &mut u32) {
    ui.label(tokens.muted(format!(
        "Permissions  {}  ({:04o})",
        mode_string(*mode),
        *mode
    )));
    egui::Grid::new("fm-properties-perms")
        .num_columns(4)
        .show(ui, |ui| {
            ui.label("");
            for h in ["Read", "Write", "Execute"] {
                ui.label(tokens.muted(h));
            }
            ui.end_row();
            for (label, shift) in CLASSES {
                ui.label(label);
                for bit in [4u32, 2, 1] {
                    let mask = bit << shift;
                    let mut on = *mode & mask != 0;
                    if ui.checkbox(&mut on, "").changed() {
                        *mode = if on { *mode | mask } else { *mode & !mask };
                    }
                }
                ui.end_row();
            }
        });
    ui.horizontal(|ui| {
        for (label, mask, tip) in SPECIAL {
            let mut on = *mode & mask != 0;
            if ui.checkbox(&mut on, label).on_hover_text(tip).changed() {
                *mode = if on { *mode | mask } else { *mode & !mask };
            }
        }
    });
}

/// `1234567` → `1,234,567`.
fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_render_like_ls() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o640), "rw-r-----");
        assert_eq!(mode_string(0), "---------");
    }

    #[test]
    fn digits_are_grouped_in_thousands() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1234567), "1,234,567");
    }
}
