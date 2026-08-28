# Pasqal Error Classification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Classify Pasqal Cloud and Pasqal Local API failures into specific `QrmiError` variants (`ResourceNotFound`, `TaskNotFound`, `AuthenticationFailed`, `InvalidInput`), closing the gap called out in `docs/migration/0.24.0.md` ("Pasqal Cloud / Local — Not yet — still reports `Other` for everything").

**Architecture:** Add a typed `ApiError { status, body }` to each of `pasqal_cloud_client` and `pasqal_local_client` (replacing their current `bail!`-based failure path, which discards the HTTP status). Add `classify_cloud`/`classify_local` functions to `qrmi`'s `src/pasqal/error.rs` that downcast to the relevant crate's `ApiError` and map `status` to a `QrmiError` variant, exactly like `src/alice_bob/error.rs::classify`. Wire `.map_err(|e| classify_*(e, ResourceKind::...))` into every fallible call site in `src/pasqal/cloud.rs` and `src/pasqal/local.rs`.

**Tech Stack:** Rust, `thiserror`, `anyhow`, `reqwest`, `http::StatusCode`, `tokio::test` + raw `TcpListener` mock servers (existing pattern in `src/pasqal/tests/cloud.rs`).

## Global Constraints

- Match the existing classification pattern exactly (see `src/alice_bob/error.rs`, `src/ibm/error.rs`, `src/iqm/error.rs`) — do not invent a new shape.
- Status mapping (confirmed with maintainer, no vendor-specific exceptions known): 400/422 → `InvalidInput`, 401 → `AuthenticationFailed`, 404 → `ResourceNotFound` (backend/device) or `TaskNotFound` (job/batch), everything else (403 included) and non-HTTP failures → `Other`.
- `create_session`/`revoke_session` (Local) have no natural `ResourceKind` (a session isn't a resource or a task) — leave unclassified, falls to `Other`. Same reasoning as IBM's `Session` `ResourceKind` case.
- Don't touch `PasqalError` (`InvalidDeviceType`, `InvalidCudaqSequence`) — already classified.
- Don't touch `docs/migration/0.24.0.md`.
- Each client crate keeps its own `ApiError` — no new shared crate.

---

### Task 1: `ApiError` in `pasqal_cloud_client`

**Files:**
- Create: `dependencies/pasqal_cloud_client/src/error.rs`
- Modify: `dependencies/pasqal_cloud_client/src/lib.rs`
- Modify: `dependencies/pasqal_cloud_client/src/client.rs:238-248` (`handle_request`)
- Test: `dependencies/pasqal_cloud_client/src/error.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `pub struct ApiError { pub status: reqwest::StatusCode, pub body: String }`, implementing `std::error::Error` (via `thiserror::Error`) and `Display` as `"status {status}: {body}"`. Re-exported as `pasqal_cloud_api::ApiError`.

- [ ] **Step 1: Write the failing test**

Create `dependencies/pasqal_cloud_client/src/error.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pasqal-cloud-api error:: -- --nocapture`
Expected: FAIL to compile — `mod error` is not yet declared in `lib.rs`, so `cargo test -p pasqal-cloud-api` won't even find the module (adjust: run `cargo test -p pasqal-cloud-api` and confirm it errors with "unresolved module" once you add the `mod error;` line in the next step, or simply confirm the crate doesn't build with the module referenced but empty file wired — the meaningful failure to observe is step 4's compile before `ApiError` is wired into `client.rs`. If step 1 alone already compiles once `lib.rs` is updated, that's fine; the important check is step 4).

- [ ] **Step 3: Wire the module in and re-run**

In `dependencies/pasqal_cloud_client/src/lib.rs`, add:

```rust
mod client;
mod error;
mod models;

pub use auth::AccessTokenRequest;
pub use client::{Client, ClientBuilder};
pub use error::ApiError;
pub use models::AuthError;
pub use models::DeviceType;
pub use models::JobStatus;
```

Run: `cargo test -p pasqal-cloud-api error::`
Expected: PASS (2 tests: `api_error_display_includes_status_and_body`, `api_error_is_downcastable_from_anyhow`)

- [ ] **Step 4: Use `ApiError` in `handle_request`**

In `dependencies/pasqal_cloud_client/src/client.rs`, replace the `handle_request` body (currently at lines 238-248):

```rust
async fn handle_request<T: DeserializeOwned>(&self, resp: reqwest::Response) -> Result<T> {
    if resp.status().is_success() {
        let json_text = resp.text().await?;
        let val = serde_json::from_str(&json_text)?;
        Ok(val)
    } else {
        let status = resp.status();
        let body = resp.text().await?;
        Err(crate::error::ApiError { status, body }.into())
    }
}
```

(Only the `else` branch changes: `bail!("Status: {}, Fail {}", status, json_text)` becomes constructing and returning `ApiError`.)

- [ ] **Step 5: Run the crate's full test suite**

Run: `cargo test -p pasqal-cloud-api`
Expected: PASS, no regressions in existing tests (auth, client mock tests).

- [ ] **Step 6: Commit**

```bash
git add dependencies/pasqal_cloud_client/src/error.rs dependencies/pasqal_cloud_client/src/lib.rs dependencies/pasqal_cloud_client/src/client.rs
git commit -m "[PASQAL] Preserve HTTP status in pasqal_cloud_client failures via ApiError"
```

---

### Task 2: `ApiError` in `pasqal_local_client`

**Files:**
- Create: `dependencies/pasqal_local_client/src/error.rs`
- Modify: `dependencies/pasqal_local_client/src/lib.rs`
- Modify: `dependencies/pasqal_local_client/src/client.rs:219-229` (`handle_request`)
- Test: `dependencies/pasqal_local_client/src/error.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `pub struct ApiError { pub status: reqwest::StatusCode, pub body: String }`, same shape as Task 1's, re-exported as `pasqal_local_api::ApiError`. (Deliberately a separate type from `pasqal_cloud_api::ApiError` — no shared crate exists; see plan header.)

- [ ] **Step 1: Write the test file**

Create `dependencies/pasqal_local_client/src/error.rs` — identical content to Task 1's `dependencies/pasqal_cloud_client/src/error.rs` (same struct, same tests), except the copyright header matches this crate's existing convention:

```rust
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
```

- [ ] **Step 2: Wire the module into `lib.rs`**

In `dependencies/pasqal_local_client/src/lib.rs`:

```rust
mod client;
mod error;
mod models;
#[cfg(feature = "munge")]
mod munge;

pub use client::{Client, ClientBuilder};
pub use error::ApiError;
pub use models::JobStatus;
```

Run: `cargo test -p pasqal-local-api error::`
Expected: PASS (2 tests, same names as Task 1)

- [ ] **Step 3: Use `ApiError` in `handle_request`**

In `dependencies/pasqal_local_client/src/client.rs`, replace the `handle_request` body (currently at lines 219-229):

```rust
async fn handle_request<T: DeserializeOwned>(&self, resp: reqwest::Response) -> Result<T> {
    if resp.status().is_success() {
        let json_text = resp.text().await?;
        let val = serde_json::from_str(&json_text)?;
        Ok(val)
    } else {
        let status = resp.status();
        let body = resp.text().await?;
        Err(crate::error::ApiError { status, body }.into())
    }
}
```

- [ ] **Step 4: Run the crate's full test suite**

Run: `cargo test -p pasqal-local-api --features munge`
Expected: PASS, no regressions.

- [ ] **Step 5: Commit**

```bash
git add dependencies/pasqal_local_client/src/error.rs dependencies/pasqal_local_client/src/lib.rs dependencies/pasqal_local_client/src/client.rs
git commit -m "[PASQAL] Preserve HTTP status in pasqal_local_client failures via ApiError"
```

---

### Task 3: `classify_cloud` / `classify_local` in `src/pasqal/error.rs`

**Files:**
- Modify: `src/pasqal/error.rs`

**Interfaces:**
- Consumes: `pasqal_cloud_api::ApiError { status: reqwest::StatusCode, body: String }` (Task 1), `pasqal_local_api::ApiError { status: reqwest::StatusCode, body: String }` (Task 2), `crate::error::{QrmiError, QrmiErrorKind}`.
- Produces: `pub(crate) enum ResourceKind { Backend, Job }`, `pub(crate) fn classify_cloud(err: anyhow::Error, resource_kind: ResourceKind) -> QrmiError`, `pub(crate) fn classify_local(err: anyhow::Error, resource_kind: ResourceKind) -> QrmiError`. Both consumed by Tasks 4 and 5.

- [ ] **Step 1: Write the failing tests**

Add to the bottom of `src/pasqal/error.rs`:

```rust
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
            classify_cloud(err(reqwest::StatusCode::BAD_REQUEST, "bad"), ResourceKind::Backend).kind(),
            QrmiErrorKind::InvalidInput
        );
        assert_eq!(
            classify_cloud(err(reqwest::StatusCode::UNPROCESSABLE_ENTITY, "bad"), ResourceKind::Backend).kind(),
            QrmiErrorKind::InvalidInput
        );
        assert_eq!(
            classify_cloud(err(reqwest::StatusCode::UNAUTHORIZED, "no"), ResourceKind::Backend).kind(),
            QrmiErrorKind::AuthenticationFailed
        );
        assert_eq!(
            classify_cloud(err(reqwest::StatusCode::NOT_FOUND, "gone"), ResourceKind::Backend).kind(),
            QrmiErrorKind::ResourceNotFound
        );
        assert_eq!(
            classify_cloud(err(reqwest::StatusCode::NOT_FOUND, "gone"), ResourceKind::Job).kind(),
            QrmiErrorKind::TaskNotFound
        );
        assert_eq!(
            classify_cloud(err(reqwest::StatusCode::FORBIDDEN, "nope"), ResourceKind::Backend).kind(),
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
            status: reqwest::StatusCode::NOT_FOUND,
            body: "job not found".to_string(),
        });
        assert_eq!(
            classify_local(err, ResourceKind::Job).kind(),
            QrmiErrorKind::TaskNotFound
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p qrmi --lib pasqal::error::tests`
Expected: FAIL to compile — `classify_cloud`, `classify_local`, and `ResourceKind` don't exist yet.

- [ ] **Step 3: Implement**

Add above the `#[cfg(test)]` block in `src/pasqal/error.rs` (keep the existing `PasqalError` enum and its `impl` untouched above this):

```rust
use http::StatusCode;

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
```

Also update the module doc comment at the top of the file to mention the new classification functions alongside the existing `PasqalError` description (one added sentence, no need to restate everything).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p qrmi --lib pasqal::error::tests`
Expected: PASS (3 tests)

- [ ] **Step 5: Commit**

```bash
git add src/pasqal/error.rs
git commit -m "[PASQAL] Classify Pasqal Cloud/Local API errors by HTTP status"
```

---

### Task 4: Wire `classify_cloud` into `src/pasqal/cloud.rs`

**Files:**
- Modify: `src/pasqal/cloud.rs`
- Modify: `src/pasqal/tests/cloud.rs` (add classification tests)

**Interfaces:**
- Consumes: `crate::pasqal::error::{classify_cloud, ResourceKind}` (Task 3).

- [ ] **Step 1: Write the failing test**

Add to `src/pasqal/tests/cloud.rs` (append at the end, after `normalize_cudaq_result_normalizes_all_supported_counter_shapes`):

```rust
use crate::error::QrmiErrorKind;

fn spawn_json_response_server(status_line: &str, body: &str) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind should succeed");
    let addr = listener.local_addr().expect("local_addr should succeed");
    let status_line = status_line.to_string();
    let body = body.to_string();
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf).unwrap_or(0);
            let response = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                status_line,
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    (addr, handle)
}

