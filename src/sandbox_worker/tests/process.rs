#![cfg(unix)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use hyperlight_sandbox::{HyperlightError, ToolRegistry};
use hyperlight_sandbox_javascript::HyperlightJs;
use hyperlight_sandbox_wasm::Wasm;
use hyperlight_sandbox_worker::{Pool, PoolConfig, SandboxBuilder};

#[path = "../src/protocol.rs"]
mod protocol;

fn worker_path() -> std::path::PathBuf {
    std::env::var_os("HYPERLIGHT_SANDBOX_WORKER_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_hyperlight-sandbox-worker").into())
}

fn pool(maximum: usize) -> Arc<Pool> {
    Pool::new(
        worker_path(),
        PoolConfig {
            initial: 1,
            maximum,
        },
    )
    .unwrap()
}

#[test]
fn callbacks_run_on_the_execution_thread() {
    let caller = std::thread::current().id();
    let mut tools = ToolRegistry::new();
    tools.register_unvalidated("caller", move |_| {
        Ok(serde_json::json!(std::thread::current().id() == caller))
    });
    let mut sandbox = SandboxBuilder::new()
        .pool(pool(1))
        .with_tools(tools)
        .guest(HyperlightJs)
        .build()
        .unwrap();
    assert_eq!(
        sandbox
            .run("console.log(call_tool('caller', {}));")
            .unwrap()
            .stdout
            .trim(),
        "true"
    );
}

#[test]
fn invalid_input_directory_preserves_io_error() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("not-a-directory");
    std::fs::write(&input, b"file").unwrap();
    let error = SandboxBuilder::new()
        .pool(pool(1))
        .input_dir(input)
        .guest(HyperlightJs)
        .build()
        .err()
        .expect("a regular file must not be accepted as an input directory");
    assert!(
        error.downcast_ref::<std::io::Error>().is_some(),
        "I/O error type was lost across the process boundary: {error:#}"
    );
}

fn run_in_isolated_process(name: &str) -> bool {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    if std::env::var("HYPERLIGHT_WORKER_PROCESS_TEST").as_deref() == Ok(name) {
        return false;
    }
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("HYPERLIGHT_WORKER_PROCESS_TEST", name)
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "isolated {name} failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[test]
#[cfg(target_os = "macos")]
fn staging_helper_does_not_buffer_the_entire_executable() {
    if run_in_isolated_process("staging_helper_does_not_buffer_the_entire_executable") {
        return;
    }
    let peak_resident_bytes = || {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        assert_eq!(
            unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
            0
        );
        unsafe { usage.assume_init() }.ru_maxrss
    };
    let before = peak_resident_bytes();
    let _pool = Pool::new(
        worker_path(),
        PoolConfig {
            initial: 0,
            maximum: 1,
        },
    )
    .unwrap();
    let growth = peak_resident_bytes().saturating_sub(before);
    assert!(
        growth < 32 * 1024 * 1024,
        "staging the helper used {growth} additional resident bytes"
    );
}

#[test]
fn worker_starts_with_closed_standard_streams() {
    if run_in_isolated_process("worker_starts_with_closed_standard_streams") {
        return;
    }
    let stdin = unsafe { libc::fcntl(0, libc::F_DUPFD_CLOEXEC, 3) };
    let stdout = unsafe { libc::fcntl(1, libc::F_DUPFD_CLOEXEC, 3) };
    assert!(stdin >= 3 && stdout >= 3);
    unsafe {
        libc::close(0);
        libc::close(1);
    }
    let result = Pool::new(
        worker_path(),
        PoolConfig {
            initial: 1,
            maximum: 1,
        },
    );
    unsafe {
        assert!(libc::dup2(stdin, 0) >= 0);
        assert!(libc::dup2(stdout, 1) >= 0);
        libc::close(stdin);
        libc::close(stdout);
    }
    result.unwrap();
}

#[test]
fn application_sigint_does_not_kill_workers() {
    if run_in_isolated_process("application_sigint_does_not_kill_workers") {
        return;
    }
    extern "C" fn handle_interrupt(_: libc::c_int) {}
    unsafe {
        assert_ne!(
            libc::signal(
                libc::SIGINT,
                handle_interrupt as *const () as libc::sighandler_t
            ),
            libc::SIG_ERR
        );
    }
    let mut sandbox = SandboxBuilder::new()
        .pool(pool(1))
        .guest(HyperlightJs)
        .build()
        .unwrap();
    assert_eq!(unsafe { libc::kill(0, libc::SIGINT) }, 0);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        sandbox.run("console.log('alive');").unwrap().stdout.trim(),
        "alive"
    );
}

#[test]
fn disconnect_stops_guest_and_worker() {
    assert_worker_stops_after_disconnect(false, false);
}

#[test]
fn disconnect_after_guest_started_stops_worker() {
    assert_worker_stops_after_disconnect(true, false);
}

#[test]
fn disconnect_stops_worker_blocked_in_native_file_read() {
    assert_worker_stops_after_disconnect(true, true);
}

