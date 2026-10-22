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
    /// `(pref, system_dark)` currently applied to egui; re-applied when either changes.
    applied: (ThemePreference, bool),
}

impl App {
    /// Re-apply tokens whenever the resolved palette (preference × OS theme) changes.
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let system_dark = system_is_dark(ctx);
        let current = (self.session.prefs.theme, system_dark);
        if current == self.applied {
            return;
        }
        self.applied = current;
        set_egui_theme_preference(ctx, current.0);
        theme::apply_to(ctx, &theme::Tokens::resolve(current.0, current.1));
    }
}

/// The OS-reported dark preference, when the platform provides one.
fn system_is_dark(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.raw.system_theme)
        .is_some_and(|theme| theme == egui::Theme::Dark)
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
        PaneNode::Split { axis, ratio, a, b } => {
            let handle = a.panes().first().copied();
            let mut open = None;
            match axis {
                Axis::Horizontal => {
                    ui.horizontal(|ui| {
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
                        let (rect, drag) = ui.allocate_exact_size(
                            egui::vec2(handle_w, ui.available_height()),
                            egui::Sense::click_and_drag(),
                        );
                        ui.painter().rect_filled(rect, 0.0, tokens.border);
                        if drag.dragged()
                            && let Some(handle) = handle
                        {
                            let dx = drag.drag_delta().x;
                            let new_ratio = *ratio + dx / total.max(1.0);
                            session.layout.set_ratio(handle, new_ratio);
                        }
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
                        let (rect, drag) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), handle_w),
                            egui::Sense::click_and_drag(),
                        );
                        ui.painter().rect_filled(rect, 0.0, tokens.border);
                        if drag.dragged()
                            && let Some(handle) = handle
                        {
                            let dy = drag.drag_delta().y;
                            let new_ratio = *ratio + dy / total.max(1.0);
                            session.layout.set_ratio(handle, new_ratio);
                        }
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.sync_theme(&ctx);
        let _ = self.session.poll();

        egui::Panel::top("menu_bar").show(ui, |ui| {
            menu::menu_bar(ui, &mut self.session);
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
                ui.label(tokens.muted(location));
                ui.label(tokens.muted(format!("{count} items")));
                if selected > 0 {
                    ui.label(tokens.muted(format!("{selected} selected")));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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

        if self.session.about_open {
            let mut open = self.session.about_open;
            egui::Window::new("About")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ui, |ui| {
                    ui.heading(fm_brand::APP_NAME);
                    ui.label(format!("version {}", env!("CARGO_PKG_VERSION")));
                    ui.label(fm_brand::ORG_NAME);
                });
            self.session.about_open = open;
        }

        menu::consume_shortcuts(&ctx, &mut self.session);

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
            .with_min_inner_size(egui::vec2(640.0, 400.0)),
        ..Default::default()
    };

    eframe::run_native(
        fm_brand::APP_NAME,
        options,
        Box::new(move |cc| {
            // Seed the theme before the first frame; `sync_theme` keeps it current.
            let system_dark = system_is_dark(&cc.egui_ctx);
            set_egui_theme_preference(&cc.egui_ctx, session.prefs.theme);
            theme::apply_to(
                &cc.egui_ctx,
                &theme::Tokens::resolve(session.prefs.theme, system_dark),
            );
            Ok(Box::new(App {
                session,
                config_dir,
                applied: (ThemePreference::System, system_dark),
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
