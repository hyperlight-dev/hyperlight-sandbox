//! I/O trait implementations: Error, Poll, Streams.
#![allow(unused_variables)]

use hyperlight_common::resource::BorrowedResourceGuard;
use wasi::io::streams;

use crate::HostState;
use crate::bindings::wasi;
use crate::wasi_impl::resource::{BlockOn, Resource};
use crate::wasi_impl::types::pollable::AnyPollable;
use crate::wasi_impl::types::stream::Stream;

type HlResult<T> = T;

/// Maximum byte count for a single write-zeroes or random-bytes allocation.
/// 16 MiB is generous for any legitimate use while preventing host OOM.
const MAX_ALLOC_BYTES: u64 = 16 * 1024 * 1024;

// ---------------------------------------------------------------------------
// IO: Error
// ---------------------------------------------------------------------------

impl wasi::io::error::Error<crate::HostBindings> for HostState {
    type T = anyhow::Error;
    fn to_debug_string(&mut self, self_: BorrowedResourceGuard<anyhow::Error>) -> HlResult<String> {
        self_.to_string()
    }
}

impl wasi::io::Error<crate::HostBindings> for HostState {}

// ---------------------------------------------------------------------------
// IO: Poll
// ---------------------------------------------------------------------------

impl wasi::io::poll::Pollable<crate::HostBindings> for HostState {
    type T = Resource<AnyPollable>;
    fn ready(&mut self, self_: BorrowedResourceGuard<Resource<AnyPollable>>) -> HlResult<bool> {
        self_.write().block_on().ready().block_on()
    }
    fn block(&mut self, self_: BorrowedResourceGuard<Resource<AnyPollable>>) -> HlResult<()> {
        self_.write().block_on().block().block_on();
    }
}

impl wasi::io::Poll<crate::HostBindings> for HostState {
    fn poll(
        &mut self,
        pollables: Vec<BorrowedResourceGuard<Resource<AnyPollable>>>,
    ) -> HlResult<Vec<u32>> {
        use std::future::poll_fn;
        use std::task::Poll;

        let mut guards = pollables
            .into_iter()
            .map(|p| p.write().block_on())
            .collect::<Vec<_>>();

        poll_fn(move |cx| {
            for (i, pollable) in guards.iter_mut().enumerate() {
                if let Poll::Ready(true) = pollable.poll(cx) {
                    return Poll::Ready(vec![i as u32]);
                }
            }
            Poll::Pending
        })
        .block_on()
    }
}

// ---------------------------------------------------------------------------
// IO: Streams
// ---------------------------------------------------------------------------

impl streams::InputStream<crate::HostBindings, anyhow::Error, Resource<AnyPollable>> for HostState {
    type T = Resource<Stream>;
    fn read(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
        len: u64,
    ) -> HlResult<Result<Vec<u8>, streams::StreamError<anyhow::Error>>> {
        let mut guard = self_.write().block_on();
        guard
            .read(len as usize)
            .map_err(|_| streams::StreamError::Closed)
    }
    fn blocking_read(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
        len: u64,
    ) -> HlResult<Result<Vec<u8>, streams::StreamError<anyhow::Error>>> {
        let mut guard = self_.write_wait_until(Stream::readable).block_on();
        guard
            .read(len as usize)
            .map_err(|_| streams::StreamError::Closed)
    }
    fn skip(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
        len: u64,
    ) -> HlResult<Result<u64, streams::StreamError<anyhow::Error>>> {
        let mut guard = self_.write().block_on();
        guard
            .read(len as usize)
            .map(|d| d.len() as u64)
            .map_err(|_| streams::StreamError::Closed)
    }
    fn blocking_skip(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
        len: u64,
    ) -> HlResult<Result<u64, streams::StreamError<anyhow::Error>>> {
        let mut guard = self_.write_wait_until(Stream::readable).block_on();
        guard
            .read(len as usize)
            .map(|d| d.len() as u64)
            .map_err(|_| streams::StreamError::Closed)
    }
    fn subscribe(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
    ) -> HlResult<Resource<AnyPollable>> {
        self_.poll(|b| b.readable())
    }
}

