use super::headers::Headers;
use super::http_future::FutureHttp;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;

pub type FutureHeaders = FutureHttp<Result<Resource<Headers>, wasi::http::types::ErrorCode>>;
