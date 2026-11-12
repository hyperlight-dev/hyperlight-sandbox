//! Experimental, developer-only process worker, following HIP-0004.
//! This is an internal SDK interface, not a stable remote protocol.

#[cfg(unix)]
mod client;
#[cfg(unix)]
mod protocol;
#[cfg(unix)]
mod server;

#[cfg(unix)]
pub use client::{InterruptHandle, Pool, PoolConfig, Sandbox, SandboxBuilder, Snapshot};

/// Run the packaged worker. Only the private binary entry point calls this.
#[cfg(unix)]
#[doc(hidden)]
pub fn serve() -> anyhow::Result<()> {
    server::serve()
}
