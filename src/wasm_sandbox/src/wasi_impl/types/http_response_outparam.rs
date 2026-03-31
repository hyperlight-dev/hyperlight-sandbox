use super::http_outgoing_response::OutgoingResponse;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;

#[derive(Default)]
pub struct ResponseOutparam {
    pub response: Option<Result<Resource<OutgoingResponse>, wasi::http::types::ErrorCode>>,
}
