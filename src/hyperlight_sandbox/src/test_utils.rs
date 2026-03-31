//! Test utilities for sandbox backend integration tests.
//!
//! Gated behind the `test-utils` feature. Enable it in your backend's
//! `[dev-dependencies]`:
//!
//! ```toml
//! [dev-dependencies]
//! hyperlight-sandbox = { workspace = true, features = ["test-utils"] }
//! ```
//!
//! Provides [`EchoServer`] — a local HTTP server that echoes the request
//! method, headers, and body back as JSON.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::Notify;

/// A local HTTP echo server for integration tests.
///
/// Returns a JSON response containing the request method, headers, and body.
pub struct EchoServer {
    pub addr: SocketAddr,
    shutdown: Arc<Notify>,
}

impl EchoServer {
    /// Start the echo server on a random available port.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shutdown = Arc::new(Notify::new());
        let shutdown_clone = shutdown.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept = listener.accept() => {
                        let (stream, _) = accept.unwrap();
                        let io = TokioIo::new(stream);
                        tokio::spawn(async move {
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, service_fn(Self::handle))
                                .await;
                        });
                    }
                    _ = shutdown_clone.notified() => break,
                }
            }
        });

        Self { addr, shutdown }
    }

    /// The base URL of the running server (e.g. `http://127.0.0.1:12345`).
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    /// Shut down the server.
    pub fn stop(&self) {
        self.shutdown.notify_one();
    }

    async fn handle(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
        let method = req.method().to_string();
        let mut headers: HashMap<String, String> = HashMap::new();
        for (k, v) in req.headers() {
            headers.insert(k.to_string(), v.to_str().unwrap_or("").to_string());
        }
        let body_bytes = req.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body_bytes).to_string();

        let response_json = serde_json::json!({
            "method": method,
            "headers": headers,
            "body": body_str,
        });

        Ok(Response::new(Full::new(Bytes::from(
            serde_json::to_string(&response_json).unwrap(),
        ))))
    }
}

impl Drop for EchoServer {
    fn drop(&mut self) {
        self.stop();
    }
}
