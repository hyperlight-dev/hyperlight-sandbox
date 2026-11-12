use std::collections::HashMap;
use std::ffi::CStr;
use std::marker::PhantomData;
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use hyperlight_sandbox::{
    DirPerms, ExecutionResult, FilePerms, FilesystemLimits, MethodFilter, SandboxConfig,
    ToolRegistry,
};
use hyperlight_sandbox_javascript::HyperlightJs;
use hyperlight_sandbox_wasm::Wasm;

use crate::protocol::{
    BUILD, Backend, Message, Options, Request, Response, VERSION, read_frame, write_frame,
};

enum RequestEvent {
    Response(Result<Response>),
    HostCall {
        id: u64,
        name: String,
        args: String,
        tools: Arc<ToolRegistry>,
    },
}

struct PendingRequest {
    generation: u64,
    accepts_callbacks: bool,
    events: mpsc::Sender<RequestEvent>,
}

type Pending = HashMap<u64, PendingRequest>;
type ActiveTools = Option<(u64, Arc<ToolRegistry>)>;

struct Connection {
    writer: Mutex<UnixStream>,
    child: Mutex<Child>,
    pending: Mutex<Pending>,
    tools: Mutex<ActiveTools>,
    next_id: AtomicU64,
    healthy: AtomicBool,
}

impl Connection {
    fn spawn(path: &Path, root: &Path) -> Result<Arc<Self>> {
        let (reader, child_socket) = UnixStream::pair()?;
        let fd = unsafe { libc::fcntl(child_socket.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("duplicate worker socket");
        }
        let worker_socket = unsafe { OwnedFd::from_raw_fd(fd) };
        drop(child_socket);
        let mut command = Command::new(path);
        command
            .arg("--sdk-worker")
            .arg(root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .process_group(0);
        // Only async-signal-safe descriptor operations run between fork and exec.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(fd, 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().with_context(|| {
            format!(
                "start developer sandbox worker at {}; rebuild or reinstall the SDK helper",
                path.display()
            )
        })?;
        drop(worker_socket);
        let connection = Arc::new(Self {
            writer: Mutex::new(reader.try_clone()?),
            child: Mutex::new(child),
            pending: Mutex::new(HashMap::new()),
            tools: Mutex::new(None),
            next_id: AtomicU64::new(1),
            healthy: AtomicBool::new(true),
        });
        let weak = Arc::downgrade(&connection);
        std::thread::Builder::new()
            .name("sandbox-worker-reader".into())
            .spawn(move || {
                Self::read_messages(reader, weak);
            })?;
        let (id, reply) = connection.start_request(
            0,
            Request::Hello {
                version: VERSION,
                build: BUILD.into(),
            },
        )?;
        match reply.recv_timeout(Duration::from_secs(30)) {
            Ok(RequestEvent::Response(result)) => expect_ack(result?)?,
            Ok(RequestEvent::HostCall { .. }) => {
                bail!("unexpected sandbox worker handshake callback");
            }
            Err(error) => {
                connection.fail(&format!("sandbox worker handshake failed: {error}"));
                connection
                    .pending
                    .lock()
                    .map_err(|_| anyhow::anyhow!("pending requests poisoned"))?
                    .remove(&id);
                bail!("sandbox worker handshake failed: {error}");
            }
        }
        Ok(connection)
    }

    fn send(&self, message: &Message) -> Result<()> {
        ensure!(
            self.healthy.load(Ordering::Acquire),
            "sandbox worker exited or its connection failed"
        );
        let result = write_frame(
            &mut *self
                .writer
                .lock()
                .map_err(|_| anyhow::anyhow!("worker writer poisoned"))?,
            message,
        );
        if let Err(error) = &result {
            self.fail(&format!("sandbox worker send failed: {error:#}"));
        }
        result
    }

