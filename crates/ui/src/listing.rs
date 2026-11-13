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
    Page(isize, bool),
    SelectAll,
    Open,
}

pub(crate) fn key_outcome(key: Key, modifiers: Modifiers) -> KeyOutcome {
    match key {
        Key::ArrowUp => KeyOutcome::Move(-1, modifiers.shift),
        Key::ArrowDown => KeyOutcome::Move(1, modifiers.shift),
        Key::Home => KeyOutcome::Absolute(0, modifiers.shift),
        Key::End => KeyOutcome::Absolute(usize::MAX, modifiers.shift),
        Key::PageUp => KeyOutcome::Page(-1, modifiers.shift),
        Key::PageDown => KeyOutcome::Page(1, modifiers.shift),
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
    filter_bar(ui, s, pane, tokens);
    let content = content_widths(ui, pane, &location, &dir_state, tokens);
    let mut sizing = ColumnSizing {
        content,
        requested: s.layout.state(pane)?.col_widths,
    };
    if let Some(state) = s.layout.state(pane) {
        let action = header(ui, pane, &state.listing.sort, sizing, tokens);
        match action {
            Some(HeaderAction::Sort(key)) => {
                if let Some(state) = s.layout.state_mut(pane) {
                    state.listing.sort.header_click(key);
                    s.invalidate_view(pane);
                }
            }
            Some(HeaderAction::Resize(widths)) => {
                s.dispatch_or_notice(
                    "view.columns.resize",
                    &serde_json::json!({"pane": pane, "widths": widths}),
                );
                sizing.requested = s.layout.state(pane)?.col_widths;
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
            open = ready_rows(ui, s, pane, tokens, pane_focused, parent.as_ref(), sizing);
        }
    }
    open
}

/// Name takes spare space; only details compress below their content widths.
const MIN_NAME: f32 = 80.0;
/// Grab width of a column border in the header.
const GRIP_W: f32 = 6.0;

#[derive(Clone, Copy, Debug)]
struct ColumnSizing {
    content: [f32; 3],
    requested: [f32; 3],
}

impl ColumnSizing {
    fn preferred(self) -> [f32; 3] {
        std::array::from_fn(|i| {
            let request = self.requested.get(i).copied().unwrap_or(0.0);
            let content = self
                .content
                .get(i)
                .copied()
                .unwrap_or(fm_layout::MIN_COL_WIDTH);
            if request.is_finite() && request > 0.0 {
                request.clamp(fm_layout::MIN_COL_WIDTH, fm_layout::MAX_COL_WIDTH)
            } else {
                content
            }
        })
    }

    fn fit(self, width: f32) -> [f32; 3] {
        let mut widths = self.preferred();
        let budget = (width - MIN_NAME).max(0.0);
        // Give back manually added whitespace before compressing any content.
        let content: [f32; 3] = std::array::from_fn(|i| {
            self.content
                .get(i)
                .copied()
                .unwrap_or(fm_layout::MIN_COL_WIDTH)
                .min(widths.get(i).copied().unwrap_or(0.0))
        });
        shrink_to(&mut widths, &content, budget);
        shrink_to(&mut widths, &[40.0, 80.0, 40.0], budget);
        // Retire the least useful detail gradually instead of leaving three slivers
        // of unrelated text in a narrow split.
        for column in [2, 0, 1] {
            let excess = (widths.iter().sum::<f32>() - budget).max(0.0);
            if let Some(width) = widths.get_mut(column) {
                *width = (*width - excess).max(0.0);
            }
        }
        widths
    }
}

fn shrink_to(widths: &mut [f32], floors: &[f32], budget: f32) {
    let sum: f32 = widths.iter().sum();
    let slack: f32 = widths
        .iter()
        .zip(floors)
        .map(|(w, floor)| (w - floor).max(0.0))
        .sum();
    if sum > budget && slack > 0.0 {
        let fraction = ((sum - budget) / slack).min(1.0);
        for (width, floor) in widths.iter_mut().zip(floors) {
            *width -= (*width - floor).max(0.0) * fraction;
        }
    }
}

#[derive(Clone)]
struct ContentWidths {
    location: VfsUri,
    revision: Option<u64>,
    font: egui::FontId,
    padding: f32,
    pixels_per_point: f32,
    widths: [f32; 3],
}

fn content_widths(
    ui: &Ui,
    pane: PaneId,
    location: &VfsUri,
    state: &DirState,
    tokens: &Tokens,
) -> [f32; 3] {
    let id = egui::Id::NULL.with(("fm-column-content", pane));
    let pixels_per_point = ui.ctx().pixels_per_point();
    let previous = ui.ctx().data_mut(|d| d.get_temp::<ContentWidths>(id));
    if let Some(cache) = &previous
        && cache.location == *location
        && cache.font == tokens.font()
        && cache.padding == tokens.padding
        && cache.pixels_per_point == pixels_per_point
        && (cache.revision == state.revision() || matches!(state, DirState::Loading { .. }))
    {
        return cache.widths;
    }
    let mut size_examples = vec!["<DIR>".to_string()];
    let mut time_examples = vec!["2000-00-00 00:00".to_string()];
    if let DirState::Ready { entries, .. } = state {
        // Only five numeric maxima and two dates need formatting; never shape text
        // for every entry or rescan an unchanged directory while dragging.
        let mut maxima = [0u64; 5];
        let mut earliest = None;
        let mut latest = None;
        for entry in entries.iter() {
            if !entry.is_dir_like()
                && let Some(bytes) = entry.size
            {
                let mut value = bytes;
                let mut bucket = 0;
                while value >= 1024 && bucket < 4 {
                    value /= 1024;
                    bucket += 1;
                }
                if let Some(maximum) = maxima.get_mut(bucket) {
                    *maximum = (*maximum).max(bytes);
                }
            }
            if let Some(time) = entry.modified {
                earliest = Some(earliest.map_or(time, |old: SystemTime| old.min(time)));
                latest = Some(latest.map_or(time, |old: SystemTime| old.max(time)));
            }
        }
        size_examples.extend(maxima.into_iter().map(human_size));
        time_examples.extend([earliest, latest].into_iter().flatten().map(format_mtime));
    }
    let measure = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.into(), tokens.font(), tokens.text)
            .size()
            .x
    };
    let header = |label: &str| measure(label) + tokens.font_size * 0.7 + tokens.padding;
    let widths = [
        size_examples
            .iter()
            .map(|s| measure(s))
            .fold(header("Size"), f32::max),
        time_examples
            .iter()
            .map(|s| measure(s))
            .fold(header("Modified"), f32::max),
        ["Dir", "File", "Link", "Other"]
            .into_iter()
            .map(measure)
            .fold(header("Type"), f32::max),
    ]
    .map(|w| {
        (w + 2.0 * tokens.padding)
            .ceil()
            .clamp(fm_layout::MIN_COL_WIDTH, fm_layout::MAX_COL_WIDTH)
    });
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            id,
            ContentWidths {
                location: location.clone(),
                revision: state.revision(),
                font: tokens.font(),
                padding: tokens.padding,
                pixels_per_point,
                widths,
            },
        )
    });
    widths
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

