//! Shared HTTP constants, request sending, and response handling for all
//! sandbox backends.
//!
//! Both the JS and WASM backends funnel outbound HTTP through
//! [`send_http_request`], which uses `wasmtime_wasi_http`'s
//! `default_send_request_handler` under the hood.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};

/// Body type for outgoing sandbox HTTP requests.
///
/// This is our own alias over `http-body-util` types so that wasmtime
/// internals don't leak into the public API.
pub type RequestBody = http_body_util::combinators::UnsyncBoxBody<Bytes, anyhow::Error>;

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

/// Maximum HTTP response body size (16 MiB).
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Maximum number of response headers to retain.
pub const MAX_RESPONSE_HEADER_COUNT: usize = 128;

/// Maximum total bytes across all response header names + values (1 MiB).
pub const MAX_RESPONSE_HEADER_BYTES: usize = 1024 * 1024;

/// Maximum concurrent outbound HTTP requests per sandbox.
pub const MAX_CONCURRENT_REQUESTS: usize = 16;

/// Default HTTP request timeout in seconds.
pub const REQUEST_TIMEOUT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// Forbidden headers
// ---------------------------------------------------------------------------

/// Headers that sandbox guests must not set on outgoing requests.
///
/// This is the union of the WASI HTTP spec's forbidden headers and
/// headers that could be used for header injection or smuggling.
pub const FORBIDDEN_REQUEST_HEADERS: &[&str] = &[
    "connection",
    "host",
    "http2-settings",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
];

