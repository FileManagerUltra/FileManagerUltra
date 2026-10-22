//! Virtualized directory listing: one pane's sorted view over the shared
//! [`fm_engine::DirCache`], rendered with `ScrollArea::show_rows`. Sort,
//! selection and scroll live in [`fm_layout::ListingState`]; opening an
//! entry is returned for the caller to dispatch `nav.open`.

use std::time::SystemTime;

use egui::{Key, Modifiers, Ui};
use fm_engine::{DirState, Session};
use fm_layout::{PaneId, SortDir, SortKey, SortSpec};
use fm_vfs::{Entry, EntryKind, VfsUri};

use crate::theme::Tokens;

/// Keyboard/selection outcomes on the entry rows (pure, unit-testable).
#[derive(Debug, PartialEq)]
pub(crate) enum KeyOutcome {
    None,
    Move(isize, bool),
    Absolute(usize, bool),
    SelectAll,
    Open,
}

pub(crate) fn key_outcome(key: Key, modifiers: Modifiers) -> KeyOutcome {
    match key {
        Key::ArrowUp => KeyOutcome::Move(-1, modifiers.shift),
        Key::ArrowDown => KeyOutcome::Move(1, modifiers.shift),
        Key::Home => KeyOutcome::Absolute(0, modifiers.shift),
        Key::End => KeyOutcome::Absolute(usize::MAX, modifiers.shift),
        Key::Enter => KeyOutcome::Open,
        Key::A if modifiers.command => KeyOutcome::SelectAll,
        _ => KeyOutcome::None,
    }
}

/// Render the listing for one pane. `pane_focused` gates keyboard handling.
/// Returns the uri to open (`nav.open`), if any.
pub fn show(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    pane_focused: bool,
) -> Option<VfsUri> {
    let location = s.layout.state(pane).map(|p| p.location.clone())?;
    let parent = location.parent();
    let dir_state = s.dirs.state(&location);

    let mut open: Option<VfsUri> = None;
    if let Some(state) = s.layout.state(pane) {
        let clicked = header(ui, &state.listing.sort, tokens);
        if let Some(key) = clicked
            && let Some(state) = s.layout.state_mut(pane)
        {
            state.listing.sort.header_click(key);
            s.invalidate_view(pane);
        }
    }

    match dir_state {
        DirState::Absent | DirState::Loading { .. } => {
            ui.label(tokens.muted("Loading…"));
        }
        DirState::Failed { message, .. } => {
            ui.colored_label(tokens.error, message);
            ui.label(tokens.muted("Ctrl+R to retry"));
        }
        DirState::Ready { .. } => {
            open = ready_rows(ui, s, pane, tokens, pane_focused, parent.as_ref());
        }
    }
    open
}

fn header(ui: &mut Ui, sort: &SortSpec, tokens: &Tokens) -> Option<SortKey> {
    let mut clicked = None;
    egui::Frame::NONE
        .fill(tokens.panel)
        .inner_margin(egui::Margin::symmetric(
            tokens.padding as i8,
            (tokens.padding * 0.5) as i8,
        ))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (key, label) in [
                    (SortKey::Name, "Name"),
                    (SortKey::Size, "Size"),
                    (SortKey::Mtime, "Modified"),
                    (SortKey::Type, "Type"),
                ] {
                    let arrow = if sort.key == key {
                        match sort.dir {
                            SortDir::Asc => " ^",
                            SortDir::Desc => " v",
                        }
                    } else {
                        ""
                    };
                    if ui
                        .selectable_label(sort.key == key, format!("{label}{arrow}"))
                        .clicked()
                    {
                        clicked = Some(key);
                    }
                }
            });
        });
    clicked
}