    fn start_request(
        &self,
        generation: u64,
        request: Request,
    ) -> Result<(u64, mpsc::Receiver<RequestEvent>)> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("pending requests poisoned"))?
            .insert(
                id,
                PendingRequest {
                    generation,
                    accepts_callbacks: matches!(&request, Request::Call(_) | Request::Evolve(_)),
                    events: tx,
                },
            );
        if let Err(error) = self.send(&Message::Request {
            id,
            generation,
            request,
        }) {
            self.pending
                .lock()
                .map_err(|_| anyhow::anyhow!("pending requests poisoned"))?
                .remove(&id);
            return Err(error);
        }
        Ok((id, rx))
    }

    fn request(&self, generation: u64, request: Request) -> Result<Response> {
        let (_, reply) = self.start_request(generation, request)?;
        loop {
            match reply
                .recv()
                .context("sandbox worker stopped before replying")?
            {
                RequestEvent::Response(result) => return result,
                RequestEvent::HostCall {
                    id,
                    name,
                    args,
                    tools,
                } => {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                        || -> Result<String> {
                            let args = serde_json::from_str(&args)?;
                            let result = tools.dispatch(&name, args)?;
                            Ok(serde_json::to_string(&result)?)
                        },
                    ))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("application tool callback panicked")));
                    if let Err(error) = &result {
                        log::debug!("sandbox tool callback failed: {error:#}");
                    }
                    self.send(&Message::HostReply {
                        id,
                        result: result.map_err(|error| format!("{error:#}")),
                    })?;
                }
            }
        }
    }

    fn fail(&self, message: &str) {
        self.healthy.store(false, Ordering::Release);
        if let Ok(mut pending) = self.pending.lock() {
            for (_, reply) in pending.drain() {
                let _ = reply
                    .events
                    .send(RequestEvent::Response(Err(anyhow::anyhow!("{message}"))));
            }
        }
    }

    fn terminate(&self) {
        if let Ok(writer) = self.writer.lock() {
            let _ = writer.shutdown(Shutdown::Both);
        }
        if let Ok(mut child) = self.child.lock() {
            match child.try_wait() {
                Ok(Some(_)) => {}
                _ => {
                    if let Err(error) = child.kill() {
                        log::debug!("sandbox worker termination: {error}");
                    }
                    if let Err(error) = child.wait() {
                        log::error!("sandbox worker reap failed: {error}");
                    }
                }
            }
        }
    }

    fn read_messages(mut reader: UnixStream, weak: Weak<Self>) {
        loop {
            let message = read_frame(&mut reader);
            let Some(connection) = weak.upgrade() else {
                return;
            };
            let result = (|| -> Result<()> {
                match message? {
                    Message::Response { id, result } => {
                        let reply = connection
                            .pending
                            .lock()
                            .map_err(|_| anyhow::anyhow!("pending requests poisoned"))?
                            .remove(&id)
                            .context("unexpected worker response")?;
                        let _ = reply.events.send(RequestEvent::Response(
                            result.map_err(|error| error.into_error()),
                        ));
                    }
                    Message::HostCall {
                        id,
                        generation,
                        name,
                        args,
                    } => {
                        let tools = connection
                            .tools
                            .lock()
                            .map_err(|_| anyhow::anyhow!("callback registry poisoned"))?
                            .as_ref()
                            .filter(|(active, _)| *active == generation)
                            .map(|(_, tools)| tools.clone())
                            .context("stale host callback")?;
                        connection
                            .pending
                            .lock()
                            .map_err(|_| anyhow::anyhow!("pending requests poisoned"))?
                            .values()
                            .find(|request| {
                                request.generation == generation && request.accepts_callbacks
                            })
                            .context("host callback without an active request")?
                            .events
                            .send(RequestEvent::HostCall {
                                id,
                                name,
                                args,
                                tools,
                            })
                            .context("sandbox request stopped during callback")?;
                    }
                    _ => bail!("unexpected sandbox worker message"),
                }
                Ok(())
            })();
            if let Err(error) = result {
                connection.fail(&format!("sandbox worker connection failed: {error:#}"));
                let _ = reader.shutdown(Shutdown::Both);
                connection.terminate();
                return;
            }
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// Bounded SDK worker pool settings.
#[derive(Clone, Copy)]
pub struct PoolConfig {
    pub initial: usize,
    pub maximum: usize,
}

impl PoolConfig {
    fn from_env() -> Result<Self> {
        fn number(name: &str, default: usize) -> Result<usize> {
            match std::env::var(name) {
                Ok(value) => value.parse().with_context(|| format!("invalid {name}")),
                Err(std::env::VarError::NotPresent) => Ok(default),
                Err(error) => Err(error).with_context(|| format!("invalid {name}")),
            }
        }
        Ok(Self {
            initial: number("HYPERLIGHT_SANDBOX_INITIAL_WORKERS", 0)?,
            maximum: number("HYPERLIGHT_SANDBOX_MAX_WORKERS", 16)?,
        })
    }
}

struct PoolState {
    total: usize,
    idle: Vec<Arc<Connection>>,
    connections: Vec<Weak<Connection>>,
    stopped: bool,
}

/// Developer-only process pool. Each checked-out worker owns one sandbox.
pub struct Pool {
    path: PathBuf,
    root: Arc<tempfile::TempDir>,
    maximum: usize,
    state: Mutex<PoolState>,
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Pool {
    pub fn new(path: impl AsRef<Path>, config: PoolConfig) -> Result<Arc<Self>> {
        ensure!(
            config.maximum > 0 && config.initial <= config.maximum,
            "invalid worker pool limits"
        );
        let path = path
            .as_ref()
            .canonicalize()
            .context("sandbox helper missing; build or install hyperlight-sandbox-worker")?;
        let root = Arc::new(
            tempfile::Builder::new()
                .prefix("hyperlight-snapshots-")
                .tempdir()?,
        );
        #[cfg(target_os = "macos")]
        let path = Self::extract_helper(&path, root.path())?;
        let mut idle = Vec::new();
        for _ in 0..config.initial {
            idle.push(Connection::spawn(&path, root.path())?);
        }
        Ok(Arc::new(Self {
            path,
            root,
            maximum: config.maximum,
            state: Mutex::new(PoolState {
                total: idle.len(),
                connections: idle.iter().map(Arc::downgrade).collect(),
                idle,
                stopped: false,
            }),
        }))
    }

    #[cfg(target_os = "macos")]
    fn extract_helper(source: &Path, directory: &Path) -> Result<PathBuf> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        use sha2::{Digest, Sha256};

        // NuGet extraction need not preserve executable permissions. Copy only our
        // signed helper into a private directory; never modify the installed host.
        let bytes = std::fs::read(source)?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let path = directory.join(format!("hyperlight-sandbox-worker-{hash}"));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)?;
        file.write_all(&bytes)?;
        drop(file);
        let status = Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(&path)
            .status()?;
        ensure!(
            status.success(),
            "packaged sandbox helper signature is invalid; rebuild or reinstall the SDK"
        );
        Ok(path)
    }

    fn checkout(&self) -> Result<Arc<Connection>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("worker pool poisoned"))?;
        ensure!(!state.stopped, "worker pool is shutting down");
        while let Some(connection) = state.idle.pop() {
            if connection.healthy.load(Ordering::Acquire) {
                return Ok(connection);
            }
            state.total -= 1;
        }
        if state.total >= self.maximum {
            log::warn!(
                "developer sandbox worker pool exhausted (maximum {})",
                self.maximum
            );
            bail!(
                "worker pool exhausted (maximum {}); release a sandbox or increase HYPERLIGHT_SANDBOX_MAX_WORKERS",
                self.maximum
            );
        }
        state.total += 1;
        drop(state);
        match Connection::spawn(&self.path, self.root.path()) {
            Ok(connection) => {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| anyhow::anyhow!("worker pool poisoned"))?;
                if state.stopped {
                    state.total -= 1;
                    connection.terminate();
                    bail!("worker pool is shutting down");
                }
                state
                    .connections
                    .retain(|connection| connection.strong_count() > 0);
                state.connections.push(Arc::downgrade(&connection));
                Ok(connection)
            }
            Err(error) => {
                self.state
                    .lock()
                    .map_err(|_| anyhow::anyhow!("worker pool poisoned"))?
                    .total -= 1;
                Err(error)
            }
        }
    }

    fn release(&self, connection: Arc<Connection>) {
        if !connection.healthy.load(Ordering::Acquire) {
            connection.terminate();
        }
        match self.state.lock() {
            Ok(mut state) => {
                if !state.stopped && connection.healthy.load(Ordering::Acquire) {
                    state.idle.push(connection);
                } else {
                    state.total -= 1;
                }
            }

            Err(error) => log::error!("worker pool release failed: {error}"),
        }
    }

    fn shutdown(&self) {
        let connections = match self.state.lock() {
            Ok(mut state) => {
                if state.stopped {
                    return;
                }
                state.stopped = true;
                state
                    .connections
                    .iter()
                    .filter_map(Weak::upgrade)
                    .collect::<Vec<_>>()
            }
            Err(error) => {
                log::error!("worker shutdown failed: {error}");
                return;
            }
        };
        // Includes checked-out children: language finalizers need not run at exit.
        for connection in connections {
            connection.fail("application is shutting down");
            connection.terminate();
        }
        if let Err(error) = std::fs::remove_dir_all(self.root.path())
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::error!("sandbox temporary directory cleanup failed: {error}");
        }
    }
}

