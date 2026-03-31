//! HTTP OutgoingHandler implementation.
//!
//! Delegates to [`hyperlight_sandbox::http::send_http_request`] which
//! centralises request building, header filtering, and response capping.
#![allow(unused_variables)]

use std::sync::atomic::Ordering;

use hyperlight_host::HyperlightError;
use hyperlight_sandbox::http as sandbox_http;
use wasi::http::types::ErrorCode;

use crate::HostState;
use crate::bindings::wasi;
use crate::wasi_impl::resource::{BlockOn, Resource};
use crate::wasi_impl::types::headers::Headers;
use crate::wasi_impl::types::http_future_incoming_response::FutureIncomingResponse;
use crate::wasi_impl::types::http_incoming_body::IncomingBody;
use crate::wasi_impl::types::http_incoming_response::IncomingResponse;
use crate::wasi_impl::types::http_outgoing_request::OutgoingRequest;
use crate::wasi_impl::types::http_request_options::RequestOptions;
use crate::wasi_impl::types::stream::Stream;

type HlResult<T> = Result<T, HyperlightError>;

fn wasi_method_to_http_method(
    method: &wasi::http::types::Method,
) -> Result<hyperlight_sandbox::HttpMethod, ErrorCode> {
    match method {
        wasi::http::types::Method::Get => Ok(hyperlight_sandbox::HttpMethod::Get),
        wasi::http::types::Method::Post => Ok(hyperlight_sandbox::HttpMethod::Post),
        wasi::http::types::Method::Put => Ok(hyperlight_sandbox::HttpMethod::Put),
        wasi::http::types::Method::Delete => Ok(hyperlight_sandbox::HttpMethod::Delete),
        wasi::http::types::Method::Head => Ok(hyperlight_sandbox::HttpMethod::Head),
        wasi::http::types::Method::Options => Ok(hyperlight_sandbox::HttpMethod::Options),
        wasi::http::types::Method::Patch => Ok(hyperlight_sandbox::HttpMethod::Patch),
        wasi::http::types::Method::Connect => Ok(hyperlight_sandbox::HttpMethod::Connect),
        wasi::http::types::Method::Trace => Ok(hyperlight_sandbox::HttpMethod::Trace),
        wasi::http::types::Method::Other(_) => Err(ErrorCode::HTTPRequestMethodInvalid),
    }
}

impl
    wasi::http::OutgoingHandler<
        ErrorCode,
        Resource<FutureIncomingResponse>,
        Resource<OutgoingRequest>,
        Resource<RequestOptions>,
    > for HostState
{
    fn handle(
        &mut self,
        request: Resource<OutgoingRequest>,
        options: Option<Resource<RequestOptions>>,
    ) -> HlResult<Result<Resource<FutureIncomingResponse>, ErrorCode>> {
        let request_data = request.read().block_on();

        // Network permission check — always performed.
        let authority = match request_data.authority {
            Some(ref a) => a,
            None => return Ok(Err(ErrorCode::HTTPRequestDenied)),
        };

        let http_method = match wasi_method_to_http_method(&request_data.method) {
            Ok(m) => m,
            Err(e) => return Ok(Err(e)),
        };

        let scheme_str = match &request_data.scheme {
            Some(wasi::http::types::Scheme::HTTP) => "http",
            Some(wasi::http::types::Scheme::HTTPS) => "https",
            Some(wasi::http::types::Scheme::Other(_)) => {
                return Ok(Err(ErrorCode::InternalError(Some(
                    "only http and https schemes are supported".to_string(),
                ))));
            }
            None => "https",
        };
        let path = request_data.path_with_query.as_deref().unwrap_or("/");
        let request_url = url::Url::parse(&format!("{scheme_str}://{authority}{path}"))
            .map_err(|e| HyperlightError::Error(format!("invalid request URL: {e}")))?;

        {
            let Ok(network) = self.network.lock() else {
                return Ok(Err(ErrorCode::HTTPRequestDenied));
            };
            if !network.is_allowed(&request_url, &http_method) {
                return Ok(Err(ErrorCode::HTTPRequestDenied));
            }
        }

        // Limit concurrent outbound HTTP requests.
        let active = self.active_requests.fetch_add(1, Ordering::SeqCst);
        if active >= sandbox_http::MAX_CONCURRENT_REQUESTS {
            self.active_requests.fetch_sub(1, Ordering::SeqCst);
            return Ok(Err(ErrorCode::InternalError(Some(
                "too many concurrent HTTP requests".to_string(),
            ))));
        }
        let active_requests = self.active_requests.clone();

        // Collect guest headers eagerly in sync context.
        let guest_headers: Vec<(String, String)> = request_data
            .headers
            .read()
            .block_on()
            .entries()
            .into_iter()
            .map(|(k, v)| (k, String::from_utf8_lossy(&v).into_owned()))
            .collect();

        let future_response = Resource::new(FutureIncomingResponse::default());
        let future_response_clone = future_response.clone();
        let future_response_panic = future_response.clone();

        let handle = async move {
            // Wait for the guest to finish writing the outgoing body.
            let body_resource = request_data.body.clone();
            {
                let mut guard = body_resource.write().await;
                if !guard.body_taken && !guard.is_finished() {
                    let _ = guard.body.write().await.close();
                    guard.finished = true;
                }
            }
            let mut body_guard = body_resource.write_wait_until(|b| b.is_finished()).await;

            // Take the buffer and stream chunks directly — no full-body copy.
            let body_data = body_guard.take_data().await;
            let body_stream = crate::wasi_impl::body_stream::WasiBodyStream::new(body_data);
            let request_body: sandbox_http::RequestBody =
                http_body_util::BodyExt::boxed_unsync(body_stream);

            let http_req = sandbox_http::HttpRequest {
                url: request_url,
                method: http_method.to_string(),
                headers: guest_headers,
                body: request_body,
            };

            let result = sandbox_http::send_http_request(http_req)
                .await
                .map_err(|e| ErrorCode::InternalError(Some(format!("{e:#}"))));

            match result {
                Err(err) => {
                    future_response_clone.write().await.set(Err(err));
                }
                Ok(resp) => {
                    let mut hdr_map = http::HeaderMap::new();
                    for (k, v) in &resp.headers {
                        if let (Ok(name), Ok(val)) = (
                            http::header::HeaderName::try_from(k.as_str()),
                            http::header::HeaderValue::try_from(v.as_str()),
                        ) {
                            hdr_map.append(name, val);
                        }
                    }
                    let headers = Headers::from_http_headers(hdr_map);

                    let mut stream = Stream::new();
                    let _ = stream.write(&resp.body);
                    stream.close();

                    let body = IncomingBody {
                        stream: Resource::new(stream),
                        trailers: Resource::default(),
                        stream_taken: false,
                    };

                    let response = IncomingResponse {
                        status: resp.status,
                        headers: Resource::new(headers),
                        body: Resource::new(body),
                        body_taken: false,
                    };

                    future_response_clone
                        .write()
                        .await
                        .set(Ok(Resource::new(response)));
                }
            }
        }
        .spawn();

        // Monitor task: if the HTTP handler panics, surface the error.
        async move {
            if let Err(join_err) = handle.await {
                let msg = format!("HTTP handler task failed: {join_err}");
                future_response_panic
                    .write()
                    .await
                    .set(Err(ErrorCode::InternalError(Some(msg))));
            }
            active_requests.fetch_sub(1, Ordering::SeqCst);
        }
        .spawn();

        Ok(Ok(future_response))
    }
}
