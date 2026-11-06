//! XDND (version 5) drag *source* on X11 / XWayland, pure Rust via `x11rb`.
//!
//! A helper input-only window owns the `XdndSelection` and serves the `text/uri-list` payload.
//! The drag is driven by polling the pointer (the caller's window keeps its implicit pointer
//! grab, so its own event stream is undisturbed): the window under the pointer that
//! advertises `XdndAware` gets Enter / Position / Leave / Drop messages and answers with
//! Status / Finished.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, KeyButMask,
    MapState, PropMode, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent,
    Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE};

use crate::{DragError, Outcome};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        XdndAware,
        XdndEnter,
        XdndPosition,
        XdndStatus,
        XdndLeave,
        XdndDrop,
        XdndFinished,
        XdndSelection,
        XdndActionCopy,
        TARGETS,
        FM_DRAG_TIME,
    }
}

/// Highest XDND version spoken; a receiver advertising less is spoken to at its level.
const XDND_VERSION: u32 = 5;
const TICK: Duration = Duration::from_millis(8);
const STATUS_WAIT: Duration = Duration::from_millis(300);
const REENTER_WAIT: Duration = Duration::from_millis(500);
const FINISH_WAIT: Duration = Duration::from_secs(10);
const MAX_DRAG: Duration = Duration::from_secs(300);
const MAX_DEPTH: u32 = 6;
const MAX_CHILDREN: usize = 512;

fn sys(e: impl std::fmt::Display) -> DragError {
    DragError::System(e.to_string())
}

/// One reading of the pointer, in root-window coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub x: i16,
    pub y: i16,
    /// Whether the primary button is still held.
    pub down: bool,
}

pub type Pointer<'a> = dyn FnMut(&RustConnection, Window) -> Result<Sample, DragError> + 'a;

pub fn real_pointer(conn: &RustConnection, root: Window) -> Result<Sample, DragError> {
    let r = conn
        .query_pointer(root)
        .map_err(sys)?
        .reply()
        .map_err(sys)?;
    Ok(Sample {
        x: r.root_x,
        y: r.root_y,
        down: r.mask.contains(KeyButMask::BUTTON1),
    })
}

struct Target {
    win: Window,
    accepted: bool,
    awaiting: bool,
    pending: Option<(i16, i16)>,
    pos: (i16, i16),
    version: u32,
}

pub struct Source {
    conn: RustConnection,
    root: Window,
    win: Window,
    atoms: Atoms,
    uri_list: Atom,
    list: String,
    time: u32,
    /// Under Xwayland, owning the drag selection makes the compositor start its own drag
    /// (which swallows the button state), so the selection is only claimed at drop time.
    late_claim: bool,
}

impl Source {
    pub fn connect(list: String) -> Result<Self, DragError> {
        let (conn, screen) = RustConnection::connect(None)
            .map_err(|e| DragError::Unsupported(leak_reason(&e.to_string())))?;
        let root = conn
            .setup()
            .roots
            .get(screen)
            .ok_or_else(|| sys("no screen"))?
            .root;
        let win = conn.generate_id().map_err(sys)?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(sys)?;
        let late_claim = conn
            .query_extension(b"XWAYLAND")
            .map_err(sys)?
            .reply()
            .map_err(sys)?
            .present;
        let atoms = Atoms::new(&conn).map_err(sys)?.reply().map_err(sys)?;
        let uri_list = conn
            .intern_atom(false, b"text/uri-list")
            .map_err(sys)?
            .reply()
            .map_err(sys)?
            .atom;
        let mut source = Self {
            conn,
            root,
            win,
            atoms,
            uri_list,
            list,
            time: CURRENT_TIME,
            late_claim,
        };
        source.time = source.server_time();
        if !source.late_claim {
            source.claim()?;
        }
        Ok(source)
    }