type Pools = Mutex<HashMap<PathBuf, Arc<Pool>>>;
static POOLS: OnceLock<Pools> = OnceLock::new();

extern "C" fn cleanup_pools() {
    if let Some(pools) = POOLS.get()
        && let Ok(mut pools) = pools.try_lock()
    {
        for pool in pools.values() {
            pool.shutdown();
        }
        pools.clear();
    }
}

fn cached_pool(path: PathBuf, config: PoolConfig) -> Result<Arc<Pool>> {
    let registry = POOLS.get_or_init(|| {
        // Rust statics do not run destructors at normal process exit.
        if unsafe { libc::atexit(cleanup_pools) } != 0 {
            log::error!("could not register developer sandbox worker cleanup");
        }
        Mutex::new(HashMap::new())
    });
    let mut pools = registry
        .lock()
        .map_err(|_| anyhow::anyhow!("worker pool registry poisoned"))?;
    if let Some(pool) = pools.get(&path) {
        return Ok(pool.clone());
    }
    let pool = Pool::new(&path, config)?;
    pools.insert(path, pool.clone());
    Ok(pool)
}

fn default_pool() -> Result<Arc<Pool>> {
    let path = match std::env::var_os("HYPERLIGHT_SANDBOX_WORKER_PATH") {
        Some(path) => {
            let path = PathBuf::from(path);
            ensure!(
                path.is_absolute(),
                "HYPERLIGHT_SANDBOX_WORKER_PATH must be an absolute path"
            );
            path
        }
        None => {
            let mut info = std::mem::MaybeUninit::<libc::Dl_info>::uninit();
            // Locate the SDK library, not the application's Python/dotnet executable.
            ensure!(
                unsafe {
                    libc::dladdr(
                        default_pool as *const () as *const libc::c_void,
                        info.as_mut_ptr(),
                    )
                } != 0,
                "cannot locate the native SDK library"
            );
            let info = unsafe { info.assume_init() };
            ensure!(
                !info.dli_fname.is_null(),
                "native SDK library path is missing"
            );
            let name = unsafe { CStr::from_ptr(info.dli_fname) };
            Path::new(std::ffi::OsStr::from_bytes(name.to_bytes()))
                .parent()
                .context("native SDK library has no directory")?
                .join("hyperlight-sandbox-worker")
        }
    };
    let path = path.canonicalize().with_context(|| format!("sandbox helper missing at {}; reinstall the SDK or set HYPERLIGHT_SANDBOX_WORKER_PATH for a source build", path.display()))?;
    cached_pool(path, PoolConfig::from_env()?)
}

