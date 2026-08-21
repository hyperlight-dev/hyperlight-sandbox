//! CLI trait implementations: Environment, Exit, Stdin/Stdout/Stderr, Terminals.
#![allow(unused_variables)]

use crate::HostState;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;
use crate::wasi_impl::types::stream::Stream;

type HlResult<T> = T;

// ---------------------------------------------------------------------------
// CLI: Environment, Exit, Stdin/Stdout/Stderr
// ---------------------------------------------------------------------------

impl wasi::cli::Environment<crate::HostBindings> for HostState {
    fn get_environment(&mut self) -> HlResult<Vec<(String, String)>> {
        Vec::new()
    }
    fn get_arguments(&mut self) -> HlResult<Vec<String>> {
        Vec::new()
    }
    fn initial_cwd(&mut self) -> HlResult<Option<String>> {
        None
    }
}

impl wasi::cli::Exit<crate::HostBindings> for HostState {
    fn exit(&mut self, _status: Result<(), ()>) -> HlResult<()> {}
}

impl wasi::cli::Stdin<crate::HostBindings, Resource<Stream>> for HostState {
    fn get_stdin(&mut self) -> HlResult<Resource<Stream>> {
        Resource::new(Stream::new())
    }
}

impl wasi::cli::Stdout<crate::HostBindings, Resource<Stream>> for HostState {
    fn get_stdout(&mut self) -> HlResult<Resource<Stream>> {
        Resource::new(Stream::new())
    }
}

impl wasi::cli::Stderr<crate::HostBindings, Resource<Stream>> for HostState {
    fn get_stderr(&mut self) -> HlResult<Resource<Stream>> {
        Resource::new(Stream::new())
    }
}

// ---------------------------------------------------------------------------
// CLI: Terminals (stubs — no terminal support)
// ---------------------------------------------------------------------------

impl wasi::cli::terminal_input::TerminalInput<crate::HostBindings> for HostState {
    type T = u32;
}
impl wasi::cli::TerminalInput<crate::HostBindings> for HostState {}

impl wasi::cli::terminal_output::TerminalOutput<crate::HostBindings> for HostState {
    type T = u32;
}
impl wasi::cli::TerminalOutput<crate::HostBindings> for HostState {}

impl wasi::cli::TerminalStdin<crate::HostBindings, u32> for HostState {
    fn get_terminal_stdin(&mut self) -> HlResult<Option<u32>> {
        None
    }
}

impl wasi::cli::TerminalStdout<crate::HostBindings, u32> for HostState {
    fn get_terminal_stdout(&mut self) -> HlResult<Option<u32>> {
        None
    }
}

impl wasi::cli::TerminalStderr<crate::HostBindings, u32> for HostState {
    fn get_terminal_stderr(&mut self) -> HlResult<Option<u32>> {
        None
    }
}