fn ready_rows(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    pane_focused: bool,
    parent: Option<&VfsUri>,
) -> Option<VfsUri> {
    let (entries, ix) = s.pane_view(pane)?;
    let mut open: Option<VfsUri> = None;
    let has_parent = parent.is_some();
    let total = ix.order.len() + usize::from(has_parent);
    if total == 0 {
        ui.label(tokens.muted("(empty)"));
        return None;
    }
    let names: Vec<String> = ix
        .order
        .iter()
        .filter_map(|i| entries.get(*i as usize).map(|e| e.name.clone()))
        .collect();

    // Keyboard once per frame, before painting (so the scroll offset can be
    // applied on the same frame without fighting the wheel).
    let mut scroll_to: Option<f32> = None;
    let mut open_key = false;
    if pane_focused {
        let outcomes: Vec<KeyOutcome> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|ev| match ev {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some(key_outcome(*key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        if !outcomes.is_empty()
            && let Some(state) = s.layout.state_mut(pane)
        {
            let listing = &mut state.listing;
            for outcome in outcomes {
                match outcome {
                    KeyOutcome::None => {}
                    KeyOutcome::Move(delta, extend) => listing.move_focus(&names, delta, extend),
                    KeyOutcome::Absolute(idx, extend) => {
                        let idx = if idx == usize::MAX {
                            names.len().saturating_sub(1)
                        } else {
                            idx
                        };
                        listing.focus_absolute(&names, idx, extend);
                    }
                    KeyOutcome::SelectAll => listing.select_all(&names),
                    KeyOutcome::Open => open_key = true,
                }
            }
            let row = listing.focused_row.min(names.len().saturating_sub(1));
            scroll_to = Some(row as f32 * tokens.row_height);
        }
    }
    if open_key {
        let focus = s.layout.state(pane).map(|p| p.listing.focused_row);
        let name = focus.and_then(|row| names.get(row).cloned());
        if let Some(name) = name
            && let Some(entry) = entries.iter().find(|e| e.name == name && e.is_dir())
        {
            open = Some(entry.uri.clone());
        }
    }

    let row_height = tokens.row_height;
    let builder = egui::ScrollArea::vertical()
        .id_salt(format!("fm-list-{pane}"))
        .auto_shrink([false, false]);
    let builder = match scroll_to {
        Some(offset) => builder.vertical_scroll_offset(offset),
        None => builder,
    };
    builder.show_rows(ui, row_height, total, |ui, row_range| {
        for row in row_range {
            if has_parent && row == 0 {
                parent_row(ui, tokens, parent, &mut open);
                continue;
            }
            let entry_row = if has_parent { row - 1 } else { row };
            let Some(idx) = ix.order.get(entry_row).copied() else {
                continue;
            };
            let Some(entry) = entries.get(idx as usize) else {
                continue;
            };
            if let Some(target) = entry_row_ui(ui, s, pane, tokens, pane_focused, entry, entry_row)
            {
                open = Some(target);
            }
        }
    });
    open
}

fn parent_row(ui: &mut Ui, tokens: &Tokens, parent: Option<&VfsUri>, open: &mut Option<VfsUri>) {
    let available = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available, tokens.row_height),
        egui::Sense::click(),
    );
    let activated = response.clicked() || response.double_clicked();
    if activated && let Some(parent) = parent {
        *open = Some((*parent).clone());
    }
    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, tokens.hover_bg);
    }
    ui.painter().text(
        rect.left_center() + egui::vec2(tokens.padding, 0.0),
        egui::Align2::LEFT_CENTER,
        "..",
        tokens.font(),
        tokens.text,
    );
}

