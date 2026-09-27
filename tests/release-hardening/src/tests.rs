use crate::{http, resource_limits::ResourceLimits};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

async fn delayed_headers(client: reqwest::Client, path: &'static str) -> anyhow::Result<Vec<u8>> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}{path}", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = [0; 4096];
        let count = socket.read(&mut request).unwrap();
        assert!(count > 0);
        assert!(String::from_utf8_lossy(&request[..count]).contains(path));
        std::thread::sleep(Duration::from_secs(6));
        // C04 closes early; tolerate that write failing so the client assertion
        // identifies the regression rather than panicking in the fixture.
        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}");
    });
    let result = async {
        let response = http::send(client.post(url).body("fixture")).await?;
        http::body(response, http::Payload::Metadata, Default::default()).await
    }
    .await;
    tokio::task::spawn_blocking(move || thread.join())
        .await
        .unwrap()
        .unwrap();
    result
}

#[tokio::test]
async fn valid_slow_headers_keep_the_existing_ten_second_budget() {
    // Exercise real HTTP request futures, not a constants-only assertion.
    // This reference builder matches the pre-hardening total request deadline.
    let reference = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let (before, sign_in, consoles, stream) = tokio::join!(
        delayed_headers(reference, "/reference"),
        delayed_headers(http::client().unwrap(), "/oauth/token"),
        delayed_headers(http::client().unwrap(), "/v6/servers/home"),
        delayed_headers(http::client().unwrap(), "/v5/sessions/home/play"),
    );
    assert_eq!(before.unwrap(), b"{}");
    for response in [sign_in, consoles, stream] {
        assert_eq!(
            response.expect("valid response within the original ten-second budget"),
            b"{}"
        );
    }
}

