//! Application shell (the product name comes from `fm_brand::APP_NAME`): eframe
//! window, menus, shortcuts, pane tree, status bar.
//!
//! The shell owns a [`fm_engine::Session`] and routes every interaction through
//! `Session::dispatch` — it performs no filesystem work itself.

mod menu;

use std::path::PathBuf;

use eframe::egui;

use fm_engine::{Params, Session, ThemePreference};
use fm_layout::{Axis, PaneNode};
use fm_ui::theme;
use fm_vfs::VfsUri;

struct App {
    session: Session,
    config_dir: Option<PathBuf>,
    /// `(pref, dark)` currently applied to egui; re-applied when either changes.
    applied: (ThemePreference, bool),
    /// Set once the native window title has been explicitly sent (see `ui`).
    title_sent: bool,
    dialogs: fm_ui::dialogs::Dialogs,
}

impl App {
    /// Re-apply tokens whenever the resolved palette (preference × OS theme) changes.
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let pref = self.session.prefs.theme;
        // Tell egui the preference, then read back the theme *egui* resolved
        // (`System` → OS theme, falling back to egui's `fallback_theme` when the
        // platform doesn't report one). Our tokens must track egui's own
        // resolution, or the panels (styled by egui visuals) and the pane bodies
        // (painted from tokens) drift apart.
        set_egui_theme_preference(ctx, pref);
        let dark = ctx.theme() == egui::Theme::Dark;
        if (pref, dark) == self.applied {
            return;
        }
        self.applied = (pref, dark);
        theme::apply_to(ctx, &theme::Tokens::resolve(pref, dark));
    }
}

fn set_egui_theme_preference(ctx: &egui::Context, pref: ThemePreference) {
    ctx.set_theme(match pref {
        ThemePreference::System => egui::ThemePreference::System,
        ThemePreference::Dark => egui::ThemePreference::Dark,
        ThemePreference::Light => egui::ThemePreference::Light,
    });
}

/// Navigate a pane to `uri` through the engine (`nav.open`).
fn open_uri(session: &mut Session, uri: &VfsUri) {
    if let Err(err) = session.dispatch("nav.open", &Params::String(uri.path_str().to_string())) {
        eprintln!("nav.open failed: {err}");
    }
}

/// A split handle: resize cursor, accent on hover/drag, double-click resets to the equal split.
#[allow(clippy::too_many_arguments)]
fn divider(
    ui: &mut egui::Ui,
    session: &mut Session,
    handle: Option<fm_layout::PaneId>,
    axis: Axis,
    ratio: f32,
    size: egui::Vec2,
    total: f32,
    tokens: &theme::Tokens,
) {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let active = resp.hovered() || resp.dragged();
    if active {
        ui.ctx().set_cursor_icon(match axis {
            Axis::Horizontal => egui::CursorIcon::ResizeHorizontal,
            Axis::Vertical => egui::CursorIcon::ResizeVertical,
        });
    }
    ui.painter().rect_filled(
        rect,
        0.0,
        if active { tokens.accent } else { tokens.border },
    );
    let Some(handle) = handle else {
        return;
    };
    if resp.double_clicked() {
        if let Some(r) = session.layout.equal_ratio(handle) {
            session.layout.set_ratio(handle, r);
        }
    } else if resp.dragged() {
        let d = resp.drag_delta();
        let delta = match axis {
            Axis::Horizontal => d.x,
            Axis::Vertical => d.y,
        };
        session
            .layout
            .set_ratio(handle, ratio + delta / total.max(1.0));
    }
}