/// One continuous layout shared by header, hit-testing and entry rows.
fn columns(rect: egui::Rect, sizing: ColumnSizing) -> Columns {
    let [size_w, mtime_w, type_w] = sizing.fit(rect.width());
    let split = (rect.right() - (size_w + mtime_w + type_w)).max(rect.left());
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
fn paint_sort_arrow(
    painter: &egui::Painter,
    c: egui::Pos2,
    size: f32,
    dir: SortDir,
    tokens: &Tokens,
) {
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
    painter.add(egui::Shape::convex_polygon(
        pts,
        tokens.text_muted,
        egui::Stroke::NONE,
    ));
}

/// What a header interaction asks for.
enum HeaderAction {
    Sort(SortKey),
    Resize([f32; 3]),
}

#[derive(Clone, Copy)]
struct ColumnDrag {
    start_x: f32,
    widths: [f32; 3],
    floors: [f32; 3],
    available: f32,
}

impl ColumnDrag {
    fn resize(self, boundary: usize, pointer_x: f32) -> [f32; 3] {
        let mut widths = self.widths;
        let Some(right) = widths.get(boundary).copied() else {
            return widths;
        };
        let delta = pointer_x - self.start_x;
        let Some(right_widths) = self.widths.get(boundary..) else {
            return widths;
        };
        let original_right: f32 = right_widths.iter().sum();
        let minimum = right_widths.len() as f32 * fm_layout::MIN_COL_WIDTH;
        let maximum = (self.available - MIN_NAME - boundary as f32 * fm_layout::MIN_COL_WIDTH)
            .min(original_right - right + fm_layout::MAX_COL_WIDTH)
            .max(minimum);
        let target = (original_right - delta).clamp(minimum, maximum);
        let minimums = [fm_layout::MIN_COL_WIDTH; 3];
        if let Some(width) = widths.get_mut(boundary) {
            *width = (right + target - original_right)
                .max(self.floors.get(boundary).copied().unwrap_or(right))
                .min(fm_layout::MAX_COL_WIDTH);
        }
        if let (Some(right), Some(floors)) =
            (widths.get_mut(boundary..), self.floors.get(boundary..))
        {
            shrink_to(right, floors, target);
            shrink_to(
                right,
                minimums.get(..right.len()).unwrap_or_default(),
                target,
            );
        }
        if boundary > 0 {
            if let (Some(left), Some(floor)) =
                (widths.get_mut(boundary - 1), self.floors.get(boundary - 1))
            {
                *left = (*left + original_right - target)
                    .max(*floor)
                    .min(fm_layout::MAX_COL_WIDTH);
            }
            if let (Some(left), Some(floors)) =
                (widths.get_mut(..boundary), self.floors.get(..boundary))
            {
                let budget = self.available - MIN_NAME - target;
                shrink_to(left, floors, budget);
                shrink_to(left, minimums.get(..left.len()).unwrap_or_default(), budget);
            }
        }
        widths.map(|w| w.clamp(fm_layout::MIN_COL_WIDTH, fm_layout::MAX_COL_WIDTH))
    }
}

fn header(
    ui: &mut Ui,
    pane: PaneId,
    sort: &SortSpec,
    mut sizing: ColumnSizing,
    tokens: &Tokens,
) -> Option<HeaderAction> {
    let available = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available, tokens.row_height),
        egui::Sense::click(),
    );
    let cols = columns(rect, sizing);
    let mut action = None;
    let mut active_x = None;
    for (ix, cell) in [cols.size, cols.mtime, cols.ty].into_iter().enumerate() {
        let id = egui::Id::NULL.with(("fm-col-grip", pane, ix));
        if cell.width() < fm_layout::MIN_COL_WIDTH - 0.01 {
            continue;
        }
        let grip = egui::Rect::from_center_size(
            egui::pos2(cell.left(), rect.center().y),
            egui::vec2(GRIP_W, rect.height()),
        );
        let resp = ui.interact(grip, id, egui::Sense::click_and_drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            active_x = Some(ix);
        }
        if resp.double_clicked() {
            if let Some(width) = sizing.requested.get_mut(ix) {
                *width = 0.0;
            }
            action = Some(HeaderAction::Resize(sizing.requested));
        } else if resp.dragged()
            && sizing
                .fit(available)
                .iter()
                .all(|w| *w >= fm_layout::MIN_COL_WIDTH - 0.01)
            && let Some(pointer) = resp.interact_pointer_pos()
        {
            if resp.drag_started() {
                let widths = sizing.fit(available);
                let floors = std::array::from_fn(|i| {
                    sizing
                        .content
                        .get(i)
                        .copied()
                        .unwrap_or(fm_layout::MIN_COL_WIDTH)
                        .min(widths.get(i).copied().unwrap_or(0.0))
                });
                let start_x = pointer.x - resp.total_drag_delta().unwrap_or_default().x;
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        id,
                        ColumnDrag {
                            start_x,
                            widths,
                            floors,
                            available,
                        },
                    )
                });
            }
            if let Some(drag) = ui.ctx().data_mut(|d| d.get_temp::<ColumnDrag>(id)) {
                sizing.requested = drag.resize(ix, pointer.x);
                action = Some(HeaderAction::Resize(sizing.requested));
            }
        }
        if resp.drag_stopped() {
            ui.ctx().data_mut(|d| d.remove::<ColumnDrag>(id));
        }
    }
    // Paint the updated layout now, so header and rows follow the pointer in the same frame.
    let cols = columns(rect, sizing);
    ui.painter().rect_filled(rect, 0.0, tokens.panel);

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
        let arrow_x = if key == SortKey::Name {
            let x = cell.left() + tokens.padding;
            (x + galley.size().x + tokens.padding + size).min(cell.right() - tokens.padding - size)
        } else {
            cell.right() - tokens.padding - size
        };
        let painter = ui.painter().with_clip_rect(cell.intersect(ui.clip_rect()));
        let mut text_rect = cell.shrink2(egui::vec2(tokens.padding, 0.0));
        text_rect.max.x -= arrow_space;
        paint_cell(
            &painter,
            text_rect,
            label,
            tokens.font(),
            tokens.text_muted,
            key != SortKey::Name,
        );
        if sorted {
            paint_sort_arrow(&painter, egui::pos2(arrow_x, y), size, sort.dir, tokens);
        }
    }
    column_separators(ui.painter(), &cols, rect, tokens);

    if let Some(ix) = active_x
        && let Some(cell) = [cols.size, cols.mtime, cols.ty].get(ix)
    {
        ui.painter().line_segment(
            [
                egui::pos2(cell.left(), rect.top()),
                egui::pos2(cell.left(), rect.bottom()),
            ],
            egui::Stroke::new(tokens.border_width.max(2.0), tokens.accent),
        );
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

#[allow(clippy::too_many_arguments)]
fn ready_rows(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    pane_focused: bool,
    parent: Option<&VfsUri>,
    widths: ColumnSizing,
) -> Option<VfsUri> {
    let (entries, ix) = s.pane_view(pane)?;
    let mut open: Option<VfsUri> = None;
    // Row 0 is always `.` (the pane's own folder); `..` follows below a root.
    let has_parent = parent.is_some();
    let specials = 1 + usize::from(has_parent);
    let total = ix.order.len() + specials;
    if total == 0 {
        ui.label(tokens.muted("(empty)"));
        return None;
    }
    let names: Vec<String> = ix
        .order
        .iter()
        .filter_map(|i| entries.get(*i as usize).map(|e| e.name.clone()))
        .collect();

    // An empty directory owns no entry row: park the cursor on `.` unless the user
    // already chose a special row.
    if names.is_empty()
        && let Some(state) = s.layout.state_mut(pane)
        && !state.listing.on_current
        && !state.listing.on_parent
    {
        state.listing.focus_current();
    }

    // Keyboard once per frame, before painting (so the scroll offset can be
    // applied on the same frame without fighting the wheel).
    let saved_scroll = s
        .layout
        .state(pane)
        .map(|p| p.listing.scroll)
        .unwrap_or(0.0);
    let mut scroll_to: Option<f32> = None;
    let mut open_key = false;
    let pitch = tokens.row_height + ui.spacing().item_spacing.y;
    let page_rows = ((ui.available_height() / pitch).floor() as isize - 1).max(1);
    let tree_active = s.layout.state(pane).is_some_and(|p| p.tree.active);
    // Show in Folder: select the revealed item once its folder is listed.
    let mut moved = false;
    let mut revealed = false;
    if let Some(state) = s.layout.state_mut(pane)
        && let Some(name) = state.listing.reveal.take()
        && let Some(row) = names.iter().position(|n| *n == name)
    {
        state.listing.select_only(Some(&name), row);
        moved = true;
        revealed = true;
    }
    // A dialog such as Preferences owns the keyboard even while this pane stays focused.
    if pane_focused
        && !tree_active
        && !crate::dialogs::window_owns_keyboard(s)
        && !ui.ctx().text_edit_focused()
        && !crate::preview::grid_has_keyboard(ui.ctx())
    {
        esc_clears_filter(ui, s, pane);
        type_to_filter(ui, s, pane);
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
                moved |= matches!(
                    outcome,
                    KeyOutcome::Move(..)
                        | KeyOutcome::Absolute(..)
                        | KeyOutcome::Page(..)
                        | KeyOutcome::SelectAll
                );
                match outcome {
                    KeyOutcome::None => {}
                    KeyOutcome::Move(delta, extend) => {
                        listing.move_focus_with_specials(&names, delta, extend, true, has_parent);
                    }
                    KeyOutcome::Page(dir, extend) => {
                        let last = names.len().saturating_sub(1) as isize;
                        let from = if listing.on_current || listing.on_parent {
                            0
                        } else {
                            listing.focused_row as isize
                        };
                        let to = (from + dir * page_rows).clamp(0, last) as usize;
                        if to == 0 && !extend && dir < 0 && has_parent {
                            listing.focus_parent();
                        } else if to == 0 && !extend && dir < 0 && names.is_empty() {
                            listing.focus_current();
                        } else {
                            listing.focus_absolute(&names, to, extend);
                        }
                    }
                    KeyOutcome::Absolute(idx, extend) => {
                        let idx = if idx == usize::MAX {
                            names.len().saturating_sub(1)
                        } else {
                            idx
                        };
                        if idx == 0 && !extend {
                            listing.focus_current();
                        } else {
                            listing.focus_absolute(&names, idx, extend);
                        }
                    }
                    KeyOutcome::SelectAll => listing.select_all(&names),
                    KeyOutcome::Open => open_key = true,
                }
            }
        }
    }
    // Keys that do not move focus (bare Shift/Ctrl, letters) must not re-scroll.
    if moved && let Some(state) = s.layout.state(pane) {
        let listing = &state.listing;
        let row = if listing.on_current {
            0
        } else if listing.on_parent {
            1 // `..` sits directly below the always-present `.`
        } else {
            listing.focused_row.min(names.len().saturating_sub(1)) + specials
        };
        // Scroll just enough to keep the focused row visible.
        let top = row as f32 * pitch;
        let bottom = top + tokens.row_height;
        let view = ui.available_height();
        if revealed {
            // A revealed item sits mid-list, where it stands out, not on an edge.
            scroll_to = Some((top + tokens.row_height / 2.0 - view / 2.0).max(0.0));
        } else if top < saved_scroll {
            scroll_to = Some(top);
        } else if bottom > saved_scroll + view {
            scroll_to = Some(bottom - view);
        }
    }
    let on_parent = s.layout.state(pane).is_some_and(|p| p.listing.on_parent);
    let on_current = s.layout.state(pane).is_some_and(|p| p.listing.on_current);
    if open_key && on_parent {
        open = parent.cloned();
    } else if open_key && !on_current {
        let focus = s.layout.state(pane).map(|p| p.listing.focused_row);
        let name = focus.and_then(|row| names.get(row).cloned());
        if let Some(name) = name
            && let Some(entry) = entries.iter().find(|e| e.name == name)
        {
            open = Some(entry.uri.clone());
        }
    }

    let row_height = tokens.row_height;
    let builder = egui::ScrollArea::both()
        .id_salt(format!("fm-list-{pane}"))
        .auto_shrink([false, false]);
    // The pane state's offset is the source of truth: no ScrollArea is
    // rendered while a refresh shows "Loading…", so the stored position must
    // not depend on egui's state surviving that gap on its own.
    let builder = match scroll_to {
        Some(offset) => builder.vertical_scroll_offset(offset),
        None => builder.vertical_scroll_offset(saved_scroll),
    };
    let mut focus_parent = false;
    let marks = match s.layout.state(pane).map(|st| st.location.clone()) {
        Some(loc) => crate::git_marks::Marks::for_folder(s, &loc),
        None => crate::git_marks::Marks::none(),
    };
    let output = builder.show_rows(ui, row_height, total, |ui, row_range| {
        for row in row_range {
            if row == 0 {
                current_row(ui, s, pane, tokens, widths, on_current && pane_focused);
                continue;
            }
            if has_parent && row == 1 {
                parent_row(
                    ui,
                    tokens,
                    parent,
                    widths,
                    on_parent && pane_focused,
                    &mut open,
                    &mut focus_parent,
                );
                continue;
            }
            let entry_row = row - specials;
            let Some(idx) = ix.order.get(entry_row).copied() else {
                continue;
            };
            let Some(entry) = entries.get(idx as usize) else {
                continue;
            };
            if let Some(target) = entry_row_ui(
                ui,
                s,
                pane,
                tokens,
                pane_focused,
                entry,
                entry_row,
                widths,
                &marks,
            ) {
                open = Some(target);
            }
        }
    });
    let rows_bottom = output.inner_rect.top() + total as f32 * pitch
        - ui.spacing().item_spacing.y
        - output.state.clamped_offset().y;
    let empty = egui::Rect::from_min_max(
        egui::pos2(
            output.inner_rect.left(),
            rows_bottom.max(output.inner_rect.top()),
        ),
        output.inner_rect.max,
    );
    if empty.is_positive() {
        let response = ui.interact(
            empty,
            ui.make_persistent_id(("listing-folder-menu", pane)),
            egui::Sense::click(),
        );
        crate::context_menu::attach_folder(&response, s, pane, None);
    }
    if let Some(state) = s.layout.state_mut(pane) {
        state.listing.scroll = output.state.clamped_offset().y;
        if focus_parent {
            state.listing.focus_parent();
        }
    }
    open
}

