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

//! The error type [`Client`](crate::Client)'s request helpers return on a
//! non-2xx HTTP response, carrying the status code as a typed value instead
//! of folding it into a formatted string. This lets callers (in particular
//! `qrmi`'s `src/pasqal/error.rs`) classify failures by status.

use reqwest::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("status {status}: {body}")]
pub struct ApiError {
    pub status: StatusCode,
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_display_includes_status_and_body() {
        let err = ApiError {
            status: StatusCode::NOT_FOUND,
            body: "job not found".to_string(),
        };
        assert_eq!(err.to_string(), "status 404 Not Found: job not found");
    }

    #[test]
    fn api_error_is_downcastable_from_anyhow() {
        let err: anyhow::Error = ApiError {
            status: StatusCode::BAD_REQUEST,
            body: "bad sequence".to_string(),
        }
        .into();
        let downcast = err.downcast::<ApiError>().expect("should downcast");
        assert_eq!(downcast.status, StatusCode::BAD_REQUEST);
        assert_eq!(downcast.body, "bad sequence");
    }
}