    /// A real server timestamp (an empty property append reports one); `CurrentTime` if the
    /// server does not answer.
    fn server_time(&self) -> u32 {
        let probe = self.conn.change_property8(
            PropMode::APPEND,
            self.win,
            self.atoms.FM_DRAG_TIME,
            AtomEnum::STRING,
            &[],
        );
        if probe.is_err() || self.conn.flush().is_err() {
            return CURRENT_TIME;
        }
        let end = Instant::now() + Duration::from_secs(1);
        while Instant::now() < end {
            if let Ok(Some(Event::PropertyNotify(p))) = self.conn.poll_for_event() {
                return p.time;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        CURRENT_TIME
    }

    fn claim(&self) -> Result<(), DragError> {
        self.conn
            .set_selection_owner(self.win, self.atoms.XdndSelection, self.time)
            .map_err(sys)?;
        let owner = self
            .conn
            .get_selection_owner(self.atoms.XdndSelection)
            .map_err(sys)?
            .reply()
            .map_err(sys)?
            .owner;
        if owner != self.win {
            return Err(sys("could not take the drag selection"));
        }
        Ok(())
    }

    /// Treat the display as a plain X server (used by the tests).
    #[cfg(test)]
    pub fn direct(mut self) -> Self {
        self.late_claim = false;
        let _ = self.claim();
        self
    }

    pub fn run(self, cancel: &AtomicBool, pointer: &mut Pointer<'_>) -> Result<Outcome, DragError> {
        let started = Instant::now();
        if !pointer(&self.conn, self.root)?.down {
            return Ok(Outcome::Cancelled);
        }
        let mut cur: Option<Target> = None;
        let mut finished = false;
        let mut last: Option<(i16, i16)> = None;
        let mut tick = 0u32;
        loop {
            self.pump(&mut cur, &mut finished)?;
            if cancel.load(Ordering::Relaxed) || started.elapsed() > MAX_DRAG {
                self.leave(&mut cur);
                return Ok(Outcome::Cancelled);
            }
            let s = pointer(&self.conn, self.root)?;
            let pos = (s.x, s.y);
            let moved = last != Some(pos);
            last = Some(pos);
            if moved || (cur.is_none() && tick.is_multiple_of(12)) {
                let found = self.find_target(pos.0, pos.1);
                if cur.as_ref().map(|t| t.win) != found.map(|f| f.0) {
                    self.leave(&mut cur);
                    if let Some((win, version)) = found {
                        cur = Some(self.enter(win, version, pos));
                    }
                } else if let Some(t) = cur.as_mut() {
                    self.position(t, pos);
                }
            }
            if !s.down {
                return self.finish(cur, cancel);
            }
            let _ = self.conn.flush();
            std::thread::sleep(TICK);
            tick = tick.wrapping_add(1);
        }
    }

    fn finish(&self, mut cur: Option<Target>, cancel: &AtomicBool) -> Result<Outcome, DragError> {
        let mut finished = false;
        let end = Instant::now() + STATUS_WAIT;
        while cur
            .as_ref()
            .is_some_and(|t| (t.awaiting || t.pending.is_some()) && Instant::now() < end)
        {
            self.pump(&mut cur, &mut finished)?;
            std::thread::sleep(Duration::from_millis(4));
        }
        if cur.as_ref().is_none_or(|t| !t.accepted) {
            self.leave(&mut cur);
            return Ok(Outcome::NoTarget);
        }
        if self.late_claim {
            // Receivers read the offered types on XdndEnter, so re-enter once we own the selection.
            let (win, version, pos) = cur
                .as_ref()
                .map_or((0, 0, (0, 0)), |t| (t.win, t.version, t.pos));
            self.leave(&mut cur);
            self.claim()?;
            cur = Some(self.enter(win, version, pos));
            // The first answer may precede the data fetch; ask again once it has had time.
            for _ in 0..2 {
                let end = Instant::now() + REENTER_WAIT;
                while cur.as_ref().is_some_and(|t| t.awaiting) && Instant::now() < end {
                    self.pump(&mut cur, &mut finished)?;
                    std::thread::sleep(Duration::from_millis(4));
                }
                if cur.as_ref().is_some_and(|t| t.accepted) {
                    break;
                }
                let settle = Instant::now() + Duration::from_millis(200);
                while Instant::now() < settle {
                    self.pump(&mut cur, &mut finished)?;
                    std::thread::sleep(Duration::from_millis(4));
                }
                if let Some(t) = cur.as_mut() {
                    let pos = t.pos;
                    self.position(t, pos);
                }
            }
        }
        let Some(target) = cur.as_ref().filter(|t| t.accepted) else {
            self.leave(&mut cur);
            return Ok(Outcome::NoTarget);
        };
        self.send(
            target.win,
            self.atoms.XdndDrop,
            [self.win, 0, self.time, 0, 0],
        );
        let end = Instant::now() + FINISH_WAIT;
        while !finished && Instant::now() < end && !cancel.load(Ordering::Relaxed) {
            self.pump(&mut cur, &mut finished)?;
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(Outcome::Dropped)
    }

    fn send(&self, to: Window, kind: Atom, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, to, kind, data);
        let _ = self.conn.send_event(false, to, EventMask::NO_EVENT, event);
        let _ = self.conn.flush();
    }

    fn enter(&self, win: Window, version: u32, pos: (i16, i16)) -> Target {
        let proto = version.min(XDND_VERSION);
        self.send(
            win,
            self.atoms.XdndEnter,
            [self.win, proto << 24, self.uri_list, 0, 0],
        );
        let mut target = Target {
            win,
            accepted: false,
            awaiting: false,
            pending: None,
            pos,
            version,
        };
        self.position(&mut target, pos);
        target
    }

    /// Send a Position now, or remember it until the previous one has been answered.
    fn position(&self, t: &mut Target, pos: (i16, i16)) {
        if t.awaiting {
            t.pending = Some(pos);
            return;
        }
        let packed = (u32::from(pos.0.max(0) as u16) << 16) | u32::from(pos.1.max(0) as u16);
        self.send(
            t.win,
            self.atoms.XdndPosition,
            [self.win, 0, packed, self.time, self.atoms.XdndActionCopy],
        );
        t.awaiting = true;
    }

    fn leave(&self, cur: &mut Option<Target>) {
        if let Some(t) = cur.take() {
            self.send(t.win, self.atoms.XdndLeave, [self.win, 0, 0, 0, 0]);
        }
    }

    fn pump(&self, cur: &mut Option<Target>, finished: &mut bool) -> Result<(), DragError> {
        while let Some(event) = self.conn.poll_for_event().map_err(sys)? {
            match event {
                Event::SelectionRequest(r) => self.serve(&r),
                Event::ClientMessage(m) if m.type_ == self.atoms.XdndStatus => {
                    let d = m.data.as_data32();
                    if let Some(t) = cur.as_mut().filter(|t| t.win == d[0]) {
                        t.accepted = d[1] & 1 == 1;
                        t.awaiting = false;
                        if let Some(p) = t.pending.take() {
                            self.position(t, p);
                        }
                    }
                }
                Event::ClientMessage(m) if m.type_ == self.atoms.XdndFinished => {
                    *finished = true;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Answer a receiver asking for the dragged data.
    fn serve(&self, r: &SelectionRequestEvent) {
        let property = if r.property == NONE {
            r.target
        } else {
            r.property
        };
        let ok = r.selection == self.atoms.XdndSelection
            && if r.target == self.atoms.TARGETS {
                self.conn
                    .change_property32(
                        PropMode::REPLACE,
                        r.requestor,
                        property,
                        AtomEnum::ATOM,
                        &[self.atoms.TARGETS, self.uri_list],
                    )
                    .is_ok()
            } else if r.target == self.uri_list {
                self.conn
                    .change_property8(
                        PropMode::REPLACE,
                        r.requestor,
                        property,
                        self.uri_list,
                        self.list.as_bytes(),
                    )
                    .is_ok()
            } else {
                false
            };
        let reply = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: r.time,
            requestor: r.requestor,
            selection: r.selection,
            target: r.target,
            property: if ok { property } else { NONE },
        };
        let _ = self
            .conn
            .send_event(false, r.requestor, EventMask::NO_EVENT, reply);
        let _ = self.conn.flush();
    }

    /// The `XdndAware` window (and its version, at least 3) under the root point.
    fn find_target(&self, x: i16, y: i16) -> Option<(Window, u32)> {
        self.aware_in(self.root, (0, 0), (i32::from(x), i32::from(y)), 0)
    }

    fn aware_in(
        &self,
        win: Window,
        origin: (i32, i32),
        point: (i32, i32),
        depth: u32,
    ) -> Option<(Window, u32)> {
        if win != self.root {
            let prop = self
                .conn
                .get_property(false, win, self.atoms.XdndAware, AtomEnum::ATOM, 0, 1)
                .ok()?
                .reply()
                .ok()?;
            if let Some(version) = prop.value32().and_then(|mut v| v.next()) {
                return (version >= 3).then_some((win, version));
            }
        }
        if depth >= MAX_DEPTH {
            return None;
        }
        let tree = self.conn.query_tree(win).ok()?.reply().ok()?;
        // Children are listed bottom to top: the first hit from the end is the visible one.
        for child in tree.children.iter().rev().take(MAX_CHILDREN) {
            if *child == self.win {
                continue;
            }
            let Some(attrs) = self
                .conn
                .get_window_attributes(*child)
                .ok()
                .and_then(|c| c.reply().ok())
            else {
                continue;
            };
            if attrs.map_state != MapState::VIEWABLE {
                continue;
            }
            // Input-only windows are invisible; they only count when they are the drop target.
            let invisible = attrs.class == WindowClass::INPUT_ONLY;
            let Some(g) = self
                .conn
                .get_geometry(*child)
                .ok()
                .and_then(|c| c.reply().ok())
            else {
                continue;
            };
            let border = i32::from(g.border_width);
            let (left, top) = (origin.0 + i32::from(g.x), origin.1 + i32::from(g.y));
            let (w, h) = (
                i32::from(g.width) + 2 * border,
                i32::from(g.height) + 2 * border,
            );
            if point.0 < left || point.1 < top || point.0 >= left + w || point.1 >= top + h {
                continue;
            }
            let hit = self.aware_in(*child, (left + border, top + border), point, depth + 1);
            if hit.is_some() || !invisible {
                return hit;
            }
        }
        None
    }
}

/// `DragError::Unsupported` carries a static reason; connection failures all mean the same.
fn leak_reason(_detail: &str) -> &'static str {
    "no X11 display (X11 drag out needs XWayland or an X11 session)"
}

#[cfg(test)]
mod tests;