/// The quick-filter bar above the listing, shown while the pane has a filter. Typing narrows
/// the listing (`view.filter`); Esc or the clear button removes it; Enter or Down hands the
/// keyboard back to the rows.
/// Typing in a focused listing opens the filter with the typed text (appending to an
/// open, unfocused filter) and moves the cursor into it. A leading space does not start a
/// filter, so Space keeps its listing meaning.
fn type_to_filter(ui: &Ui, s: &mut Session, pane: PaneId) {
    let typed: String = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_control())
            .collect()
    });
    let Some(existing) = s
        .layout
        .state(pane)
        .map(|state| state.listing.filter.clone())
    else {
        return;
    };
    if typed.is_empty() || (existing.is_none() && typed.trim().is_empty()) {
        return;
    }
    let mut text = existing.unwrap_or_default();
    text.push_str(&typed);
    s.dispatch_or_notice(
        "view.filter",
        &serde_json::json!({"text": text, "focus": true}),
    );
}

fn filter_bar(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    let Some(state) = s.layout.state(pane) else {
        return;
    };
    let Some(mut text) = state.listing.filter.clone() else {
        return;
    };
    let focus = state.listing.filter_focus;
    let shown = s.pane_view(pane).map(|(_, ix)| ix.order.len());
    let (mut clear, mut back, mut changed) = (false, false, false);
    ui.horizontal(|ui| {
        let caption = ui.label(tokens.muted("Filter"));
        let edit = egui::TextEdit::singleline(&mut text)
            .id(egui::Id::NULL.with(("fm-listing-filter", pane)))
            .hint_text("part of a name, or * ? [a-z]")
            .desired_width((ui.available_width() - 120.0).max(80.0));
        let response = ui.add(edit).labelled_by(caption.id).on_hover_text(
            "Show only matching names. Wildcards: * any text, ? one character, [a-z] a range; \
             separate alternatives with ;",
        );
        if focus {
            response.request_focus();
            // Typing into the listing opened the filter: keep typing after that text.
            let id = response.id;
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            let end = egui::text::CCursor::new(text.chars().count());
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), id);
        }
        changed = response.changed();
        let (esc, enter, down) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::ArrowDown),
            )
        });
        if (response.has_focus() || response.lost_focus()) && esc {
            clear = true;
        } else if (response.lost_focus() && enter) || (response.has_focus() && down) {
            back = true;
            response.surrender_focus();
        }
        if let Some(n) = shown {
            ui.label(tokens.muted(format!("{n} shown")));
        }
        if ui
            .small_button("×")
            .on_hover_text("Clear the filter (Esc)")
            .clicked()
        {
            clear = true;
        }
    });
    if let Some(state) = s.layout.state_mut(pane) {
        state.listing.filter_focus = false;
        if back {
            state.tree.active = false;
        }
    }
    if clear || changed || back {
        let _ = s.layout.focus_pane(pane);
    }
    if clear {
        s.dispatch_or_notice("view.filter.clear", &serde_json::Value::Null);
    } else if changed {
        s.dispatch_or_notice("view.filter", &serde_json::json!({ "text": text }));
    }
}

