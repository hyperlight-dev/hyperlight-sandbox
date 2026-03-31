//! HTTP type trait implementations.
#![allow(unused_variables)]

use hyperlight_common::resource::BorrowedResourceGuard;
use hyperlight_host::HyperlightError;
use wasi::http::types as http_types;

use crate::HostState;
use crate::bindings::wasi;
use crate::wasi_impl::resource::{BlockOn, Resource};
use crate::wasi_impl::types::headers::{HeaderError, Headers};
use crate::wasi_impl::types::http_future_headers::FutureHeaders;
use crate::wasi_impl::types::http_future_incoming_response::FutureIncomingResponse;
use crate::wasi_impl::types::http_incoming_body::IncomingBody;
use crate::wasi_impl::types::http_incoming_request::IncomingRequest;
use crate::wasi_impl::types::http_incoming_response::IncomingResponse;
use crate::wasi_impl::types::http_outgoing_body::OutgoingBody;
use crate::wasi_impl::types::http_outgoing_request::OutgoingRequest;
use crate::wasi_impl::types::http_outgoing_response::OutgoingResponse;
use crate::wasi_impl::types::http_request_options::RequestOptions;
use crate::wasi_impl::types::http_response_outparam::ResponseOutparam;
use crate::wasi_impl::types::pollable::AnyPollable;
use crate::wasi_impl::types::stream::Stream;

type HlResult<T> = Result<T, HyperlightError>;

// ---------------------------------------------------------------------------
// Fields (Headers/Trailers)
// ---------------------------------------------------------------------------

impl From<HeaderError> for http_types::HeaderError {
    fn from(err: HeaderError) -> Self {
        match err {
            HeaderError::Immutable => http_types::HeaderError::Immutable,
            HeaderError::InvalidHeader => http_types::HeaderError::InvalidSyntax,
            HeaderError::Forbidden => http_types::HeaderError::Forbidden,
            HeaderError::TooMany => http_types::HeaderError::Forbidden,
        }
    }
}

impl http_types::Fields for HostState {
    type T = Resource<Headers>;

    fn new(&mut self) -> Resource<Headers> {
        Resource::default()
    }
    fn from_list(
        &mut self,
        entries: Vec<(String, Vec<u8>)>,
    ) -> HlResult<Result<Resource<Headers>, http_types::HeaderError>> {
        Ok(Headers::from_list(entries)
            .map(Resource::new)
            .map_err(Into::into))
    }
    fn get(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Headers>>,
        name: String,
    ) -> HlResult<Vec<Vec<u8>>> {
        Ok(self_.read().block_on().get(name).unwrap_or_default())
    }
    fn has(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Headers>>,
        name: String,
    ) -> HlResult<bool> {
        Ok(self_.read().block_on().has(name))
    }
    fn set(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Headers>>,
        name: String,
        value: Vec<Vec<u8>>,
    ) -> HlResult<Result<(), http_types::HeaderError>> {
        Ok(self_
            .write()
            .block_on()
            .set(name, value)
            .map_err(Into::into))
    }
    fn delete(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Headers>>,
        name: String,
    ) -> HlResult<Result<(), http_types::HeaderError>> {
        Ok(self_.write().block_on().delete(name).map_err(Into::into))
    }
    fn append(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Headers>>,
        name: String,
        value: Vec<u8>,
    ) -> HlResult<Result<(), http_types::HeaderError>> {
        Ok(self_
            .write()
            .block_on()
            .append(name, value)
            .map_err(Into::into))
    }
    fn entries(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Headers>>,
    ) -> HlResult<Vec<(String, Vec<u8>)>> {
        Ok(self_.read().block_on().entries())
    }
    fn clone(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Headers>>,
    ) -> HlResult<Resource<Headers>> {
        Ok(self_.clone())
    }
}

// ---------------------------------------------------------------------------
// IncomingRequest
// ---------------------------------------------------------------------------

