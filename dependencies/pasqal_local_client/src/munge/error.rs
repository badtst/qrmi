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

use std::fmt;

#[derive(Debug)]
pub enum MungeError {
    EncodeFailed(String),
    /// `libmunge` could not be loaded (or a symbol resolved) at runtime, e.g.
    /// because the host doesn't have munge installed. Unlike the previous
    /// link-time dependency, this is a normal, expected error on hosts that
    /// were never meant to authenticate against a munge-protected service.
    LibraryUnavailable(String),
}

impl fmt::Display for MungeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MungeError::EncodeFailed(msg) => write!(f, "munge encode failed: {msg}"),
            MungeError::LibraryUnavailable(msg) => write!(f, "munge is not available: {msg}"),
        }
    }
}

impl std::error::Error for MungeError {}