/// Render the pane tree recursively. `Split` nodes lay out two children along
/// their axis with a drag handle that writes the ratio back into the layout.
fn render_pane_node(
    ui: &mut egui::Ui,
    session: &mut Session,
    node: &PaneNode,
    tokens: &theme::Tokens,
) -> Option<VfsUri> {
    match node {
        PaneNode::Leaf(id) => fm_ui::pane::show(ui, session, *id, tokens),
        PaneNode::Split { .. } if session.layout.zoomed.is_some() => {
            let zoomed = session.layout.zoomed?;
            fm_ui::pane::show(ui, session, zoomed, tokens)
        }
        PaneNode::Split { axis, ratio, a, b } => {
            let handle = a.panes().first().copied();
            let mut open = None;
            match axis {
                Axis::Horizontal => {
                    // `ui.horizontal` only starts one interact-size tall; the
                    // row must be told to take the full available height or
                    // both panes collapse to a strip.
                    ui.horizontal_top(|ui| {
                        let total = ui.available_width();
                        let handle_w = 6.0;
                        let a_w = ((total - handle_w) * ratio).clamp(80.0, total - handle_w - 80.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(a_w, ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                if let Some(target) = render_pane_node(ui, session, a, tokens) {
                                    open = Some(target);
                                }
                            },
                        );
                        divider(
                            ui,
                            session,
                            handle,
                            *axis,
                            *ratio,
                            egui::vec2(handle_w, ui.available_height()),
                            total,
                            tokens,
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                if let Some(target) = render_pane_node(ui, session, b, tokens) {
                                    open = Some(target);
                                }
                            },
                        );
                    });
                }
                Axis::Vertical => {
                    ui.vertical(|ui| {
                        let total = ui.available_height();
                        let handle_w = 6.0;
                        let a_h = ((total - handle_w) * ratio).clamp(60.0, total - handle_w - 60.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), a_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                if let Some(target) = render_pane_node(ui, session, a, tokens) {
                                    open = Some(target);
                                }
                            },
                        );
                        divider(
                            ui,
                            session,
                            handle,
                            *axis,
                            *ratio,
                            egui::vec2(ui.available_width(), handle_w),
                            total,
                            tokens,
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                if let Some(target) = render_pane_node(ui, session, b, tokens) {
                                    open = Some(target);
                                }
                            },
                        );
                    });
                }
            }
            open
        }
    }
}

/// Files dropped from the OS onto the window are copied into the pane under the pointer
/// (the focused pane when the pointer position is unknown). Paths are untrusted: the engine
/// rejects bad ones individually and reports the count.
fn import_os_drops(ctx: &egui::Context, session: &mut Session) {
    let paths: Vec<String> = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path())
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    });
    if paths.is_empty() {
        return;
    }
    let pane = ctx
        .input(|i| i.pointer.latest_pos())
        .and_then(|pos| fm_ui::dnd::pane_at(ctx, pos))
        .unwrap_or_else(|| session.layout.focused());
    let _ = session.layout.focus_pane(pane);
    if session.pane_kind(pane) == Some(fm_layout::PaneKind::Terminal) {
        session.dispatch_or_notice(
            "term.drop_paths",
            &serde_json::json!({"paths": paths, "pane": pane}),
        );
        return;
    }
    let Some(dest) = session.layout.state(pane).map(|p| p.location.to_string()) else {
        return;
    };
    session.dispatch_or_notice(
        "dnd.drop",
        &serde_json::json!({"sources": paths, "dest": dest, "origin": "external"}),
    );
}

/// winit's Wayland backend cannot receive file drops from other apps, so on Linux we use
/// XWayland (XDND) when an X display exists. `FM_WAYLAND=1` opts back into native Wayland.
#[cfg(target_os = "linux")]
fn prefer_x11() -> Option<eframe::EventLoopBuilderHook> {
    use winit::platform::x11::EventLoopBuilderExtX11;
    let has_x = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
    if !has_x || std::env::var_os("FM_WAYLAND").is_some() {
        return None;
    }
    Some(Box::new(|builder| {
        builder.with_x11();
    }))
}

