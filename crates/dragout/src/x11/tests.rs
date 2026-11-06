//! Protocol tests against the real display: a receiver window answers like a browser would.
//! Without an X display (CI) they skip.

#![allow(clippy::expect_used)]

use std::sync::atomic::AtomicBool as Flag;
use std::sync::{Arc, Mutex, MutexGuard, mpsc};

use super::*;

type Ready = mpsc::Receiver<(i16, i16)>;

fn spawn_target(
    aware: bool,
    stop: Arc<Flag>,
) -> Option<(Ready, std::thread::JoinHandle<Option<String>>)> {
    let (conn, screen) = RustConnection::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let atoms = Atoms::new(&conn).ok()?.reply().ok()?;
    let uri_list = conn
        .intern_atom(false, b"text/uri-list")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let win = conn.generate_id().ok()?;
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        win,
        root,
        40,
        40,
        300,
        300,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
    )
    .ok()?;
    if aware {
        conn.change_property32(
            PropMode::REPLACE,
            win,
            atoms.XdndAware,
            AtomEnum::ATOM,
            &[5],
        )
        .ok()?;
    }
    conn.map_window(win).ok()?;
    conn.flush().ok()?;
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let mut announced = false;
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
            let Ok(Some(event)) = conn.poll_for_event() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            match event {
                Event::MapNotify(_) if !announced => {
                    announced = true;
                    let at = conn
                        .translate_coordinates(win, root, 100, 100)
                        .ok()?
                        .reply()
                        .ok()?;
                    let _ = tx.send((at.dst_x, at.dst_y));
                }
                Event::ClientMessage(m) if m.type_ == atoms.XdndPosition => {
                    let d = m.data.as_data32();
                    let reply = ClientMessageEvent::new(
                        32,
                        d[0],
                        atoms.XdndStatus,
                        [win, 1, 0, 0, atoms.XdndActionCopy],
                    );
                    conn.send_event(false, d[0], EventMask::NO_EVENT, reply)
                        .ok()?;
                    conn.flush().ok()?;
                }
                Event::ClientMessage(m) if m.type_ == atoms.XdndDrop => {
                    let d = m.data.as_data32();
                    conn.convert_selection(
                        win,
                        atoms.XdndSelection,
                        uri_list,
                        atoms.FM_DRAG_TIME,
                        d[2],
                    )
                    .ok()?;
                    conn.flush().ok()?;
                }
                Event::SelectionNotify(n) if n.property != NONE => {
                    let p = conn
                        .get_property(true, win, n.property, AtomEnum::ANY, 0, 1 << 16)
                        .ok()?
                        .reply()
                        .ok()?;
                    let owner = conn
                        .get_selection_owner(atoms.XdndSelection)
                        .ok()?
                        .reply()
                        .ok()?
                        .owner;
                    let done = ClientMessageEvent::new(
                        32,
                        owner,
                        atoms.XdndFinished,
                        [win, 1, atoms.XdndActionCopy, 0, 0],
                    );
                    conn.send_event(false, owner, EventMask::NO_EVENT, done)
                        .ok()?;
                    conn.flush().ok()?;
                    return Some(String::from_utf8_lossy(&p.value).into_owned());
                }
                _ => {}
            }
        }
        None
    });
    Some((rx, handle))
}

/// Pointer parked at `at`, button released after `held` samples.
fn scripted(
    at: (i16, i16),
    held: u32,
) -> impl FnMut(&RustConnection, Window) -> Result<Sample, DragError> {
    let mut n = 0u32;
    move |_, _| {
        n += 1;
        Ok(Sample {
            x: at.0,
            y: at.1,
            down: n <= held,
        })
    }
}

/// The receiver window; it is unmapped when this is dropped so tests do not see each other.
struct Receiver {
    at: (i16, i16),
    handle: Option<std::thread::JoinHandle<Option<String>>>,
    stop: Arc<Flag>,
    _serial: MutexGuard<'static, ()>,
}

impl Receiver {
    fn received(mut self) -> Option<String> {
        self.handle.take()?.join().ok()?
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

static SERIAL: Mutex<()> = Mutex::new(());

/// Receiver mapped and positioned, or `None` (no display / no window manager reaction).
fn ready(aware: bool) -> Option<Receiver> {
    let serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let stop = Arc::new(Flag::new(false));
    let Some((rx, handle)) = spawn_target(aware, stop.clone()) else {
        eprintln!("no X display; skipping");
        return None;
    };
    let at = rx.recv_timeout(Duration::from_secs(5)).ok()?;
    Some(Receiver {
        at,
        handle: Some(handle),
        stop,
        _serial: serial,
    })
}

#[test]
fn delivers_the_uri_list_to_an_aware_window() {
    let Some(target) = ready(true) else {
        return;
    };
    let list = "file:///tmp/a%20b.png\r\n".to_string();
    let source = Source::connect(list.clone()).expect("source").direct();
    let outcome = source
        .run(&AtomicBool::new(false), &mut scripted(target.at, 25))
        .expect("run");
    assert_eq!(outcome, Outcome::Dropped);
    assert_eq!(target.received(), Some(list));
}

#[test]
fn release_over_an_unaware_window_is_no_target() {
    let Some(target) = ready(false) else {
        return;
    };
    let source = Source::connect("file:///x\r\n".into())
        .expect("source")
        .direct();
    let outcome = source
        .run(&AtomicBool::new(false), &mut scripted(target.at, 25))
        .expect("run");
    assert_eq!(outcome, Outcome::NoTarget);
}

#[test]
fn a_cancelled_drag_ends_without_dropping() {
    let Some(target) = ready(true) else {
        return;
    };
    let source = Source::connect("file:///x\r\n".into())
        .expect("source")
        .direct();
    let outcome = source
        .run(&AtomicBool::new(true), &mut scripted(target.at, 25))
        .expect("run");
    assert_eq!(outcome, Outcome::Cancelled);
}

#[test]
fn a_drag_that_starts_with_the_button_up_does_nothing() {
    let Some(target) = ready(true) else {
        return;
    };
    let source = Source::connect("file:///x\r\n".into())
        .expect("source")
        .direct();
    let outcome = source
        .run(&AtomicBool::new(false), &mut scripted(target.at, 0))
        .expect("run");
    assert_eq!(outcome, Outcome::Cancelled);
}