/// Esc in a focused listing clears its quick filter.
fn esc_clears_filter(ui: &Ui, s: &mut Session, pane: PaneId) {
    let has_filter = s
        .layout
        .state(pane)
        .is_some_and(|p| p.listing.filter.is_some());
    if has_filter && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        s.dispatch_or_notice("view.filter.clear", &serde_json::Value::Null);
    }
}

/// Where an entry's name starts: the Name column after the icon gutter, shared by the
/// special rows so `.`/`..` line up with the files below them.
fn name_origin(rect: egui::Rect, widths: ColumnSizing, tokens: &Tokens) -> egui::Pos2 {
    columns(rect, widths).name.left_center() + egui::vec2(tokens.padding + ICON_GUTTER, 0.0)
}

fn current_row(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    widths: ColumnSizing,
    focused: bool,
) {
    let available = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available, tokens.row_height),
        egui::Sense::click(),
    );
    if response.clicked()
        && let Some(state) = s.layout.state_mut(pane)
    {
        state.listing.focus_current();
    }
    response.clone().on_hover_text("Current folder");
    crate::context_menu::attach_folder(&response, s, pane, None);
    if focused {
        ui.painter().rect_filled(rect, 0.0, tokens.selection_bg);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, tokens.hover_bg);
    }
    ui.painter().text(
        name_origin(rect, widths, tokens),
        egui::Align2::LEFT_CENTER,
        ".",
        tokens.font(),
        if focused {
            tokens.selection_text
        } else {
            tokens.text
        },
    );
}

