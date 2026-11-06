//! Native Wayland drag and drop on the window's own connection.
//!
//! * **Drag out:** a `wl_data_source` offering a `text/uri-list`, started with the serial of
//!   the latest button press (seen through our own `wl_pointer` on the shared connection).
//! * **Drop in:** our own `wl_data_device` receives `enter`/`drop` for the window's surface and
//!   reads the dropped `text/uri-list`.
//!
//! winit 0.30 supports neither, and Wayland only lets the connection that owns the window
//! start a drag, so this attaches a private event queue to winit's `wl_display`. It is
//! temporary: replace it with winit 0.31's `start_drag` (see `docs/roadmap.md`).
#![cfg(all(unix, not(target_os = "macos")))]

mod foreign;
mod uri;

use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_pointer::{self, ButtonState, WlPointer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
};

const URI_LIST: &str = "text/uri-list";
/// A source the receiver never finishes is dropped after this.
const SOURCE_LINGER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn err(e: impl fmt::Display) -> Error {
    Error(e.to_string())
}

/// How a drag that left the window ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragEnd {
    Dropped,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    DragEnded(DragEnd),
    /// Files dropped on the window by another application; `pos` is in logical pixels.
    FilesDropped {
        paths: Vec<PathBuf>,
        pos: (f32, f32),
    },
}

#[derive(Default)]
struct Offer {
    mimes: Vec<String>,
    action: bool,
}

struct Hover {
    offer: WlDataOffer,
    pos: (f64, f64),
    mime: Option<&'static str>,
}

struct Dropped {
    offer: WlDataOffer,
    pos: (f64, f64),
    bytes: Vec<u8>,
}

struct ActiveSource {
    source: WlDataSource,
    list: Arc<String>,
    performed: Option<Instant>,
}

struct State {
    conn: Connection,
    surface: WlSurface,
    manager: WlDataDeviceManager,
    press_serial: Option<u32>,
    source: Option<ActiveSource>,
    offers: HashMap<wayland_client::backend::ObjectId, Offer>,
    hover: Option<Hover>,
    events: Vec<Event>,
    reads_tx: Sender<Dropped>,
}

pub struct Dnd {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    device: WlDataDevice,
    _pointer: WlPointer,
    reads_rx: Receiver<Dropped>,
    stop: Arc<AtomicBool>,
    /// A drag is running: the waker thread reads the socket itself and wakes quickly.
    live: Arc<AtomicBool>,
}

