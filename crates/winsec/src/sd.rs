//! The Win32 security calls: the only module of this crate allowed to use `unsafe`.
//!
//! Every block states the invariant it relies on. Buffers handed to Windows are owned
//! `Vec<u64>` (8-byte aligned, as ACL structures expect) that outlive the call; the
//! descriptor is validated in safe code first and checked with `IsValidSecurityDescriptor`
//! before any pointer inside it is used.
#![allow(unsafe_code)]

use std::ffi::c_void;

use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_OWNER,
    ERROR_INVALID_PRIMARY_GROUP, ERROR_NOT_SUPPORTED, ERROR_PATH_NOT_FOUND,
    ERROR_PRIVILEGE_NOT_HELD, ERROR_SUCCESS, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{SE_FILE_OBJECT, SetNamedSecurityInfoW};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, GetFileSecurityW,
    GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetSecurityDescriptorGroup,
    GetSecurityDescriptorLength, GetSecurityDescriptorOwner, IsValidSecurityDescriptor,
    LookupAccountSidW, OBJECT_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE,
    UNPROTECTED_DACL_SECURITY_INFORMATION,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

use crate::validate::MAX_SD_BYTES;
use crate::{Parts, SecError, Security};

const SE_DACL_PROTECTED: u16 = 0x1000;
const NAME_UNITS: usize = 256;

fn map_error(code: WIN32_ERROR) -> SecError {
    match code {
        ERROR_ACCESS_DENIED
        | ERROR_PRIVILEGE_NOT_HELD
        | ERROR_INVALID_OWNER
        | ERROR_INVALID_PRIMARY_GROUP => SecError::PermissionDenied,
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => SecError::NotFound,
        ERROR_NOT_SUPPORTED => SecError::Unsupported,
        other => SecError::Os(other.0),
    }
}

fn last_error() -> SecError {
    let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    map_error(WIN32_ERROR(code as u32))
}

/// An 8-byte aligned copy of `bytes`, zero padded to a whole number of words.
fn aligned(bytes: &[u8]) -> Vec<u64> {
    let mut words = vec![0u64; bytes.len().div_ceil(8)];
    for (w, chunk) in words.iter_mut().zip(bytes.chunks(8)) {
        let mut b = [0u8; 8];
        b[..chunk.len()].copy_from_slice(chunk);
        *w = u64::from_ne_bytes(b);
    }
    words
}

fn psd(words: &mut [u64]) -> PSECURITY_DESCRIPTOR {
    PSECURITY_DESCRIPTOR(words.as_mut_ptr().cast::<c_void>())
}

pub fn read(wide: &[u16]) -> Result<Security, SecError> {
    let info = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let name = PCWSTR(wide.as_ptr());
    let mut needed = 0u32;
    // SAFETY: `wide` is NUL-terminated (checked by `wide_path`) and outlives the call; a null
    // descriptor with length 0 only asks for the required size.
    let first = unsafe { GetFileSecurityW(name, info.0, None, 0, &mut needed) };
    if first.as_bool() {
        return Err(SecError::Invalid("empty descriptor"));
    }
    let err = last_error();
    if err != SecError::Os(ERROR_INSUFFICIENT_BUFFER.0) {
        return Err(err);
    }
    let needed = needed as usize;
    if needed == 0 || needed > MAX_SD_BYTES {
        return Err(SecError::Invalid("too large"));
    }
    let mut buf = vec![0u64; needed.div_ceil(8)];
    let mut written = 0u32;
    // SAFETY: `buf` holds at least `needed` bytes and lives past the call.
    let second = unsafe {
        GetFileSecurityW(
            name,
            info.0,
            Some(psd(&mut buf)),
            needed as u32,
            &mut written,
        )
    };
    if !second.as_bool() {
        return Err(last_error());
    }
    // SAFETY: Windows just filled `buf` with a descriptor it produced.
    let len = unsafe { GetSecurityDescriptorLength(psd(&mut buf)) } as usize;
    if len == 0 || len > needed || len > MAX_SD_BYTES {
        return Err(SecError::Invalid("inconsistent length"));
    }
    let mut descriptor = Vec::with_capacity(buf.len() * 8);
    for w in &buf {
        descriptor.extend_from_slice(&w.to_ne_bytes());
    }
    descriptor.truncate(len);
    let (owner, group) = names(&mut buf);
    Ok(Security {
        descriptor,
        owner,
        group,
    })
}

