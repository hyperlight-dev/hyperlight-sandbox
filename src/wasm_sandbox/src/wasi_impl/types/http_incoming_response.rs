use super::headers::Headers;
use super::http_incoming_body::IncomingBody;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;

pub struct IncomingResponse {
    pub status: wasi::http::types::StatusCode,
    pub headers: Resource<Headers>,
    pub body: Resource<IncomingBody>,
    pub body_taken: bool,
}