/// Guest backends supported by the developer worker.
pub trait GuestBackend {
    #[doc(hidden)]
    fn is_wasm() -> bool;
}

impl GuestBackend for Wasm {
    fn is_wasm() -> bool {
        true
    }
}
impl GuestBackend for HyperlightJs {
    fn is_wasm() -> bool {
        false
    }
}
fn backend<G: GuestBackend>() -> Backend {
    if G::is_wasm() {
        Backend::Wasm
    } else {
        Backend::JavaScript
    }
}

pub struct NoGuest;

/// Internal SDK builder with the same configuration methods as the local builder.
pub struct SandboxBuilder<G = NoGuest> {
    options: Options,
    tools: ToolRegistry,
    pool: Option<Arc<Pool>>,
    guest: PhantomData<G>,
}

impl Default for SandboxBuilder {
    fn default() -> Self {
        let config = SandboxConfig::default();
        let limits = FilesystemLimits::default();
        let tools = ToolRegistry::new();
        Self {
            options: Options {
                backend: Backend::Wasm,
                module_path: config.module_path,
                heap_size: config.heap_size,
                stack_size: config.stack_size,
                input_dir: None,
                output_dir: None,
                temp_output: false,
                limits: Some((
                    limits.max_file_size().unwrap(),
                    limits.max_total_size().unwrap(),
                    limits.max_file_count().unwrap(),
                )),
                tools: Vec::new(),
                max_args_size: tools.max_args_size(),
            },
            tools,
            pool: None,
            guest: PhantomData,
        }
    }
}
impl SandboxBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn guest<G: GuestBackend>(mut self, _: G) -> SandboxBuilder<G> {
        self.options.backend = backend::<G>();
        SandboxBuilder {
            options: self.options,
            tools: self.tools,
            pool: self.pool,
            guest: PhantomData,
        }
    }
}
impl<G> SandboxBuilder<G> {
    pub fn module_path(mut self, path: impl Into<String>) -> Self {
        self.options.module_path = path.into();
        self
    }
    pub fn heap_size(mut self, size: u64) -> Self {
        self.options.heap_size = size;
        self
    }
    pub fn stack_size(mut self, size: u64) -> Self {
        self.options.stack_size = size;
        self
    }
    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }
    pub fn input_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.options.input_dir = Some(path.into());
        self
    }
    pub fn output_dir(mut self, path: impl Into<PathBuf>, dir: DirPerms, file: FilePerms) -> Self {
        self.options.output_dir = Some((path.into(), dir.bits(), file.bits()));
        self
    }
    pub fn temp_output(mut self) -> Self {
        self.options.temp_output = true;
        self
    }
    pub fn filesystem_limits(mut self, limits: FilesystemLimits) -> Self {
        self.options.limits = limits
            .max_file_size()
            .zip(limits.max_total_size())
            .zip(limits.max_file_count())
            .map(|((file, total), count)| (file, total, count));
        self
    }
    pub fn pool(mut self, pool: Arc<Pool>) -> Self {
        self.pool = Some(pool);
        self
    }
}