fn entry_row_ui(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    pane_focused: bool,
    entry: &Entry,
    row: usize,
) -> Option<VfsUri> {
    let state = s.layout.state(pane)?;
    let selected = state.listing.is_selected(&entry.name);
    let focused = state.listing.focused_row == row;

    let available = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available, tokens.row_height),
        egui::Sense::click(),
    );

    if response.clicked() {
        let modifiers = ui.input(|i| i.modifiers);
        let names = display_names(s, pane);
        if let Some(state) = s.layout.state_mut(pane) {
            if modifiers.shift {
                state.listing.extend_to(&names, row);
            } else if modifiers.command {
                state.listing.toggle(&entry.name, row);
            } else {
                state.listing.select_only(Some(&entry.name), row);
            }
        }
    }
    let open = if response.double_clicked() && entry.is_dir() {
        Some(entry.uri.clone())
    } else {
        None
    };

    let bg = if selected || (focused && pane_focused) {
        Some(tokens.selection_bg)
    } else if response.hovered() {
        Some(tokens.hover_bg)
    } else {
        None
    };
    if let Some(bg) = bg {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    let text_color = if selected {
        tokens.selection_text
    } else {
        tokens.text
    };
    let name = ellipsize_name(&entry.name, rect.width(), tokens);
    let type_label = type_label(&entry.kind);
    let size = entry.size.map(human_size).unwrap_or_default();
    let mtime = entry.modified.map(format_mtime).unwrap_or_default();
    ui.painter().text(
        rect.left_center() + egui::vec2(tokens.padding, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        tokens.font(),
        text_color,
    );
    let detail = format!("{type_label} · {mtime}");
    ui.painter().text(
        rect.right_center() + egui::vec2(-tokens.padding, 0.0),
        egui::Align2::RIGHT_CENTER,
        detail,
        tokens.font(),
        tokens.text_muted,
    );
    ui.painter().text(
        rect.right_center() + egui::vec2(-tokens.padding - 100.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        size,
        tokens.font(),
        tokens.text_muted,
    );
    response.clone().on_hover_text(entry.uri.path_str());
    open
}

fn display_names(s: &mut Session, pane: PaneId) -> Vec<String> {
    let Some((entries, ix)) = s.pane_view(pane) else {
        return Vec::new();
    };
    ix.order
        .iter()
        .filter_map(|i| entries.get(*i as usize).map(|e| e.name.clone()))
        .collect()
}

fn ellipsize_name(name: &str, width: f32, tokens: &Tokens) -> String {
    let max_chars = ((width / (tokens.font_size * 0.55)).max(8.0)) as usize;
    let count = name.chars().count();
    if count <= max_chars {
        return name.to_string();
    }
    let mut out: String = name.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub(crate) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(crate) fn type_label(kind: &EntryKind) -> &'static str {
    match kind {
        EntryKind::Dir => "Dir",
        EntryKind::File => "File",
        EntryKind::Symlink { .. } => "Link",
        EntryKind::Other => "Other",
    }
}

fn format_mtime(t: SystemTime) -> String {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => {
            let (y, mo, day, h, mi) = civil_from_unix(d.as_secs() as i64);
            format!("{y:04}-{mo:02}-{day:02} {h:02}:{mi:02}")
        }
        Err(_) => "—".to_string(),
    }
}

/// Days-since-epoch → civil date (Howard Hinnant's algorithm, no deps).
fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let h = rem / 3600;
    let mi = (rem % 3600) / 60;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32, h as u32, mi as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_layout::ListingState;
    use fm_testkit::MemFs;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[allow(clippy::expect_used)]
    fn ready_session() -> (Session, VfsUri) {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/d/sub").expect("mkdirs");
        mem.put_str("/d/big.bin", &[0u8; 2048]).expect("put");
        mem.put_str("/d/small.txt", b"s").expect("put");
        s.backends.register(Arc::new(mem));
        let uri = VfsUri::parse("/d").expect("uri");
        s.layout.state_mut(1).expect("pane").location = uri.clone();
        s.submit_read(uri.clone()).expect("submit");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            if s.dirs.state(&uri).is_ready() {
                break;
            }
            assert!(Instant::now() < deadline, "read never ready");
            std::thread::sleep(Duration::from_millis(5));
        }
        (s, uri)
    }

    /// Headless render: must not panic in either palette.
    #[allow(clippy::expect_used)]
    fn render_frame(tokens: &Tokens, mut f: impl FnMut(&mut Ui)) {
        let ctx = egui::Context::default();
        crate::theme::apply_to(&ctx, tokens);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| f(ui));
        });
        // No renderer in tests: drop the font-atlas delta intentionally.
        output.textures_delta.clear();
    }

    #[test]
    fn key_outcome_mapping() {
        assert_eq!(
            key_outcome(Key::ArrowDown, Modifiers::NONE),
            KeyOutcome::Move(1, false)
        );
        assert_eq!(
            key_outcome(Key::ArrowUp, Modifiers::SHIFT),
            KeyOutcome::Move(-1, true)
        );
        assert_eq!(
            key_outcome(Key::End, Modifiers::NONE),
            KeyOutcome::Absolute(usize::MAX, false)
        );
        assert_eq!(
            key_outcome(Key::A, Modifiers::COMMAND),
            KeyOutcome::SelectAll
        );
        assert_eq!(key_outcome(Key::Enter, Modifiers::NONE), KeyOutcome::Open);
        assert_eq!(key_outcome(Key::Escape, Modifiers::NONE), KeyOutcome::None);
    }

    #[test]
    fn move_focus_via_outcomes() {
        let order: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let mut listing = ListingState::default();
        listing.select_only(Some("a"), 0);
        if let KeyOutcome::Move(d, e) = key_outcome(Key::ArrowDown, Modifiers::NONE) {
            listing.move_focus(&order, d, e);
        }
        assert_eq!(listing.focused_row, 1);
        assert!(listing.is_selected("b"));
        if let KeyOutcome::SelectAll = key_outcome(Key::A, Modifiers::COMMAND) {
            listing.select_all(&order);
        }
        assert_eq!(listing.selected_count(), 3);
        if let KeyOutcome::Absolute(row, e) = key_outcome(Key::End, Modifiers::SHIFT) {
            let idx = if row == usize::MAX {
                order.len() - 1
            } else {
                row
            };
            listing.focus_absolute(&order, idx, e);
        }
        assert_eq!(listing.focused_row, 2);
        // Shift+End extends from the anchor (row 1) → rows 1..=2.
        assert_eq!(listing.selected_count(), 2);
        assert!(listing.is_selected("c") && !listing.is_selected("a"));
    }

    #[test]
    fn human_size_and_type_labels() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(2048), "2.0 KiB");
        assert_eq!(type_label(&EntryKind::Dir), "Dir");
        assert_eq!(
            type_label(&EntryKind::Symlink { target: "x".into() }),
            "Link"
        );
    }

    #[test]
    fn mtime_formatting_is_stable() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400);
        assert_eq!(format_mtime(t), "1970-01-02 00:00");
        let future = SystemTime::UNIX_EPOCH + Duration::from_secs(2_145_916_800);
        assert!(format_mtime(future).starts_with("2038"));
        let before = SystemTime::UNIX_EPOCH - Duration::from_secs(86_400);
        assert_eq!(format_mtime(before), "—");
    }

    #[test]
    fn listing_renders_headless_in_both_palettes() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let (mut s, _uri) = ready_session();
            render_frame(&tokens, |ui| {
                let open = show(ui, &mut s, 1, &tokens, true);
                assert!(open.is_none(), "no interaction → no open");
            });
            let mut fresh = Session::default();
            render_frame(&tokens, |ui| {
                let _ = show(ui, &mut fresh, 1, &tokens, false);
            });
        }
    }
}