impl http_types::IncomingRequest<Resource<Headers>, Resource<IncomingBody>> for HostState {
    type T = Resource<IncomingRequest>;
    fn method(
        &mut self,
        self_: BorrowedResourceGuard<Resource<IncomingRequest>>,
    ) -> HlResult<http_types::Method> {
        Ok(self_.read().block_on().method.clone())
    }
    fn path_with_query(
        &mut self,
        self_: BorrowedResourceGuard<Resource<IncomingRequest>>,
    ) -> HlResult<Option<String>> {
        Ok(self_.read().block_on().path_with_query.clone())
    }
    fn scheme(
        &mut self,
        self_: BorrowedResourceGuard<Resource<IncomingRequest>>,
    ) -> HlResult<Option<http_types::Scheme>> {
        Ok(self_.read().block_on().scheme.clone())
    }
    fn authority(
        &mut self,
        self_: BorrowedResourceGuard<Resource<IncomingRequest>>,
    ) -> HlResult<Option<String>> {
        Ok(self_.read().block_on().authority.clone())
    }
    fn headers(
        &mut self,
        self_: BorrowedResourceGuard<Resource<IncomingRequest>>,
    ) -> HlResult<Resource<Headers>> {
        Ok(self_.read().block_on().headers.clone())
    }
    fn consume(
        &mut self,
        self_: BorrowedResourceGuard<Resource<IncomingRequest>>,
    ) -> HlResult<Result<Resource<IncomingBody>, ()>> {
        let mut guard = self_.write().block_on();
        if guard.body_taken {
            return Ok(Err(()));
        }
        guard.body_taken = true;
        Ok(Ok(guard.body.clone()))
    }
}

// ---------------------------------------------------------------------------
// OutgoingRequest
// ---------------------------------------------------------------------------

impl http_types::OutgoingRequest<Resource<Headers>, Resource<OutgoingBody>> for HostState {
    type T = Resource<OutgoingRequest>;
    fn new(&mut self, headers: Resource<Headers>) -> Resource<OutgoingRequest> {
        Resource::new(OutgoingRequest::new(headers))
    }
    fn body(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingRequest>>,
    ) -> HlResult<Result<Resource<OutgoingBody>, ()>> {
        let mut guard = self_.write().block_on();
        Ok(guard.take_body().ok_or(()))
    }
    fn method(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingRequest>>,
    ) -> HlResult<http_types::Method> {
        Ok(self_.read().block_on().method.clone())
    }
    fn set_method(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingRequest>>,
        method: http_types::Method,
    ) -> HlResult<Result<(), ()>> {
        self_.write().block_on().method = method;
        Ok(Ok(()))
    }
    fn path_with_query(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingRequest>>,
    ) -> HlResult<Option<String>> {
        Ok(self_.read().block_on().path_with_query.clone())
    }
    fn set_path_with_query(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingRequest>>,
        path_with_query: Option<String>,
    ) -> HlResult<Result<(), ()>> {
        self_.write().block_on().path_with_query = path_with_query;
        Ok(Ok(()))
    }
    fn scheme(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingRequest>>,
    ) -> HlResult<Option<http_types::Scheme>> {
        Ok(self_.read().block_on().scheme.clone())
    }
    fn set_scheme(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingRequest>>,
        scheme: Option<http_types::Scheme>,
    ) -> HlResult<Result<(), ()>> {
        self_.write().block_on().scheme = scheme;
        Ok(Ok(()))
    }
    fn authority(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingRequest>>,
    ) -> HlResult<Option<String>> {
        Ok(self_.read().block_on().authority.clone())
    }
    fn set_authority(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingRequest>>,
        authority: Option<String>,
    ) -> HlResult<Result<(), ()>> {
        self_.write().block_on().authority = authority;
        Ok(Ok(()))
    }
    fn headers(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingRequest>>,
    ) -> HlResult<Resource<Headers>> {
        Ok(self_.read().block_on().headers.clone())
    }
}

// ---------------------------------------------------------------------------
// RequestOptions
// ---------------------------------------------------------------------------

