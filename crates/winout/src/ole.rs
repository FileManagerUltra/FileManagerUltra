//! The OLE drag source: the only module of this crate allowed to use `unsafe`.
//!
//! Every `unsafe` block states the invariant it relies on. The data object offers exactly
//! one format, `CF_HDROP` (the classic file list, built in safe code by
//! `paths::dropfiles`), so receivers take their plain "dropped files" route and nothing
//! else is exposed; the files themselves are never opened.
#![allow(unsafe_code)]

use std::ptr;

use fm_dragout::{DragError, Outcome};
use windows::Win32::Foundation::{
    DATA_S_SAMEFORMATETC, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
    DV_E_FORMATETC, DV_E_TYMED, E_NOTIMPL, E_OUTOFMEMORY, GlobalFree, HGLOBAL,
    OLE_E_ADVISENOTSUPPORTED, S_OK,
};
use windows::Win32::System::Com::{
    DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IAdviseSink, IDataObject, IDataObject_Impl,
    IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE, DoDragDrop,
    IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::Shell::SHCreateStdEnumFmtEtc;
use windows::core::{BOOL, HRESULT, Ref, Result, implement};

fn system(what: &str, err: windows::core::Error) -> DragError {
    DragError::System(format!("{what}: {err}"))
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
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

/// A data object holding one `CF_HDROP` block (a `DROPFILES` header and the paths).
#[implement(IDataObject)]
struct Files {
    blob: Vec<u8>,
}

/// `Ok` when `format` asks for the file list in global memory.
///
/// # Safety
/// `format` must be null or point to a valid `FORMATETC`.
unsafe fn check(format: *const FORMATETC) -> Result<()> {
    // SAFETY: the caller guarantees a null or valid pointer.
    let Some(f) = (unsafe { format.as_ref() }) else {
        return Err(DV_E_FORMATETC.into());
    };
    if f.cfFormat != CF_HDROP.0 || f.dwAspect & DVASPECT_CONTENT.0 == 0 {
        return Err(DV_E_FORMATETC.into());
    }
    if f.tymed & TYMED_HGLOBAL.0 as u32 == 0 {
        return Err(DV_E_TYMED.into());
    }
    Ok(())
}

impl IDataObject_Impl for Files_Impl {
    fn GetData(&self, format: *const FORMATETC) -> Result<STGMEDIUM> {
        // SAFETY: COM passes a valid `FORMATETC` pointer (or null, which `check` rejects).
        unsafe { check(format)? };
        let len = self.blob.len();
        // SAFETY: the block is allocated for `len` bytes, locked while copying exactly `len`
        // bytes from a slice of that length, then unlocked. Ownership of the HGLOBAL passes
        // to the receiver, which releases it (`pUnkForRelease` is null).
        unsafe {
            let global: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, len)?;
            let dst = GlobalLock(global);
            if dst.is_null() {
                let _ = GlobalFree(Some(global));
                return Err(E_OUTOFMEMORY.into());
            }
            ptr::copy_nonoverlapping(self.blob.as_ptr(), dst.cast::<u8>(), len);
            let _ = GlobalUnlock(global);
            Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: global },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            })
        }
    }

    fn GetDataHere(&self, _format: *const FORMATETC, _medium: *mut STGMEDIUM) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        // SAFETY: COM passes a valid `FORMATETC` pointer (or null, which `check` rejects).
        match unsafe { check(format) } {
            Ok(()) => S_OK,
            Err(e) => e.code(),
        }
    }

    fn GetCanonicalFormatEtc(&self, _in: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        // SAFETY: COM passes a valid output pointer (or null, which is skipped).
        if let Some(out) = unsafe { out.as_mut() } {
            out.ptd = ptr::null_mut();
        }
        DATA_S_SAMEFORMATETC
    }

    fn SetData(&self, _f: *const FORMATETC, _m: *const STGMEDIUM, _release: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, direction: u32) -> Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        // SAFETY: the slice is valid for the call; the shell copies the formats.
        unsafe { SHCreateStdEnumFmtEtc(&[hdrop_format()]) }
    }

    fn DAdvise(&self, _f: *const FORMATETC, _advf: u32, _sink: Ref<IAdviseSink>) -> Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _connection: u32) -> Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

/// Runs the drag; blocks until the button is released or Escape. Must be called on the
/// thread that owns the window the button was pressed in: OLE reads the button state and
/// takes the mouse capture from the calling thread. `blob` is the `CF_HDROP` payload from
/// `paths::dropfiles`.
pub fn run(blob: Vec<u8>) -> Result2 {
    // SAFETY: OLE is initialised on this thread before any COM call and uninitialised after
    // the last COM object created here is released (`drag` returns before it).
    unsafe {
        OleInitialize(None).map_err(|e| system("OLE initialisation failed", e))?;
        let result = drag(blob);
        // The window system's own capture (taken on button press) never sees the button
        // release that ended the drag; drop it so other windows get the mouse back.
        let _ = ReleaseCapture();
        OleUninitialize();
        result
    }
}

type Result2 = std::result::Result<Outcome, DragError>;

/// # Safety
/// OLE must be initialised on the calling thread.
unsafe fn drag(blob: Vec<u8>) -> Result2 {
    let data: IDataObject = Files { blob }.into();
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
