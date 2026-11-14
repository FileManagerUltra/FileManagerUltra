//! Application shell (the product name comes from `fm_brand::APP_NAME`): eframe
//! window, menus, shortcuts, pane tree, status bar.
//!
//! The shell owns a [`fm_engine::Session`] and routes every interaction through
//! `Session::dispatch` — it performs no filesystem work itself.

// Release builds on Windows use the GUI subsystem, so no console window opens.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod dragout;
mod media_controls;
mod menu;
mod renderer;
mod window;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

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
    /// Tokens currently applied (accent, density and sizes included).
    look: theme::Tokens,
    /// Set once the native window title has been explicitly sent (see `ui`).
    title_sent: bool,
    was_focused: bool,
    /// Native fullscreen state last sent to the window (`window.fullscreen`).
    window_fullscreen_sent: bool,
    dialogs: fm_ui::dialogs::Dialogs,
    last_ws_tick: std::time::Instant,
    window: window::Tracker,
    out_drag: dragout::OutDrag,
    /// MPRIS / SMTC / Now Playing (`media-integration`).
    media_controls: media_controls::MediaControls,
    /// Why frames were drawn since the last diagnostic sample.
    diag_frames: DiagFrames,
}

/// Per-second tally of why frames were drawn (diagnostic log only).
#[derive(Default)]
struct DiagFrames {
    causes: std::collections::HashMap<String, u32>,
    input: u32,
    other: u32,
}

/// `crates/ui/src/media.rs` (or a registry path with `\`) → `media.rs`.
fn short_source(file: &str) -> &str {
    file.rsplit(['/', '\\']).next().unwrap_or(file)
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
        let zoom = self.session.prefs.zoom_factor();
        if (ctx.zoom_factor() - zoom).abs() > f32::EPSILON {
            ctx.set_zoom_factor(zoom);
        }
        let dark = ctx.theme() == egui::Theme::Dark;
        fm_ui::listing::set_display(
            self.session.prefs.size_units,
            self.session.prefs.date_format,
        );
        fm_ui::listing::set_time_zone(&self.session.file_time_zone);
        let look = theme::Tokens::resolve_with(&self.session.prefs, dark);
        if (pref, dark) == self.applied && look == self.look {
            return;
        }
        self.applied = (pref, dark);
        self.look = look;
        theme::apply_to(ctx, &look);
    }
}

fn set_egui_theme_preference(ctx: &egui::Context, pref: ThemePreference) {
    ctx.set_theme(match pref {
        ThemePreference::System => egui::ThemePreference::System,
        ThemePreference::Dark => egui::ThemePreference::Dark,
        ThemePreference::Light => egui::ThemePreference::Light,
    });
}

/// Activate `uri` (double click / Enter): folders navigate, documents open in the default
/// application, programs ask first. All through engine commands.
fn open_uri(session: &mut Session, dialogs: &mut fm_ui::dialogs::Dialogs, uri: &VfsUri) {
    use fm_engine::open_cmds::{OpenPlan, plan};
    match plan(session, uri) {
        OpenPlan::Navigate => {
            session.dispatch_or_notice("nav.open", &Params::String(uri.to_string()));
        }
        OpenPlan::Launch => {
            session.dispatch_or_notice("file.open", &serde_json::json!({ "uri": uri.to_string() }));
        }
        OpenPlan::PlayMedia => {
            session.dispatch_or_notice(
                "media.play",
                &serde_json::json!({ "uris": [uri.to_string()] }),
            );
        }
        OpenPlan::ConfirmRun { name } => dialogs.request(fm_ui::dialogs::Dialog::ConfirmRun {
            uri: uri.clone(),
            name,
        }),
    }
}

/// A split handle: resize cursor, accent on hover/drag, double-click resets to the equal split.
#[allow(clippy::too_many_arguments)]
fn divider(
    ui: &mut egui::Ui,
    session: &mut Session,
    handle: Option<(fm_layout::PaneId, fm_layout::PaneId)>,
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
    let Some((first, second)) = handle else {
        return;
    };
    if resp.double_clicked() {
        if let Some(r) = session.layout.equal_ratio_at(first, second) {
            session.layout.set_ratio_at(first, second, r);
        }
    } else if resp.dragged() {
        let d = resp.drag_delta();
        let delta = match axis {
            Axis::Horizontal => d.x,
            Axis::Vertical => d.y,
        };
        session
            .layout
            .set_ratio_at(first, second, ratio + delta / total.max(1.0));
    }
}

