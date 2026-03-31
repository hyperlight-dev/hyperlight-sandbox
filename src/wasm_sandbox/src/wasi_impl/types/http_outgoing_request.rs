use super::headers::Headers;
use super::http_outgoing_body::OutgoingBody;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;

pub struct OutgoingRequest {
    pub method: wasi::http::types::Method,
    pub path_with_query: Option<String>,
    pub scheme: Option<wasi::http::types::Scheme>,
    pub authority: Option<String>,
    pub headers: Resource<Headers>,
    pub body: Resource<OutgoingBody>,
    body_taken: bool,
}

impl OutgoingRequest {
    pub fn new(headers: Resource<Headers>) -> Self {
        Self {
            method: wasi::http::types::Method::Get,
            path_with_query: None,
            scheme: None,
            authority: None,
            headers,
            body: Resource::default(),
            body_taken: false,
        }
    }

    pub fn take_body(&mut self) -> Option<Resource<OutgoingBody>> {
        if self.body_taken {
            return None;
        }
        self.body_taken = true;
        Some(self.body.clone())
    }
}

impl Clone for wasi::http::types::Method {
    fn clone(&self) -> Self {
        use wasi::http::types::Method;
        match self {
            Method::Get => Method::Get,
            Method::Head => Method::Head,
            Method::Post => Method::Post,
            Method::Put => Method::Put,
            Method::Delete => Method::Delete,
            Method::Connect => Method::Connect,
            Method::Options => Method::Options,
            Method::Trace => Method::Trace,
            Method::Patch => Method::Patch,
            Method::Other(method) => Method::Other(method.clone()),
        }
    }
}

impl Clone for wasi::http::types::Scheme {
    fn clone(&self) -> Self {
        use wasi::http::types::Scheme;
        match self {
            Scheme::HTTP => Scheme::HTTP,
            Scheme::HTTPS => Scheme::HTTPS,
            Scheme::Other(scheme) => Scheme::Other(scheme.clone()),
        }
    }
}
