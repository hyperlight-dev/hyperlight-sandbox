use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use hyperlight_sandbox::{ExecutionResult, HyperlightError};
use serde::{Deserialize, Serialize};

pub(crate) const VERSION: u32 = 3;
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
    pub max_args_size: usize,
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
    Io {
        kind: IoErrorKind,
        raw_os_error: Option<i32>,
    },
    Other,
}

macro_rules! io_error_kinds {
    ($($kind:ident),+ $(,)?) => {
        #[derive(Serialize, Deserialize)]
        pub(crate) enum IoErrorKind {
            $($kind,)+
            Other,
        }

        impl From<std::io::ErrorKind> for IoErrorKind {
            fn from(kind: std::io::ErrorKind) -> Self {
                match kind {
                    $(std::io::ErrorKind::$kind => Self::$kind,)+
                    _ => Self::Other,
                }
            }
        }

        impl From<IoErrorKind> for std::io::ErrorKind {
            fn from(kind: IoErrorKind) -> Self {
                match kind {
                    $(IoErrorKind::$kind => Self::$kind,)+
                    IoErrorKind::Other => Self::Other,
                }
            }
        }
    };
}

io_error_kinds! {
    NotFound, PermissionDenied, ConnectionRefused, ConnectionReset,
    HostUnreachable, NetworkUnreachable, ConnectionAborted, NotConnected,
    AddrInUse, AddrNotAvailable, NetworkDown, BrokenPipe, AlreadyExists,
    WouldBlock, NotADirectory, IsADirectory, DirectoryNotEmpty,
    ReadOnlyFilesystem, StaleNetworkFileHandle, InvalidInput, InvalidData,
    TimedOut, WriteZero, StorageFull, NotSeekable, QuotaExceeded, FileTooLarge,
    ResourceBusy, ExecutableFileBusy, Deadlock, CrossesDevices, TooManyLinks,
    InvalidFilename, ArgumentListTooLong, Interrupted, Unsupported,
    UnexpectedEof, OutOfMemory,
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
            _ => match error.downcast_ref::<std::io::Error>() {
                Some(io) => ErrorKind::Io {
                    kind: io.kind().into(),
                    raw_os_error: io.raw_os_error(),
                },
                None => ErrorKind::Other,
            },
        };
        let mut contexts: Vec<_> = error.chain().map(ToString::to_string).collect();
        let message = contexts.pop().expect("anyhow errors have a root cause");
        Self {
            kind,
            message,
            contexts,
        }
    }

    pub fn into_error(self) -> anyhow::Error {
        let mut error = match self.kind {
            ErrorKind::Cancelled => anyhow::Error::new(HyperlightError::ExecutionCanceledByHost()),
            ErrorKind::Poisoned => anyhow::Error::new(HyperlightError::PoisonedSandbox),
            ErrorKind::Io { kind, raw_os_error } => anyhow::Error::new(match raw_os_error {
                Some(code) => std::io::Error::from_raw_os_error(code),
                None => std::io::Error::new(kind.into(), self.message),
            }),
            ErrorKind::Other => anyhow::anyhow!(self.message),
        };
        for context in self.contexts.into_iter().rev() {
            error = error.context(context);
        }
        error
    }
}

pub(crate) fn write_frame(writer: &mut impl Write, message: &Message) -> Result<()> {
    write_frame_with_limit(writer, message, MAX_FRAME)
}

fn write_frame_with_limit(
    writer: &mut impl Write,
    message: &Message,
    maximum: usize,
) -> Result<()> {
    let mut bytes = postcard::to_stdvec(message).context("encode worker message")?;
    if bytes.len() > maximum
        && let Message::Response { id, .. } = message
    {
        bytes = postcard::to_stdvec(&Message::Response {
            id: *id,
            result: Err(RemoteError::from_error(anyhow::anyhow!(
                "worker message exceeds {maximum} bytes"
            ))),
        })
        .context("encode worker frame-size error")?;
    }
    ensure!(
        bytes.len() <= maximum,
        "worker message exceeds {maximum} bytes"
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
    fn remote_errors_preserve_typed_roots_and_context_once() {
        for root in [
            HyperlightError::ExecutionCanceledByHost(),
            HyperlightError::PoisonedSandbox,
        ] {
            let error = anyhow::Error::new(root).context("run guest");
            let expected = format!("{error:#}");
            let restored = RemoteError::from_error(error).into_error();
            assert!(restored.downcast_ref::<HyperlightError>().is_some());
            assert_eq!(format!("{restored:#}"), expected);
        }
    }

    #[test]
    fn remote_io_errors_preserve_classification_and_context() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "input path is not a directory",
        ))
        .context("open input directory")
        .context("create sandbox");
        let expected = format!("{error:#}");
        let restored = RemoteError::from_error(error).into_error();
        assert!(restored.downcast_ref::<std::io::Error>().is_some());
        assert_eq!(
            restored.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::NotADirectory
        );
        assert_eq!(format!("{restored:#}"), expected);
    }

    #[test]
    fn remote_io_errors_preserve_os_codes() {
        for code in [libc::ENOENT, libc::EACCES, libc::ENOTDIR] {
            let original = std::io::Error::from_raw_os_error(code);
            let kind = original.kind();
            let restored = RemoteError::from_error(
                anyhow::Error::new(original).context("open input directory"),
            )
            .into_error();
            let io = restored.downcast_ref::<std::io::Error>().unwrap();
            assert_eq!(io.kind(), kind);
            assert_eq!(io.raw_os_error(), Some(code));
        }
    }

    #[test]
    fn oversized_responses_report_the_request_error_without_closing_transport() {
        let mut frame = Vec::new();
        write_frame_with_limit(
            &mut frame,
            &Message::Response {
                id: 41,
                result: Ok(Response::Files(vec!["x".repeat(4096)])),
            },
            512,
        )
        .unwrap();
        write_frame_with_limit(
            &mut frame,
            &Message::Response {
                id: 42,
                result: Ok(Response::Ack),
            },
            512,
        )
        .unwrap();
        let mut reader = frame.as_slice();
        let Message::Response {
            id: 41,
            result: Err(error),
        } = read_frame(&mut reader).unwrap()
        else {
            panic!("expected an explicit frame-size error for the original request");
        };
        assert!(error.into_error().to_string().contains("exceeds 512 bytes"));
        assert!(matches!(
            read_frame(&mut reader).unwrap(),
            Message::Response {
                id: 42,
                result: Ok(Response::Ack)
            }
        ));
    }

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
