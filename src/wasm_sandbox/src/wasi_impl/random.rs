//! Random trait implementations.

use crate::HostState;
use crate::bindings::wasi;

type HlResult<T> = T;

/// Maximum byte count for a single random-bytes allocation (16 MiB).
const MAX_ALLOC_BYTES: u64 = 16 * 1024 * 1024;

fn fill_random(buf: &mut [u8]) {
    if let Err(err) = getrandom::fill(buf) {
        log::error!("getrandom failed: {err}");
    }
}

fn random_u64() -> u64 {
    getrandom::u64().unwrap_or_else(|err| {
        log::error!("getrandom failed: {err}");
        0
    })
}

impl wasi::random::Random<crate::HostBindings> for HostState {
    fn get_random_bytes(&mut self, len: u64) -> HlResult<Vec<u8>> {
        let capped = len.min(MAX_ALLOC_BYTES) as usize;
        let mut buf = vec![0u8; capped];
        fill_random(&mut buf);
        buf
    }
    fn get_random_u64(&mut self) -> HlResult<u64> {
        random_u64()
    }
}

impl wasi::random::Insecure<crate::HostBindings> for HostState {
    fn get_insecure_random_bytes(&mut self, len: u64) -> HlResult<Vec<u8>> {
        let capped = len.min(MAX_ALLOC_BYTES) as usize;
        let mut buf = vec![0u8; capped];
        fill_random(&mut buf);
        buf
    }
    fn get_insecure_random_u64(&mut self) -> HlResult<u64> {
        random_u64()
    }
}

impl wasi::random::InsecureSeed<crate::HostBindings> for HostState {
    fn insecure_seed(&mut self) -> HlResult<(u64, u64)> {
        (random_u64(), random_u64())
    }
}
