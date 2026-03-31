//! Integration tests for [`hyperlight_sandbox::http::send_http_request`].
//!
//! Uses [`hyperlight_sandbox::test_utils::EchoServer`] to verify the full
//! request/response path without external network access.

use bytes::Bytes;
use http_body_util::BodyExt;
use hyperlight_sandbox::http::{self, HttpRequest};
use hyperlight_sandbox::runtime::BlockOn;
use hyperlight_sandbox::test_utils::EchoServer;

#[test]
fn send_get_request() {
    async {
        let server = EchoServer::start().await;

        let req = HttpRequest {
            url: url::Url::parse(&server.url("/test")).unwrap(),
            method: "GET".to_string(),
            headers: vec![],
            body: HttpRequest::body_from_bytes(None),
        };

        let resp = http::send_http_request(req).await.unwrap();
        assert_eq!(resp.status, 200);

        let echo: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(echo["method"], "GET");
    }
    .block_on();
}

#[test]
fn send_post_with_body() {
    async {
        let server = EchoServer::start().await;

        let body_content = r#"{"hello": "world"}"#;
        let req = HttpRequest {
            url: url::Url::parse(&server.url("/submit")).unwrap(),
            method: "POST".to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: HttpRequest::body_from_bytes(Some(body_content.as_bytes().to_vec())),
        };

        let resp = http::send_http_request(req).await.unwrap();
        assert_eq!(resp.status, 200);

        let echo: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(echo["method"], "POST");
        assert_eq!(echo["body"], body_content);
    }
    .block_on();
}

#[test]
fn forbidden_headers_are_stripped() {
    async {
        let server = EchoServer::start().await;

        let req = HttpRequest {
            url: url::Url::parse(&server.url("/headers")).unwrap(),
            method: "GET".to_string(),
            headers: vec![
                ("host".to_string(), "evil.com".to_string()),
                ("connection".to_string(), "keep-alive".to_string()),
                ("x-custom".to_string(), "allowed".to_string()),
                ("transfer-encoding".to_string(), "chunked".to_string()),
            ],
            body: HttpRequest::body_from_bytes(None),
        };

        let resp = http::send_http_request(req).await.unwrap();
        let echo: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        let headers = echo["headers"].as_object().unwrap();

        assert_eq!(headers.get("x-custom").unwrap(), "allowed");
        assert_ne!(
            headers.get("host").map(|v| v.as_str().unwrap()),
            Some("evil.com")
        );
    }
    .block_on();
}

#[test]
fn chunked_body_streams_correctly() {
    async {
        let server = EchoServer::start().await;

        // Build a chunked body that yields 8KB frames, matching WasiBodyStream.
        let total = 1024 * 1024; // 1 MiB
        let chunk_size = 8192;
        let chunks: Vec<Result<http_body::Frame<Bytes>, anyhow::Error>> = (0..total)
            .step_by(chunk_size)
            .map(|offset| {
                let n = chunk_size.min(total - offset);
                Ok(http_body::Frame::data(Bytes::from(vec![b'x'; n])))
            })
            .collect();
        let stream_body = http_body_util::StreamBody::new(futures_util::stream::iter(chunks));
        let request_body: hyperlight_sandbox::http::RequestBody =
            BodyExt::boxed_unsync(stream_body);

        let req = HttpRequest {
            url: url::Url::parse(&server.url("/large")).unwrap(),
            method: "POST".to_string(),
            headers: vec![],
            body: request_body,
        };

        let resp = http::send_http_request(req).await.unwrap();
        assert_eq!(resp.status, 200);

        let echo: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        let received_body = echo["body"].as_str().unwrap();
        assert_eq!(received_body.len(), total);
    }
    .block_on();
}
