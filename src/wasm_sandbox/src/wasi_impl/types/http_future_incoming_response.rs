use super::http_future::FutureHttp;
use super::http_incoming_response::IncomingResponse;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;

pub type FutureIncomingResponse =
    FutureHttp<Result<Resource<IncomingResponse>, wasi::http::types::ErrorCode>>;
