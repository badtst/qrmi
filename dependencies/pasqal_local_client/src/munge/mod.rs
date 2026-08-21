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

mod error;
mod ffi;

use crate::munge::error::MungeError;

use std::ffi::CStr;
use std::ptr;

pub fn encode(payload: &[u8]) -> Result<String, MungeError> {
    let mut cred_ptr = ptr::null_mut();

    let rc = ffi::munge_encode(
        &mut cred_ptr,
        ptr::null_mut(),
        payload.as_ptr() as *const _,
        payload.len(),
    )?;

    if rc != 0 {
        let msg = unsafe {
            CStr::from_ptr(ffi::munge_strerror(rc)?)
                .to_string_lossy()
                .into_owned()
        };
        return Err(MungeError::EncodeFailed(msg));
    }

    if cred_ptr.is_null() {
        return Err(MungeError::EncodeFailed("null credential".into()));
    }

    let token = unsafe { CStr::from_ptr(cred_ptr).to_string_lossy().into_owned() };

    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `libmunge` is now loaded via `dlopen` instead of link-time linking,
    /// so a host without it must fail gracefully (`LibraryUnavailable`)
    /// rather than the crate failing to build. On a host that does have
    /// munge running (e.g. this devcontainer), `encode` must still produce
    /// a real credential -- anything else (in particular `EncodeFailed`,
    /// which would mean the library loaded but the call itself is broken)
    /// is a genuine bug.
    #[test]
    fn encode_succeeds_or_reports_library_unavailable() {
        match encode(b"") {
            Ok(token) => assert!(
                token.starts_with("MUNGE:"),
                "unexpected credential format: {token:?}"
            ),
            Err(MungeError::LibraryUnavailable(_)) => {
                eprintln!("munge not available on this host, skipping credential check");
            }
            Err(other) => panic!("unexpected error from encode(): {other}"),
        }
    }
}