impl Dnd {
    /// Attach to the Wayland connection of `window`. `wake` is called (from another thread)
    /// when the compositor sent something, so the UI can run a frame and call [`Dnd::poll`].
    ///
    /// The window must outlive the returned value. Fails when the window is not a Wayland one.
    pub fn new(
        display: &dyn HasDisplayHandle,
        window: &dyn HasWindowHandle,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, Error> {
        let RawDisplayHandle::Wayland(d) = display.display_handle().map_err(err)?.as_raw() else {
            return Err(Error("not a Wayland window".into()));
        };
        let RawWindowHandle::Wayland(w) = window.window_handle().map_err(err)?.as_raw() else {
            return Err(Error("not a Wayland window".into()));
        };
        let conn = foreign::connection(d.display);
        let surface =
            foreign::surface(&conn, w.surface).ok_or_else(|| Error("no window surface".into()))?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(err)?;
        let qh = queue.handle();
        let seat: WlSeat = globals.bind(&qh, 1..=5, ()).map_err(err)?;
        let manager: WlDataDeviceManager = globals.bind(&qh, 1..=3, ()).map_err(err)?;
        let pointer = seat.get_pointer(&qh, ());
        let device = manager.get_data_device(&seat, &qh, ());
        let (reads_tx, reads_rx) = channel();
        let mut state = State {
            conn: conn.clone(),
            surface,
            manager,
            press_serial: None,
            source: None,
            offers: HashMap::new(),
            hover: None,
            events: Vec::new(),
            reads_tx,
        };
        queue.roundtrip(&mut state).map_err(err)?;
        let stop = Arc::new(AtomicBool::new(false));
        let live = Arc::new(AtomicBool::new(false));
        spawn_waker(&conn, wake, stop.clone(), live.clone())?;
        Ok(Self {
            conn,
            queue,
            state,
            device,
            _pointer: pointer,
            reads_rx,
            stop,
            live,
        })
    }

    /// Whether a drag started by [`Dnd::start_drag`] is still running.
    pub fn drag_active(&self) -> bool {
        self.state.source.is_some()
    }

    /// Begin dragging the files in `uri_list` (RFC 2483). Must be called while the button
    /// that started the in-app drag is still held.
    pub fn start_drag(&mut self, uri_list: String) -> Result<(), Error> {
        self.pump();
        let serial = self
            .state
            .press_serial
            .ok_or_else(|| Error("no mouse button press to start the drag from".into()))?;
        self.cancel();
        let qh = self.queue.handle();
        let source = self.state.manager.create_data_source(&qh, ());
        source.offer(URI_LIST.to_string());
        if source.version() >= 3 {
            // Like other file managers: pages that insist on a "move" effect reject copy-only
            // sources. We never delete the originals, so a move degrades to a copy.
            source.set_actions(DndAction::Copy | DndAction::Move);
        }
        self.device
            .start_drag(Some(&source), &self.state.surface, None, serial);
        self.state.source = Some(ActiveSource {
            source,
            list: Arc::new(uri_list),
            performed: None,
        });
        self.live.store(true, Ordering::Relaxed);
        self.conn.flush().map_err(err)
    }

    /// Abandon a running drag.
    pub fn cancel(&mut self) {
        if let Some(active) = self.state.source.take() {
            active.source.destroy();
            let _ = self.conn.flush();
        }
    }

    fn pump(&mut self) {
        // Reading the socket steals winit's wake-ups (window not shown, stale redraws), so
        // only do it while a drag is running, when the compositor addresses us and not winit.
        if self.busy()
            && let Some(guard) = self.conn.prepare_read()
        {
            let _ = guard.read();
        }
        let _ = self.queue.dispatch_pending(&mut self.state);
    }

    /// Process compositor messages; call once per frame.
    pub fn poll(&mut self) -> Vec<Event> {
        self.pump();
        for read in self.reads_rx.try_iter() {
            self.state.finish_read(read);
        }
        if self
            .state
            .source
            .as_ref()
            .and_then(|s| s.performed)
            .is_some_and(|t| t.elapsed() > SOURCE_LINGER)
        {
            self.cancel();
        }
        let _ = self.conn.flush();
        self.live.store(self.busy(), Ordering::Relaxed);
        std::mem::take(&mut self.state.events)
    }

    /// Something is in flight that needs frequent polling.
    pub fn busy(&self) -> bool {
        self.state.source.is_some() || self.state.hover.is_some()
    }
}

impl Drop for Dnd {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.cancel();
    }
}

