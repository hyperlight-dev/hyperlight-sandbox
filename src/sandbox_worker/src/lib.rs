//! Experimental, developer-only process worker, following HIP-0004.
//! This is an internal SDK interface, not a stable remote protocol.

#[cfg(unix)]
mod protocol;
#[cfg(unix)]
mod server;

/// Run the packaged worker. Only the private binary entry point calls this.
#[cfg(unix)]
#[doc(hidden)]
pub fn serve() -> anyhow::Result<()> {
    server::serve()
}
