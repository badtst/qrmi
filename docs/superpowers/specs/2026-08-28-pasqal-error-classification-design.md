# Pasqal error classification (QrmiError)

## Context

QRMI 0.24.0 introduced `QrmiError`, a typed error enum with a `.kind()`
method, replacing bare `anyhow::Error` for public APIs (see
`docs/migration/0.24.0.md`). IBM, IQM, and Alice & Bob backends were
updated to classify their vendor API failures into specific `QrmiError`
variants (`ResourceNotFound`, `TaskNotFound`, `AuthenticationFailed`,
`InvalidInput`, ...) instead of the generic `QrmiError::Other`. The
migration guide explicitly lists Pasqal Cloud and Pasqal Local as **not
yet done** ("still reports `Other` for everything"). This PR closes that
gap, following the exact same pattern already used for IBM
(`src/ibm/error.rs`), IQM (`src/iqm/error.rs`), and Alice & Bob
(`src/alice_bob/error.rs`).

## Problem specific to Pasqal

Unlike `quantum_compute_client`, `iqm_server_api`, and `alice_bob_felis`
(OpenAPI-generated clients whose `Error<T>::ResponseError` carries a typed
HTTP `status`), `pasqal_cloud_client` and `pasqal_local_client` are
hand-written by us. Their shared `handle_request` helper currently
discards the status code into a formatted string:

```rust
bail!("Status: {}, Fail {}", status, json_text);
```

There is nothing left for a caller to match on. Classification requires
first preserving the status code as a typed value.

## Design

### 1. `ApiError` in each client crate

Add, in both `dependencies/pasqal_cloud_client` and
`dependencies/pasqal_local_client`:

```rust
#[derive(Debug, thiserror::Error)]
#[error("status {status}: {body}")]
pub struct ApiError {
    pub status: reqwest::StatusCode,
    pub body: String,
}
```

`handle_request` returns `Err(ApiError { status, body: json_text }.into())`
instead of `bail!`. Everything else about these crates (their `Result<T>
= anyhow::Result<T>` return types) stays unchanged — this only swaps what
gets constructed on the failure path, so it's additive from the call
sites' perspective except where noted in step 3.

Each crate gets its own `ApiError` (no shared `pasqal_common` crate
exists in the current tree) — small duplication, but avoids introducing
a new shared dependency for two structurally-identical-but-independent
types.

### 2. `classify()` in `src/pasqal/error.rs`

Add a `pub(crate) fn classify(err: anyhow::Error, resource_kind:
ResourceKind) -> QrmiError` that downcasts to the relevant crate's
`ApiError` and matches on `status`, mirroring
`src/alice_bob/error.rs::classify`:

- 400 / 422 → `QrmiError::InvalidInput(body)`
- 401 → `QrmiError::AuthenticationFailed(body)`
- 404, `ResourceKind::Backend` → `QrmiError::ResourceNotFound(body)`
- 404, `ResourceKind::Job` → `QrmiError::TaskNotFound(body)`
- everything else (403 included), and any error that isn't a downcastable
  `ApiError` (transport/JSON/etc. failures) → `QrmiError::Other`, `err`
  kept as `source`.

Since `PasqalCloud` and `PasqalLocal` use two distinct client crates with
two distinct `ApiError` types, `classify` takes the crate-specific error
type per call site (two small classify functions, or one generic one —
decided during implementation, see plan) rather than one signature shared
across both clients.

`ResourceKind` distinguishes:
- Cloud: `Backend` (`get_device`, `get_device_specs`) vs `Job`
  (`get_job`, `get_cudaq_job`, `get_batch`, `cancel_batch`,
  `get_batch_results`).
- Local: `Backend` (`get_device_specs`) vs `Job` (`get_job`, `cancel_job`).
  `create_session`/`revoke_session` failures don't fit either
  `ResourceNotFound` nor `TaskNotFound` cleanly (a session isn't a
  compute resource, same reasoning IBM's `error.rs` gives for its
  `Session` `ResourceKind` variant) — left unclassified, falling to
  `Other`.

### 3. Call sites: `src/pasqal/cloud.rs`, `src/pasqal/local.rs`

Every `self.api_client.<call>().await?` that can fail with an HTTP status
becomes `self.api_client.<call>().await.map_err(|e| classify(e,
ResourceKind::...))?`. `create_batch`/`create_cudaq_job` (validation of
the sequence/payload) classify as `Job`-adjacent but really "invalid
input" — 400/422 is handled generically by status code regardless of
`resource_kind`, so which kind is passed there doesn't change behavior;
`Backend` is used there for consistency since no job exists yet at that
point.

### 4. Tests

Extend `src/pasqal/tests/cloud.rs` (and add local tests if none exist)
with cases asserting `.kind()` for a 401/404/400 mock response, matching
how IBM/IQM/AnB test their classifiers.

### Out of scope

- No changes to `PasqalError` (`InvalidDeviceType`,
  `InvalidCudaqSequence`) — those are already classified.
- No change to the migration guide (`docs/migration/0.24.0.md`) itself —
  out of scope for this PR unless review asks for a follow-up doc update.
- No behavior change for non-HTTP failures (connection errors, JSON
  decode errors) — these still fall to `QrmiError::Other`, same as every
  other vendor.
