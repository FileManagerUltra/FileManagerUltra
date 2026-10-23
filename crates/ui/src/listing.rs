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
        let action = header(ui, pane, &state.listing.sort, state.col_widths, tokens);
        match action {
            Some(HeaderAction::Sort(key)) => {
                if let Some(state) = s.layout.state_mut(pane) {
                    state.listing.sort.header_click(key);
                    s.invalidate_view(pane);
                }
            }
            Some(HeaderAction::Resize(ix, w)) => {
                if let Some(state) = s.layout.state_mut(pane) {
                    state.set_col_width(ix, w);
                }
            }
            Some(HeaderAction::Reset(ix)) => {
                if let Some(state) = s.layout.state_mut(pane) {
                    state.reset_col_width(ix);
                }
            }
            None => {}
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

/// Minimum Name-column width before the detail columns collapse entirely.
const MIN_NAME: f32 = 80.0;
/// Grab width of a column border in the header.
const GRIP_W: f32 = 6.0;

/// Detail-column widths per layout, ordered by how much room they need:
/// `(size, mtime, type)` — a zero width drops the column. Narrow panes degrade
/// by dropping Type first, then Size, so columns never overlap.
fn layouts([size, mtime, ty]: [f32; 3]) -> [[f32; 3]; 5] {
    [
        [size, mtime, ty],
        [size, mtime, 0.0],
        [0.0, mtime, 0.0],
        [size, 0.0, 0.0],
        [0.0, 0.0, 0.0],
    ]
}

/// Per-column rects for one full-width listing row. Shared by the header and
/// the entry rows so clicks, labels and separators all agree on where each
/// column starts and ends.
struct Columns {
    name: egui::Rect,
    size: egui::Rect,
    mtime: egui::Rect,
    ty: egui::Rect,
}

/// Space before every name reserved for the row icon (link arrow).
const ICON_GUTTER: f32 = 16.0;

/// A small bent arrow drawn with strokes, so it never depends on font glyphs.
fn paint_link_icon(painter: &egui::Painter, origin: egui::Pos2, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.0, color.gamma_multiply(0.6));
    let (x0, y0) = (origin.x + 1.0, origin.y);
    let corner = egui::pos2(x0 + 1.0, y0 - 2.5);
    let tip = egui::pos2(x0 + 8.0, y0 - 2.5);
    painter.line_segment([egui::pos2(x0 + 1.0, y0 + 3.0), corner], stroke);
    painter.line_segment([corner, tip], stroke);
    painter.line_segment([tip, egui::pos2(x0 + 5.5, y0 - 5.0)], stroke);
    painter.line_segment([tip, egui::pos2(x0 + 5.5, y0)], stroke);
}

/// Lay out the columns for `rect`, picking the widest layout that fits beside a
/// usable Name column; dropped columns get zero width (and are not drawn).
fn columns(rect: egui::Rect, widths: [f32; 3]) -> Columns {
    let [size_w, mtime_w, type_w] = layouts(widths)
        .into_iter()
        .find(|[s, m, t]| rect.width() >= MIN_NAME + s + m + t)
        .unwrap_or([0.0, 0.0, 0.0]);
    let split = rect.right() - (size_w + mtime_w + type_w);
    let mut right = rect.right();
    let mut take = |width: f32| {
        let col = egui::Rect::from_min_max(
            egui::pos2(right - width, rect.top()),
            egui::pos2(right, rect.bottom()),
        );
        right -= width;
        col
    };
    let ty = take(type_w);
    let mtime = take(mtime_w);
    let size = take(size_w);
    Columns {
        name: egui::Rect::from_min_max(rect.min, egui::pos2(split, rect.bottom())),
        size,
        mtime,
        ty,
    }
}

/// Vertical hairlines between columns (skipped when the detail columns have
/// collapsed, or when a boundary falls on the row edge).
fn column_separators(painter: &egui::Painter, cols: &Columns, rect: egui::Rect, tokens: &Tokens) {
    for x in [cols.size.left(), cols.mtime.left(), cols.ty.left()] {
        if x > rect.left() + 1.0 && x < rect.right() - 1.0 {
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(tokens.border_width, tokens.border),
            );
        }
    }
}

