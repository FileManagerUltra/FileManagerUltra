//! The only `unsafe` in the project: adopting the window system connection that winit owns.
//!
//! Wayland starts a drag only for the connection that received the button press, so a second
//! connection cannot do it. Until winit (0.31) offers a drag source API, we attach our own
//! event queue to winit's `wl_display`. See `docs/roadmap.md` (priority 1: remove this crate).
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;

use wayland_backend::client::{Backend, ObjectId};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Proxy};

/// A connection sharing `display`, which is left open when the connection is dropped.
///
/// The caller guarantees `display` is a live `wl_display*` that outlives the result (the
/// pointer comes from a window handle, and the window outlives its owner).
pub(crate) fn connection(display: NonNull<c_void>) -> Connection {
    // SAFETY: the pointer is the live `wl_display` of the window; a foreign display is never
    // disconnected by the backend.
    let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
    Connection::from_backend(backend)
}

/// The window's `wl_surface` as a proxy on `conn`.
pub(crate) fn surface(conn: &Connection, surface: NonNull<c_void>) -> Option<WlSurface> {
    // SAFETY: the pointer is the live `wl_surface` of the window on this display.
    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) }.ok()?;
    WlSurface::from_id(conn, id).ok()
}
