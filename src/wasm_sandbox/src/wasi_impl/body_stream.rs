//! Bridge between a WASI [`OutgoingBody`] and [`http_body::Body`].
//!
//! Takes the internal buffer from the finished WASI body stream and
//! yields it as chunks without an extra full-body copy.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body::Frame;

const CHUNK_SIZE: usize = 8192;

/// An [`http_body::Body`] that yields chunks from a taken `VecDeque<u8>`.
pub struct WasiBodyStream {
    buffer: VecDeque<u8>,
}

impl WasiBodyStream {
    pub fn new(buffer: VecDeque<u8>) -> Self {
        Self { buffer }
    }
}

impl http_body::Body for WasiBodyStream {
    type Data = Bytes;
    type Error = anyhow::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if self.buffer.is_empty() {
            return Poll::Ready(None);
        }
        let n = self.buffer.len().min(CHUNK_SIZE);
        let chunk: Vec<u8> = self.buffer.drain(..n).collect();
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(chunk)))))
    }
}