fn parent_row(
    ui: &mut Ui,
    tokens: &Tokens,
    parent: Option<&VfsUri>,
    widths: ColumnSizing,
    focused: bool,
    open: &mut Option<VfsUri>,
    focus_parent: &mut bool,
) {
    let available = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available, tokens.row_height),
        egui::Sense::click(),
    );
    if let Some(parent) = parent {
        crate::dnd::register_zone(ui.ctx(), rect, parent.clone(), crate::dnd::ZONE_ROW);
    }
    // Like any folder: one click selects, a double click opens.
    if response.double_clicked()
        && let Some(parent) = parent
    {
        *open = Some((*parent).clone());
    } else if response.clicked() {
        *focus_parent = true;
    }
    if focused {
        ui.painter().rect_filled(rect, 0.0, tokens.selection_bg);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, tokens.hover_bg);
    }
    ui.painter().text(
        name_origin(rect, widths, tokens),
        egui::Align2::LEFT_CENTER,
        "..",
        tokens.font(),
        if focused {
            tokens.selection_text
        } else {
            tokens.text
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn entry_row_ui(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    pane_focused: bool,
    entry: &Entry,
    row: usize,
    widths: ColumnSizing,
    marks: &crate::git_marks::Marks,
) -> Option<VfsUri> {
    let state = s.layout.state(pane)?;
    let selected = state.listing.is_selected(&entry.name);
    let state_selected_count = state.listing.selected_count();
    let focused =
        state.listing.focused_row == row && !state.listing.on_parent && !state.listing.on_current;

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
        // A tree row left holding egui focus would turn the arrows into widget-focus moves
        // (moving the tree cursor) instead of listing moves.
        ui.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        if let Some(state) = s.layout.state_mut(pane) {
            state.tree.active = false;
            if modifiers.shift {
                state.listing.extend_click(&names, row);
            } else if modifiers.command {
                state.listing.toggle(&entry.name, row);
            } else if selected && state.listing.selected_count() == 1 && !response.double_clicked()
            {
                // Clicking the only selected item again lets go of it: the folder itself (`.`)
                // becomes the target.
                state.listing.focus_current();
            } else {
                state.listing.select_only(Some(&entry.name), row);
            }
        }
    }
    // Right-click acts on the row under the pointer unless it is already part of the selection.
    if response.secondary_clicked() {
        ui.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        if let Err(err) = s.layout.focus_pane(pane) {
            s.notice = Some(err.to_string());
        }
        if let Some(state) = s.layout.state_mut(pane) {
            state.tree.active = false;
            if !selected {
                state.listing.select_only(Some(&entry.name), row);
            }
        }
    }
    let plain_click = response.clicked() && ui.input(|i| i.modifiers.is_none());
    let mut open = if response.double_clicked() || (s.prefs.single_click_open && plain_click) {
        Some(entry.uri.clone())
    } else {
        None
    };
    if s.layout
        .state(pane)
        .is_some_and(|st| st.location.scheme() == fm_vfs::Scheme::Servers)
    {
        crate::context_menu::attach_server(&response, s, &entry.name, &entry.uri, &mut open);
    } else {
        crate::context_menu::attach(&response, s, &entry.uri, &mut open);
    }

    // With a selection, an unselected cursor row (after a Ctrl-click deselect) only gets an
    // outline, so it does not look selected.
    let cursor_only = !selected && focused && pane_focused && state_selected_count > 0;
    let bg = if selected || (focused && pane_focused && !cursor_only) {
        Some(tokens.selection_bg)
    } else if response.hovered() {
        Some(tokens.hover_bg)
    } else {
        None
    };
    if let Some(bg) = bg {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    if cursor_only {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            0.0,
            egui::Stroke::new(1.0, tokens.selection_bg),
            egui::StrokeKind::Inside,
        );
    }
    // Norton-style cues: directories get the accent colour and a trailing slash, links the
    // muted colour and an `@`; plain files keep the primary text colour.
    let mark = marks.mark(&entry.uri);
    let text_color = if selected {
        tokens.selection_text
    } else if let Some(tint) = marks.tint_of(mark, tokens) {
        tint
    } else if entry.is_dir_like() {
        tokens.accent_text
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
        format!("{}/", entry.display_name())
    } else {
        entry.display_name().to_string()
    };
    let cols = columns(rect, widths);
    if cols.name.width() >= 1.0 {
        let mut cell = cols.name.shrink2(egui::vec2(tokens.padding, 0.0));
        // The icon gutter shows the git mark, else the link icon.
        let git = mark.filter(|m| marks.gutter && *m != fm_engine::git::Mark::Ignored);
        if let Some(m) = git {
            crate::git_marks::paint_gutter(ui.painter(), cell, m, entry.is_dir_like(), tokens);
        } else if entry.is_symlink() {
            let origin = cols.name.left_center() + egui::vec2(tokens.padding, 0.0);
            paint_link_icon(ui.painter(), origin, text_color);
        }
        cell.min.x += ICON_GUTTER;
        paint_cell(
            ui.painter(),
            cell,
            &display_name,
            tokens.font(),
            text_color,
            false,
        );
    }
    if cols.size.width() >= 1.0 {
        let size = if entry.is_dir_like() {
            "<DIR>".to_string()
        } else {
            entry.size.map(human_size).unwrap_or_default()
        };
        paint_cell(
            ui.painter(),
            cols.size.shrink2(egui::vec2(tokens.padding, 0.0)),
            &size,
            tokens.font(),
            tokens.text_muted,
            true,
        );
    }
    if cols.mtime.width() >= 1.0 {
        let mtime = entry.modified.map(format_mtime).unwrap_or_default();
        paint_cell(
            ui.painter(),
            cols.mtime.shrink2(egui::vec2(tokens.padding, 0.0)),
            &mtime,
            tokens.font(),
            tokens.text_muted,
            true,
        );
    }
    if cols.ty.width() >= 1.0 {
        let label = type_label(&entry.kind);
        paint_cell(
            ui.painter(),
            cols.ty.shrink2(egui::vec2(tokens.padding, 0.0)),
            label,
            tokens.font(),
            tokens.text_muted,
            true,
        );
    }
    column_separators(ui.painter(), &cols, rect, tokens);
    let mut hover = fm_engine::item_cmds::display_path(&entry.uri);
    if let Some(m) = mark {
        hover = format!("{hover}\nGit: {}", crate::git_marks::describe(m));
    }
    response.clone().on_hover_text(hover);
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

fn paint_cell(
    painter: &egui::Painter,
    rect: egui::Rect,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    right_aligned: bool,
) {
    if rect.width() < font.size {
        return;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width());
    let galley = painter.layout_job(job);
    let x = if right_aligned {
        rect.right() - galley.size().x
    } else {
        rect.left()
    };
    painter
        .with_clip_rect(rect.intersect(painter.clip_rect()))
        .galley(
            egui::pos2(x, rect.center().y - galley.size().y * 0.5),
            galley,
            color,
        );
}

thread_local! {
    /// Size and date style, set by the app each frame from the preferences (UI thread only).
    static DISPLAY: std::cell::Cell<(fm_engine::SizeUnits, fm_engine::DateFormat)> =
        const { std::cell::Cell::new((fm_engine::SizeUnits::Binary, fm_engine::DateFormat::Iso)) };
    static TIME_ZONE: std::cell::RefCell<jiff::tz::TimeZone> =
        const { std::cell::RefCell::new(jiff::tz::TimeZone::UTC) };
}

/// Choose how sizes and dates are written everywhere in the UI.
pub fn set_display(units: fm_engine::SizeUnits, dates: fm_engine::DateFormat) {
    DISPLAY.with(|d| d.set((units, dates)));
}

/// Use the engine's already-resolved OS timezone; formatting never reads timezone files.
pub fn set_time_zone(zone: &jiff::tz::TimeZone) {
    TIME_ZONE.with(|current| current.replace(zone.clone()));
}

pub(crate) fn human_size(bytes: u64) -> String {
    let (units, _) = DISPLAY.with(std::cell::Cell::get);
    let (base, names): (f64, [&str; 5]) = match units {
        fm_engine::SizeUnits::Binary => (1024.0, ["B", "KiB", "MiB", "GiB", "TiB"]),
        fm_engine::SizeUnits::Decimal => (1000.0, ["B", "kB", "MB", "GB", "TB"]),
        fm_engine::SizeUnits::Bytes => return format!("{bytes} B"),
    };
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= base && unit < names.len() - 1 {
        value /= base;
        unit += 1;
    }
    match names.get(unit) {
        Some(name) if unit > 0 => format!("{value:.1} {name}"),
        _ => format!("{bytes} B"),
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
    let (_, format) = DISPLAY.with(std::cell::Cell::get);
    match jiff::Timestamp::try_from(t) {
        Ok(timestamp) => {
            let date = TIME_ZONE.with(|zone| zone.borrow().to_datetime(timestamp));
            let (y, mo, day, h, mi, sec) = (
                date.year(),
                date.month(),
                date.day(),
                date.hour(),
                date.minute(),
                date.second(),
            );
            match format {
                fm_engine::DateFormat::Iso => format!("{y:04}-{mo:02}-{day:02} {h:02}:{mi:02}"),
                fm_engine::DateFormat::IsoSeconds => {
                    format!("{y:04}-{mo:02}-{day:02} {h:02}:{mi:02}:{sec:02}")
                }
                fm_engine::DateFormat::Eu => format!("{day:02}/{mo:02}/{y:04} {h:02}:{mi:02}"),
                fm_engine::DateFormat::Us => {
                    let (h12, suffix) = match h {
                        0 => (12, "AM"),
                        1..=11 => (h, "AM"),
                        12 => (12, "PM"),
                        _ => (h - 12, "PM"),
                    };
                    format!("{mo:02}/{day:02}/{y:04} {h12}:{mi:02} {suffix}")
                }
            }
        }
        Err(err) => format!("Invalid file time: {err}"),
    }
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
        assert_eq!(
            key_outcome(Key::PageDown, Modifiers::SHIFT),
            KeyOutcome::Page(1, true)
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
    fn columns_keep_preferred_widths_and_compress_continuously_when_narrow() {
        let rect = |w: f32| egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(w, 20.0));
        let sizing = ColumnSizing {
            content: [60.0, 150.0, 50.0],
            requested: [100.0, 200.0, 50.0],
        };
        let c = columns(rect(600.0), sizing);
        assert_eq!(
            (c.size.width(), c.mtime.width(), c.ty.width()),
            (100.0, 200.0, 50.0)
        );
        assert_eq!(c.name.width(), 250.0);
        let compact = columns(rect(340.0), sizing);
        assert_eq!(
            (
                compact.size.width(),
                compact.mtime.width(),
                compact.ty.width()
            ),
            (60.0, 150.0, 50.0)
        );
        let narrow = columns(rect(300.0), sizing);
        assert!(narrow.ty.width() > 0.0 && narrow.size.width() > 0.0);
        assert_eq!(narrow.name.width(), MIN_NAME);
        let tiny = columns(rect(100.0), sizing);
        assert!((tiny.name.width() - MIN_NAME).abs() < 0.01);
        let mut previous = sizing.fit(0.0);
        for width in 1..1000 {
            let fitted = sizing.fit(width as f32);
            for (now, before) in fitted.into_iter().zip(previous) {
                assert!(
                    now >= before - 0.001 && now - before <= 1.01,
                    "no column jumps at {width}"
                );
            }
            let c = columns(rect(width as f32), sizing);
            for cell in [c.name, c.size, c.mtime, c.ty] {
                assert!(cell.width() >= -0.001);
                assert!(cell.left() >= -0.001 && cell.right() <= width as f32 + 0.001);
            }
            previous = fitted;
        }
    }

    #[test]
    fn dragging_inner_dividers_trades_adjacent_space_without_moving_name() {
        let drag = ColumnDrag {
            start_x: 300.0,
            widths: [100.0, 200.0, 60.0],
            floors: [60.0, 150.0, 60.0],
            available: 600.0,
        };
        assert_eq!(drag.resize(1, 320.0), [120.0, 180.0, 60.0]);
        assert_eq!(drag.resize(1, 290.0), [90.0, 210.0, 60.0]);
        assert_eq!(drag.resize(1, 1000.0), [280.0, 40.0, 40.0]);
        assert_eq!(drag.resize(1, -1000.0), [60.0, 400.0, 60.0]);
        assert_eq!(drag.resize(0, 330.0), [70.0, 200.0, 60.0]);
        assert_eq!(drag.resize(0, -1000.0), [260.0, 200.0, 60.0]);
        // Absolute pointer positions, not accumulated frame deltas: reverse after hitting a limit.
        assert_eq!(drag.resize(1, 305.0), [105.0, 195.0, 60.0]);
    }

    #[test]
    fn content_fitted_dividers_are_not_locked_when_neighbors_have_no_whitespace() {
        let sizing = ColumnSizing {
            content: [80.0, 160.0, 60.0],
            requested: [0.0; 3],
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 20.0));
        let before = columns(rect, sizing);
        for (boundary, cell) in [before.size, before.mtime, before.ty]
            .into_iter()
            .enumerate()
        {
            let drag = ColumnDrag {
                start_x: cell.left(),
                widths: sizing.fit(rect.width()),
                floors: sizing.content,
                available: rect.width(),
            };
            for delta in [-15.0, -10.0, 10.0, 15.0] {
                let requested = drag.resize(boundary, cell.left() + delta);
                let after = columns(
                    rect,
                    ColumnSizing {
                        requested,
                        ..sizing
                    },
                );
                let edge = [after.size, after.mtime, after.ty]
                    .get(boundary)
                    .expect("column")
                    .left();
                assert!(
                    (edge - cell.left() - delta).abs() < 0.01,
                    "boundary {boundary} must move by {delta}"
                );
            }
        }
    }

    #[test]
    fn details_fit_content_not_long_names_and_follow_font_changes() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            render_frame(&tokens, |ui| {
                let location = VfsUri::parse("/columns").expect("uri");
                let mut entry = Entry::new(location.join("x").expect("uri"), EntryKind::File);
                entry.size = Some(u64::MAX);
                entry.modified = Some(SystemTime::UNIX_EPOCH);
                let state = |revision, entry: Entry| DirState::Ready {
                    revision,
                    entries: vec![entry].into(),
                };
                let widths = content_widths(ui, 1, &location, &state(1, entry.clone()), &tokens);
                let size_width = ui
                    .painter()
                    .layout_no_wrap(human_size(u64::MAX), tokens.font(), tokens.text)
                    .size()
                    .x;
                let date_width = ui
                    .painter()
                    .layout_no_wrap(
                        format_mtime(SystemTime::UNIX_EPOCH),
                        tokens.font(),
                        tokens.text,
                    )
                    .size()
                    .x;
                assert!(widths[0] >= size_width + 2.0 * tokens.padding);
                assert!(widths[1] >= date_width + 2.0 * tokens.padding);
                entry.name = "very-long-variable-filename".repeat(100);
                assert_eq!(
                    content_widths(ui, 1, &location, &state(2, entry.clone()), &tokens),
                    widths
                );
                let larger = Tokens {
                    font_size: tokens.font_size * 1.5,
                    ..tokens
                };
                let enlarged = content_widths(ui, 1, &location, &state(2, entry), &larger);
                assert!(enlarged.iter().zip(widths).all(|(large, old)| *large > old));
                assert_eq!(
                    content_widths(ui, 1, &location, &DirState::Loading { job: 1 }, &larger),
                    enlarged
                );
            });
        }
    }

    #[test]
    fn header_drag_follows_pointer_and_double_click_restores_content_fit() {
        let ctx = egui::Context::default();
        let tokens = Tokens::dark();
        crate::theme::apply_to(&ctx, &tokens);
        let mut sizing = ColumnSizing {
            content: [60.0, 150.0, 60.0],
            requested: [100.0, 200.0, 60.0],
        };
        let mut time = 0.0;
        let mut run = |sizing: &mut ColumnSizing, events: Vec<egui::Event>| {
            time += 0.05;
            let mut action = None;
            let mut area = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 200.0),
                    )),
                    events,
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.set_width(600.0);
                        area = egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(ui.available_width(), tokens.row_height),
                        );
                        action = header(ui, 1, &SortSpec::default(), *sizing, &tokens);
                    });
                },
            );
            output.textures_delta.clear();
            if let Some(HeaderAction::Resize(widths)) = &action {
                sizing.requested = *widths;
            }
            (area, action)
        };
        let (rect, _) = run(&mut sizing, vec![]);
        let initial = columns(rect, sizing);
        let start = egui::pos2(initial.mtime.left(), rect.center().y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Modifiers::NONE,
        };
        run(
            &mut sizing,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        for delta in [10.0, 20.0, 30.0, 15.0] {
            let pointer = start + egui::vec2(delta, 0.0);
            let (_, action) = run(&mut sizing, vec![egui::Event::PointerMoved(pointer)]);
            assert!(matches!(action, Some(HeaderAction::Resize(_))));
            let after = columns(rect, sizing);
            assert!(
                (after.mtime.left() - pointer.x).abs() < 0.01,
                "divider must track pointer"
            );
            assert!((after.name.width() - initial.name.width()).abs() < 0.01);
        }
        let end = start + egui::vec2(15.0, 0.0);
        run(&mut sizing, vec![button(end, false)]);
        // Begin a separate double-click at the Size/Name boundary.
        let edge = egui::pos2(columns(rect, sizing).size.left(), rect.center().y);
        run(
            &mut sizing,
            vec![egui::Event::PointerMoved(edge), button(edge, true)],
        );
        run(&mut sizing, vec![button(edge, false)]);
        run(&mut sizing, vec![button(edge, true)]);
        let (_, action) = run(&mut sizing, vec![button(edge, false)]);
        assert!(matches!(action, Some(HeaderAction::Resize(_))));
        assert_eq!(sizing.requested[0], 0.0);
        assert_eq!(columns(rect, sizing).size.width(), sizing.content[0]);
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
    fn display_preferences_change_sizes_and_dates() {
        use fm_engine::{DateFormat, SizeUnits};
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(13 * 3600 + 5 * 60 + 9);
        set_display(SizeUnits::Decimal, DateFormat::Us);
        assert_eq!(human_size(2_000_000), "2.0 MB");
        assert_eq!(format_mtime(t), "01/01/1970 1:05 PM");
        set_display(SizeUnits::Bytes, DateFormat::IsoSeconds);
        assert_eq!(human_size(2048), "2048 B");
        assert_eq!(format_mtime(t), "1970-01-01 13:05:09");
        set_display(SizeUnits::Binary, DateFormat::Eu);
        assert_eq!(format_mtime(t), "01/01/1970 13:05");
        set_display(SizeUnits::Binary, DateFormat::Iso);
    }

    #[test]
    fn mtime_formatting_is_stable() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400);
        assert_eq!(format_mtime(t), "1970-01-02 00:00");
        let future = SystemTime::UNIX_EPOCH + Duration::from_secs(2_145_916_800);
        assert!(format_mtime(future).starts_with("2038"));
        let before = SystemTime::UNIX_EPOCH - Duration::from_secs(86_400);
        assert_eq!(format_mtime(before), "1969-12-31 00:00");
        let out_of_range = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(400_000_000_000))
            .expect("system time");
        assert!(format_mtime(out_of_range).starts_with("Invalid file time:"));
    }

    #[test]
    fn file_times_use_the_dates_dst_offset_not_the_current_offset() {
        let zone = jiff::tz::TimeZone::get("Europe/Amsterdam").expect("timezone");
        set_time_zone(&zone);
        let format = |raw: &str| {
            let timestamp: jiff::Timestamp = raw.parse().expect("timestamp");
            format_mtime(timestamp.into())
        };
        assert_eq!(format("2026-01-15T12:00:00Z"), "2026-01-15 13:00");
        assert_eq!(format("2026-07-15T12:00:00Z"), "2026-07-15 14:00");
        assert_eq!(format("2026-03-29T00:59:00Z"), "2026-03-29 01:59");
        assert_eq!(format("2026-03-29T01:00:00Z"), "2026-03-29 03:00");
        assert_eq!(format("2026-10-25T00:59:00Z"), "2026-10-25 02:59");
        assert_eq!(format("2026-10-25T01:00:00Z"), "2026-10-25 02:00");
        assert_eq!(format("2026-12-31T23:30:00Z"), "2027-01-01 00:30");
        set_time_zone(&jiff::tz::TimeZone::UTC);
    }

    #[test]
    fn local_times_keep_date_preferences_and_support_fractional_offsets() {
        use fm_engine::{DateFormat, SizeUnits};
        let timestamp: jiff::Timestamp = "2026-10-29T20:30:09Z".parse().expect("timestamp");
        let t = timestamp.into();
        set_time_zone(&jiff::tz::TimeZone::get("Asia/Kathmandu").expect("timezone"));
        for (style, expected) in [
            (DateFormat::Iso, "2026-10-30 02:15"),
            (DateFormat::IsoSeconds, "2026-10-30 02:15:09"),
            (DateFormat::Eu, "30/10/2026 02:15"),
            (DateFormat::Us, "10/30/2026 2:15 AM"),
        ] {
            set_display(SizeUnits::Binary, style);
            assert_eq!(format_mtime(t), expected);
        }
        set_display(SizeUnits::Binary, DateFormat::Iso);
        set_time_zone(&jiff::tz::TimeZone::get("America/New_York").expect("timezone"));
        let midnight: jiff::Timestamp = "2026-01-01T00:30:00Z".parse().expect("timestamp");
        assert_eq!(format_mtime(midnight.into()), "2025-12-31 19:30");
        set_time_zone(&jiff::tz::TimeZone::UTC);
    }

    #[test]
    fn listing_paints_local_times_without_changing_cached_timestamps() {
        let timestamp: jiff::Timestamp = "2026-07-15T12:00:00Z".parse().expect("timestamp");
        let modified = timestamp.into();
        for tokens in [Tokens::dark(), Tokens::light()] {
            let (mut s, uri) = ready_session();
            let mut entries = s.dirs.entries(&uri).expect("entries").to_vec();
            for entry in &mut entries {
                entry.modified = Some(modified);
            }
            s.dirs.refresh(&uri, entries.into()).expect("new revision");
            set_time_zone(&jiff::tz::TimeZone::get("Europe/Amsterdam").expect("timezone"));
            let ctx = egui::Context::default();
            crate::theme::apply_to(&ctx, &tokens);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        show(ui, &mut s, 1, &tokens, true);
                    });
                },
            );
            assert!(output.shapes.iter().any(|shape| matches!(
                &shape.shape, egui::Shape::Text(text) if text.galley.job.text == "2026-07-15 14:00"
            )), "listing must paint local time");
            assert!(
                s.dirs
                    .entries(&uri)
                    .expect("entries")
                    .iter()
                    .all(|e| e.modified == Some(modified))
            );
            output.textures_delta.clear();
            set_time_zone(&jiff::tz::TimeZone::UTC);
        }
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
