//! WASI trait implementations for the Wasm sandbox HostState.
//!
//! Uses proper typed resources (Resource<Stream>, Resource<Headers>, etc.)
//! instead of raw u32 handles. HTTP is handled via wasmtime-wasi-http.

pub mod resource;
pub mod types;

mod body_stream;
mod cli;
mod clocks;
mod filesystem;
mod http;
mod http_handler;
mod io;
mod random;
mod sockets;