fn as_u64_nanos_saturating(duration: &std::time::Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

impl http_types::RequestOptions<u64> for HostState {
    type T = Resource<RequestOptions>;
    fn new(&mut self) -> Resource<RequestOptions> {
        Resource::default()
    }
    fn connect_timeout(
        &mut self,
        self_: BorrowedResourceGuard<Resource<RequestOptions>>,
    ) -> HlResult<Option<u64>> {
        Ok(self_
            .read()
            .block_on()
            .connect_timeout
            .as_ref()
            .map(as_u64_nanos_saturating))
    }
    fn set_connect_timeout(
        &mut self,
        self_: BorrowedResourceGuard<Resource<RequestOptions>>,
        duration: Option<u64>,
    ) -> HlResult<Result<(), ()>> {
        self_.write().block_on().connect_timeout = duration.map(std::time::Duration::from_nanos);
        Ok(Ok(()))
    }
    fn first_byte_timeout(
        &mut self,
        self_: BorrowedResourceGuard<Resource<RequestOptions>>,
    ) -> HlResult<Option<u64>> {
        Ok(self_
            .read()
            .block_on()
            .first_byte_timeout
            .as_ref()
            .map(as_u64_nanos_saturating))
    }
    fn set_first_byte_timeout(
        &mut self,
        self_: BorrowedResourceGuard<Resource<RequestOptions>>,
        duration: Option<u64>,
    ) -> HlResult<Result<(), ()>> {
        self_.write().block_on().first_byte_timeout = duration.map(std::time::Duration::from_nanos);
        Ok(Ok(()))
    }
    fn between_bytes_timeout(
        &mut self,
        self_: BorrowedResourceGuard<Resource<RequestOptions>>,
    ) -> HlResult<Option<u64>> {
        Ok(self_
            .read()
            .block_on()
            .between_bytes_timeout
            .as_ref()
            .map(as_u64_nanos_saturating))
    }
    fn set_between_bytes_timeout(
        &mut self,
        self_: BorrowedResourceGuard<Resource<RequestOptions>>,
        duration: Option<u64>,
    ) -> HlResult<Result<(), ()>> {
        self_.write().block_on().between_bytes_timeout =
            duration.map(std::time::Duration::from_nanos);
        Ok(Ok(()))
    }
}

// ---------------------------------------------------------------------------
// ResponseOutparam
// ---------------------------------------------------------------------------

impl http_types::ResponseOutparam<Resource<OutgoingResponse>> for HostState {
    type T = Resource<ResponseOutparam>;
    fn set(
        &mut self,
        param: Resource<ResponseOutparam>,
        response: Result<Resource<OutgoingResponse>, http_types::ErrorCode>,
    ) -> HlResult<()> {
        param.write().block_on().response = Some(response);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// IncomingResponse
// ---------------------------------------------------------------------------

impl http_types::IncomingResponse<Resource<Headers>, Resource<IncomingBody>> for HostState {
    type T = Resource<IncomingResponse>;
    fn status(
        &mut self,
        self_: BorrowedResourceGuard<Resource<IncomingResponse>>,
    ) -> HlResult<u16> {
        Ok(self_.read().block_on().status)
    }
    fn headers(
        &mut self,
        self_: BorrowedResourceGuard<Resource<IncomingResponse>>,
    ) -> HlResult<Resource<Headers>> {
        Ok(self_.read().block_on().headers.clone())
    }
    fn consume(
        &mut self,
        self_: BorrowedResourceGuard<Resource<IncomingResponse>>,
    ) -> HlResult<Result<Resource<IncomingBody>, ()>> {
        let mut guard = self_.write().block_on();
        if guard.body_taken {
            return Ok(Err(()));
        }
        guard.body_taken = true;
        Ok(Ok(guard.body.clone()))
    }
}

// ---------------------------------------------------------------------------
// IncomingBody
// ---------------------------------------------------------------------------

impl http_types::IncomingBody<Resource<FutureHeaders>, Resource<Stream>> for HostState {
    type T = Resource<IncomingBody>;
    fn stream(
        &mut self,
        self_: BorrowedResourceGuard<Resource<IncomingBody>>,
    ) -> HlResult<Result<Resource<Stream>, ()>> {
        let mut guard = self_.write().block_on();
        if guard.stream_taken {
            return Ok(Err(()));
        }
        guard.stream_taken = true;
        Ok(Ok(guard.stream.clone()))
    }
    fn finish(&mut self, this: Resource<IncomingBody>) -> HlResult<Resource<FutureHeaders>> {
        Ok(this.read().block_on().trailers.clone())
    }
}

// ---------------------------------------------------------------------------
// OutgoingBody
// ---------------------------------------------------------------------------

impl http_types::OutgoingBody<Resource<Headers>, Resource<Stream>> for HostState {
    type T = Resource<OutgoingBody>;
    fn write(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingBody>>,
    ) -> HlResult<Result<Resource<Stream>, ()>> {
        let mut guard = self_.write().block_on();
        if guard.body_taken {
            return Ok(Err(()));
        }
        guard.body_taken = true;
        Ok(Ok(guard.body.clone()))
    }
    fn finish(
        &mut self,
        this: Resource<OutgoingBody>,
        trailers: Option<Resource<Headers>>,
    ) -> HlResult<Result<(), http_types::ErrorCode>> {
        let mut guard = this.write().block_on();
        let (_, written) = guard.body.write().block_on().close();
        guard.finished = true;
        guard.trailers = trailers.unwrap_or_default();
        Ok(Ok(()))
    }
}

// ---------------------------------------------------------------------------
// OutgoingResponse
// ---------------------------------------------------------------------------

impl http_types::OutgoingResponse<Resource<Headers>, Resource<OutgoingBody>> for HostState {
    type T = Resource<OutgoingResponse>;
    fn new(&mut self, headers: Resource<Headers>) -> Resource<OutgoingResponse> {
        Resource::new(OutgoingResponse::new(headers))
    }
    fn status_code(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingResponse>>,
    ) -> HlResult<u16> {
        Ok(self_.read().block_on().status_code)
    }
    fn set_status_code(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingResponse>>,
        status_code: u16,
    ) -> HlResult<Result<(), ()>> {
        self_.write().block_on().status_code = status_code;
        Ok(Ok(()))
    }
    fn headers(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingResponse>>,
    ) -> HlResult<Resource<Headers>> {
        Ok(self_.read().block_on().headers.clone())
    }
    fn body(
        &mut self,
        self_: BorrowedResourceGuard<Resource<OutgoingResponse>>,
    ) -> HlResult<Result<Resource<OutgoingBody>, ()>> {
        let mut guard = self_.write().block_on();
        Ok(guard.take_body().ok_or(()))
    }
}

// ---------------------------------------------------------------------------
// FutureTrailers
// ---------------------------------------------------------------------------

impl http_types::FutureTrailers<Resource<Headers>, Resource<AnyPollable>> for HostState {
    type T = Resource<FutureHeaders>;
    fn subscribe(
        &mut self,
        self_: BorrowedResourceGuard<Resource<FutureHeaders>>,
    ) -> HlResult<Resource<AnyPollable>> {
        Ok(self_.poll(|r| r.is_ready()))
    }
    fn get(
        &mut self,
        self_: BorrowedResourceGuard<Resource<FutureHeaders>>,
    ) -> HlResult<Option<Result<Result<Option<Resource<Headers>>, http_types::ErrorCode>, ()>>>
    {
        Ok(match self_.write().block_on().get() {
            Some(Ok(Ok(headers))) if headers.read().block_on().is_empty() => Some(Ok(Ok(None))),
            Some(Ok(Ok(headers))) => Some(Ok(Ok(Some(headers)))),
            Some(Ok(Err(e))) => Some(Ok(Err(e))),
            Some(Err(())) => Some(Err(())),
            None => None,
        })
    }
}

// ---------------------------------------------------------------------------
// FutureIncomingResponse
// ---------------------------------------------------------------------------

impl http_types::FutureIncomingResponse<Resource<IncomingResponse>, Resource<AnyPollable>>
    for HostState
{
    type T = Resource<FutureIncomingResponse>;
    fn subscribe(
        &mut self,
        self_: BorrowedResourceGuard<Resource<FutureIncomingResponse>>,
    ) -> HlResult<Resource<AnyPollable>> {
        Ok(self_.poll(|r| r.is_ready()))
    }
    fn get(
        &mut self,
        self_: BorrowedResourceGuard<Resource<FutureIncomingResponse>>,
    ) -> HlResult<Option<Result<Result<Resource<IncomingResponse>, http_types::ErrorCode>, ()>>>
    {
        Ok(self_.write().block_on().get())
    }
}

// ---------------------------------------------------------------------------
// HTTP Types namespace
// ---------------------------------------------------------------------------

impl
    wasi::http::Types<u64, anyhow::Error, Resource<Stream>, Resource<Stream>, Resource<AnyPollable>>
    for HostState
{
    fn http_error_code(
        &mut self,
        err: BorrowedResourceGuard<anyhow::Error>,
    ) -> HlResult<Option<http_types::ErrorCode>> {
        Ok(Some(http_types::ErrorCode::InternalError(Some(
            err.to_string(),
        ))))
    }
}