/// Resolve owner and group to `DOMAIN\name`; unresolvable accounts just yield `None`.
fn names(buf: &mut [u64]) -> (Option<String>, Option<String>) {
    let mut owner = PSID::default();
    let mut group = PSID::default();
    let mut defaulted = BOOL(0);
    // SAFETY: `buf` holds a valid descriptor; the returned SIDs point inside it and are only
    // used while `buf` is alive.
    let (o, g) = unsafe {
        (
            GetSecurityDescriptorOwner(psd(buf), &mut owner, &mut defaulted).is_ok(),
            GetSecurityDescriptorGroup(psd(buf), &mut group, &mut defaulted).is_ok(),
        )
    };
    (
        o.then(|| lookup(owner)).flatten(),
        g.then(|| lookup(group)).flatten(),
    )
}

fn lookup(sid: PSID) -> Option<String> {
    if sid.0.is_null() {
        return None;
    }
    let mut name = [0u16; NAME_UNITS];
    let mut domain = [0u16; NAME_UNITS];
    let (mut nl, mut dl) = (NAME_UNITS as u32, NAME_UNITS as u32);
    let mut kind = SID_NAME_USE::default();
    // SAFETY: both buffers are `NAME_UNITS` long and their lengths are passed alongside;
    // `sid` points into a live, valid descriptor.
    let ok = unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid,
            Some(PWSTR(name.as_mut_ptr())),
            &mut nl,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut dl,
            &mut kind,
        )
    };
    ok.ok()?;
    let n = String::from_utf16(name.get(..nl as usize)?).ok()?;
    let d = String::from_utf16(domain.get(..dl as usize)?).ok()?;
    Some(if d.is_empty() { n } else { format!("{d}\\{n}") })
}

pub fn write(wide: &[u16], descriptor: &[u8], parts: Parts) -> Result<(), SecError> {
    let mut buf = aligned(descriptor);
    let sd = psd(&mut buf);
    // SAFETY: `buf` is a private, aligned copy of the (size- and offset-checked) blob.
    if !unsafe { IsValidSecurityDescriptor(sd) }.as_bool() {
        return Err(SecError::Invalid("rejected by Windows"));
    }

    let mut control = 0u16;
    let mut revision = 0u32;
    // SAFETY: `sd` is valid (above) and the out-pointers are live locals.
    unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) }
        .map_err(|_| SecError::Invalid("no control flags"))?;

    let (mut owner, mut group) = (PSID::default(), PSID::default());
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let (mut defaulted, mut present) = (BOOL(0), BOOL(0));
    // SAFETY: as above; the SID and ACL pointers returned point inside `buf`, which lives
    // until the end of this function.
    unsafe {
        GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted)
            .map_err(|_| SecError::Invalid("bad owner"))?;
        GetSecurityDescriptorGroup(sd, &mut group, &mut defaulted)
            .map_err(|_| SecError::Invalid("bad group"))?;
        GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted)
            .map_err(|_| SecError::Invalid("bad DACL"))?;
    }

    let mut info = OBJECT_SECURITY_INFORMATION(0);
    if parts.owner && !owner.0.is_null() {
        info |= OWNER_SECURITY_INFORMATION;
    }
    if parts.owner && !group.0.is_null() {
        info |= GROUP_SECURITY_INFORMATION;
    }
    if parts.dacl && present.as_bool() {
        info |= DACL_SECURITY_INFORMATION;
        info |= if control & SE_DACL_PROTECTED != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
    }
    if info.0 == 0 {
        return Ok(());
    }
    // SAFETY: `wide` is NUL-terminated; the SID/ACL pointers are null or point into `buf`,
    // which outlives the call. Windows only reads them.
    let code = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            info,
            (parts.owner && !owner.0.is_null()).then_some(owner),
            (parts.owner && !group.0.is_null()).then_some(group),
            (parts.dacl && present.as_bool() && !dacl.is_null()).then_some(dacl.cast_const()),
            None,
        )
    };
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(map_error(code))
    }
}
