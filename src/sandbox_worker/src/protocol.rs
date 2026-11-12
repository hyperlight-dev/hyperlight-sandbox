use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use hyperlight_sandbox::{ExecutionResult, HyperlightError};
use serde::{Deserialize, Serialize};

pub(crate) const VERSION: u32 = 2;
pub(crate) const BUILD: &str = env!("SANDBOX_WORKER_BUILD");
const MAX_FRAME: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Backend {
    Wasm,
    JavaScript,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Options {
    pub backend: Backend,
    pub module_path: String,
    pub heap_size: u64,
    pub stack_size: u64,
    pub input_dir: Option<PathBuf>,
    pub output_dir: Option<(PathBuf, u8, u8)>,
    pub temp_output: bool,
    pub limits: Option<(u64, u64, usize)>,
    pub tools: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum Request {
    Hello {
        version: u32,
        build: String,
    },
    Evolve(Options),
    Call(String),
    Snapshot(PathBuf),
    Restore {
        backend: Backend,
        directory: PathBuf,
    },
    AllowDomain {
        target: String,
        methods: Option<Vec<String>>,
    },
    OutputFiles,
    OutputPath,
    Interrupt,
    Drop,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum Response {
    Ack,
    Execution(ExecutionResult),
    Files(Vec<String>),
    Path(Option<PathBuf>),
    Interrupted(bool),
}

#[derive(Serialize, Deserialize)]
pub(crate) enum Message {
    Request {
        id: u64,
        generation: u64,
        request: Request,
    },
    Response {
        id: u64,
        result: Result<Response, RemoteError>,
    },
    HostCall {
        id: u64,
        generation: u64,
        name: String,
        args: String,
    },
    HostReply {
        id: u64,
        result: Result<String, String>,
    },
}

#[derive(Serialize, Deserialize)]
pub(crate) enum ErrorKind {
    Cancelled,
    Poisoned,
    Io,
    Other,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct RemoteError {
    kind: ErrorKind,
    message: String,
    contexts: Vec<String>,
}

impl RemoteError {
    pub fn from_error(error: anyhow::Error) -> Self {
        let kind = match error.downcast_ref::<HyperlightError>() {
            Some(HyperlightError::ExecutionCanceledByHost()) => ErrorKind::Cancelled,
            Some(HyperlightError::PoisonedSandbox) => ErrorKind::Poisoned,
            _ if error.downcast_ref::<std::io::Error>().is_some() => ErrorKind::Io,
            _ => ErrorKind::Other,
        };
        let mut contexts: Vec<_> = error.chain().map(ToString::to_string).collect();
        let message = contexts.pop().expect("anyhow errors have a root cause");
        Self {
            kind,
            message,
            contexts,
        }
    }
}

pub(crate) fn write_frame(writer: &mut impl Write, message: &Message) -> Result<()> {
    let bytes = postcard::to_stdvec(message).context("encode worker message")?;
    ensure!(
        bytes.len() <= MAX_FRAME,
        "worker message exceeds {MAX_FRAME} bytes"
    );
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    Ok(())
}

pub(crate) fn read_frame(reader: &mut impl Read) -> Result<Message> {
    let mut length = [0; 4];
    reader
        .read_exact(&mut length)
        .context("sandbox worker connection closed")?;
    let length = u32::from_le_bytes(length) as usize;
    ensure!(
        length > 0 && length <= MAX_FRAME,
        "invalid worker frame length: {length}"
    );
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .context("incomplete worker frame")?;
    let (message, remaining) = postcard::take_from_bytes(&bytes).context("invalid worker frame")?;
    if !remaining.is_empty() {
        bail!("trailing bytes in worker frame");
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_reject_oversized_truncated_and_trailing_data() {
        assert!(read_frame(&mut u32::MAX.to_le_bytes().as_slice()).is_err());
        assert!(read_frame(&mut [3, 0, 0, 0, 1].as_slice()).is_err());
        let mut frame = Vec::new();
        write_frame(
            &mut frame,
            &Message::HostReply {
                id: 7,
                result: Ok("hello".into()),
            },
        )
        .unwrap();
        assert!(matches!(
            read_frame(&mut frame.as_slice()).unwrap(),
            Message::HostReply { id: 7, .. }
        ));
        let size = u32::from_le_bytes(frame[..4].try_into().unwrap());
        frame[..4].copy_from_slice(&(size + 1).to_le_bytes());
        frame.push(0);
        assert!(read_frame(&mut frame.as_slice()).is_err());
    }
}