fn assert_worker_stops_after_disconnect(wait_for_guest: bool, blocked_read: bool) {
    use std::net::Shutdown;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    use protocol::{BUILD, Message, Options, Request, Response, VERSION, read_frame, write_frame};

    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    std::fs::create_dir(&output).unwrap();
    let started = output.join("started");
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    let fifo = input.join("blocked");
    if blocked_read {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    }
    let (mut stream, child_socket) = UnixStream::pair().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let fd = child_socket.as_raw_fd();
    let mut command = Command::new(worker_path());
    command
        .arg("--sdk-worker")
        .arg(root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(fd, 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    drop(child_socket);
    let mut _fifo_writer = None;
    let result = (|| -> anyhow::Result<()> {
        write_frame(
            &mut stream,
            &Message::Request {
                id: 1,
                generation: 0,
                request: Request::Hello {
                    version: VERSION,
                    build: BUILD.into(),
                },
            },
        )?;
        anyhow::ensure!(
            matches!(
                read_frame(&mut stream)?,
                Message::Response {
                    id: 1,
                    result: Ok(Response::Ack)
                }
            ),
            "worker handshake failed"
        );
        let config = hyperlight_sandbox::SandboxConfig::default();
        write_frame(
            &mut stream,
            &Message::Request {
                id: 2,
                generation: 1,
                request: Request::Evolve(Options {
                    backend: protocol::Backend::JavaScript,
                    module_path: config.module_path,
                    heap_size: config.heap_size,
                    stack_size: config.stack_size,
                    input_dir: Some(input),
                    output_dir: Some((
                        output,
                        (hyperlight_sandbox::DirPerms::READ | hyperlight_sandbox::DirPerms::MUTATE)
                            .bits(),
                        (hyperlight_sandbox::FilePerms::READ
                            | hyperlight_sandbox::FilePerms::WRITE)
                            .bits(),
                    )),
                    temp_output: false,
                    limits: None,
                    tools: Vec::new(),
                    max_args_size: ToolRegistry::new().max_args_size(),
                }),
            },
        )?;
        anyhow::ensure!(
            matches!(
                read_frame(&mut stream)?,
                Message::Response {
                    id: 2,
                    result: Ok(Response::Ack)
                }
            ),
            "worker initialization failed"
        );
        write_frame(
            &mut stream,
            &Message::Request {
                id: 3,
                generation: 1,
                request: Request::Call(
                    if blocked_read {
                        "write_file('/output/started', 'ready'); read_file('/input/blocked');"
                    } else if wait_for_guest {
                        "write_file('/output/started', 'ready'); while (true) {}"
                    } else {
                        "while (true) {}"
                    }
                    .into(),
                ),
            },
        )?;
        if wait_for_guest {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !started.try_exists()? {
                anyhow::ensure!(
                    Instant::now() < deadline,
                    "guest did not write its start marker"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            if blocked_read {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    match std::fs::OpenOptions::new()
                        .write(true)
                        .custom_flags(libc::O_NONBLOCK)
                        .open(&fifo)
                    {
                        Ok(writer) => {
                            _fifo_writer = Some(writer);
                            break;
                        }
                        Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                            anyhow::ensure!(
                                Instant::now() < deadline,
                                "guest did not open the FIFO"
                            );
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
        stream.shutdown(Shutdown::Both)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait()? {
                anyhow::ensure!(
                    blocked_read || status.success(),
                    "worker shutdown failed: {status}"
                );
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        anyhow::bail!("worker kept running guest code after disconnect");
    })();
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    child.wait().unwrap();
    result.unwrap();
}

fn wasm(pool: Arc<Pool>) -> SandboxBuilder<Wasm> {
    SandboxBuilder::new()
        .module_path(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../wasm_sandbox/guests/python/python-sandbox.aot"
        ))
        .pool(pool)
        .guest(Wasm)
}

#[test]
fn javascript_callbacks_snapshots_and_pool_reuse() {
    let pool = pool(2);
    let mut tools = ToolRegistry::new();
    tools.register_unvalidated("echo", Ok);
    let mut sandbox = SandboxBuilder::new()
        .pool(pool.clone())
        .with_tools(tools)
        .temp_output()
        .guest(HyperlightJs)
        .build()
        .unwrap();
    let stale = sandbox.interrupt_handle().unwrap();
    let result = sandbox
        .run("console.log(call_tool('echo', {value: 42}).value); globalThis.saved = 7;")
        .unwrap();
    assert_eq!(result.stdout.trim(), "42");
    let snapshot = sandbox.snapshot().unwrap();
    sandbox
        .run("globalThis.saved = 99; write_file('/output/file.txt', 'data');")
        .unwrap();
    assert_eq!(sandbox.get_output_files().unwrap(), vec!["file.txt"]);
    sandbox.restore(&snapshot).unwrap();
    assert!(sandbox.get_output_files().unwrap().is_empty());
    assert_eq!(
        sandbox
            .run("console.log(globalThis.saved);")
            .unwrap()
            .stdout
            .trim(),
        "7"
    );
    drop(sandbox);
    let mut tools = ToolRegistry::new();
    tools.register_unvalidated("echo", Ok);
    let mut other = SandboxBuilder::new()
        .pool(pool.clone())
        .with_tools(tools)
        .guest(HyperlightJs)
        .build()
        .unwrap();
    assert!(!stale.kill());
    other.restore(&snapshot).unwrap();
    drop(snapshot);
    assert_eq!(
        other
            .run("console.log(globalThis.saved);")
            .unwrap()
            .stdout
            .trim(),
        "7"
    );
}

#[test]
fn wasm_state_and_file_snapshot_transfer() {
    let pool = pool(2);
    let mut sandbox = wasm(pool.clone()).build().unwrap();
    assert_eq!(
        sandbox
            .run("value = 7\nprint(value)")
            .unwrap()
            .stdout
            .trim(),
        "7"
    );
    let snapshot = sandbox.snapshot().unwrap();
    sandbox.run("value = 99").unwrap();
    sandbox.restore(&snapshot).unwrap();
    assert_eq!(sandbox.run("print(value)").unwrap().stdout.trim(), "7");
    let mut other = wasm(pool).build().unwrap();
    other.restore(&snapshot).unwrap();
    drop(snapshot);
    drop(sandbox);
    assert_eq!(other.run("print(value)").unwrap().stdout.trim(), "7");
}

#[test]
fn cancellation_is_typed_and_snapshot_restore_allows_reuse() {
    let pool = pool(1);
    let mut sandbox = SandboxBuilder::new()
        .pool(pool)
        .guest(HyperlightJs)
        .build()
        .unwrap();
    let snapshot = sandbox.snapshot().unwrap();
    let interrupt = sandbox.interrupt_handle().unwrap();
    let worker = std::thread::spawn(move || {
        let error = sandbox.run("while (true) {}").unwrap_err();
        assert!(matches!(
            error.downcast_ref::<HyperlightError>(),
            Some(HyperlightError::ExecutionCanceledByHost())
        ));
        let cancelled = HyperlightError::ExecutionCanceledByHost().to_string();
        assert_eq!(format!("{error:#}").matches(&cancelled).count(), 1);
        let error = sandbox.run("console.log('poisoned');").unwrap_err();
        assert!(matches!(
            error.downcast_ref::<HyperlightError>(),
            Some(HyperlightError::PoisonedSandbox)
        ));
        let poisoned = HyperlightError::PoisonedSandbox.to_string();
        assert_eq!(format!("{error:#}").matches(&poisoned).count(), 1);
        sandbox.restore(&snapshot).unwrap();
        assert_eq!(
            sandbox
                .run("console.log('restored');")
                .unwrap()
                .stdout
                .trim(),
            "restored"
        );
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while !worker.is_finished() && Instant::now() < deadline {
        if interrupt.kill() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    while !worker.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(worker.is_finished(), "interrupted operation did not finish");
    worker.join().unwrap();
}

#[test]
fn bounded_pool_fails_fast_and_reuses_released_process() {
    let pool = pool(1);
    let sandbox = SandboxBuilder::new()
        .pool(pool.clone())
        .guest(HyperlightJs)
        .build()
        .unwrap();
    let result = SandboxBuilder::new()
        .pool(pool.clone())
        .guest(HyperlightJs)
        .build();
    assert!(result.err().unwrap().to_string().contains("pool exhausted"));
    drop(sandbox);
    let mut sandbox = SandboxBuilder::new()
        .pool(pool)
        .guest(HyperlightJs)
        .build()
        .unwrap();
    assert_eq!(
        sandbox.run("console.log('reused');").unwrap().stdout.trim(),
        "reused"
    );
}

#[test]
fn interruption_remains_responsive_during_application_callback() {
    let pool = pool(1);
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    let proceed = std::sync::Mutex::new(proceed);
    let mut tools = ToolRegistry::new();
    tools.register_unvalidated("wait", move |_| {
        entered.send(())?;
        proceed
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(30))?;
        Ok(serde_json::Value::Null)
    });
    let mut sandbox = SandboxBuilder::new()
        .pool(pool)
        .with_tools(tools)
        .guest(HyperlightJs)
        .build()
        .unwrap();
    let interrupt = sandbox.interrupt_handle().unwrap();
    let worker = std::thread::spawn(move || sandbox.run("call_tool('wait', {}); while (true) {}"));
    waiting.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(
        !interrupt.kill(),
        "the guest is blocked in its host callback"
    );
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !worker.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        worker.is_finished(),
        "callback completion did not deliver cancellation"
    );
    let error = worker.join().unwrap().unwrap_err();
    assert!(matches!(
        error.downcast_ref::<HyperlightError>(),
        Some(HyperlightError::ExecutionCanceledByHost())
    ));
}