/// Render the pane tree recursively. `Split` nodes lay out two children along
/// their axis with a drag handle that writes the ratio back into the layout.
/// A pane's slot is a fixed rectangle: content that wants more room is clipped, never allowed
/// to grow the split and push its neighbours under the status bar.
fn slot(ui: &mut egui::Ui, size: egui::Vec2, add: impl FnOnce(&mut egui::Ui)) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child.set_min_size(rect.size());
    add(&mut child);
}

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
            let handle = a.panes().first().copied().zip(b.panes().first().copied());
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
                        slot(ui, egui::vec2(a_w, ui.available_height()), |ui| {
                            if let Some(target) = render_pane_node(ui, session, a, tokens) {
                                open = Some(target);
                            }
                        });
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
                        slot(
                            ui,
                            egui::vec2(ui.available_width(), ui.available_height()),
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
                        slot(ui, egui::vec2(ui.available_width(), a_h), |ui| {
                            if let Some(target) = render_pane_node(ui, session, a, tokens) {
                                open = Some(target);
                            }
                        });
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
                        slot(
                            ui,
                            egui::vec2(ui.available_width(), ui.available_height()),
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
    import_paths(ctx, session, paths, None);
}

/// `at` overrides the pointer position (Wayland reports it with the drop).
fn import_paths(
    ctx: &egui::Context,
    session: &mut Session,
    paths: Vec<String>,
    at: Option<egui::Pos2>,
) {
    if paths.is_empty() {
        return;
    }
    let pane = at
        .or_else(|| ctx.input(|i| i.pointer.latest_pos()))
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
    if session.pane_kind(pane) == Some(fm_layout::PaneKind::Media) {
        // On the player the files play at once; on the playlist they join it where dropped.
        let mut params = at
            .or_else(|| ctx.input(|i| i.pointer.latest_pos()))
            .map_or_else(
                || serde_json::json!({}),
                |pos| fm_ui::media::external_drop_params(ctx, pos),
            );
        if let Some(map) = params.as_object_mut() {
            map.insert("uris".into(), serde_json::json!(paths));
        }
        session.dispatch_or_notice("media.drop", &params);
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

/// Native Wayland is the default on a Wayland session: drag and drop with other apps runs
/// through `fm-wayland`. `FM_X11=1` forces XWayland (XDND) when an X display exists.
#[cfg(target_os = "linux")]
fn prefer_x11() -> Option<eframe::EventLoopBuilderHook> {
    use winit::platform::x11::EventLoopBuilderExtX11;
    let has_x = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
    if !has_x || std::env::var_os("FM_X11").is_none() {
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

impl App {
    /// Write `workspaces.json` when the saved set changed (checked about once a second).
    fn persist_workspaces(&mut self, ctx: &egui::Context, force: bool) {
        let due = force || self.last_ws_tick.elapsed() >= std::time::Duration::from_secs(1);
        if !due {
            return;
        }
        self.last_ws_tick = std::time::Instant::now();
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let Some(text) = self.session.workspace_tick() else {
            return;
        };
        let Some(dir) = &self.config_dir else {
            return;
        };
        if let Err(err) = fm_engine::workspace::save_file(dir, &text) {
            eprintln!("failed to save workspaces: {err}");
            self.session.workspace_retry();
        }
    }
}

impl eframe::App for App {
    // eframe's default is near-black, which shows as dark bars around panels in light mode.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.out_drag.raw_input(raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.window.reposition(&ctx);
        window::capture(&ctx, &mut self.session.prefs);
        if self.window.save_due(&ctx, &self.session.prefs)
            && let Some(dir) = &self.config_dir
            && let Err(err) = fm_engine::prefs::save(dir, &self.session.prefs)
        {
            eprintln!("failed to save preferences: {err}");
        }
        // Shortcuts run before any widget renders: `Tab` drives egui's focus
        // traversal, which is committed at `begin_pass`; `consume_shortcuts`
        // cancels it before the menu/tree/listing can act on it.
        menu::consume_shortcuts(&ctx, &mut self.session, &mut self.dialogs);
        import_os_drops(&ctx, &mut self.session);
        for drop in self.out_drag.update(&ctx, &mut self.session) {
            let paths = drop
                .paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            import_paths(&ctx, &mut self.session, paths, Some(drop.pos));
        }
        self.sync_theme(&ctx);
        let _ = self.session.poll();
        let events_ctx = ctx.clone();
        self.media_controls.tick(&mut self.session, &mut |events| {
            media_controls::Souvlaki::connect(
                media_controls::window_handle(frame),
                events_ctx.clone(),
                events,
            )
        });
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        self.session.devices_tick(focused);
        ctx.request_repaint_after(if self.session.volumes.scanning() {
            std::time::Duration::from_millis(100)
        } else {
            fm_engine::devices::SCAN_INTERVAL
        });
        // egui only repaints on input; keep frames flowing while jobs run.
        if self.session.is_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        // Discovery results arrive without input: keep frames flowing while Network shows.
        if self.session.shows_network() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        // Remote panes refresh on a timer and when the window regains focus.
        if self.session.remote.has_ready() {
            ctx.request_repaint_after(self.session.prefs.remote_poll());
            let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
            if focused && !self.was_focused {
                self.session.refresh_remote_panes();
            }
            self.was_focused = focused;
        }
        // Shell output arrives without any input event; poll terminals at ~60 Hz while any exist.
        if self.session.has_terminals() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        if let Some(text) = self.session.clipboard_out.take() {
            ctx.copy_text(text);
        }
        if let Some(img) = self.session.clipboard_image.take() {
            ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied(
                [img.width, img.height],
                &img.rgba,
            ));
        }

        // The title is set on the native window at creation, but some
        // compositors only show it after an explicit `set_title`. Send it once
        // after the first frame so the title bar is never empty.
        if !self.title_sent {
            self.title_sent = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(fm_brand::APP_NAME.to_string()));
        }

        if self.session.window_fullscreen != self.window_fullscreen_sent {
            self.window_fullscreen_sent = self.session.window_fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                self.window_fullscreen_sent,
            ));
        }
        // Media fullscreen shows only the Media pane; chrome and other panes keep their state.
        let media_fullscreen = self
            .session
            .media_fullscreen_active()
            .then(|| self.session.media_pane())
            .flatten();
        if media_fullscreen.is_none() {
            egui::Panel::top("menu_bar").show(ui, |ui| {
                menu::menu_bar(ui, &mut self.session, &mut self.dialogs);
            });
            let tab_tokens = self.look;
            egui::Panel::top("workspace_tabs").show(ui, |ui| {
                fm_ui::workspaces::tab_bar(ui, &mut self.session, &tab_tokens, &mut self.dialogs);
            });
        }
        self.persist_workspaces(&ctx, false);

        let tokens = self.look;
        let mut status_bar = media_fullscreen.is_none();
        egui::Panel::bottom("status_bar").show_collapsible(ui, &mut status_bar, |ui| {
            let focused = self.session.layout.focused();
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
            let space = self
                .session
                .layout
                .state(focused)
                .map(|p| p.location.clone())
                .and_then(|uri| fm_ui::remote::free_space_label(&mut self.session, &uri));
            let branch = self
                .session
                .layout
                .state(focused)
                .map(|p| p.location.clone())
                .and_then(|uri| fm_ui::git_marks::branch_label(&mut self.session, &uri));
            ui.horizontal(|ui| {
                if let Some((msg, at)) = self.session.info.clone() {
                    const SHOWN: std::time::Duration = std::time::Duration::from_secs(3);
                    let left = SHOWN.saturating_sub(at.elapsed());
                    if !left.is_zero() {
                        ui.label(
                            egui::RichText::new(msg)
                                .color(tokens.success)
                                .font(tokens.font()),
                        );
                        ctx.request_repaint_after(left);
                    } else {
                        self.session.info = None;
                    }
                }
                ui.label(tokens.muted(format!("{count} items")));
                if let Some(space) = space {
                    ui.label(tokens.muted(space));
                }
                if selected > 0 {
                    ui.label(tokens.muted(format!("{selected} selected")));
                }
                if let Some(branch) = branch {
                    ui.label(tokens.muted(branch));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Right to left: the jobs button, its progress text, then media controls.
                    fm_ui::jobs::icon_button(ui, &self.session, &tokens);
                    fm_ui::jobs::indicator(ui, &self.session, &tokens);
                    fm_ui::media::indicator(ui, &mut self.session, &tokens);
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
                });
            });
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                if let Some(pane) = media_fullscreen {
                    fm_ui::media::show(ui, &mut self.session, pane, &tokens);
                } else {
                    let root = self.session.layout.root.clone();
                    if let Some(uri) = render_pane_node(ui, &mut self.session, &root, &tokens) {
                        open_uri(&mut self.session, &mut self.dialogs, &uri);
                    }
                }
                if let Some(id) = fm_ui::context_menu::take_command(ui.ctx()) {
                    menu::dispatch(&mut self.session, &mut self.dialogs, id);
                }
                if let Some(action) = fm_ui::context_menu::take_folder_action(ui.ctx()) {
                    menu::dispatch_folder(&mut self.session, &mut self.dialogs, action);
                }
                if let Some(text) = self.session.clipboard_outbox.take() {
                    ui.ctx().copy_text(text);
                }
            });

        self.dialogs.show(&ctx, &mut self.session, &tokens);
        fm_ui::jobs::show(&ctx, &mut self.session, &tokens);
        fm_ui::preferences::show(&ctx, &mut self.session, &tokens);
        fm_ui::properties::show(&ctx, &mut self.session, &tokens);
        fm_ui::search::show(&ctx, &mut self.session, &tokens);
        fm_ui::open_with::show(&ctx, &mut self.session, &tokens);
        fm_ui::ffmpeg_prompt::show(&ctx, &mut self.session, &tokens);
        fm_ui::compare_with::show(&ctx, &mut self.session, &tokens);
        fm_ui::remote::host_key_dialog(&ctx, &mut self.session, &tokens);
        fm_ui::remote::login_dialog(&ctx, &mut self.session, &tokens);
        fm_ui::sftp_popup::sftp_prompt_dialog(&ctx, &mut self.session, &tokens);
        fm_ui::remote::host_key_alert(&ctx, &mut self.session, &tokens);

        if self.session.about_open {
            let mut open = self.session.about_open;
            fm_ui::dialog_style::window(&ctx, &tokens, "About", &mut open).show(ui, |ui| {
                fm_ui::dialog_style::apply(ui, &tokens);
                ui.vertical_centered(|ui| {
                    if let Some(logo) = logo_texture(&ctx) {
                        ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(96.0, 96.0)));
                    }
                    ui.heading(fm_brand::APP_NAME);
                    ui.label(format!("version {}", env!("CARGO_PKG_VERSION")));
                    ui.label(format!("build {}", fm_engine::diag::build_commit()));
                    ui.label(fm_brand::ORG_NAME);
                });
            });
            self.session.about_open = open;
        }

        if !self.session.running {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.session.unsaved_prompt.is_some() {
            ctx.request_repaint();
        }
        // Textures and pictures of panes that were not drawn this frame (closed workspaces
        // and panes, hidden tabs) are released.
        fm_ui::frame_cache::sweep(&ctx);
        diagnostics_frame(&ctx, &self.session, &mut self.diag_frames);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(err) = self.session.media_store_finish() {
            eprintln!("failed to save media queue: {err}");
        }
        if let Err(err) = fm_engine::video_cmds::shutdown_players() {
            eprintln!("failed to stop video playback: {err}");
        }
        fm_engine::open_cmds::sweep_open_temp(true);
        self.persist_workspaces(&egui::Context::default(), true);
        if let Some(dir) = &self.config_dir
            && let Err(err) = fm_engine::prefs::save(dir, &self.session.prefs)
        {
            eprintln!("failed to save preferences: {err}");
        }
        // Flushes and closes the diagnostic log (a no-op while it is off).
        fm_engine::diag::stop();
    }
}