#[cfg(not(target_os = "linux"))]
fn prefer_x11() -> Option<eframe::EventLoopBuilderHook> {
    None
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Shortcuts run before any widget renders: `Tab` drives egui's focus
        // traversal, which is committed at `begin_pass`; `consume_shortcuts`
        // cancels it before the menu/tree/listing can act on it.
        menu::consume_shortcuts(&ctx, &mut self.session, &mut self.dialogs);
        import_os_drops(&ctx, &mut self.session);
        self.sync_theme(&ctx);
        let _ = self.session.poll();
        // egui only repaints on input; keep frames flowing while jobs run.
        if self.session.is_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        // Shell output arrives without any input event; poll terminals at ~60 Hz while any exist.
        if self.session.has_terminals() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        if let Some(text) = self.session.clipboard_out.take() {
            ctx.copy_text(text);
        }

        // The title is set on the native window at creation, but some
        // compositors only show it after an explicit `set_title`. Send it once
        // after the first frame so the title bar is never empty.
        if !self.title_sent {
            self.title_sent = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(fm_brand::APP_NAME.to_string()));
        }

        egui::Panel::top("menu_bar").show(ui, |ui| {
            menu::menu_bar(ui, &mut self.session, &mut self.dialogs);
        });

        let tokens = theme::Tokens::resolve(self.session.prefs.theme, self.applied.1);
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            let focused = self.session.layout.focused();
            let location = self
                .session
                .layout
                .state(focused)
                .map(|p| p.location.path_str().to_string())
                .unwrap_or_default();
            let (count, selected) = {
                let state = self.session.layout.state(focused);
                let selected = state.map(|p| p.listing.selected_count()).unwrap_or(0);
                let count = self
                    .session
                    .pane_view(focused)
                    .map(|(_, ix)| ix.order.len())
                    .unwrap_or(0);
                (count, selected)
            };
            ui.horizontal(|ui| {
                if let Some(notice) = self.session.notice.clone() {
                    let text = egui::RichText::new(notice)
                        .color(tokens.error)
                        .font(tokens.font());
                    if ui.add(egui::Button::new(text).frame(false)).clicked() {
                        self.session.notice = None;
                    }
                }
                fm_ui::jobs::indicator(ui, &self.session, &tokens);
                ui.label(tokens.muted(location));
                ui.label(tokens.muted(format!("{count} items")));
                if selected > 0 {
                    ui.label(tokens.muted(format!("{selected} selected")));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.session.show_hidden {
                        "Hidden: shown (Ctrl+H)"
                    } else {
                        "Hidden: off (Ctrl+H)"
                    };
                    if ui
                        .add(egui::Button::new(tokens.muted(label)).frame(false))
                        .on_hover_text("Show or hide hidden files")
                        .clicked()
                    {
                        self.session
                            .dispatch_or_notice("view.hidden.toggle", &fm_engine::Params::Null);
                    }
                    ui.label(tokens.muted(fm_brand::APP_NAME));
                    ui.label(tokens.muted(format!(
                        "Theme: {}",
                        match self.session.prefs.theme {
                            ThemePreference::System => "system",
                            ThemePreference::Dark => "dark",
                            ThemePreference::Light => "light",
                        }
                    )));
                });
            });
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let root = self.session.layout.root.clone();
                if let Some(uri) = render_pane_node(ui, &mut self.session, &root, &tokens) {
                    open_uri(&mut self.session, &uri);
                }
            });

        self.dialogs.show(&ctx, &mut self.session, &tokens);
        fm_ui::jobs::show(&ctx, &mut self.session, &tokens);
        fm_ui::preferences::show(&ctx, &mut self.session, &tokens);

        if self.session.about_open {
            let mut open = self.session.about_open;
            egui::Window::new("About")
                .collapsible(false)
                .movable(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .open(&mut open)
                .show(ui, |ui| {
                    ui.heading(fm_brand::APP_NAME);
                    ui.label(format!("version {}", env!("CARGO_PKG_VERSION")));
                    ui.label(fm_brand::ORG_NAME);
                });
            self.session.about_open = open;
        }

        if !self.session.running {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(dir) = &self.config_dir
            && let Err(err) = fm_engine::prefs::save(dir, &self.session.prefs)
        {
            eprintln!("failed to save preferences: {err}");
        }
    }
}

