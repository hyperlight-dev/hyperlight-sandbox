//! CLI trait implementations: Environment, Exit, Stdin/Stdout/Stderr, Terminals.
#![allow(unused_variables)]

use hyperlight_host::HyperlightError;

use crate::HostState;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;
use crate::wasi_impl::types::stream::Stream;

type HlResult<T> = Result<T, HyperlightError>;

// ---------------------------------------------------------------------------
// CLI: Environment, Exit, Stdin/Stdout/Stderr
// ---------------------------------------------------------------------------

impl wasi::cli::Environment for HostState {
    fn get_environment(&mut self) -> HlResult<Vec<(String, String)>> {
        Ok(Vec::new())
    }
    fn get_arguments(&mut self) -> HlResult<Vec<String>> {
        Ok(Vec::new())
    }
    fn initial_cwd(&mut self) -> HlResult<Option<String>> {
        Ok(None)
    }
}

impl wasi::cli::Exit for HostState {
    fn exit(&mut self, _status: Result<(), ()>) -> HlResult<()> {
        Ok(())
    }
}

impl wasi::cli::Stdin<Resource<Stream>> for HostState {
    fn get_stdin(&mut self) -> HlResult<Resource<Stream>> {
        Ok(Resource::new(Stream::new()))
    }
}

impl wasi::cli::Stdout<Resource<Stream>> for HostState {
    fn get_stdout(&mut self) -> HlResult<Resource<Stream>> {
        Ok(Resource::new(Stream::new()))
    }
}

impl wasi::cli::Stderr<Resource<Stream>> for HostState {
    fn get_stderr(&mut self) -> HlResult<Resource<Stream>> {
        Ok(Resource::new(Stream::new()))
    }
}

// ---------------------------------------------------------------------------
// CLI: Terminals (stubs — no terminal support)
// ---------------------------------------------------------------------------

impl wasi::cli::terminal_input::TerminalInput for HostState {
    type T = u32;
}
impl wasi::cli::TerminalInput for HostState {}

impl wasi::cli::terminal_output::TerminalOutput for HostState {
    type T = u32;
}
impl wasi::cli::TerminalOutput for HostState {}

impl wasi::cli::TerminalStdin<u32> for HostState {
    fn get_terminal_stdin(&mut self) -> HlResult<Option<u32>> {
        Ok(None)
    }
}

impl wasi::cli::TerminalStdout<u32> for HostState {
    fn get_terminal_stdout(&mut self) -> HlResult<Option<u32>> {
        Ok(None)
    }
}

impl wasi::cli::TerminalStderr<u32> for HostState {
    fn get_terminal_stderr(&mut self) -> HlResult<Option<u32>> {
        Ok(None)
    }
}