impl<G: GuestBackend> SandboxBuilder<G> {
    pub fn build(mut self) -> Result<Sandbox<G>> {
        static GENERATION: AtomicU64 = AtomicU64::new(1);
        if !self.options.module_path.is_empty() {
            self.options.module_path = std::path::absolute(&self.options.module_path)?
                .to_str()
                .context("module path is not UTF-8")?
                .to_owned();
        }
        self.options.input_dir = self
            .options
            .input_dir
            .map(std::path::absolute)
            .transpose()?;
        self.options.output_dir = self
            .options
            .output_dir
            .map(|(path, dir, file)| {
                Ok::<_, std::io::Error>((std::path::absolute(path)?, dir, file))
            })
            .transpose()?;
        let pool = match self.pool {
            Some(pool) => pool,
            None => default_pool()?,
        };
        let output = if self.options.temp_output && self.options.output_dir.is_none() {
            let directory = tempfile::Builder::new()
                .prefix("output-")
                .tempdir_in(pool.root.path())?;
            self.options.output_dir = Some((
                directory.path().to_path_buf(),
                (DirPerms::READ | DirPerms::MUTATE).bits(),
                (FilePerms::READ | FilePerms::WRITE).bits(),
            ));
            Some(directory)
        } else {
            None
        };
        let connection = pool.checkout()?;
        let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
        self.options.tools = self.tools.names().map(str::to_owned).collect();
        self.options.max_args_size = self.tools.max_args_size();
        *connection
            .tools
            .lock()
            .map_err(|_| anyhow::anyhow!("callback registry poisoned"))? =
            Some((generation, Arc::new(self.tools)));
        let result = connection
            .request(generation, Request::Evolve(self.options))
            .and_then(expect_ack);
        if let Err(error) = result {
            connection.fail("sandbox initialization failed");
            pool.release(connection);
            return Err(error);
        }
        Ok(Sandbox {
            connection: Some(connection),
            pool,
            generation,
            snapshots: Vec::new(),
            _output: output,
            guest: PhantomData,
        })
    }
}

fn expect_ack(response: Response) -> Result<()> {
    ensure!(
        matches!(response, Response::Ack),
        "unexpected sandbox worker response"
    );
    Ok(())
}

struct SnapshotFile {
    directory: tempfile::TempDir,
    pool: Arc<Pool>,
}

pub struct Snapshot<G> {
    file: Arc<SnapshotFile>,
    guest: PhantomData<fn() -> G>,
}
impl<G> Clone for Snapshot<G> {
    fn clone(&self) -> Self {
        Self {
            file: self.file.clone(),
            guest: PhantomData,
        }
    }
}

pub struct Sandbox<G> {
    connection: Option<Arc<Connection>>,
    pool: Arc<Pool>,
    generation: u64,
    // Loaded snapshots can remain memory-mapped by the worker after restore.
    snapshots: Vec<Arc<SnapshotFile>>,
    _output: Option<tempfile::TempDir>,
    guest: PhantomData<G>,
}

