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
            StreamKind::CapFs { stream_id, fs } => {
                let cap_fs = fs.lock().map_err(|_| BufferClosed)?;
                if cap_fs.has_stream(*stream_id) && cap_fs.is_write_stream(*stream_id) {
                    Ok(65536)
                } else {
                    Err(BufferClosed)
                }
            }
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
                    match cap_fs.stream_write(*stream_id, data.as_ref()) {
                        Ok(_) => Ok(()),
                        Err(error) => {
                            cap_fs.close_stream(*stream_id);
                            Err(filesystem_error(error))
                        }
                    }
                } else {
                    Err(StreamError::Closed)
                }
            }
        }
    }

    pub fn flush(&mut self) -> Result<(), StreamError> {
        match &self.kind {
            StreamKind::Buffer(buf) => {
                if buf.is_closed() {
                    Err(StreamError::Closed)
                } else {
                    Ok(())
                }
            }
            StreamKind::CapFs { stream_id, fs } => {
                let Ok(cap_fs) = fs.lock() else {
                    return Err(StreamError::Closed);
                };
                if cap_fs.has_stream(*stream_id) && cap_fs.is_write_stream(*stream_id) {
                    Ok(())
                } else {
                    Err(StreamError::Closed)
                }
            }
        }
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

    pub fn write_ready(&self) -> bool {
        match &self.kind {
            StreamKind::Buffer(buf) => buf.is_closed() || buf.writable(),
            StreamKind::CapFs { stream_id, fs } => {
                let Ok(cap_fs) = fs.lock() else {
                    return true;
                };
                !cap_fs.has_stream(*stream_id) || cap_fs.is_write_stream(*stream_id)
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

#[cfg(test)]
mod tests {
    use hyperlight_sandbox::{DirPerms, FilePerms, FilesystemLimits, OpenFlags};

    use super::*;

    #[test]
    fn filesystem_write_failure_closes_output_stream() {
        let output = tempfile::tempdir().unwrap();
        let mut cap_fs = CapFs::with_limits(FilesystemLimits::new(4, 4, 1).unwrap())
            .with_output_dir(
                output.path(),
                DirPerms::READ | DirPerms::MUTATE,
                FilePerms::READ | FilePerms::WRITE,
            )
            .unwrap();
        let output_fd = cap_fs
            .preopens()
            .into_iter()
            .find_map(|(fd, name)| (name == "/output").then_some(fd))
            .unwrap();
        let file_fd = cap_fs
            .open_at(output_fd, "limited.bin", OpenFlags::CREATE)
            .unwrap();
        let stream_id = cap_fs.create_write_stream(file_fd, 0).unwrap();
        let cap_fs = Arc::new(Mutex::new(cap_fs));
        let mut stream = Stream::from_cap_fs(stream_id, cap_fs.clone());

        assert!(matches!(stream.check_write(), Ok(65536)));
        assert!(matches!(
            stream.write(b"abcde"),
            Err(StreamError::LastOperationFailed(_))
        ));
        assert!(!cap_fs.lock().unwrap().has_stream(stream_id));
        assert!(stream.write_ready());
        assert!(stream.check_write().is_err());
        assert!(matches!(stream.write(b"x"), Err(StreamError::Closed)));
        assert!(matches!(stream.flush(), Err(StreamError::Closed)));
    }
}
