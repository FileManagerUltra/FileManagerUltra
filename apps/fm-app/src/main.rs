//! Application shell (the product name comes from `fm_brand::APP_NAME`): eframe
//! window, menus, shortcuts, status bar.
//!
//! The shell owns a [`fm_engine::Session`] and routes every interaction through
//! `Session::dispatch` — it performs no filesystem work itself.

mod menu;

use std::path::PathBuf;

use eframe::egui;

use fm_engine::{Session, ThemePreference};
use fm_ui::theme;

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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.sync_theme(&ctx);

        egui::Panel::top("menu_bar").show(ui, |ui| {
            menu::menu_bar(ui, &mut self.session);
        });

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            let tokens = theme::Tokens::resolve(self.session.prefs.theme, self.applied.1);
            ui.horizontal(|ui| {
                ui.label(tokens.muted(match self.session.prefs.theme {
                    ThemePreference::System => "Theme: system",
                    ThemePreference::Dark => "Theme: dark",
                    ThemePreference::Light => "Theme: light",
                }));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(tokens.muted(fm_brand::APP_NAME));
                });
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(fm_brand::APP_NAME);
            ui.label("no panes yet — the multi-pane shell lands in change 2");
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
        session
            .dispatch(menu::QUIT, &fm_engine::Params::Null)
            .expect("quit");
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
