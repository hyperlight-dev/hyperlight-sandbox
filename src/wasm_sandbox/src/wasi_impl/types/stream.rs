use std::sync::{Arc, Mutex};

use hyperlight_sandbox::{CapFs, FsError};

use super::buffer::{Buffer, BufferClosed};
use crate::bindings::wasi;

#[derive(Default)]
pub struct Stream {
    kind: StreamKind,
}

enum StreamKind {
    Buffer(Buffer),
    CapFs {
        stream_id: u32,
        fs: Arc<Mutex<CapFs>>,
    },
}

impl Default for StreamKind {
    fn default() -> Self {
        StreamKind::Buffer(Buffer::default())
    }
}

impl<E> From<BufferClosed> for wasi::io::streams::StreamError<E> {
    fn from(_: BufferClosed) -> Self {
        wasi::io::streams::StreamError::Closed
    }
}

type StreamError = wasi::io::streams::StreamError<anyhow::Error>;

fn filesystem_error(error: FsError) -> StreamError {
    StreamError::LastOperationFailed(anyhow::Error::new(error))
}

impl Stream {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_cap_fs(stream_id: u32, fs: Arc<Mutex<CapFs>>) -> Self {
        Self {
            kind: StreamKind::CapFs { stream_id, fs },
        }
    }

    pub fn check_write(&self) -> Result<u64, BufferClosed> {
        match &self.kind {
            StreamKind::Buffer(buf) => {
                if buf.is_closed() {
                    return Err(BufferClosed);
                }
                if buf.writable() { Ok(4096) } else { Ok(0) }
            }
            StreamKind::CapFs { .. } => Ok(65536),
        }
    }

    pub fn write(&mut self, data: impl AsRef<[u8]>) -> Result<(), StreamError> {
        match &mut self.kind {
            StreamKind::Buffer(buf) => buf.write(data).map_err(Into::into),
            StreamKind::CapFs { stream_id, fs } => {
                let Ok(mut cap_fs) = fs.lock() else {
                    return Err(StreamError::Closed);
                };
                if cap_fs.has_stream(*stream_id) && cap_fs.is_write_stream(*stream_id) {
                    cap_fs
                        .stream_write(*stream_id, data.as_ref())
                        .map(|_| ())
                        .map_err(filesystem_error)
                } else {
                    Err(StreamError::Closed)
                }
            }
        }
    }

    pub fn flush(&mut self) -> Result<(), StreamError> {
        Ok(())
    }

    pub fn splice(&mut self, src: &mut Stream, len: usize) -> Result<usize, StreamError> {
        let n = self.check_write().map_err(StreamError::from)? as usize;
        let n = n.min(len);
        let data = src.read(n).map_err(StreamError::from)?;
        self.write(&data)?;
        Ok(data.len())
    }

    pub fn read(&mut self, len: usize) -> Result<Vec<u8>, BufferClosed> {
        match &mut self.kind {
            StreamKind::Buffer(buf) => buf.read(len),
            StreamKind::CapFs { stream_id, fs } => {
                let Ok(mut cap_fs) = fs.lock() else {
                    return Err(BufferClosed);
                };
                if cap_fs.has_stream(*stream_id) && !cap_fs.is_write_stream(*stream_id) {
                    cap_fs
                        .stream_read(*stream_id, len as u64)
                        .map_err(|_| BufferClosed)
                } else {
                    Err(BufferClosed)
                }
            }
        }
    }

    pub fn readable(&self) -> bool {
        match &self.kind {
            StreamKind::Buffer(buf) => buf.readable(),
            StreamKind::CapFs { stream_id, fs } => {
                let Ok(cap_fs) = fs.lock() else {
                    return false;
                };
                cap_fs.has_stream(*stream_id) && !cap_fs.is_write_stream(*stream_id)
            }
        }
    }

    pub fn writable(&self) -> bool {
        match &self.kind {
            StreamKind::Buffer(buf) => buf.writable(),
            StreamKind::CapFs { stream_id, fs } => {
                let Ok(cap_fs) = fs.lock() else {
                    return false;
                };
                cap_fs.has_stream(*stream_id) && cap_fs.is_write_stream(*stream_id)
            }
        }
    }

    pub fn close(&mut self) -> (usize, usize) {
        match &mut self.kind {
            StreamKind::Buffer(buf) => buf.close(),
            StreamKind::CapFs { stream_id, fs } => {
                if let Ok(mut cap_fs) = fs.lock() {
                    cap_fs.close_stream(*stream_id);
                }
                (0, 0)
            }
        }
    }

    /// Take ownership of the internal buffer data for streaming out.
    /// Only works for buffer-backed streams (outgoing request bodies).
    pub fn take_data(&mut self) -> std::collections::VecDeque<u8> {
        match &mut self.kind {
            StreamKind::Buffer(buf) => buf.take_data(),
            StreamKind::CapFs { .. } => std::collections::VecDeque::new(),
        }
    }
}
