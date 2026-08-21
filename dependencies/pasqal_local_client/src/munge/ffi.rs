//
// (C) Copyright Pasqal SAS 2026
//
// This code is licensed under the Apache License, Version 2.0. You may
// obtain a copy of this license in the LICENSE.txt file in the root directory
// of this source tree or at http://www.apache.org/licenses/LICENSE-2.0.
//
// Any modifications or derivative works of this code must retain this
// copyright notice, and modified files need to carry a notice indicating
// that they have been altered from the originals.

//! Loads `libmunge` at runtime via `dlopen` instead of linking it in at
//! build time. This means a binary built with the `munge` feature no longer
//! needs `libmunge`/`munge.h` present in the build environment (e.g. a
//! manylinux container building a PyPI wheel): it just needs `libmunge` to
//! be present on whatever host actually calls [`crate::munge::encode`]. On
//! a host without it, that call fails with [`MungeError::LibraryUnavailable`]
//! instead of the whole crate failing to link.

use std::ffi::c_void;
use std::os::raw::{c_char, c_int};
use std::sync::OnceLock;

use libloading::{Library, Symbol};

use super::error::MungeError;

type MungeEncodeFn = unsafe extern "C" fn(
    cred: *mut *mut c_char,
    ctx: *mut c_void,
    data: *const c_void,
    len: usize,
) -> c_int;
type MungeStrerrorFn = unsafe extern "C" fn(err: c_int) -> *const c_char;

/// Function pointers resolved from `libmunge`, plus the `Library` handle
/// that must stay loaded for as long as they're callable.
struct MungeLib {
    _lib: Library,
    encode: MungeEncodeFn,
    strerror: MungeStrerrorFn,
}

// SAFETY: `MungeLib` only holds a loaded `Library` and plain C function
// pointers copied out of it; neither has any thread-affine state.
unsafe impl Send for MungeLib {}
unsafe impl Sync for MungeLib {}

/// The library's SONAME on Linux is `libmunge.so.2`; `libmunge.so` (no
/// version) is the dev-package symlink, kept as a fallback for systems that
/// only have that installed.
const LIB_NAMES: &[&str] = &["libmunge.so.2", "libmunge.so"];

fn load() -> Result<&'static MungeLib, MungeError> {
    static LIB: OnceLock<Result<MungeLib, String>> = OnceLock::new();
    LIB.get_or_init(|| {
        let lib = LIB_NAMES
            .iter()
            // SAFETY: loading an arbitrary shared library is inherently
            // unsafe (its code runs unchecked), but `libmunge` is a
            // well-known system library, not untrusted input.
            .find_map(|name| unsafe { Library::new(name) }.ok())
            .ok_or_else(|| {
                format!("could not load any of {LIB_NAMES:?}; is munge installed on this host?")
            })?;

        // SAFETY: these signatures must match libmunge's real ones exactly;
        // munge's public C API (munge.h) has been stable since 0.5.x.
        let (encode, strerror) = unsafe {
            let encode: Symbol<MungeEncodeFn> = lib
                .get(b"munge_encode\0")
                .map_err(|e| format!("failed to resolve munge_encode: {e}"))?;
            let strerror: Symbol<MungeStrerrorFn> = lib
                .get(b"munge_strerror\0")
                .map_err(|e| format!("failed to resolve munge_strerror: {e}"))?;
            // Copy the raw function pointers out so they don't borrow from
            // `Symbol`/`lib` -- both are plain `fn` pointers, `Copy`, and
            // remain valid as long as `lib` (stored alongside them below)
            // stays loaded.
            (*encode, *strerror)
        };

        Ok(MungeLib {
            _lib: lib,
            encode,
            strerror,
        })
    })
    .as_ref()
    .map_err(|e| MungeError::LibraryUnavailable(e.clone()))
}

pub(crate) fn munge_encode(
    cred: *mut *mut c_char,
    ctx: *mut c_void,
    data: *const c_void,
    len: usize,
) -> Result<c_int, MungeError> {
    let lib = load()?;
    // SAFETY: `cred`/`ctx`/`data`/`len` are forwarded verbatim from the
    // caller, which upholds the same contract callers of the raw C API
    // always had to uphold.
    Ok(unsafe { (lib.encode)(cred, ctx, data, len) })
}

pub(crate) fn munge_strerror(err: c_int) -> Result<*const c_char, MungeError> {
    let lib = load()?;
    // SAFETY: `munge_strerror` returns a pointer to a static string owned
    // by libmunge; it is always safe to call with any `c_int`.
    Ok(unsafe { (lib.strerror)(err) })
}
