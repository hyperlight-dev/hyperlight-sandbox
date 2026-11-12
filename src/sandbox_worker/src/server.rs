use std::collections::HashMap;
use std::net::Shutdown;
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use hyperlight_sandbox::{
    DirPerms, FilePerms, FilesystemLimits, HttpMethod, InterruptHandle, Sandbox, SandboxBuilder,
    Snapshot, ToolRegistry,
};
use hyperlight_sandbox_javascript::HyperlightJs;
use hyperlight_sandbox_wasm::Wasm;

use crate::protocol::{
    BUILD, Backend, Message, Options, RemoteError, Request, Response, VERSION, read_frame,
    write_frame,
};

type Writer = Arc<Mutex<UnixStream>>;
type CallbackReplies = Arc<Mutex<HashMap<u64, mpsc::Sender<Result<String, String>>>>>;

struct CloseConnectionOnDrop(Writer);

impl Drop for CloseConnectionOnDrop {
    fn drop(&mut self) {
        if let Ok(stream) = self.0.lock() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

enum Runtime {
    Wasm(Sandbox<Wasm>),
    Js(Sandbox<HyperlightJs>),
}

macro_rules! runtime {
    ($runtime:expr, $sandbox:ident => $body:expr) => {
        match $runtime {
            Runtime::Wasm($sandbox) => $body,
            Runtime::Js($sandbox) => $body,
        }
    };
}

fn build_runtime(options: Options, tools: ToolRegistry) -> Result<Runtime> {
    let mut builder = SandboxBuilder::new()
        .module_path(options.module_path)
        .heap_size(options.heap_size)
        .stack_size(options.stack_size)
        .with_tools(tools);
    builder = builder.filesystem_limits(match options.limits {
        Some((file, total, count)) => FilesystemLimits::new(file, total, count)?,
        None => FilesystemLimits::unlimited(),
    });
    if let Some(path) = options.input_dir {
        builder = builder.input_dir(path);
    }
    if let Some((path, dir, file)) = options.output_dir {
        builder = builder.output_dir(
            path,
            DirPerms::from_bits(dir).context("invalid directory permissions")?,
            FilePerms::from_bits(file).context("invalid file permissions")?,
        );
    } else if options.temp_output {
        builder = builder.temp_output();
    }
    match options.backend {
        Backend::Wasm => builder.guest(Wasm).build().map(Runtime::Wasm),
        Backend::JavaScript => builder.guest(HyperlightJs).build().map(Runtime::Js),
    }
}

fn send(writer: &Writer, message: &Message) -> Result<()> {
    write_frame(
        &mut *writer
            .lock()
            .map_err(|_| anyhow::anyhow!("worker writer poisoned"))?,
        message,
    )
}

fn snapshot_directory(root: &Path, path: &Path) -> Result<PathBuf> {
    ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "snapshot directory is a symlink"
    );
    let path = path.canonicalize()?;
    ensure!(
        path.parent() == Some(root),
        "snapshot directory is outside this host's store"
    );
    Ok(path)
}

fn execute(runtime: &mut Runtime, request: Request, root: &Path) -> Result<Response> {
    match request {
        Request::Call(code) => runtime!(runtime, sb => sb.run(&code)).map(Response::Execution),
        Request::Snapshot(path) => {
            let path = snapshot_directory(root, &path)?;
            let tag = "snapshot".parse()?;
            match runtime {
                Runtime::Wasm(sb) => {
                    sb.snapshot()?.snapshot().save(path, &tag)?;
                }
                Runtime::Js(sb) => {
                    sb.snapshot()?.snapshot().save(path, &tag)?;
                }
            }
            Ok(Response::Ack)
        }
        Request::Restore { backend, directory } => {
            let path = snapshot_directory(root, &directory)?;
            match (runtime, backend) {
                (Runtime::Wasm(sb), Backend::Wasm) => {
                    let snapshot = hyperlight_wasm::Snapshot::checked_load(
                        path,
                        "snapshot".parse::<hyperlight_wasm::OciTag>()?,
                    )?;
                    sb.restore(&Snapshot::new("wasm-component", Arc::new(snapshot)))?;
                }
                (Runtime::Js(sb), Backend::JavaScript) => {
                    let snapshot = hyperlight_js::Snapshot::checked_load(
                        path,
                        "snapshot".parse::<hyperlight_js::OciTag>()?,
                    )?;
                    sb.restore(&Snapshot::new("hyperlight-js", Arc::new(snapshot)))?;
                }
                _ => bail!("snapshot backend does not match sandbox"),
            }
            Ok(Response::Ack)
        }
        Request::AllowDomain { target, methods } => {
            let methods = HttpMethod::parse_list(methods)?;
            runtime!(runtime, sb => sb.allow_domain(&target, methods))?;
            Ok(Response::Ack)
        }
        Request::OutputFiles => runtime!(runtime, sb => sb.get_output_files()).map(Response::Files),
        Request::OutputPath => runtime!(runtime, sb => sb.output_path()).map(Response::Path),
        _ => bail!("unexpected sandbox operation"),
    }
}

fn finish_worker(
    worker: std::thread::JoinHandle<()>,
    mut interrupt: impl FnMut() -> Result<()>,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !worker.is_finished() {
        ensure!(
            Instant::now() < deadline,
            "sandbox worker did not stop after disconnect; terminating process"
        );
        interrupt()?;
        std::thread::sleep(Duration::from_millis(10));
    }
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("sandbox worker panicked"))
}