/// Wake the UI when the connection becomes readable; winit reads it, we only nudge a frame.
fn spawn_waker(
    conn: &Connection,
    wake: Arc<dyn Fn() + Send + Sync>,
    stop: Arc<AtomicBool>,
    live: Arc<AtomicBool>,
) -> Result<(), Error> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let conn = conn.clone();
    let fd: OwnedFd = conn.backend().poll_fd().try_clone_to_owned().map_err(err)?;
    std::thread::Builder::new()
        .name("fm-wayland-wake".into())
        .spawn(move || {
            let idle = Timespec {
                tv_sec: 0,
                tv_nsec: 500_000_000,
            };
            let fast = Timespec {
                tv_sec: 0,
                tv_nsec: 8_000_000,
            };
            while !stop.load(Ordering::Relaxed) {
                let mut fds = [PollFd::new(&fd, PollFlags::IN)];
                if live.load(Ordering::Relaxed) {
                    // The receiver may block on our data-send handler: read and wake at once.
                    let _ = poll(&mut fds, Some(&fast));
                    if let Some(guard) = conn.prepare_read() {
                        let _ = guard.read();
                    }
                    wake();
                    continue;
                }
                match poll(&mut fds, Some(&idle)) {
                    Ok(n) if n > 0 => {
                        wake();
                        std::thread::sleep(Duration::from_millis(15));
                    }
                    Ok(_) => {}
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        })
        .map_err(err)?;
    Ok(())
}

impl State {
    fn forget(&mut self, offer: &WlDataOffer) {
        self.offers.remove(&offer.id());
        offer.destroy();
    }

    fn finish_read(&mut self, read: Dropped) {
        let paths = uri::parse(&read.bytes);
        if !paths.is_empty() {
            self.events.push(Event::FilesDropped {
                paths,
                pos: (read.pos.0 as f32, read.pos.1 as f32),
            });
        }
        let negotiated = self.offers.get(&read.offer.id()).is_some_and(|o| o.action);
        if negotiated && read.offer.version() >= 3 {
            read.offer.finish();
        }
        self.forget(&read.offer);
    }

    fn drop_on_surface(&mut self) {
        let Some(h) = self.hover.take() else {
            return;
        };
        let Some(mime) = h.mime else {
            self.forget(&h.offer);
            return;
        };
        let Ok((read_end, write_end)) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
        else {
            self.forget(&h.offer);
            return;
        };
        h.offer.receive(mime.to_string(), write_end.as_fd());
        drop(write_end);
        let _ = self.conn.flush();
        let tx = self.reads_tx.clone();
        let (offer, pos) = (h.offer.clone(), h.pos);
        let spawned = std::thread::Builder::new()
            .name("fm-wayland-read".into())
            .spawn(move || {
                let mut bytes = Vec::new();
                let _ = std::fs::File::from(read_end)
                    .take(uri::MAX_BYTES as u64)
                    .read_to_end(&mut bytes);
                let _ = tx.send(Dropped { offer, pos, bytes });
            });
        if spawned.is_err() {
            self.forget(&h.offer);
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ignore WlDataDeviceManager);

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            state: WEnum::Value(ButtonState::Pressed),
            ..
        } = event
        {
            state.press_serial = Some(serial);
        }
    }
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(info) = state.offers.get_mut(&offer.id()) else {
            return;
        };
        match event {
            wl_data_offer::Event::Offer { mime_type } if info.mimes.len() < 64 => {
                info.mimes.push(mime_type);
            }
            wl_data_offer::Event::Action { dnd_action } => {
                info.action = matches!(dnd_action, WEnum::Value(a) if !a.is_empty());
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);

    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                // A runaway compositor cannot grow this without bound.
                if state.offers.len() < 32 {
                    state.offers.insert(id.id(), Offer::default());
                } else {
                    id.destroy();
                }
            }
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id: Some(offer),
            } => {
                if let Some(old) = state.hover.take() {
                    state.forget(&old.offer);
                }
                // Our own drag coming back over the window is not an import.
                let wants = surface == state.surface
                    && state.source.is_none()
                    && state
                        .offers
                        .get(&offer.id())
                        .is_some_and(|o| o.mimes.iter().any(|m| m == URI_LIST));
                let mime = wants.then_some(URI_LIST);
                offer.accept(serial, mime.map(str::to_string));
                if mime.is_some() && offer.version() >= 3 {
                    offer.set_actions(DndAction::Copy, DndAction::Copy);
                }
                if surface == state.surface {
                    state.hover = Some(Hover {
                        offer,
                        pos: (x, y),
                        mime,
                    });
                } else {
                    state.forget(&offer);
                }
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if let Some(h) = state.hover.as_mut() {
                    h.pos = (x, y);
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(h) = state.hover.take() {
                    state.forget(&h.offer);
                }
            }
            wl_data_device::Event::Drop => state.drop_on_surface(),
            // The clipboard is not ours to manage; release the offer.
            wl_data_device::Event::Selection { id: Some(offer) } => state.forget(&offer),
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, ()> for State {
    fn event(
        state: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(active) = state
            .source
            .as_mut()
            .filter(|a| a.source.id() == source.id())
        else {
            return;
        };
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                if mime_type == URI_LIST {
                    send_list(active.list.clone(), fd);
                }
            }
            wl_data_source::Event::DndDropPerformed => {
                active.performed = Some(Instant::now());
                state.events.push(Event::DragEnded(DragEnd::Dropped));
            }
            wl_data_source::Event::Cancelled => {
                let performed = active.performed.is_some();
                source.destroy();
                state.source = None;
                if !performed {
                    state.events.push(Event::DragEnded(DragEnd::Cancelled));
                }
            }
            wl_data_source::Event::DndFinished => {
                source.destroy();
                state.source = None;
            }
            _ => {}
        }
    }
}

/// Write the list on its own thread so a slow receiver cannot stall the UI.
fn send_list(list: Arc<String>, fd: OwnedFd) {
    let _ = std::thread::Builder::new()
        .name("fm-wayland-send".into())
        .spawn(move || {
            let _ = std::fs::File::from(fd).write_all(list.as_bytes());
        });
}