/// Sort direction triangle, same style as the tree's disclosure triangles
/// (up = ascending, down = descending).
fn paint_sort_arrow(ui: &Ui, c: egui::Pos2, size: f32, dir: SortDir, tokens: &Tokens) {
    let pts = match dir {
        SortDir::Asc => vec![
            c + egui::vec2(-size, size * 0.5),
            c + egui::vec2(size, size * 0.5),
            c + egui::vec2(0.0, -size * 0.7),
        ],
        SortDir::Desc => vec![
            c + egui::vec2(-size, -size * 0.5),
            c + egui::vec2(size, -size * 0.5),
            c + egui::vec2(0.0, size * 0.7),
        ],
    };
    ui.painter().add(egui::Shape::convex_polygon(
        pts,
        tokens.text_muted,
        egui::Stroke::NONE,
    ));
}

/// What a header interaction asks for.
enum HeaderAction {
    Sort(SortKey),
    /// Column index (0 Size, 1 Modified, 2 Type) and its new width.
    Resize(usize, f32),
    Reset(usize),
}

fn header(
    ui: &mut Ui,
    pane: PaneId,
    sort: &SortSpec,
    widths: [f32; 3],
    tokens: &Tokens,
) -> Option<HeaderAction> {
    // Paint fixed-width columns into one full-width row. An auto-sizing
    // `ui.horizontal` of labels would exceed the pane's allocated width and
    // expand the whole split past the window.
    let available = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available, tokens.row_height),
        egui::Sense::click(),
    );
    ui.painter().rect_filled(rect, 0.0, tokens.panel);
    let cols = columns(rect, widths);

    let cells = [
        (SortKey::Name, cols.name, "Name"),
        (SortKey::Size, cols.size, "Size"),
        (SortKey::Mtime, cols.mtime, "Modified"),
        (SortKey::Type, cols.ty, "Type"),
    ];
    for &(key, cell, label) in &cells {
        if cell.width() < 1.0 {
            continue;
        }
        let sorted = sort.key == key;
        let size = tokens.font_size * 0.35;
        let arrow_space = if sorted {
            size * 2.0 + tokens.padding
        } else {
            0.0
        };
        let galley =
            ui.painter()
                .layout_no_wrap(label.to_string(), tokens.font(), tokens.text_muted);
        let y = cell.center().y;
        let (text_pos, arrow_x) = if key == SortKey::Name {
            let x = cell.left() + tokens.padding;
            (
                egui::pos2(x, y),
                x + galley.size().x + tokens.padding + size,
            )
        } else {
            let right = cell.right() - tokens.padding;
            (egui::pos2(right - arrow_space, y), right - size)
        };
        let align = if key == SortKey::Name {
            egui::Align2::LEFT_CENTER
        } else {
            egui::Align2::RIGHT_CENTER
        };
        ui.painter()
            .text(text_pos, align, label, tokens.font(), tokens.text_muted);
        if sorted {
            paint_sort_arrow(ui, egui::pos2(arrow_x, y), size, sort.dir, tokens);
        }
    }
    column_separators(ui.painter(), &cols, rect, tokens);

    // Registered after the row's click sense, so a border wins hit-testing over a sort click.
    let mut action = None;
    for (ix, cell) in [cols.size, cols.mtime, cols.ty].into_iter().enumerate() {
        let x = cell.left();
        if cell.width() < 1.0 || x <= rect.left() + 1.0 {
            continue;
        }
        let grip = egui::Rect::from_center_size(
            egui::pos2(x, rect.center().y),
            egui::vec2(GRIP_W, rect.height()),
        );
        let resp = ui.interact(
            grip,
            egui::Id::NULL.with(("fm-col-grip", pane, ix)),
            egui::Sense::click_and_drag(),
        );
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            ui.painter().line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(tokens.border_width.max(2.0), tokens.accent),
            );
        }
        if resp.double_clicked() {
            action = Some(HeaderAction::Reset(ix));
        } else if resp.dragged() {
            // The columns are right-anchored: dragging a border right shrinks its column.
            action = Some(HeaderAction::Resize(ix, cell.width() - resp.drag_delta().x));
        }
    }
    if action.is_some() {
        return action;
    }

    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        for &(key, cell, _) in &cells {
            if cell.width() >= 1.0 && cell.contains(pos) {
                return Some(HeaderAction::Sort(key));
            }
        }
    }
    None
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
    let tree_active = s.layout.state(pane).is_some_and(|p| p.tree.active);
    if pane_focused && !tree_active && !ui.ctx().text_edit_focused() {
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
            let mut moved = false;
            for outcome in outcomes {
                moved |= matches!(
                    outcome,
                    KeyOutcome::Move(..) | KeyOutcome::Absolute(..) | KeyOutcome::SelectAll
                );
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
            // Keys that do not move focus (bare Shift/Ctrl, letters) must not re-scroll.
            if moved {
                let row = listing.focused_row.min(names.len().saturating_sub(1));
                scroll_to = Some(row as f32 * tokens.row_height);
            }
        }
    }
    if open_key {
        let focus = s.layout.state(pane).map(|p| p.listing.focused_row);
        let name = focus.and_then(|row| names.get(row).cloned());
        if let Some(name) = name
            && let Some(entry) = entries.iter().find(|e| e.name == name && e.is_dir_like())
        {
            open = Some(entry.uri.clone());
        }
    }

    let row_height = tokens.row_height;
    let saved_scroll = s
        .layout
        .state(pane)
        .map(|p| p.listing.scroll)
        .unwrap_or(0.0);
    let builder = egui::ScrollArea::vertical()
        .id_salt(format!("fm-list-{pane}"))
        .auto_shrink([false, false]);
    // The pane state's offset is the source of truth: no ScrollArea is
    // rendered while a refresh shows "Loading…", so the stored position must
    // not depend on egui's state surviving that gap on its own.
    let builder = match scroll_to {
        Some(offset) => builder.vertical_scroll_offset(offset),
        None => builder.vertical_scroll_offset(saved_scroll),
    };
    let output = builder.show_rows(ui, row_height, total, |ui, row_range| {
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
    if let Some(state) = s.layout.state_mut(pane) {
        state.listing.scroll = output.state.clamped_offset().y;
    }
    open
}

fn parent_row(ui: &mut Ui, tokens: &Tokens, parent: Option<&VfsUri>, open: &mut Option<VfsUri>) {
    let available = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available, tokens.row_height),
        egui::Sense::click(),
    );
    if let Some(parent) = parent {
        crate::dnd::register_zone(ui.ctx(), rect, parent.clone(), crate::dnd::ZONE_ROW);
    }
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
    let widths = state.col_widths;

    let available = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available, tokens.row_height),
        egui::Sense::click_and_drag(),
    );
    if response.drag_started() {
        let payload = crate::dnd::payload_for(s, pane, &entry.name, &entry.uri);
        response.dnd_set_drag_payload(payload);
    }
    if entry.is_dir() {
        crate::dnd::register_zone(ui.ctx(), rect, entry.uri.clone(), crate::dnd::ZONE_ROW);
    }

    if response.clicked() {
        let modifiers = ui.input(|i| i.modifiers);
        let names = display_names(s, pane);
        if let Some(state) = s.layout.state_mut(pane) {
            state.tree.active = false;
            if modifiers.shift {
                state.listing.extend_to(&names, row);
            } else if modifiers.command {
                state.listing.toggle(&entry.name, row);
            } else {
                state.listing.select_only(Some(&entry.name), row);
            }
        }
    }
    let open = if response.double_clicked() && entry.is_dir_like() {
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
    // Norton-style cues: directories get the accent colour and a trailing slash, links the
    // muted colour and an `@`; plain files keep the primary text colour.
    let text_color = if selected {
        tokens.selection_text
    } else if entry.is_dir_like() {
        tokens.accent
    } else if entry.is_symlink() {
        tokens.text_muted
    } else {
        tokens.text
    };
    let display_name = if let EntryKind::Symlink { target } = &entry.kind {
        let slash = if entry.link_to_dir { "/" } else { "" };
        if s.show_link_targets {
            format!("{}{slash} -> {target}", entry.name)
        } else {
            format!("{}{slash}", entry.name)
        }
    } else if entry.is_dir() {
        format!("{}/", entry.name)
    } else {
        entry.name.clone()
    };
    let cols = columns(rect, widths);
    if cols.name.width() >= 1.0 {
        let name = ellipsize_name(
            &display_name,
            cols.name.width() - 2.0 * tokens.padding - ICON_GUTTER,
            tokens,
        );
        if entry.is_symlink() {
            let origin = cols.name.left_center() + egui::vec2(tokens.padding, 0.0);
            paint_link_icon(ui.painter(), origin, text_color);
        }
        ui.painter().text(
            cols.name.left_center() + egui::vec2(tokens.padding + ICON_GUTTER, 0.0),
            egui::Align2::LEFT_CENTER,
            name,
            tokens.font(),
            text_color,
        );
    }
    if cols.size.width() >= 1.0 {
        let size = if entry.is_dir_like() {
            "<DIR>".to_string()
        } else {
            entry.size.map(human_size).unwrap_or_default()
        };
        ui.painter().text(
            cols.size.right_center() - egui::vec2(tokens.padding, 0.0),
            egui::Align2::RIGHT_CENTER,
            size,
            tokens.font(),
            tokens.text_muted,
        );
    }
    if cols.mtime.width() >= 1.0 {
        let mtime = entry.modified.map(format_mtime).unwrap_or_default();
        ui.painter().text(
            cols.mtime.right_center() - egui::vec2(tokens.padding, 0.0),
            egui::Align2::RIGHT_CENTER,
            mtime,
            tokens.font(),
            tokens.text_muted,
        );
    }
    if cols.ty.width() >= 1.0 {
        let label = type_label(&entry.kind);
        ui.painter().text(
            cols.ty.right_center() - egui::vec2(tokens.padding, 0.0),
            egui::Align2::RIGHT_CENTER,
            label,
            tokens.font(),
            tokens.text_muted,
        );
    }
    column_separators(ui.painter(), &cols, rect, tokens);
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

pub(crate) fn format_mtime(t: SystemTime) -> String {
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
        wait_ready(&mut s, &uri);
        (s, uri)
    }

    fn wait_ready(s: &mut Session, uri: &VfsUri) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            if s.dirs.state(uri).is_ready() {
                break;
            }
            assert!(Instant::now() < deadline, "read never ready");
            std::thread::sleep(Duration::from_millis(5));
        }
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
    fn columns_follow_the_configured_widths_and_degrade_when_narrow() {
        let rect = |w: f32| egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(w, 20.0));
        let c = columns(rect(600.0), [100.0, 200.0, 50.0]);
        assert_eq!(
            (c.size.width(), c.mtime.width(), c.ty.width()),
            (100.0, 200.0, 50.0)
        );
        assert_eq!(c.name.width(), 250.0);
        let narrow = columns(rect(300.0), [100.0, 200.0, 50.0]);
        assert_eq!(narrow.ty.width(), 0.0);
        assert_eq!(narrow.size.width(), 0.0);
        assert_eq!(narrow.mtime.width(), 200.0);
        let tiny = columns(rect(100.0), [100.0, 200.0, 50.0]);
        assert_eq!(tiny.name.width(), 100.0);
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

    /// Session with `n` files in `/d`, so the listing is taller than a frame.
    #[allow(clippy::expect_used)]
    fn many_session(n: usize) -> (Session, VfsUri) {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/d").expect("mkdirs");
        for i in 0..n {
            mem.put_str(&format!("/d/f{i:03}.txt"), b"x").expect("put");
        }
        s.backends.register(Arc::new(mem));
        let uri = VfsUri::parse("/d").expect("uri");
        s.layout.state_mut(1).expect("pane").location = uri.clone();
        s.submit_read(uri.clone()).expect("submit");
        wait_ready(&mut s, &uri);
        (s, uri)
    }

    /// One headless listing frame on a fixed viewport (scroll offsets only
    /// clamp against a real screen rect).
    fn run_listing_frame(ctx: &egui::Context, s: &mut Session, tokens: &Tokens) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let _ = show(ui, s, 1, tokens, true);
                    });
            },
        );
        output.textures_delta.clear();
    }

    /// Regression for "a reload must not change my position": the listing's
    /// scroll offset is owned by the pane state and must round-trip through
    /// frames and survive the loading gap of a refresh.
    #[test]
    fn listing_scroll_round_trips_and_survives_a_refresh() {
        let (mut s, uri) = many_session(100);
        let tokens = Tokens::dark();
        let ctx = egui::Context::default();
        run_listing_frame(&ctx, &mut s, &tokens);

        s.layout.state_mut(1).expect("pane").listing.scroll = 300.0;
        run_listing_frame(&ctx, &mut s, &tokens);
        let parked = s.layout.state(1).expect("pane").listing.scroll;
        assert!(
            (parked - 300.0).abs() < 1.0,
            "parked offset round-trips: {parked}"
        );

        // Refresh: the "Loading…" frames must not touch the offset …
        s.refresh_read(uri.clone()).expect("refresh");
        for _ in 0..3 {
            s.poll();
            run_listing_frame(&ctx, &mut s, &tokens);
        }
        assert_eq!(
            s.layout.state(1).expect("pane").listing.scroll,
            300.0,
            "loading frames must not clobber the offset"
        );

        // … and the rows come back at the same place.
        wait_ready(&mut s, &uri);
        run_listing_frame(&ctx, &mut s, &tokens);
        let restored = s.layout.state(1).expect("pane").listing.scroll;
        assert!(
            (restored - 300.0).abs() < 1.0,
            "scroll restored: {restored}"
        );
    }
}