impl
    streams::OutputStream<
        crate::HostBindings,
        anyhow::Error,
        Resource<Stream>,
        Resource<AnyPollable>,
    > for HostState
{
    type T = Resource<Stream>;
    fn check_write(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
    ) -> HlResult<Result<u64, streams::StreamError<anyhow::Error>>> {
        let guard = self_.read().block_on();
        guard
            .check_write()
            .map_err(|_| streams::StreamError::Closed)
    }
    fn write(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
        contents: Vec<u8>,
    ) -> HlResult<Result<(), streams::StreamError<anyhow::Error>>> {
        let mut guard = self_.write().block_on();
        guard
            .write(&contents)
            .map_err(|_| streams::StreamError::Closed)
    }
    fn blocking_write_and_flush(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
        contents: Vec<u8>,
    ) -> HlResult<Result<(), streams::StreamError<anyhow::Error>>> {
        let mut guard = self_.write_wait_until(Stream::writable).block_on();
        if guard.write(&contents).is_err() {
            return Err(streams::StreamError::Closed);
        }
        if guard.flush().is_err() {
            return Err(streams::StreamError::Closed);
        }
        Ok(())
    }
    fn flush(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
    ) -> HlResult<Result<(), streams::StreamError<anyhow::Error>>> {
        let mut guard = self_.write().block_on();
        guard.flush().map_err(|_| streams::StreamError::Closed)
    }
    fn blocking_flush(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
    ) -> HlResult<Result<(), streams::StreamError<anyhow::Error>>> {
        let mut guard = self_.write().block_on();
        guard.flush().map_err(|_| streams::StreamError::Closed)
    }
    fn subscribe(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
    ) -> HlResult<Resource<AnyPollable>> {
        self_.poll(|b| b.writable())
    }
    fn write_zeroes(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
        len: u64,
    ) -> HlResult<Result<(), streams::StreamError<anyhow::Error>>> {
        let capped = len.min(MAX_ALLOC_BYTES) as usize;
        let mut guard = self_.write().block_on();
        guard
            .write(vec![0; capped])
            .map_err(|_| streams::StreamError::Closed)
    }
    fn blocking_write_zeroes_and_flush(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
        len: u64,
    ) -> HlResult<Result<(), streams::StreamError<anyhow::Error>>> {
        let capped = len.min(MAX_ALLOC_BYTES) as usize;
        let mut guard = self_.write().block_on();
        if guard.write(vec![0; capped]).is_err() {
            return Err(streams::StreamError::Closed);
        }
        if guard.flush().is_err() {
            return Err(streams::StreamError::Closed);
        }
        Ok(())
    }
    fn splice(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
        src: BorrowedResourceGuard<Resource<Stream>>,
        len: u64,
    ) -> HlResult<Result<u64, streams::StreamError<anyhow::Error>>> {
        let mut dst_guard = self_.write().block_on();
        let mut src_guard = src.write().block_on();
        dst_guard
            .splice(&mut src_guard, len as usize)
            .map(|n| n as u64)
            .map_err(|_| streams::StreamError::Closed)
    }
    fn blocking_splice(
        &mut self,
        self_: BorrowedResourceGuard<Resource<Stream>>,
        src: BorrowedResourceGuard<Resource<Stream>>,
        len: u64,
    ) -> HlResult<Result<u64, streams::StreamError<anyhow::Error>>> {
        let mut dst_guard = self_.write_wait_until(Stream::writable).block_on();
        let mut src_guard = src.write_wait_until(Stream::readable).block_on();
        dst_guard
            .splice(&mut src_guard, len as usize)
            .map(|n| n as u64)
            .map_err(|_| streams::StreamError::Closed)
    }
}

impl wasi::io::Streams<crate::HostBindings, anyhow::Error, Resource<AnyPollable>> for HostState {}