/// Diagnostic log bookkeeping once per frame (capability `diagnostics`); an atomic load
/// while the log is off.
fn diagnostics_frame(ctx: &egui::Context, session: &Session, tally: &mut DiagFrames) {
    use fm_engine::diag;
    // The graphics stack reports its driver while the window opens; after that, debug
    // records are not needed unless the log is on.
    if ctx.cumulative_frame_nr() == 10 {
        diag::startup_done();
    }
    if !diag::enabled() {
        return;
    }
    diag::count(diag::Counter::Frames);
    // Causes of the repaint that produced this frame (egui keeps file:line of each request).
    let causes = ctx.repaint_causes();
    let input = ctx.input(|i| {
        !i.events.is_empty() || i.pointer.is_moving() || i.smooth_scroll_delta != egui::Vec2::ZERO
    });
    if input {
        tally.input += 1;
    }
    if causes.is_empty() && !input {
        tally.other += 1;
    }
    for cause in causes.iter().take(16) {
        let mut key = format!("{}:{}", short_source(cause.file), cause.line);
        if !cause.reason.is_empty() {
            key.push(' ');
            key.extend(cause.reason.chars().take(40));
        }
        // Bounded: at most a few dozen distinct call sites exist.
        if tally.causes.len() < 64 || tally.causes.contains_key(&key) {
            *tally.causes.entry(key).or_insert(0) += 1;
        }
    }
    if diag::ui_sample_due() {
        let (textures, bytes) = {
            let manager = ctx.tex_manager();
            let manager = manager.read();
            let bytes: usize = manager.allocated().map(|(_, meta)| meta.bytes_used()).sum();
            (manager.num_allocated(), bytes as u64)
        };
        let mut repaint_causes: Vec<(String, u32)> = tally.causes.drain().collect();
        repaint_causes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        repaint_causes.truncate(8);
        let frame = diag::FrameStats {
            textures,
            texture_bytes: bytes,
            pixels_per_point: ctx.pixels_per_point(),
            egui_entries: ctx.memory(|m| m.data.len()),
            input_frames: std::mem::take(&mut tally.input),
            other_frames: std::mem::take(&mut tally.other),
            repaint_causes,
        };
        diag::send_ui_sample(diag::session_sample(session, frame));
    }
}