impl<G: GuestBackend> Sandbox<G> {
    fn connection(&self) -> &Arc<Connection> {
        self.connection.as_ref().expect("live sandbox connection")
    }
    fn request(&self, request: Request) -> Result<Response> {
        self.connection().request(self.generation, request)
    }
    pub fn run(&mut self, code: &str) -> Result<ExecutionResult> {
        match self.request(Request::Call(code.into()))? {
            Response::Execution(result) => Ok(result),
            _ => bail!("unexpected execution response"),
        }
    }
    pub fn snapshot(&mut self) -> Result<Snapshot<G>> {
        let directory = tempfile::Builder::new()
            .prefix("snapshot-")
            .tempdir_in(self.pool.root.path())?;
        expect_ack(self.request(Request::Snapshot(directory.path().to_path_buf()))?)?;
        Ok(Snapshot {
            file: Arc::new(SnapshotFile {
                directory,
                pool: self.pool.clone(),
            }),
            guest: PhantomData,
        })
    }
    pub fn restore(&mut self, snapshot: &Snapshot<G>) -> Result<()> {
        ensure!(
            Arc::ptr_eq(&snapshot.file.pool, &self.pool),
            "snapshot belongs to a different helper pool"
        );
        // Retain even on error: restore can fail after mapping some snapshot blobs.
        if !self
            .snapshots
            .iter()
            .any(|file| Arc::ptr_eq(file, &snapshot.file))
        {
            self.snapshots.push(snapshot.file.clone());
        }
        expect_ack(self.request(Request::Restore {
            backend: backend::<G>(),
            directory: snapshot.file.directory.path().to_path_buf(),
        })?)
    }
    pub fn allow_domain(&mut self, target: &str, methods: impl Into<MethodFilter>) -> Result<()> {
        let methods = match methods.into() {
            MethodFilter::Any => None,
            MethodFilter::Only(methods) => Some(
                methods
                    .into_iter()
                    .map(|method| method.to_string())
                    .collect(),
            ),
        };
        expect_ack(self.request(Request::AllowDomain {
            target: target.into(),
            methods,
        })?)
    }
    pub fn get_output_files(&self) -> Result<Vec<String>> {
        match self.request(Request::OutputFiles)? {
            Response::Files(files) => Ok(files),
            _ => bail!("unexpected output-files response"),
        }
    }
    pub fn output_path(&self) -> Result<Option<PathBuf>> {
        match self.request(Request::OutputPath)? {
            Response::Path(path) => Ok(path),
            _ => bail!("unexpected output-path response"),
        }
    }
    pub fn interrupt_handle(&self) -> Result<InterruptHandle> {
        ensure!(
            self.connection().healthy.load(Ordering::Acquire),
            "sandbox worker exited"
        );
        Ok(InterruptHandle {
            connection: Arc::downgrade(self.connection()),
            generation: self.generation,
        })
    }
}

/// An interrupt request for a sandbox in a worker process.
#[derive(Clone)]
pub struct InterruptHandle {
    connection: Weak<Connection>,
    generation: u64,
}
impl InterruptHandle {
    pub fn kill(&self) -> bool {
        let Some(connection) = self.connection.upgrade() else {
            return false;
        };
        match connection.request(self.generation, Request::Interrupt) {
            Ok(Response::Interrupted(running)) => running,
            Ok(_) => {
                log::error!("unexpected sandbox interrupt response");
                false
            }
            Err(error) => {
                log::debug!("sandbox interrupt failed: {error:#}");
                false
            }
        }
    }
}

impl<G> Drop for Sandbox<G> {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            if let Err(error) = connection
                .request(self.generation, Request::Drop)
                .and_then(expect_ack)
            {
                log::debug!("sandbox worker release failed: {error:#}");
                connection.fail("sandbox release failed");
            }

