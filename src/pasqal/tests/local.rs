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

// `is_accessible` is the only `PasqalLocal` operation that doesn't require
// an authenticated (munge-signed) request, so it's the only one testable
// against a mock server without the `munge` feature (and its native
// `libmunge` dependency, not available in every build environment). Every
// other call site's status -> `QrmiError` mapping is exercised directly at
// the unit level in `src/pasqal/error.rs`'s `classify_local` tests instead.
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