fn main() -> eframe::Result {
    // Before the window opens, so the renderer's driver lines can reach the diagnostic log.
    fm_engine::diag::install_log_sink();
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
    if let Err(err) = fm_engine::diag::apply(session.prefs.diagnostics_log, || {
        fm_engine::diag::header(&session.prefs)
    }) {
        session.notice = Some(err.to_string());
    }
    session.show_hidden = session.prefs.show_hidden;
    fm_engine::display_prefs_cmds::apply_runtime(&session.prefs, &session);
    if session.prefs.check_updates
        && let Err(err) = session.updates.check(false)
    {
        session.notice = Some(err.to_string());
    }
    fm_engine::open_cmds::sweep_open_temp(false);
    if let Some(dir) = &config_dir {
        session.remote.set_config_dir(dir.clone());
        session.media_store_start(dir.clone());
    }
    if !session
        .thumbs
        .set_cache_dir(session.prefs.thumb_cache_path())
    {
        session
            .thumbs
            .set_cache_dir(fm_engine::prefs::default_cache_dir().map(|d| d.join("thumbs")));
    }
    if let Some(dir) = &config_dir
        && let Some(text) = fm_engine::workspace::load_file(dir)
        && let Some(notice) = session.workspace_startup(&text)
    {
        session.notice = Some(notice);
    }
    // Kick off the first pane's read so the UI starts in the loading state
    // and the first paint never blocks on I/O.
    // A restored SFTP pane has no connection yet: it falls back to the Saved servers folder.
    if let Some(state) = session
        .layout
        .state(session.layout.focused())
        .filter(|st| st.kind.is_files() && st.kind != fm_layout::PaneKind::Sftp)
    {
        let location = state.location.clone();
        if let Err(err) = session.ensure_read(location) {
            eprintln!("startup read failed: {err}");
        }
    }

    let prefs = session.prefs.clone();
    let attempts = renderer::attempts(&prefs);
    let first = attempts
        .first()
        .copied()
        .unwrap_or(renderer::Attempt::OpenGl);
    // The session moves into whichever attempt opens the window. A renderer that cannot
    // start fails before that, so the next one is tried with the same session.
    let pending: Pending = Rc::new(RefCell::new(Some((session, config_dir))));
    let mut failures: Vec<String> = Vec::new();
    let mut result = Ok(());
    for (i, &attempt) in attempts.iter().enumerate() {
        let start = Start {
            used: attempt,
            first,
            failures: failures.clone(),
        };
        result = run(native_options(&prefs, attempt), Rc::clone(&pending), start);
        match &result {
            Err(err) if pending.borrow().is_some() && i + 1 < attempts.len() => {
                eprintln!("{attempt:?} could not start ({err}); trying the next renderer");
                failures.push(err.to_string());
            }
            _ => break,
        }
    }
    result
}

