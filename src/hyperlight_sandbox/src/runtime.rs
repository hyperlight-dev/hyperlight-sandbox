//! Shared tokio runtime and async bridging for all sandbox backends.

use std::future::Future;
use std::sync::LazyLock;

use tokio::runtime::Runtime;

/// Process-global tokio runtime shared by all sandbox backends.
///
/// Used by [`BlockOn`] to bridge sync host-function callbacks to async
/// operations (HTTP requests, WASI resource I/O, etc.).
pub static RUNTIME: LazyLock<Result<Runtime, String>> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .map_err(|e| format!("failed to create sandbox tokio runtime: {e}"))
});

/// Returns a reference to the shared runtime, or panics with a clear message.
///
/// # Panics
///
/// Panics if the runtime failed to initialize.  Both `JsGuestSandbox` and
/// `WasmComponentSandbox` validate [`RUNTIME`] in their constructors, so
/// this should never fire when called through [`BlockOn`].
pub fn runtime() -> &'static Runtime {
    RUNTIME
        .as_ref()
        .expect("sandbox tokio runtime failed to initialize")
}

/// Extension trait for running futures on the shared sandbox runtime.
pub trait BlockOn: Future {
    /// Block the current thread until this future completes.
    fn block_on(self) -> Self::Output;

    /// Spawn this future on the shared runtime.
    fn spawn(self) -> tokio::task::JoinHandle<Self::Output>
    where
        Self: Sized + Send + 'static,
        Self::Output: Send + 'static;
}

impl<F: Future> BlockOn for F {
    fn block_on(self) -> Self::Output {
        runtime().block_on(self)
    }

    fn spawn(self) -> tokio::task::JoinHandle<Self::Output>
    where
        Self: Sized + Send + 'static,
        Self::Output: Send + 'static,
    {
        runtime().spawn(self)
    }
}
