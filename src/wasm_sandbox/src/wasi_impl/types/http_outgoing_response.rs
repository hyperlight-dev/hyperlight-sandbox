use super::headers::Headers;
use super::http_outgoing_body::OutgoingBody;
use crate::wasi_impl::resource::Resource;

pub struct OutgoingResponse {
    pub status_code: u16,
    pub headers: Resource<Headers>,
    pub body: Resource<OutgoingBody>,
    body_taken: bool,
}

impl OutgoingResponse {
    pub fn new(headers: Resource<Headers>) -> Self {
        Self {
            status_code: 200,
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