/// Which renderer opened the window, and why the ones before it did not.
struct Start {
    used: renderer::Attempt,
    first: renderer::Attempt,
    failures: Vec<String>,
}

/// The session and config folder waiting for the window to be created.
type Pending = Rc<RefCell<Option<(Session, Option<PathBuf>)>>>;

fn native_options(
    prefs: &fm_engine::prefs::Prefs,
    attempt: renderer::Attempt,
) -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default();
    if let Ok(icon) = eframe::icon_data::from_png_bytes(fm_brand::ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    let mut options = eframe::NativeOptions {
        viewport: viewport
            .with_title(fm_brand::APP_NAME)
            .with_app_id(fm_brand::WINDOW_CLASS)
            .with_inner_size(egui::vec2(1024.0, 700.0))
            .with_drag_and_drop(true)
            .with_min_inner_size(egui::vec2(640.0, 400.0)),
        event_loop_builder: prefer_x11(),
        ..Default::default()
    };
    window::restore(&mut options, prefs);
    renderer::configure(&mut options, attempt);
    options
}

fn run(options: eframe::NativeOptions, pending: Pending, start: Start) -> eframe::Result {
    eframe::run_native(
        fm_brand::APP_NAME,
        options,
        Box::new(move |cc| {
            let (mut session, config_dir) = pending
                .borrow_mut()
                .take()
                .ok_or("the window was already created")?;
            let before = session.prefs.windows_renderer_fallback;
            if let Some(notice) = renderer::note_fallback(
                &mut session.prefs,
                start.used,
                start.first,
                &start.failures,
            ) {
                session.notice = Some(notice);
            }
            // Remembered right away, so the notice is not repeated after a crash.
            if session.prefs.windows_renderer_fallback != before
                && let Some(dir) = &config_dir
                && let Err(err) = fm_engine::prefs::save(dir, &session.prefs)
            {
                eprintln!("failed to save preferences: {err}");
            }
            Ok(Box::new(App::new(cc, session, config_dir)))
        }),
    )
}

impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        session: Session,
        config_dir: Option<PathBuf>,
    ) -> Self {
        fm_engine::diag::set_renderer(renderer::describe(cc));
        let position = cc
            .winit_window()
            .and_then(|window| window::restore_native(window, &session.prefs));
        let tracker = window::Tracker::new(position, &session.prefs);
        // Seed the theme before the first frame; `sync_theme` keeps it current.
        let theme_pref = session.prefs.theme;
        set_egui_theme_preference(&cc.egui_ctx, theme_pref);
        let wake_ctx = cc.egui_ctx.clone();
        session.watch.set_waker(move || wake_ctx.request_repaint());
        // Video is drawn when a frame is ready, not on a 16 ms timer, and only while a
        // player is on screen.
        let frame_ctx = cc.egui_ctx.clone();
        fm_engine::video_cmds::set_frame_waker(move || {
            if fm_ui::media::video_on_screen(&frame_ctx) {
                frame_ctx.request_repaint();
            }
        });
        // Zoom is a command (View menu, shortcuts) persisted in prefs; egui's own keys
        // would change it behind the preference's back.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        cc.egui_ctx.set_zoom_factor(session.prefs.zoom_factor());
        let dark = cc.egui_ctx.theme() == egui::Theme::Dark;
        let look = theme::Tokens::resolve_with(&session.prefs, dark);
        theme::apply_to(&cc.egui_ctx, &look);
        App {
            session,
            config_dir,
            applied: (theme_pref, dark),
            look,
            title_sent: false,
            was_focused: true,
            window_fullscreen_sent: false,
            dialogs: fm_ui::dialogs::Dialogs::default(),
            last_ws_tick: std::time::Instant::now(),
            window: tracker,
            out_drag: dragout::OutDrag::new(cc),
            media_controls: media_controls::MediaControls::new(),
            diag_frames: DiagFrames::default(),
        }
    }
}

/// The brand logo as a texture, decoded once and kept in egui's memory.
fn logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::NULL.with("fm-brand-logo");
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(tex);
    }
    let icon = eframe::icon_data::from_png_bytes(fm_brand::ICON_PNG).ok()?;
    let size = [icon.width as usize, icon.height as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &icon.rgba);
    let tex = ctx.load_texture("fm-brand-logo", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repaint_sources_are_shortened_to_the_file_name() {
        assert_eq!(short_source("crates/ui/src/media.rs"), "media.rs");
        assert_eq!(
            short_source(r"C:\Users\runner\.cargo\registry\src\egui-0.37.0\src\context.rs"),
            "context.rs"
        );
        assert_eq!(short_source("main.rs"), "main.rs");
    }

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

    #[test]
    fn external_media_drops_enqueue_instead_of_copying_files() {
        let ctx = egui::Context::default();
        let mut session = Session::new().expect("session");
        session
            .layout
            .state_mut(session.layout.focused())
            .expect("pane")
            .kind = fm_layout::PaneKind::Media;
        import_paths(
            &ctx,
            &mut session,
            vec!["https://host/song.mp3".into()],
            None,
        );
        assert!(!session.command_enabled("media.drop"), "import submitted");
        assert!(session.fileops.jobs().is_empty(), "never submit a copy job");
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
