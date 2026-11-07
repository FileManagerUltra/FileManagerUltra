//! Dragging files out of the window: when an in-app file drag leaves the window, the same
//! files continue as an OS drag so other applications (a browser drop field, an editor)
//! can receive them. The protocol work lives in `fm-dragout`; this only decides *when*.

use std::path::PathBuf;

use eframe::egui;
use fm_dragout::{DragError, DragOut, Outcome};
use fm_engine::Session;
use fm_ui::dnd::DragPayload;

#[derive(Default)]
pub struct OutDrag {
    active: Option<DragOut>,
    /// The Windows OLE drag, polled until it ends.
    #[cfg(windows)]
    win: Option<fm_winout::Drag>,
    /// The compositor owns the pointer during an OS drag, so egui never sees the button
    /// release and would ignore the next press. Inject it where the drag started.
    release_at: Option<egui::Pos2>,
    /// Native Wayland drag and drop on the window's connection (None under X11).
    #[cfg(target_os = "linux")]
    wl: Option<fm_wayland::Dnd>,
}

/// Files another application dropped on the window, with the pointer position in points.
pub struct Incoming {
    pub paths: Vec<PathBuf>,
    pub pos: egui::Pos2,
}

/// The local paths of a drag payload; remote items cannot be handed to another application.
fn local_paths(payload: &DragPayload) -> Vec<PathBuf> {
    payload
        .sources
        .iter()
        .filter_map(|u| u.local_path_string())
        .map(PathBuf::from)
        .collect()
}

impl OutDrag {
    /// Attach to the window's Wayland connection when it is a Wayland window.
    #[cfg(target_os = "linux")]
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let wake: std::sync::Arc<dyn Fn() + Send + Sync> =
            std::sync::Arc::new(move || ctx.request_repaint());
        let wl = fm_wayland::Dnd::new(cc, cc, wake).ok();
        Self {
            active: None,
            release_at: None,
            wl,
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    /// Feed the button release egui missed while the OS drag ran. Call from `raw_input_hook`.
    pub fn raw_input(&mut self, raw: &mut egui::RawInput) {
        if let Some(pos) = self.release_at.take() {
            raw.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }

    /// Call once per frame, before the panes draw. Returns files dropped by other apps.
    pub fn update(&mut self, ctx: &egui::Context, session: &mut Session) -> Vec<Incoming> {
        let incoming = self.poll_wayland(ctx, session);
        if let Some(drag) = self.active.take() {
            if !drag.is_finished() {
                self.active = Some(drag);
            } else {
                Self::report(session, drag);
            }
        }
        #[cfg(windows)]
        self.poll_windows(ctx, session);
        self.start_if_leaving(ctx, session);
        incoming
    }

    #[cfg(target_os = "linux")]
    fn poll_wayland(&mut self, ctx: &egui::Context, session: &mut Session) -> Vec<Incoming> {
        let Some(wl) = self.wl.as_mut() else {
            return Vec::new();
        };
        let zoom = ctx.zoom_factor().max(0.1);
        let mut incoming = Vec::new();
        for event in wl.poll() {
            match event {
                fm_wayland::Event::DragEnded(fm_wayland::DragEnd::Dropped) => {
                    session.info = Some((
                        "Dropped files into another application".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                fm_wayland::Event::DragEnded(_) => {}
                fm_wayland::Event::FilesDropped { paths, pos } => incoming.push(Incoming {
                    paths,
                    pos: egui::pos2(pos.0 / zoom, pos.1 / zoom),
                }),
            }
        }
        if wl.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        incoming
    }

    #[cfg(not(target_os = "linux"))]
    fn poll_wayland(&mut self, _ctx: &egui::Context, _session: &mut Session) -> Vec<Incoming> {
        Vec::new()
    }

    #[cfg(windows)]
    fn poll_windows(&mut self, ctx: &egui::Context, session: &mut Session) {
        let Some(drag) = self.win.as_ref() else {
            return;
        };
        match drag.poll() {
            None => ctx.request_repaint_after(std::time::Duration::from_millis(30)),
            Some(result) => {
                self.win = None;
                match result {
                    Ok(Outcome::Dropped) => {
                        session.info = Some((
                            "Dropped files into another application".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                    Ok(_) => {}
                    Err(err) => session.notice = Some(err.to_string()),
                }
            }
        }
    }

    fn report(session: &mut Session, drag: DragOut) {
        match drag.finish() {
            Ok(Outcome::Dropped) => {
                session.info = Some((
                    "Dropped files into another application".to_string(),
                    std::time::Instant::now(),
                ));
            }
            Ok(_) => {}
            Err(err) => session.notice = Some(err.to_string()),
        }
    }

    fn start_if_leaving(&mut self, ctx: &egui::Context, session: &mut Session) {
        let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(ctx) else {
            return;
        };
        let content = ctx.content_rect();
        let (down, outside) = ctx.input(|i| {
            let inside = i.pointer.latest_pos().is_some_and(|p| content.contains(p));
            (i.pointer.primary_down(), !inside)
        });
        if !down || !outside {
            return;
        }
        // The in-app drag is over either way; the OS drag (if any) carries on from here.
        egui::DragAndDrop::clear_payload(ctx);
        // The pointer is usually already gone (no position). A far-away release is never a
        // click on a row.
        self.release_at = Some(
            ctx.input(|i| i.pointer.latest_pos())
                .unwrap_or(egui::pos2(-1.0e5, -1.0e5)),
        );
        let paths = local_paths(&payload);
        if paths.is_empty() {
            session.notice = Some(
                DragError::BadPayload("only local files can be dragged out".into()).to_string(),
            );
            return;
        }
        #[cfg(target_os = "linux")]
        if let Some(wl) = self.wl.as_mut() {
            let started = fm_dragout::uri_list(&paths)
                .and_then(|list| wl.start_drag(list).map_err(|e| DragError::System(e.0)));
            if let Err(err) = started {
                session.notice = Some(err.to_string());
            }
            return;
        }
        self.start_native(paths, session);
    }

    /// A still-running earlier drag is replaced (dropping it cancels it).
    #[cfg(windows)]
    fn start_native(&mut self, paths: Vec<PathBuf>, session: &mut Session) {
        match fm_winout::Drag::start(paths) {
            Ok(drag) => self.win = Some(drag),
            Err(err) => session.notice = Some(err.to_string()),
        }
    }

    /// A still-running earlier drag is replaced (dropping it cancels it).
    #[cfg(not(windows))]
    fn start_native(&mut self, paths: Vec<PathBuf>, session: &mut Session) {
        match DragOut::start(paths) {
            Ok(drag) => self.active = Some(drag),
            Err(err) => session.notice = Some(err.to_string()),
        }
    }
}