            if let Ok(mut tools) = connection.tools.lock() {
                *tools = None;
            }
            self.pool.release(connection);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper_path() -> PathBuf {
        std::env::var_os("HYPERLIGHT_SANDBOX_WORKER_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("hyperlight-sandbox-worker")
            })
    }

    #[test]
    fn repeated_restores_retain_each_snapshot_once() {
        let pool = Pool::new(
            helper_path(),
            PoolConfig {
                initial: 1,
                maximum: 1,
            },
        )
        .unwrap();
        let mut sandbox = SandboxBuilder::new()
            .pool(pool.clone())
            .guest(HyperlightJs)
            .build()
            .unwrap();
        let snapshot = sandbox.snapshot().unwrap();
        sandbox.restore(&snapshot).unwrap();
        sandbox.restore(&snapshot.clone()).unwrap();
        assert_eq!(sandbox.snapshots.len(), 1);
        let missing = Snapshot {
            file: Arc::new(SnapshotFile {
                directory: tempfile::tempdir_in(pool.root.path()).unwrap(),
                pool,
            }),
            guest: PhantomData,
        };
        assert!(sandbox.restore(&missing).is_err());
        assert!(sandbox.restore(&missing).is_err());
        assert_eq!(sandbox.snapshots.len(), 2);
    }

    fn callback_connection(tools: ToolRegistry) -> (Arc<Connection>, UnixStream) {
        let (reader, peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let connection = Arc::new(Connection {
            writer: Mutex::new(reader.try_clone().unwrap()),
            child: Mutex::new(
                Command::new(std::env::current_exe().unwrap())
                    .arg("--help")
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            ),
            pending: Mutex::new(HashMap::new()),
            tools: Mutex::new(Some((1, Arc::new(tools)))),
            next_id: AtomicU64::new(1),
            healthy: AtomicBool::new(true),
        });
        let weak = Arc::downgrade(&connection);
        std::thread::spawn(move || Connection::read_messages(reader, weak));
        (connection, peer)
    }

    fn send_callback(peer: &mut UnixStream) -> u64 {
        let Message::Request {
            id,
            generation,
            request: Request::Call(_),
        } = read_frame(peer).unwrap()
        else {
            panic!("expected execution request");
        };
        write_frame(
            peer,
            &Message::HostCall {
                id: 7,
                generation,
                name: "tool".into(),
                args: "{}".into(),
            },
        )
        .unwrap();
        id
    }

    #[test]
    fn callbacks_run_on_the_requesting_thread() {
        let caller = std::thread::current().id();
        let observed = Arc::new(Mutex::new(None));
        let callback_thread = observed.clone();
        let mut tools = ToolRegistry::new();
        tools.register_unvalidated("tool", move |_| {
            *callback_thread.lock().unwrap() = Some(std::thread::current().id());
            Ok(serde_json::Value::Null)
        });
        let (connection, mut peer) = callback_connection(tools);
        let worker = std::thread::spawn(move || {
            let id = send_callback(&mut peer);
            assert!(matches!(
                read_frame(&mut peer).unwrap(),
                Message::HostReply {
                    id: 7,
                    result: Ok(_)
                }
            ));
            write_frame(
                &mut peer,
                &Message::Response {
                    id,
                    result: Ok(Response::Ack),
                },
            )
            .unwrap();
        });
        expect_ack(connection.request(1, Request::Call(String::new())).unwrap()).unwrap();
        worker.join().unwrap();
        assert_eq!(*observed.lock().unwrap(), Some(caller));
    }

    #[test]
    fn connection_failure_waits_for_callback_to_finish() {
        let (entered, waiting) = mpsc::channel();
        let (release, proceed) = mpsc::channel();
        let proceed = Mutex::new(proceed);
        let (finished, completed) = mpsc::channel();
        let mut tools = ToolRegistry::new();
        tools.register_unvalidated("tool", move |_| {
            entered.send(())?;
            proceed
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))?;
            finished.send(())?;
            Ok(serde_json::Value::Null)
        });
        let (connection, mut peer) = callback_connection(tools);
        let (returned, result) = mpsc::channel();
        let request_connection = connection.clone();
        let worker = std::thread::spawn(move || {
            let response = request_connection.request(1, Request::Call(String::new()));
            returned.send(()).unwrap();
            response
        });
        send_callback(&mut peer);
        waiting.recv_timeout(Duration::from_secs(10)).unwrap();
        connection.fail("simulated worker exit");
        let returned_early = result.recv_timeout(Duration::from_secs(1)).is_ok();
        release.send(()).unwrap();
        completed.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert!(
            !returned_early,
            "execution returned while its callback could still invoke application code"
        );
    }

    #[test]
    fn shutdown_reaps_checked_out_children_before_removing_store() {
        let pool = Pool::new(
            helper_path(),
            PoolConfig {
                initial: 1,
                maximum: 1,
            },
        )
        .unwrap();
        let connection = pool.checkout().unwrap();
        let root = pool.root.path().to_path_buf();
        pool.shutdown();
        assert!(
            connection
                .child
                .lock()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_some()
        );
        assert!(!root.exists());
        assert!(pool.checkout().is_err());
        pool.release(connection);
    }

    #[test]
    fn default_registry_retains_idle_workers_for_sequential_use() {
        let helper = helper_path();
        let config = PoolConfig {
            initial: 1,
            maximum: 1,
        };
        let pool = cached_pool(helper.clone(), config).unwrap();
        let connection = pool.checkout().unwrap();
        let pid = connection.child.lock().unwrap().id();
        pool.release(connection);
        drop(pool);
        let pool = cached_pool(helper, config).unwrap();
        let connection = pool.checkout().unwrap();
        assert_eq!(connection.child.lock().unwrap().id(), pid);
        pool.release(connection);
    }

    #[test]
    fn child_exit_fails_pending_work_and_pool_replaces_it() {
        let pool = Pool::new(
            helper_path(),
            PoolConfig {
                initial: 1,
                maximum: 1,
            },
        )
        .unwrap();
        let connection = pool.checkout().unwrap();
        let (reply, pending) = mpsc::channel();
        connection.pending.lock().unwrap().insert(
            999,
            PendingRequest {
                generation: 0,
                accepts_callbacks: false,
                events: reply,
            },
        );
        connection.child.lock().unwrap().kill().unwrap();
        assert!(matches!(
            pending.recv_timeout(Duration::from_secs(10)).unwrap(),
            RequestEvent::Response(Err(_))
        ));
        assert!(!connection.healthy.load(Ordering::Acquire));
        pool.release(connection);
        let replacement = pool.checkout().unwrap();
        assert!(
            replacement
                .request(0, Request::Interrupt)
                .err()
                .unwrap()
                .to_string()
                .contains("stale interrupt handle")
        );
        assert!(replacement.healthy.load(Ordering::Acquire));
        pool.release(replacement);
    }

    #[test]
    fn failed_workers_are_reaped_while_still_owned() {
        let pool = Pool::new(
            helper_path(),
            PoolConfig {
                initial: 1,
                maximum: 1,
            },
        )
        .unwrap();
        let connection = pool.checkout().unwrap();
        let pid = {
            let mut child = connection.child.lock().unwrap();
            child.kill().unwrap();
            child.id() as libc::pid_t
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let reaped = loop {
            if unsafe { libc::kill(pid, 0) } < 0 {
                break std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        pool.shutdown();
        pool.release(connection);
        assert!(
            reaped,
            "failed worker remained a zombie while its connection was owned"
        );
    }

    #[test]
    fn argument_limits_are_enforced_before_forwarding_callbacks() {
        for limit in [64, 1024 * 1024] {
            let mut tools = ToolRegistry::new();
            tools.set_max_args_size(limit);
            tools.register_unvalidated("tool", Ok);
            let sandbox = SandboxBuilder::new()
                .pool(
                    Pool::new(
                        helper_path(),
                        PoolConfig {
                            initial: 1,
                            maximum: 1,
                        },
                    )
                    .unwrap(),
                )
                .with_tools(tools)
                .guest(HyperlightJs)
                .build()
                .unwrap();
            let (_, response) = sandbox
                .connection()
                .start_request(
                    sandbox.generation,
                    Request::Call(format!(
                        "call_tool('tool', {{value: 'x'.repeat({limit})}});"
                    )),
                )
                .unwrap();
            let (rejected, detail) = match response.recv_timeout(Duration::from_secs(10)).unwrap() {
                RequestEvent::Response(Ok(Response::Execution(result))) => (
                    result.exit_code != 0,
                    format!(
                        "exit {}: {}{}",
                        result.exit_code, result.stdout, result.stderr
                    ),
                ),
                RequestEvent::Response(Err(error)) => (
                    sandbox.connection().healthy.load(Ordering::Acquire),
                    format!("{error:#}"),
                ),
                RequestEvent::HostCall { args, .. } => {
                    (false, format!("forwarded {} bytes", args.len()))
                }
                _ => (false, "unexpected response".into()),
            };
            sandbox.connection().terminate();
            assert!(
                rejected,
                "oversized arguments were not rejected in the worker: {detail}"
            );
        }
    }
}
