//! The OLE drag source: the only module of this crate allowed to use `unsafe`.
//!
//! Every `unsafe` block states the invariant it relies on. The data object is the shell's
//! own (`SHCreateDataObject`) built from the dragged paths, so receivers get the standard
//! file-drop formats (`CF_HDROP`, shell ID list); nothing else is offered and the files
//! are never opened.
#![allow(unsafe_code)]

use std::ptr;

use fm_dragout::{DragError, Outcome};
use windows::Win32::Foundation::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK,
};
use windows::Win32::System::Com::{IBindCtx, IDataObject};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE, DoDragDrop, IDropSource,
    IDropSource_Impl, OleInitialize, OleUninitialize,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILFree, SHCreateDataObject, SHParseDisplayName};
use windows::core::{BOOL, HRESULT, PCWSTR, implement};

fn system(what: &str, err: windows::core::Error) -> DragError {
    DragError::System(format!("{what}: {err}"))
}

/// Ends the drag on Escape or when the primary button is released.
#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if !keys.contains(MK_LBUTTON) {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Item ID lists from `SHParseDisplayName`, released with `ILFree` on drop.
struct Pidls(Vec<*mut ITEMIDLIST>);

impl Drop for Pidls {
    fn drop(&mut self) {
        for pidl in self.0.drain(..) {
            // SAFETY: each pointer came from a successful `SHParseDisplayName`, is non-null
            // and is freed exactly once here.
            unsafe { ILFree(Some(pidl.cast_const())) };
        }
    }
}

/// Runs the drag; blocks until the button is released or Escape. Must be called on the
/// thread that owns the window the button was pressed in: OLE reads the button state and
/// takes the mouse capture from the calling thread. `wide` holds NUL-terminated UTF-16
/// absolute paths (see `paths::wide_paths`).
pub fn run(wide: &[Vec<u16>]) -> Result<Outcome, DragError> {
    // SAFETY: OLE is initialised on this thread before any COM call and uninitialised after
    // the last COM object created here is released (`drag` returns before it).
    unsafe {
        OleInitialize(None).map_err(|e| system("OLE initialisation failed", e))?;
        let result = drag(wide);
        // The window system's own capture (taken on button press) never sees the button
        // release that ended the drag; drop it so other windows get the mouse back.
        let _ = ReleaseCapture();
        OleUninitialize();
        result
    }
}

/// # Safety
/// OLE must be initialised on the calling thread.
unsafe fn drag(wide: &[Vec<u16>]) -> Result<Outcome, DragError> {
    let mut pidls = Pidls(Vec::with_capacity(wide.len()));
    for path in wide {
        let mut pidl: *mut ITEMIDLIST = ptr::null_mut();
        // SAFETY: `path` is NUL-terminated (guaranteed by `wide_paths`) and outlives the
        // call; `pidl` is a valid out pointer. On success the pidl is owned by `pidls`.
        unsafe {
            SHParseDisplayName(PCWSTR(path.as_ptr()), None::<&IBindCtx>, &mut pidl, 0, None)
                .map_err(|e| system("cannot resolve a dragged file", e))?;
        }
        pidls.0.push(pidl);
    }
    let list: Vec<*const ITEMIDLIST> = pidls.0.iter().map(|p| p.cast_const()).collect();
    // SAFETY: every pointer in `list` is a live absolute pidl owned by `pidls`, which
    // outlives the data object's creation (the shell copies what it needs).
    let data: IDataObject = unsafe {
        SHCreateDataObject(None, Some(&list), None::<&IDataObject>)
            .map_err(|e| system("cannot create the drag data", e))?
    };
    let source: IDropSource = Source.into();
    let mut effect = DROPEFFECT_NONE;
    // SAFETY: `data` and `source` are live COM objects and `effect` is a valid out pointer.
    let hr = unsafe {
        DoDragDrop(
            &data,
            &source,
            DROPEFFECT_COPY | DROPEFFECT_MOVE,
            &mut effect,
        )
    };
    if hr == DRAGDROP_S_DROP {
        Ok(if effect == DROPEFFECT_NONE {
            Outcome::NoTarget
        } else {
            Outcome::Dropped
        })
    } else if hr == DRAGDROP_S_CANCEL {
        Ok(Outcome::Cancelled)
    } else {
        Err(system(
            "the drag failed",
            windows::core::Error::from_hresult(hr),
        ))
    }
}
