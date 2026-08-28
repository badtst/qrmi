// This code is part of Qiskit.
//
// Copyright (C): 2026 UKRI-STFC (Hartree Centre)
//
// This code is licensed under the Apache License, Version 2.0. You may
// obtain a copy of this license in the LICENSE.txt file in the root directory
// of this source tree or at http://www.apache.org/licenses/LICENSE-2.0.
//
// Any modifications or derivative works of this code must retain this
// copyright notice, and modified files need to carry a notice indicating
// that they have been altered from the originals.

//! Error conditions specific to the Pasqal backends, as opposed to
//! [`crate::error::QrmiError`], which only knows about conditions that are
//! generic across every vendor. Values of this type reach callers wrapped in
//! `QrmiError::Pasqal(_)` via `?` (see the `#[from]` on that variant).
//!
//! Also home to [`classify_cloud`] and [`classify_local`], which map failed
//! `pasqal_cloud_api`/`pasqal_local_api` calls onto [`crate::QrmiError`] by
//! HTTP status, the same way [`crate::alice_bob::error::classify`] does for
//! Alice & Bob.

use crate::error::{QrmiError, QrmiErrorKind};
use http::StatusCode;
use thiserror::Error;

/// Errors that only make sense in the context of Pasqal's backends: they
/// name Pasqal-specific concepts (device types, CUDA-Q sequence payloads)
/// that the framework-level [`crate::QrmiError`] has no business knowing
/// about.
#[derive(Error, Debug)]
pub enum PasqalError {
    /// The backend name did not match any known Pasqal Cloud device type
    /// (e.g. `FRESNEL`, `EMU_MPS`).
    #[error("{0}")]
    InvalidDeviceType(String),

    /// A CUDA-Q sequence payload could not be parsed as JSON.
    #[error("failed to parse CUDA-Q sequence payload: {0}")]
    InvalidCudaqSequence(String),
}

impl PasqalError {
    pub(crate) fn kind(&self) -> QrmiErrorKind {
        match self {
            PasqalError::InvalidDeviceType(_) => QrmiErrorKind::InvalidInput,
            PasqalError::InvalidCudaqSequence(_) => QrmiErrorKind::InvalidInput,
        }
    }
}

/// Disambiguates a 404 from Pasqal's APIs: on its own, the status code
/// doesn't say whether it was a backend/device or a job/batch that wasn't
/// found.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ResourceKind {
    Backend,
    Job,
}

fn classify_status(
    status: StatusCode,
    resource_kind: ResourceKind,
) -> Option<fn(String) -> QrmiError> {
    match (status, resource_kind) {
        (StatusCode::BAD_REQUEST, _) | (StatusCode::UNPROCESSABLE_ENTITY, _) => {
            Some(QrmiError::InvalidInput)
        }
        (StatusCode::UNAUTHORIZED, _) => Some(QrmiError::AuthenticationFailed),
        (StatusCode::NOT_FOUND, ResourceKind::Backend) => Some(QrmiError::ResourceNotFound),
        (StatusCode::NOT_FOUND, ResourceKind::Job) => Some(QrmiError::TaskNotFound),
        _ => None,
    }
}

/// Maps a failed `pasqal_cloud_api` call onto [`QrmiError`]. `resource_kind`
/// disambiguates a 404 (see [`ResourceKind`]). Mirrors
/// [`crate::alice_bob::error::classify`]'s approach, adapted for
/// `pasqal_cloud_api::ApiError` (a plain `{status, body}` pair, since that
/// crate -- unlike the OpenAPI-generated clients -- is hand-written and
/// doesn't have a typed per-operation error model).
///
/// Falls through to [`QrmiError::Other`] for a status this function
/// doesn't otherwise recognize (403 included), or for any error that isn't
/// a [`pasqal_cloud_api::ApiError`] at all (a transport-level failure has no
/// status to classify by). Either way the original error is kept as
/// `source`.
pub(crate) fn classify_cloud(err: anyhow::Error, resource_kind: ResourceKind) -> QrmiError {
    match err.downcast::<pasqal_cloud_api::ApiError>() {
        Ok(api_err) => classify_status(api_err.status, resource_kind)
            .map(|make| make(api_err.body.clone()))
            .unwrap_or_else(|| QrmiError::Other(anyhow::Error::new(api_err))),
        Err(err) => QrmiError::Other(err),
    }
}

/// Same as [`classify_cloud`], for `pasqal_local_api::ApiError`.
pub(crate) fn classify_local(err: anyhow::Error, resource_kind: ResourceKind) -> QrmiError {
    match err.downcast::<pasqal_local_api::ApiError>() {
        Ok(api_err) => classify_status(api_err.status, resource_kind)
            .map(|make| make(api_err.body.clone()))
            .unwrap_or_else(|| QrmiError::Other(anyhow::Error::new(api_err))),
        Err(err) => QrmiError::Other(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_cloud_maps_known_statuses() {
        let err = |status, body: &str| {
            anyhow::Error::new(pasqal_cloud_api::ApiError {
                status,
                body: body.to_string(),
            })
        };
        assert_eq!(
            classify_cloud(err(StatusCode::BAD_REQUEST, "bad"), ResourceKind::Backend).kind(),
            QrmiErrorKind::InvalidInput
        );
        assert_eq!(
            classify_cloud(
                err(StatusCode::UNPROCESSABLE_ENTITY, "bad"),
                ResourceKind::Backend
            )
            .kind(),
            QrmiErrorKind::InvalidInput
        );
        assert_eq!(
            classify_cloud(err(StatusCode::UNAUTHORIZED, "no"), ResourceKind::Backend).kind(),
            QrmiErrorKind::AuthenticationFailed
        );
        assert_eq!(
            classify_cloud(err(StatusCode::NOT_FOUND, "gone"), ResourceKind::Backend).kind(),
            QrmiErrorKind::ResourceNotFound
        );
        assert_eq!(
            classify_cloud(err(StatusCode::NOT_FOUND, "gone"), ResourceKind::Job).kind(),
            QrmiErrorKind::TaskNotFound
        );
        assert_eq!(
            classify_cloud(err(StatusCode::FORBIDDEN, "nope"), ResourceKind::Backend).kind(),
            QrmiErrorKind::Other
        );
    }

    #[test]
    fn classify_cloud_falls_back_to_other_for_non_api_errors() {
        let err = anyhow::anyhow!("connection reset");
        assert_eq!(
            classify_cloud(err, ResourceKind::Backend).kind(),
            QrmiErrorKind::Other
        );
    }

    #[test]
    fn classify_local_maps_known_statuses() {
        let err = anyhow::Error::new(pasqal_local_api::ApiError {
            status: StatusCode::NOT_FOUND,
            body: "job not found".to_string(),
        });
        assert_eq!(
            classify_local(err, ResourceKind::Job).kind(),
            QrmiErrorKind::TaskNotFound
        );
    }
}
