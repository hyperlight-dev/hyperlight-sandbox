use super::http_future_headers::FutureHeaders;
use super::stream::Stream;
use crate::wasi_impl::resource::Resource;

#[derive(Default)]
pub struct IncomingBody {
    pub stream: Resource<Stream>,
    pub trailers: Resource<FutureHeaders>,
    pub stream_taken: bool,
}
