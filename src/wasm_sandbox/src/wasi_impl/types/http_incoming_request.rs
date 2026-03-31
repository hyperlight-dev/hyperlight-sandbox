use super::headers::Headers;
use super::http_incoming_body::IncomingBody;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;

pub struct IncomingRequest {
    pub method: wasi::http::types::Method,
    pub path_with_query: Option<String>,
    pub scheme: Option<wasi::http::types::Scheme>,
    pub authority: Option<String>,
    pub headers: Resource<Headers>,
    pub body: Resource<IncomingBody>,
    pub body_taken: bool,
}
