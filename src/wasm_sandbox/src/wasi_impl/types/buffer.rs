use std::collections::VecDeque;
use std::mem;

/// Maximum buffer size (16 MiB) to prevent guest-controlled OOM.
const MAX_BUFFER_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub struct Buffer {
    buffer: VecDeque<u8>,
    closed: bool,
    total_written: usize,
    total_read: usize,
}

pub struct BufferClosed;

impl Buffer {
    pub fn write(&mut self, data: impl AsRef<[u8]>) -> Result<(), BufferClosed> {
        if self.closed {
            return Err(BufferClosed);
        }
        let available = MAX_BUFFER_BYTES.saturating_sub(self.buffer.len());
        let to_write = &data.as_ref()[..data.as_ref().len().min(available)];
        self.buffer.extend(to_write);
        self.total_written += to_write.len();
        Ok(())
    }

    pub fn writable(&self) -> bool {
        !self.closed && self.buffer.len() < MAX_BUFFER_BYTES
    }

    pub fn read(&mut self, n: usize) -> Result<Vec<u8>, BufferClosed> {
        if self.buffer.is_empty() && self.closed {
            return Err(BufferClosed);
        }
        let n = n.min(self.buffer.len());
        let mut tail = self.buffer.split_off(n);
        mem::swap(&mut self.buffer, &mut tail);
        self.total_read += n;
        Ok(tail.into())
    }

    pub fn readable(&self) -> bool {
        self.closed || !self.buffer.is_empty()
    }

    pub fn close(&mut self) -> (usize, usize) {
        self.closed = true;
        (self.total_read, self.total_written)
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Take ownership of the buffered data, leaving the buffer empty.
    pub fn take_data(&mut self) -> VecDeque<u8> {
        mem::take(&mut self.buffer)
    }
}