/// Returns `true` if `name` (case-insensitive) is a forbidden request header.
pub fn is_forbidden_request_header(name: &str) -> bool {
    FORBIDDEN_REQUEST_HEADERS.contains(&name.to_ascii_lowercase().as_str())
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// A backend-agnostic HTTP request.
pub struct HttpRequest {
    pub url: url::Url,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: RequestBody,
}

impl HttpRequest {
    /// Create a request body from raw bytes (or empty).
    pub fn body_from_bytes(data: Option<Vec<u8>>) -> RequestBody {
        match data {
            Some(data) if !data.is_empty() => http_body_util::Full::new(Bytes::from(data))
                .map_err(|e| anyhow::anyhow!(e))
                .boxed_unsync(),
            _ => Empty::<Bytes>::new()
                .map_err(|e| anyhow::anyhow!(e))
                .boxed_unsync(),
        }
    }
}

/// A backend-agnostic HTTP response with capped headers and body.
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Shared HTTP send
// ---------------------------------------------------------------------------

/// Send an HTTP request using `default_send_request_handler`.
///
/// All sandbox safety policies are applied:
/// - Forbidden headers stripped
/// - TLS determined from URL scheme
/// - Timeouts enforced ([`REQUEST_TIMEOUT_SECS`])
/// - Response headers capped ([`MAX_RESPONSE_HEADER_COUNT`] / [`MAX_RESPONSE_HEADER_BYTES`])
/// - Response body capped ([`MAX_RESPONSE_BYTES`])
///
/// This is an **async** function. Use with [`BlockOn::block_on`](crate::runtime::BlockOn)
/// from sync contexts (e.g. JS sandbox host callbacks) or `.await` / `.spawn()`
/// from async contexts (e.g. WASM handler).
pub async fn send_http_request(req: HttpRequest) -> Result<HttpResponse> {
    let (hyper_request, use_tls) = build_hyper_request(req)?;

    let timeout = Duration::from_secs(REQUEST_TIMEOUT_SECS);
    let config = wasmtime_wasi_http::types::OutgoingRequestConfig {
        use_tls,
        connect_timeout: timeout,
        first_byte_timeout: timeout,
        between_bytes_timeout: timeout,
    };

    // Convert our body error type to wasmtime's ErrorCode at the boundary.
    let (parts, body) = hyper_request.into_parts();
    let wasi_body: wasmtime_wasi_http::body::HyperOutgoingBody = body
        .map_err(|e| {
            wasmtime_wasi_http::bindings::http::types::ErrorCode::InternalError(Some(e.to_string()))
        })
        .boxed_unsync();
    let hyper_request = hyper::Request::from_parts(parts, wasi_body);

    let incoming = wasmtime_wasi_http::types::default_send_request_handler(hyper_request, config)
        .await
        .map_err(|e| anyhow::anyhow!("HTTP request failed: {e:?}"))?;

    let wasmtime_wasi_http::types::IncomingResponse {
        resp,
        worker: _worker,
        between_bytes_timeout,
    } = incoming;

    let status = resp.status().as_u16();
    let headers = cap_response_headers(resp.headers());
    let body = collect_response_body(resp.into_body(), between_bytes_timeout).await?;

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

/// Build a `hyper::Request` from an [`HttpRequest`], stripping forbidden headers.
fn build_hyper_request(req: HttpRequest) -> Result<(hyper::Request<RequestBody>, bool)> {
    let method = hyper::Method::from_bytes(req.method.as_bytes())
        .map_err(|e| anyhow::anyhow!("invalid HTTP method: {e}"))?;

    let use_tls = req.url.scheme() == "https";
    let scheme = if use_tls {
        http::uri::Scheme::HTTPS
    } else {
        http::uri::Scheme::HTTP
    };

    let authority = req.url.authority().to_string();

    let path_and_query = req.url[url::Position::BeforePath..].to_string();

    let uri = http::Uri::builder()
        .scheme(scheme)
        .authority(authority.as_str())
        .path_and_query(path_and_query.as_str())
        .build()
        .context("failed to build request URI")?;

    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(uri)
        .header(hyper::header::HOST, &authority);

    for (name, value) in &req.headers {
        if !is_forbidden_request_header(name) {
            builder = builder.header(name.as_str(), value.as_str());
        }
    }

    let request = builder.body(req.body).context("failed to build request")?;
    Ok((request, use_tls))
}

/// Extract response headers, enforcing count and byte-size limits.
fn cap_response_headers(raw: &http::HeaderMap) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    let mut total_bytes: usize = 0;
    for (name, value) in raw {
        if headers.len() >= MAX_RESPONSE_HEADER_COUNT {
            break;
        }
        let value_str = value.to_str().unwrap_or_default();
        let entry_bytes = name.as_str().len() + value_str.len();
        total_bytes = total_bytes.saturating_add(entry_bytes);
        if total_bytes > MAX_RESPONSE_HEADER_BYTES {
            break;
        }
        headers.insert(name.to_string(), value_str.to_string());
    }
    headers
}

/// Collect a hyper response body up to [`MAX_RESPONSE_BYTES`].
async fn collect_response_body(
    mut body: wasmtime_wasi_http::body::HyperIncomingBody,
    between_bytes_timeout: Duration,
) -> Result<Vec<u8>> {
    let mut body_bytes = Vec::new();

    loop {
        let next_frame = tokio::time::timeout(between_bytes_timeout, body.frame())
            .await
            .map_err(|_| anyhow::anyhow!("response body read timed out"))?
            .transpose()
            .map_err(|e| anyhow::anyhow!("response body read error: {e}"))?;

        let Some(frame) = next_frame else {
            break;
        };

        if let Ok(data) = frame.into_data() {
            if body_bytes.len().saturating_add(data.len()) > MAX_RESPONSE_BYTES {
                anyhow::bail!(
                    "response body exceeded {} MiB limit",
                    MAX_RESPONSE_BYTES / (1024 * 1024)
                );
            }
            body_bytes.extend_from_slice(&data);
        }
    }

    Ok(body_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_headers_are_sorted() {
        let mut sorted = FORBIDDEN_REQUEST_HEADERS.to_vec();
        sorted.sort();
        assert_eq!(FORBIDDEN_REQUEST_HEADERS, sorted.as_slice());
    }

    #[test]
    fn is_forbidden_is_case_insensitive() {
        assert!(is_forbidden_request_header("Host"));
        assert!(is_forbidden_request_header("HOST"));
        assert!(is_forbidden_request_header("Transfer-Encoding"));
        assert!(!is_forbidden_request_header("Content-Type"));
        assert!(!is_forbidden_request_header("Authorization"));
    }
}