fn main() -> eframe::Result {
    let config_dir = fm_engine::prefs::default_config_dir();
    let prefs = config_dir
        .as_ref()
        .map(|dir| fm_engine::prefs::load(&dir.join(fm_engine::prefs::PREFS_FILE)))
        .unwrap_or_default();

    let mut session = match Session::new() {
        Ok(session) => session,
        Err(err) => {
            eprintln!("failed to start engine: {err}");
            std::process::exit(1);
        }
    };
    session.prefs = prefs;
    // Kick off the first pane's read so the UI starts in the loading state
    // and the first paint never blocks on I/O.
    if let Some(state) = session.layout.state(session.layout.focused()) {
        let location = state.location.clone();
        if let Err(err) = session.ensure_read(location) {
            eprintln!("startup read failed: {err}");
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(fm_brand::APP_NAME)
            .with_app_id(fm_brand::APP_ID)
            .with_inner_size(egui::vec2(1024.0, 700.0))
            .with_drag_and_drop(true)
            .with_min_inner_size(egui::vec2(640.0, 400.0)),
        event_loop_builder: prefer_x11(),
        ..Default::default()
    };

    eframe::run_native(
        fm_brand::APP_NAME,
        options,
        Box::new(move |cc| {
            // Seed the theme before the first frame; `sync_theme` keeps it current.
            let theme_pref = session.prefs.theme;
            set_egui_theme_preference(&cc.egui_ctx, theme_pref);
            let dark = cc.egui_ctx.theme() == egui::Theme::Dark;
            theme::apply_to(&cc.egui_ctx, &theme::Tokens::resolve(theme_pref, dark));
            Ok(Box::new(App {
                session,
                config_dir,
                applied: (theme_pref, dark),
                title_sent: false,
                dialogs: fm_ui::dialogs::Dialogs::default(),
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_name_matches_brand_executable() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read manifest");
        assert!(
            manifest.contains(&format!("name = \"{}\"", fm_brand::EXECUTABLE)),
            "Cargo.toml [[bin]] name must equal brand::EXECUTABLE"
        );
    }

    #[test]
    fn session_starts_with_persisted_theme() {
        let mut session = Session::new().expect("session");
        session.prefs.theme = ThemePreference::Dark;
        // The shell would persist on exit; simulate a save/load round trip.
        let dir = std::env::temp_dir().join(format!("fm-app-test-{}", std::process::id()));
        fm_engine::prefs::save(&dir, &session.prefs).expect("save");
        let loaded = fm_engine::prefs::load(&dir.join(fm_engine::prefs::PREFS_FILE));
        assert_eq!(loaded.theme, ThemePreference::Dark);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quit_command_stops_the_app_flag() {
        let mut session = Session::new().expect("session");
        session.dispatch(menu::QUIT, &Params::Null).expect("quit");
        assert!(!session.running);
    }

    /// Render the pane tree headlessly and return the solid fill rects that
    /// cover the central area (pane bodies + drag handles).
    fn render_split_fills(cmd: &str) -> (egui::Rect, Vec<egui::Rect>) {
        let mut session = Session::new().expect("session");
        session.dispatch(cmd, &Params::Null).expect("split");
        let tokens = theme::Tokens::resolve(ThemePreference::Dark, true);
        let ctx = egui::Context::default();
        let mut central = egui::Rect::NOTHING;
        let mut fills = Vec::new();
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
                        central = ui.max_rect();
                        let root = session.layout.root.clone();
                        let _ = render_pane_node(ui, &mut session, &root, &tokens);
                    });
            },
        );
        output.textures_delta.clear();
        for shape in &output.shapes {
            if let egui::epaint::Shape::Rect(r) = &shape.shape
                && r.fill.a() > 0
            {
                fills.push(r.rect);
            }
        }
        (central, fills)
    }

    /// Each pane body must span (nearly) the full cross-axis of the central
    /// area — both for horizontal (left|right) and vertical (top/bottom)
    /// splits. Guards the `horizontal_top` fix: plain `ui.horizontal` gives
    /// the row only `interact_size.y` of height, leaving most of the window
    /// empty.
    #[test]
    fn split_panes_fill_the_central_area() {
        for cmd in ["pane.split.horizontal", "pane.split.vertical"] {
            let (central, fills) = render_split_fills(cmd);
            // A pane body is one of the few large fills: tree/listing rows, the
            // sort header and the 6px drag handles are all far smaller, so size
            // is what distinguishes them from an actual pane.
            let pane_bodies: Vec<egui::Rect> = fills
                .iter()
                .copied()
                .filter(|r| {
                    r.width() >= central.width() * 0.1 && r.height() >= central.height() * 0.1
                })
                .collect();
            assert!(
                pane_bodies.len() >= 2,
                "{cmd}: expected two pane bodies, got {}",
                pane_bodies.len()
            );
            for body in &pane_bodies {
                if cmd.ends_with("horizontal") {
                    assert!(
                        body.height() >= central.height() * 0.9,
                        "{cmd}: pane body {body:?} does not fill central height {}",
                        central.height()
                    );
                } else {
                    assert!(
                        body.width() >= central.width() * 0.9,
                        "{cmd}: pane body {body:?} does not fill central width {}",
                        central.width()
                    );
                }
            }
        }
    }

    /// Opens a real window; ignored in CI. Manual checklist lives in
    /// `docs/development.md`.
    #[test]
    #[ignore = "opens a real window — run manually per docs/development.md"]
    fn window_launches_and_closes() {
        struct QuitSoon {
            frames: u8,
        }
        impl eframe::App for QuitSoon {
            fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
                self.frames = self.frames.saturating_add(1);
                if self.frames > 5 {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("fm-scaffold-smoke")
                .with_inner_size(egui::vec2(400.0, 300.0)),
            ..Default::default()
        };
        let _ = eframe::run_native(
            "fm-scaffold-smoke",
            options,
            Box::new(|_| Ok(Box::new(QuitSoon { frames: 0 }))),
        );
    }
}
