use super::headers::Headers;
use super::stream::Stream;
use crate::wasi_impl::resource::Resource;

#[derive(Default)]
pub struct OutgoingBody {
    pub body: Resource<Stream>,
    pub trailers: Resource<Headers>,
    pub body_taken: bool,
    pub finished: bool,
}

impl OutgoingBody {
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Take the buffered body data for streaming, leaving the stream empty.
    pub async fn take_data(&mut self) -> std::collections::VecDeque<u8> {
        self.body.write().await.take_data()
    }
}