pub(crate) fn serve() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    ensure!(
        args.next().as_deref() == Some(std::ffi::OsStr::new("--sdk-worker")),
        "internal developer-only SDK worker; not a command-line runner"
    );
    let root = PathBuf::from(args.next().context("missing snapshot store")?).canonicalize()?;
    ensure!(args.next().is_none(), "unexpected host arguments");
    // The parent duplicates only this connection to descriptor 3 before exec.
    let mut reader = unsafe { UnixStream::from_raw_fd(3) };
    let writer = Arc::new(Mutex::new(reader.try_clone()?));
    let Message::Request {
        id,
        generation: 0,
        request: Request::Hello { version, build },
    } = read_frame(&mut reader)?
    else {
        bail!("missing sandbox worker handshake");
    };
    let handshake = if version == VERSION && build == BUILD {
        Ok(Response::Ack)
    } else {
        Err(RemoteError::from_error(anyhow::anyhow!(
            "sandbox worker version mismatch; rebuild or reinstall the SDK and its helper together"
        )))
    };
    let accepted = handshake.is_ok();
    send(
        &writer,
        &Message::Response {
            id,
            result: handshake,
        },
    )?;
    ensure!(accepted, "sandbox worker version mismatch");

    let interrupt: Arc<Mutex<Option<(u64, InterruptHandle)>>> = Arc::new(Mutex::new(None));
    let replies: CallbackReplies = Arc::new(Mutex::new(HashMap::new()));
    let stopping = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<(u64, u64, Request)>();
    let worker_stopping = stopping.clone();
    let worker_interrupt = interrupt.clone();
    let worker_writer = writer.clone();
    let worker_replies = replies.clone();
    let worker = std::thread::spawn(move || {
        let _close_on_exit = CloseConnectionOnDrop(worker_writer.clone());
        let callback_id = Arc::new(AtomicU64::new(1));
        let mut current: Option<(u64, Runtime)> = None;
        for (id, generation, request) in rx {
            if worker_stopping.load(Ordering::Acquire) {
                break;
            }
            let result = (|| -> Result<Response> {
                match request {
                    Request::Evolve(options) => {
                        ensure!(
                            current.is_none() && generation != 0,
                            "worker already owns a sandbox"
                        );
                        let mut tools = ToolRegistry::new();
                        tools.set_max_args_size(options.max_args_size);
                        for name in &options.tools {
                            let name = name.clone();
                            let writer = worker_writer.clone();
                            let replies = worker_replies.clone();
                            let callback_id = callback_id.clone();
                            tools.register_unvalidated(&name.clone(), move |args| {
                                let id = callback_id.fetch_add(1, Ordering::Relaxed);
                                let (tx, rx) = mpsc::channel();
                                replies
                                    .lock()
                                    .map_err(|_| anyhow::anyhow!("callback map poisoned"))?
                                    .insert(id, tx);
                                if let Err(error) = send(
                                    &writer,
                                    &Message::HostCall {
                                        id,
                                        generation,
                                        name: name.clone(),
                                        args: serde_json::to_string(&args)?,
                                    },
                                ) {
                                    replies
                                        .lock()
                                        .map_err(|_| anyhow::anyhow!("callback map poisoned"))?
                                        .remove(&id);
                                    return Err(error);
                                }
                                let value = rx
                                    .recv()
                                    .context("application disconnected during callback")?
                                    .map_err(anyhow::Error::msg)?;
                                serde_json::from_str(&value).context("invalid callback result")
                            });
                        }
                        let sandbox = build_runtime(options, tools)?;
                        let handle = runtime!(&sandbox, sb => sb.interrupt_handle())?;
                        *worker_interrupt
                            .lock()
                            .map_err(|_| anyhow::anyhow!("interrupt state poisoned"))? =
                            Some((generation, handle));
                        current = Some((generation, sandbox));
                        Ok(Response::Ack)
                    }
                    Request::Drop => {
                        ensure!(
                            current
                                .as_ref()
                                .is_some_and(|(active, _)| *active == generation),
                            "stale sandbox handle"
                        );
                        *worker_interrupt
                            .lock()
                            .map_err(|_| anyhow::anyhow!("interrupt state poisoned"))? = None;
                        current = None;
                        Ok(Response::Ack)
                    }
                    request => {
                        let (active, runtime) =
                            current.as_mut().context("sandbox not initialized")?;
                        ensure!(*active == generation, "stale sandbox handle");
                        execute(runtime, request, &root)
                    }
                }
            })()
            .map_err(RemoteError::from_error);
            if let Err(error) = send(&worker_writer, &Message::Response { id, result }) {
                log::error!("sandbox worker response failed: {error:#}");
                let _ = worker_writer
                    .lock()
                    .map(|stream| stream.shutdown(Shutdown::Both));
                break;
            }
        }
    });

    let result = (|| -> Result<()> {
        loop {
            match read_frame(&mut reader)? {
                Message::Request {
                    id,
                    generation,
                    request: Request::Interrupt,
                } => {
                    let handle = interrupt
                        .lock()
                        .map_err(|_| anyhow::anyhow!("interrupt state poisoned"))?
                        .as_ref()
                        .filter(|(active, _)| *active == generation)
                        .map(|(_, handle)| handle.clone());
                    let result = handle
                        .map(|handle| Response::Interrupted(handle.kill()))
                        .context("stale interrupt handle")
                        .map_err(RemoteError::from_error);
                    send(&writer, &Message::Response { id, result })?;
                }
                Message::Request {
                    id,
                    generation,
                    request,
                } => {
                    tx.send((id, generation, request))
                        .context("sandbox worker stopped")?;
                }
                Message::HostReply { id, result } => {
                    if let Some(reply) = replies
                        .lock()
                        .map_err(|_| anyhow::anyhow!("callback map poisoned"))?
                        .remove(&id)
                    {
                        let _ = reply.send(result);
                    } else {
                        bail!("unexpected callback reply");
                    }
                }
                _ => bail!("unexpected application message"),
            }
        }
    })();
    stopping.store(true, Ordering::Release);
    if let Err(error) = reader.shutdown(Shutdown::Both) {
        log::debug!("sandbox worker disconnect shutdown failed: {error}");
    }
    replies
        .lock()
        .map_err(|_| anyhow::anyhow!("callback map poisoned"))?
        .clear();
    drop(tx);
    finish_worker(worker, || {
        let handle = interrupt
            .lock()
            .map_err(|_| anyhow::anyhow!("interrupt state poisoned"))?
            .as_ref()
            .map(|(_, handle)| handle.clone());
        if let Some(handle) = handle {
            handle.kill();
        }
        Ok(())
    })?;
    // Normal parent shutdown closes the inherited connection.
    if let Err(error) = result
        && !error.chain().any(|cause| {
            cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
                matches!(
                    io.kind(),
                    std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                )
            })
        })
    {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_retries_interrupts_that_miss_guest_entry() {
        let (release, waiting) = mpsc::channel();
        let worker = std::thread::spawn(move || waiting.recv().unwrap());
        let retry = release.clone();
        let (finished, completed) = mpsc::channel();
        let shutdown = std::thread::spawn(move || {
            let mut attempts = 0;
            let result = finish_worker(worker, || {
                attempts += 1;
                if attempts == 2 {
                    retry.send(())?;
                }
                Ok(())
            });
            finished.send(()).unwrap();
            result
        });
        let stopped = completed.recv_timeout(Duration::from_secs(1)).is_ok();
        if !stopped {
            release.send(()).unwrap();
        }
        shutdown.join().unwrap().unwrap();
        assert!(stopped, "shutdown did not retry the missed interrupt");
    }
}
