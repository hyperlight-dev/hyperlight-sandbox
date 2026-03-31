//! Random trait implementations.

use hyperlight_host::HyperlightError;

use crate::HostState;
use crate::bindings::wasi;

type HlResult<T> = Result<T, HyperlightError>;

/// Maximum byte count for a single random-bytes allocation (16 MiB).
const MAX_ALLOC_BYTES: u64 = 16 * 1024 * 1024;

impl wasi::random::Random for HostState {
    fn get_random_bytes(&mut self, len: u64) -> HlResult<Vec<u8>> {
        let capped = len.min(MAX_ALLOC_BYTES) as usize;
        let mut buf = vec![0u8; capped];
        getrandom::fill(&mut buf).map_err(|e| anyhow::anyhow!("getrandom failed: {e}"))?;
        Ok(buf)
    }
    fn get_random_u64(&mut self) -> HlResult<u64> {
        getrandom::u64().map_err(|e| anyhow::anyhow!("getrandom failed: {e}").into())
    }
}

impl wasi::random::Insecure for HostState {
    fn get_insecure_random_bytes(&mut self, len: u64) -> HlResult<Vec<u8>> {
        let capped = len.min(MAX_ALLOC_BYTES) as usize;
        let mut buf = vec![0u8; capped];
        getrandom::fill(&mut buf).map_err(|e| anyhow::anyhow!("getrandom failed: {e}"))?;
        Ok(buf)
    }
    fn get_insecure_random_u64(&mut self) -> HlResult<u64> {
        getrandom::u64().map_err(|e| anyhow::anyhow!("getrandom failed: {e}").into())
    }
}

impl wasi::random::InsecureSeed for HostState {
    fn insecure_seed(&mut self) -> HlResult<(u64, u64)> {
        let a = getrandom::u64().map_err(|e| anyhow::anyhow!("getrandom failed: {e}"))?;
        let b = getrandom::u64().map_err(|e| anyhow::anyhow!("getrandom failed: {e}"))?;
        Ok((a, b))
    }
}