#[tokio::test]
async fn request_deadline_includes_headers_and_body_without_resetting() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/private-token", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(12)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        std::thread::sleep(Duration::from_secs(6));
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{",
            )
            .unwrap();
        let closed = socket.read(&mut request);
        assert!(
            matches!(closed, Ok(0))
                || closed
                    .as_ref()
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::ConnectionReset)
        );
    });
    let started = std::time::Instant::now();
    let response = http::send(http::client().unwrap().get(url)).await.unwrap();
    let error = http::body(response, http::Payload::Metadata, Default::default())
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_secs(9) && elapsed < Duration::from_secs(12),
        "total elapsed: {elapsed:?}"
    );
    assert!(error.to_string().contains("timed out"));
    assert!(!format!("{error:#}").contains("private-token"));
    tokio::task::spawn_blocking(move || thread.join())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancellation_before_headers_closes_the_request_without_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/session/play", listener.local_addr().unwrap());
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        started_tx.send(()).unwrap();
        let closed = socket.read(&mut request);
        assert!(
            matches!(closed, Ok(0))
                || closed
                    .as_ref()
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::ConnectionReset)
        );
        listener.set_nonblocking(true).unwrap();
        assert!(
            listener
                .accept()
                .is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock)
        );
    });
    let job = tokio::spawn(async move { http::send(http::client().unwrap().post(url)).await });
    started_rx.await.unwrap();
    crate::jobs::cancel(job).await;
    tokio::task::spawn_blocking(move || thread.join())
        .await
        .unwrap()
        .unwrap();
}
fn server(reply: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/private-token", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = [0; 4096];
        let _ = stream.read(&mut request);
        let _ = stream.write_all(&reply);
    });
    (url, thread)
}
async fn read_reply(reply: &str, cap: usize) -> anyhow::Result<Vec<u8>> {
    let (url, thread) = server(reply.as_bytes().to_vec());
    let response = http::send(http::client()?.get(url)).await?;
    let result = http::body(
        response,
        http::Payload::Metadata,
        ResourceLimits {
            metadata_bytes: cap,
            ..Default::default()
        },
    )
    .await;
    tokio::task::spawn_blocking(move || thread.join())
        .await
        .unwrap()
        .unwrap();
    result
}
#[tokio::test]
async fn streamed_limit_covers_missing_and_chunked_content_length() {
    assert_eq!(
        read_reply(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{}",
            2
        )
        .await
        .unwrap(),
        b"{}"
    );
    for reply in [
        "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n123456",
        "HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\n123456",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n123\r\n3\r\n456\r\n0\r\n\r\n",
    ] {
        assert!(
            read_reply(reply, 5)
                .await
                .unwrap_err()
                .to_string()
                .contains("too large")
        );
    }
    assert!(
        read_reply("HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{}", 8)
            .await
            .is_err()
    );
    assert!(
        read_reply(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\n\r\n{}",
            8
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("unsupported")
    );
}
#[tokio::test]
async fn cancellation_closes_in_flight_body() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
            .unwrap();
        started_tx.send(()).unwrap();
        // Client cancellation must close this pending body, not leak a background reader.
        match socket.read(&mut request) {
            Ok(0) => true,
            Err(error) => error.kind() == std::io::ErrorKind::ConnectionReset,
            _ => false,
        }
    });
    let job = tokio::spawn(async move {
        let response = http::send(http::client().unwrap().get(url)).await.unwrap();
        http::body(response, http::Payload::Metadata, Default::default()).await
    });
    started_rx.await.unwrap();
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    assert!(
        tokio::task::spawn_blocking(move || thread.join())
            .await
            .unwrap()
            .unwrap()
    );
}
#[tokio::test]
async fn errors_do_not_echo_endpoint_or_response_secrets() {
    let (url, thread) = server(
        b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 19\r\n\r\nsecret-access-token".to_vec(),
    );
    let response = http::send(http::client().unwrap().get(url)).await.unwrap();
    let error = http::json::<serde_json::Value>(response).await.unwrap_err();
    assert_eq!(error.to_string(), "service rejected request (HTTP 401)");
    tokio::task::spawn_blocking(move || thread.join())
        .await
        .unwrap()
        .unwrap();
}
#[test]
fn dimensions_and_decode_limits_reject_bombs_and_malformed_payloads() {
    let valid = include_bytes!("../../../assets/xbox-logo-white.png");
    assert!(http::decode_image(valid, Default::default()).is_ok());
    assert!(
        http::decode_image(
            valid,
            ResourceLimits {
                image_pixels: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
    for (w, h) in [(0, 1), (u32::MAX, 1), (4096, 4096), (1, u32::MAX)] {
        assert!(ResourceLimits::default().image_dimensions(w, h).is_err());
    }
    assert!(http::decode_image(b"not an image", Default::default()).is_err());
    assert!(http::decode_image(&valid[..24], Default::default()).is_err());
    assert!(
        ResourceLimits {
            image_bytes: usize::MAX,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}
#[test]
fn credential_destinations_reject_untrusted_hosts_and_userinfo() {
    for host in [
        "http://region.xboxlive.com",
        "https://xboxlive.com.evil.invalid",
        "https://token@region.xboxlive.com",
        "https://region.xboxlive.com/path",
        "https://127.0.0.1",
    ] {
        assert!(http::regional_endpoint(host).is_err());
    }
    assert!(http::regional_endpoint("https://uks.core.gssv-play-prod.xboxlive.com").is_ok());
}

#[tokio::test]
async fn stalled_response_hits_timeout_without_echoing_url() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/private-token", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = [0; 4096];
        assert!(socket.read(&mut bytes).unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\nx")
            .unwrap();
        let result = socket.read(&mut bytes);
        assert!(
            matches!(result, Ok(0))
                || result
                    .as_ref()
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::ConnectionReset),
            "pending socket: {result:?}"
        );
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(100))
        .build()
        .unwrap();
    let response = http::send(client.get(url)).await.unwrap();
    let error = http::body(response, http::Payload::Metadata, Default::default())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(!error.to_string().contains("private-token"));
    tokio::task::spawn_blocking(move || thread.join())
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn catalog_drop_joins_idle_in_flight_and_full_result_worker() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    for mode in 0..3 {
        for _ in 0..10 {
            let entered = Arc::new(AtomicBool::new(false));
            let worker = crate::worker::CatalogWorker::spawn(crate::GameCatalogBackend {
                entered: entered.clone(),
                pending: mode == 1,
            });
            if mode == 1 {
                assert!(worker.request_metadata(
                    "cancel-test".into(),
                    "test".into(),
                    "US".into(),
                    "en-US".into()
                ));
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while !entered.load(Ordering::SeqCst) {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::yield_now();
                }
            } else if mode == 2 {
                // Fill all eight results, then leave additional work blocked on send.
                for i in 0..12 {
                    let deadline = std::time::Instant::now() + Duration::from_secs(2);
                    while !worker.request_metadata(
                        format!("full-{i}"),
                        "test".into(),
                        "US".into(),
                        "en-US".into(),
                    ) {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::yield_now();
                    }
                }
            }
            let before = std::time::Instant::now();
            drop(worker);
            assert!(before.elapsed() < Duration::from_secs(2));
            assert_eq!(
                Arc::strong_count(&entered),
                1,
                "worker still owns backend after Drop"
            );
        }
    }
}

#[test]
fn joining_api_path_cannot_redirect_credentials_to_another_origin() {
    for path in [
        "//evil.invalid/a",
        "/\\evil.invalid/a",
        "https://evil.invalid/",
        "/a#fragment",
        "/a\n",
    ] {
        assert!(http::api_url("https://region.xboxlive.com", path).is_err());
    }
    assert_eq!(
        http::api_url("https://region.xboxlive.com", "/v1/titles?limit=20")
            .unwrap()
            .host_str(),
        Some("region.xboxlive.com")
    );
}

#[test]
fn safe_memory_invalid_bounds_never_call_native_code() {
    use std::sync::atomic::Ordering;
    let before = crate::sdk::CALLS.load(Ordering::SeqCst);
    assert!(crate::safe_memory::load::<41>(0).is_err());
    assert!(crate::safe_memory::load::<1>(-1).is_err());
    assert!(crate::safe_memory::save(39, &[1, 2]).is_err());
    assert_eq!(before, crate::sdk::CALLS.load(Ordering::SeqCst));
}

#[test]
fn excessive_png_header_is_rejected_before_pixel_decompression() {
    let mut png = include_bytes!("../../../assets/xbox-logo-white.png").to_vec();
    // PNG IHDR dimensions + matching CRC; retain actual IDAT to distinguish the
    // header safety gate from a generic corrupt-header/decompression failure.
    png[16..20].copy_from_slice(&50_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&50_000u32.to_be_bytes());
    let mut crc = u32::MAX;
    for &byte in &png[12..29] {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    png[29..33].copy_from_slice(&(!crc).to_be_bytes());
    let error = http::decode_image(&png, Default::default()).unwrap_err();
    assert_eq!(error.to_string(), "decoded image dimensions exceed limit");
}

#[tokio::test]
async fn task_polling_preserves_pending_work_and_reports_completion() {
    use crate::jobs::{PollJob, poll_job};
    let job = tokio::spawn(async { Ok::<_, anyhow::Error>(17) });
    let job = match poll_job(job).await {
        PollJob::Pending(job) => job,
        PollJob::Done(_) => panic!("task cannot have run before first yield"),
    };
    tokio::task::yield_now().await;
    match poll_job(job).await {
        PollJob::Done(result) => assert_eq!(result.unwrap(), 17),
        PollJob::Pending(_) => panic!("completed task still pending"),
    }
}

#[test]
fn cache_names_cannot_escape_provider_directory() {
    let path = crate::cache::provider_path("../provider/secret", "../tokens");
    assert!(!path.contains("/../"));
    assert!(path.starts_with("ux0:data/green-vita-540-test/cache/catalog-v1/"));
}