#[tokio::test]
async fn task_status_maps_404_to_task_not_found() {
    let (addr, server) = spawn_json_response_server("404 Not Found", r#"{"message":"job not found"}"#);

    let mut builder = ClientBuilder::new("project-id".to_string());
    builder.with_base_url(format!("http://{}", addr));
    builder.with_token("opaque_token".to_string()); // get_batch is an authenticated call; without a token the client fails locally before ever reaching the mock server, and the server thread's accept() then blocks forever.
    let api_client = builder.build().expect("client build should succeed");

    let mut qrmi = PasqalCloud {
        api_client,
        backend_name: "EMU_FREE".to_string(),
        task_kinds: std::collections::HashMap::new(),
    };

    let err = qrmi
        .task_status("missing-batch")
        .await
        .expect_err("should fail with 404");
    server.join().expect("server thread should join");

    assert_eq!(err.kind(), QrmiErrorKind::TaskNotFound);
}

#[tokio::test]
async fn is_accessible_maps_401_to_authentication_failed() {
    let (addr, server) = spawn_json_response_server("401 Unauthorized", r#"{"message":"bad token"}"#);

    let mut builder = ClientBuilder::new("project-id".to_string());
    builder.with_base_url(format!("http://{}", addr));
    let api_client = builder.build().expect("client build should succeed");

    let mut qrmi = PasqalCloud {
        api_client,
        backend_name: "EMU_FREE".to_string(),
        task_kinds: std::collections::HashMap::new(),
    };

    let err = qrmi
        .is_accessible()
        .await
        .expect_err("should fail with 401");
    server.join().expect("server thread should join");

    assert_eq!(err.kind(), QrmiErrorKind::AuthenticationFailed);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qrmi --lib pasqal::cloud`
Expected: FAIL — `task_status_maps_404_to_task_not_found` currently returns `QrmiErrorKind::Other` (assertion fails), same for the 401 test.

- [ ] **Step 3: Implement — wire `classify_cloud` into every fallible call site**

In `src/pasqal/cloud.rs`:

Add to imports (near the top, alongside the existing `use crate::pasqal::error::PasqalError;`):

```rust
use crate::pasqal::error::{classify_cloud, PasqalError, ResourceKind};
```

Update `is_accessible`:

```rust
async fn is_accessible(&mut self) -> Result<bool> {
    let device_type = self.parse_device_type()?;

    // The device may be down temporarily but jobs can still
    // be submitted and queued through the cloud.
    // Thus we only check that the device is not retired.
    let device = self
        .api_client
        .get_device(device_type)
        .await
        .map_err(|e| classify_cloud(e, ResourceKind::Backend))?;
    Ok(device.availability == "ACTIVE")
}
```

Update `task_start`:

```rust
async fn task_start(&mut self, payload: Payload) -> Result<String> {
    debug!(
        "Starting task on PasqalCloud QRMI (backend '{}')",
        self.backend_name
    );
    let Payload::PasqalCloud { sequence, job_runs } = payload else {
        return Err(QrmiError::UnsupportedPayload(format!("{payload:?}")));
    };
    let device_type = self.parse_device_type()?;

    if Self::is_cudaq_sequence(&sequence) {
        let sequence_value: serde_json::Value = serde_json::from_str(&sequence)
            .map_err(|err| PasqalError::InvalidCudaqSequence(err.to_string()))?;
        let job = self
            .api_client
            .create_cudaq_job(sequence_value, job_runs, device_type)
            .await
            .map_err(|e| classify_cloud(e, ResourceKind::Backend))?;
        self.task_kinds
            .insert(job.data.id.clone(), PasqalTaskKind::Cudaq);
        Ok(job.data.id)
    } else {
        let batch = self
            .api_client
            .create_batch(sequence, job_runs, device_type)
            .await
            .map_err(|e| classify_cloud(e, ResourceKind::Backend))?;
        self.task_kinds
            .insert(batch.data.id.clone(), PasqalTaskKind::Pulser);
        Ok(batch.data.id)
    }
}
```

Update `task_stop`:

```rust
async fn task_stop(&mut self, task_id: &str) -> Result<()> {
    debug!(
        "Stopping task '{}' on PasqalCloud QRMI (backend '{}')",
        task_id, self.backend_name
    );
    self.api_client
        .cancel_batch(task_id)
        .await
        .map_err(|e| classify_cloud(e, ResourceKind::Job))?;
    Ok(())
}
```

Update `task_status_from_job_id` and `task_status_from_batch_id` (both `pub(crate)` helper fns used by `task_status`/`task_result`):

```rust
async fn task_status_from_job_id(&mut self, job_id: &str) -> Result<TaskStatus> {
    let job = self
        .api_client
        .get_job(job_id)
        .await
        .map_err(|e| classify_cloud(e, ResourceKind::Job))?;
    Ok(Self::map_job_status(&job.data.status))
}

async fn task_status_from_batch_id(&mut self, batch_id: &str) -> Result<TaskStatus> {
    let batch = self
        .api_client
        .get_batch(batch_id)
        .await
        .map_err(|e| classify_cloud(e, ResourceKind::Job))?;
    let job_id = batch
        .data
        .job_ids
        .first()
        .ok_or_else(|| QrmiError::TaskNotReady {
            task_id: batch_id.to_string(),
            reason: "no jobs found for this batch".to_string(),
        })?;
    self.task_status_from_job_id(job_id).await
}
```

Update `task_result_from_cudaq`:

```rust
async fn task_result_from_cudaq(&mut self, task_id: &str) -> Result<TaskResult> {
    let job = self
        .api_client
        .get_cudaq_job(task_id)
        .await
        .map_err(|e| classify_cloud(e, ResourceKind::Job))?;
    Ok(TaskResult {
        value: Self::normalize_cudaq_result(&job.data.result),
    })
}
```

Update `task_result` (the `QuantumResource` trait method):

```rust
async fn task_result(&mut self, task_id: &str) -> Result<TaskResult> {
    match self.task_kind(task_id) {
        PasqalTaskKind::Pulser => {
            let resp = self
                .api_client
                .get_batch_results(task_id)
                .await
                .map_err(|e| classify_cloud(e, ResourceKind::Job))?;
            Ok(TaskResult { value: resp })
        }
        PasqalTaskKind::Cudaq => self.task_result_from_cudaq(task_id).await,
    }
}
```

Update `target`:

```rust
async fn target(&mut self) -> Result<Target> {
    debug!(
        "Getting target information for PasqalCloud QRMI (backend '{}')",
        self.backend_name
    );
    let device_type = self.parse_device_type()?;
    let resp = self
        .api_client
        .get_device_specs(device_type)
        .await
        .map_err(|e| classify_cloud(e, ResourceKind::Backend))?;
    Ok(Target { value: resp })
}
```

Note: `is_accessible`'s `.context("failed to get device")` call is removed — `classify_cloud` supersedes it, and the underlying `ApiError`'s `Display` already includes status+body, so the context string added no longer-needed information. Remove the now-unused `use anyhow::Context;` import at the top of the file if nothing else in the file uses `Context` (check with `grep -n "Context" src/pasqal/cloud.rs` after this step — if only that one call site used it, delete the import).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p qrmi --lib pasqal::cloud`
Expected: PASS, including the two new tests and all pre-existing ones in `src/pasqal/tests/cloud.rs`.

- [ ] **Step 5: Commit**

```bash
git add src/pasqal/cloud.rs src/pasqal/tests/cloud.rs
git commit -m "[PASQAL] Classify PasqalCloud API errors into QrmiError variants"
```

---

### Task 5: Wire `classify_local` into `src/pasqal/local.rs`

**Files:**
- Modify: `src/pasqal/local.rs`
- Create: `src/pasqal/tests/local.rs`

**Interfaces:**
- Consumes: `crate::pasqal::error::{classify_local, ResourceKind}` (Task 3).

- [ ] **Step 1: Register the test module**

`src/pasqal.rs` (the module root — this crate uses `src/pasqal.rs` + `src/pasqal/` siblings, not `src/pasqal/mod.rs`) already declares `mod local;`, so nothing changes there. Mirror `src/pasqal/cloud.rs`'s existing `#[cfg(test)] #[path = "tests/cloud.rs"] mod tests;` wiring by adding to the bottom of `src/pasqal/local.rs`:

```rust
#[cfg(test)]
#[path = "tests/local.rs"]
mod tests;
```

- [ ] **Step 2: Write the failing tests**

Create `src/pasqal/tests/local.rs`:

```rust
use super::PasqalLocal;
use crate::error::QrmiErrorKind;
use crate::QuantumResource;
use pasqal_local_api::ClientBuilder;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

fn spawn_json_response_server(
    status_line: &str,
    body: &str,
) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind should succeed");
    let addr = listener.local_addr().expect("local_addr should succeed");
    let status_line = status_line.to_string();
    let body = body.to_string();
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf).unwrap_or(0);
            let response = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                status_line,
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    (addr, handle)
}

#[tokio::test]
async fn task_status_maps_404_to_task_not_found() {
    let (addr, server) =
        spawn_json_response_server("404 Not Found", r#"{"message":"job not found"}"#);

    let api_client = ClientBuilder::new(format!("http://{}", addr))
        .build()
        .expect("client build should succeed");

    let mut qrmi = PasqalLocal {
        api_client,
        backend_name: "QPU1".to_string(),
        job_uid: 1,
        job_id: "1".to_string(),
    };

    let err = qrmi
        .task_status("missing-job")
        .await
        .expect_err("should fail with 404");
    server.join().expect("server thread should join");

    assert_eq!(err.kind(), QrmiErrorKind::TaskNotFound);
}

#[tokio::test]
async fn is_accessible_maps_401_to_authentication_failed() {
    let (addr, server) =
        spawn_json_response_server("401 Unauthorized", r#"{"message":"bad token"}"#);

    let api_client = ClientBuilder::new(format!("http://{}", addr))
        .build()
        .expect("client build should succeed");

    let mut qrmi = PasqalLocal {
        api_client,
        backend_name: "QPU1".to_string(),
        job_uid: 1,
        job_id: "1".to_string(),
    };

    let err = qrmi
        .is_accessible()
        .await
        .expect_err("should fail with 401");
    server.join().expect("server thread should join");

    assert_eq!(err.kind(), QrmiErrorKind::AuthenticationFailed);
}
```

Run: `cargo test -p qrmi --lib pasqal::local --features munge`
Expected: FAIL to compile (`mod tests` not yet declared) — confirms the harness is exercised once wired.

- [ ] **Step 3: Implement — wire `classify_local` into every fallible call site**

In `src/pasqal/local.rs`, update the import line:

```rust
use crate::pasqal::error::{classify_local, ResourceKind};
```

Update `is_accessible`:

```rust
async fn is_accessible(&mut self) -> Result<bool> {
    let accessible = self
        .api_client
        .get_accessible()
        .await
        .map_err(|e| classify_local(e, ResourceKind::Backend))?;
    Ok(accessible.is_accessible)
}
```

Update `task_start`:

```rust
async fn task_start(&mut self, payload: Payload) -> Result<String> {
    let token_var = format!("{}_QRMI_JOB_ACQUISITION_TOKEN", self.backend_name);
    let session_id = required_env(&token_var)?;

    let Payload::PasqalCloud { sequence, job_runs } = payload else {
        return Err(QrmiError::UnsupportedPayload(format!("{payload:?}")));
    };
    let job = self
        .api_client
        .create_job(sequence, job_runs, &session_id)
        .await
        .map_err(|e| classify_local(e, ResourceKind::Backend))?;
    Ok(job.id.to_string())
}
```

Update `task_stop`:

```rust
async fn task_stop(&mut self, task_id: &str) -> Result<()> {
    self.api_client
        .cancel_job(task_id)
        .await
        .map_err(|e| classify_local(e, ResourceKind::Job))?;
    Ok(())
}
```

Update `task_status`:

```rust
async fn task_status(&mut self, task_id: &str) -> Result<TaskStatus> {
    let job = self
        .api_client
        .get_job(task_id)
        .await
        .map_err(|e| classify_local(e, ResourceKind::Job))?;
    Ok(match job.status {
        JobStatus::Pending => TaskStatus::Queued,
        JobStatus::Running => TaskStatus::Running,
        JobStatus::Done => TaskStatus::Completed,
        JobStatus::Canceled => TaskStatus::Cancelled,
        JobStatus::Error => TaskStatus::Failed,
    })
}
```

Update `task_result`:

```rust
async fn task_result(&mut self, task_id: &str) -> Result<TaskResult> {
    let job = self
        .api_client
        .get_job(task_id)
        .await
        .map_err(|e| classify_local(e, ResourceKind::Job))?;
    let Some(results) = job.results else {
        return Err(QrmiError::TaskNotReady {
            task_id: task_id.to_string(),
            reason: format!("results not available (current status: {:?})", job.status),
        });
    };
    Ok(TaskResult { value: results })
}
```

Update `task_logs`:

```rust
async fn task_logs(&mut self, task_id: &str) -> Result<String> {
    let resp = self
        .api_client
        .get_task_logs(task_id)
        .await
        .map_err(|e| classify_local(e, ResourceKind::Job))?;
    Ok(resp.logs)
}
```

Update `target`:

```rust
async fn target(&mut self) -> Result<Target> {
    let resp = self
        .api_client
        .get_device_specs()
        .await
        .map_err(|e| classify_local(e, ResourceKind::Backend))?;
    Ok(Target { value: resp })
}
```

Leave `acquire`/`release` (`create_session`/`revoke_session`) as plain `?` — per the plan header, a session failure doesn't fit `ResourceKind::Backend` or `::Job`, so it stays `QrmiError::Other` (via the existing blanket `anyhow::Error` → `QrmiError::Other` conversion, unchanged).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p qrmi --lib pasqal::local --features munge`
Expected: PASS (2 new tests).

- [ ] **Step 5: Commit**

```bash
git add src/pasqal/local.rs src/pasqal/tests/local.rs
git commit -m "[PASQAL] Classify PasqalLocal API errors into QrmiError variants"
```

---

### Task 6: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Build the whole workspace**

Run: `cargo build --workspace --all-features`
Expected: PASS, no warnings introduced (check `cargo build` output for new warnings vs. a baseline `git stash && cargo build --workspace --all-features 2>&1 | tail -30 && git stash pop` if unsure whether a warning pre-existed).

- [ ] **Step 2: Run the full test suite**

Run: `cargo test --workspace --all-features`
Expected: PASS, all tests including the new ones from Tasks 1, 2, 3, 4, 5.

- [ ] **Step 3: Lint**

Run: `cargo clippy --workspace --all-features -- -D warnings`
Expected: PASS, no new clippy warnings.

- [ ] **Step 4: Format check**

Run: `cargo fmt --all -- --check`
Expected: PASS. If it fails, run `cargo fmt --all` and commit the formatting fixes.

- [ ] **Step 5: Commit any fixes from steps 3/4**

```bash
git add -A
git commit -m "[PASQAL] Fix lint/format issues from error classification changes"
```

(Only if step 3 or 4 required changes — skip this commit otherwise.)

---

### Task 7: Push to fork and open the PR

**Files:** none

- [ ] **Step 1: Confirm the branch and remote**

Run: `git branch --show-current && git remote -v | grep thomas`
Expected: current branch is `pasqal-error-classification`; `thomas` remote points to `git@github.com:badtst/qrmi.git`.

- [ ] **Step 2: Push the branch to the `thomas` fork**

Run: `git push -u thomas pasqal-error-classification`

- [ ] **Step 3: Open the PR against upstream `qiskit-community/qrmi` main**

Run:
```bash
gh pr create --repo qiskit-community/qrmi --base main --head badtst:pasqal-error-classification --title "[PASQAL] Classify Pasqal Cloud & Local API errors" --body "$(cat <<'EOF'
## Summary
- Closes the gap noted in `docs/migration/0.24.0.md` ("Pasqal Cloud / Local — Not yet — still reports `Other` for everything").
- Adds a typed `ApiError { status, body }` to `pasqal_cloud_client` and `pasqal_local_client`, replacing the `bail!` that previously discarded the HTTP status.
- Classifies Pasqal Cloud/Local failures into `QrmiError::{ResourceNotFound, TaskNotFound, AuthenticationFailed, InvalidInput}` following the same status-code mapping already used for IBM/IQM/Alice & Bob (`src/ibm/error.rs`, `src/iqm/error.rs`, `src/alice_bob/error.rs`).

## Test plan
- [ ] `cargo test --workspace --all-features`
- [ ] `cargo clippy --workspace --all-features -- -D warnings`
- [ ] `cargo fmt --all -- --check`
EOF
)"
```

Expected: PR created against `qiskit-community/qrmi`, from `badtst:pasqal-error-classification`. Confirm with the user before running this step — opening a PR is externally visible.

---

## Self-Review Notes

- **Spec coverage:** ApiError in both client crates (Task 1, 2) ✓. `classify_cloud`/`classify_local` (Task 3) ✓. Call-site wiring for both cloud.rs and local.rs (Task 4, 5), including the `ResourceKind::Backend` vs `::Job` split from the spec ✓. Session calls left unclassified per spec ✓. Tests per vendor mirroring existing mock-server pattern ✓. Out-of-scope items (PasqalError, migration doc) untouched ✓.
- **Type consistency:** `classify_cloud`/`classify_local` signature (`anyhow::Error, ResourceKind -> QrmiError`) is identical across Task 3's definition and every call site in Tasks 4/5.
- Task 7 is a real PR to the upstream repo (visible externally) — get explicit user confirmation before running Step 3, even though the user already asked for "a PR on my fork."
