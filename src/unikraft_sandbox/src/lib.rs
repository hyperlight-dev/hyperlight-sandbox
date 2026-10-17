//! [hyperlight-unikraft] guest for hyperlight-sandbox.
//!
//! Runs a full CPython (the published `python`, `python-shell` and `agent`
//! images: the `agent` image adds numpy, pandas, scipy, scikit-learn and
//! friends) in a Unikraft unikernel inside a Hyperlight micro-VM, behind the
//! same [`Guest`] / [`GuestSandbox`] contract as the Wasm and HyperlightJS
//! backends:
//!
//! | hyperlight-sandbox             | hyperlight-unikraft                          |
//! |--------------------------------|----------------------------------------------|
//! | `run(code)`                    | a guest function call into a boot-time bridge |
//! | `call_tool(name, **kwargs)`    | host function `sandbox.call_tool`             |
//! | `http_get` / `http_post`       | host function `sandbox.http` (allowlisted)    |
//! | `/input`, `/output`            | hostfs mounts (read-only / read-write)        |
//! | `allow_domain`                 | checked per HTTP request on the host; raw sockets opt-in via [`Unikraft::native_network`] |
//! | `snapshot` / `restore`         | hyperlight snapshots of the whole micro-VM    |
//!
//! The module path given to the sandbox builder is the rootfs image (an
//! initrd CPIO). [`cached_rootfs`] finds one the `hluk` CLI has pulled.
//!
//! [hyperlight-unikraft]: https://github.com/hyperlight-dev/hyperlight-unikraft

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use base64::Engine as _;
use cap_std::fs::Dir as CapDir;
use hyperlight_sandbox::runtime::BlockOn;
use hyperlight_sandbox::{
    CapFs, DirPerms, ExecutionResult, FilePerms, Guest, GuestSandbox, HttpMethod,
    NetworkPermissions, SandboxConfig, Snapshot, ToolRegistry, http as sandbox_http,
};
pub use hyperlight_unikraft;
use hyperlight_unikraft::{
    AllowList, AppSandbox, Mount, NetworkPolicy, SandboxBuilder as HlukBuilder,
    Snapshot as HlukSnapshot,
};

/// The guest code that defines `call_tool`, `http_get`, `http_post` and the
/// run bridge. Run once in `__main__` at boot, so snapshots carry it.
const PRELUDE: &str = include_str!("prelude.py");

/// The guest function the prelude defines; every `run` calls it.
const RUN_FUNCTION: &str = "__hyperlight_sandbox_run";

/// Largest reply sent inline in one host call. The limit is 64 KiB for the
/// whole reply, framing included; anything larger goes through the blob
/// store and the guest pulls it in [`BLOB_CHUNK`] pieces.
const INLINE_MAX: usize = 48 * 1024;

/// Raw bytes per `sandbox.blob_read` reply: base64 grows it by 4/3, to
/// ~43 KiB, safely under the 64 KiB host call limit.
const BLOB_CHUNK: usize = 32 * 1024;

/// Cap on captured stdout and stderr per run, matching the JS backend.
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Cap on the bytes the host holds for the guest at once: replies it has
/// not finished reading and requests it is still uploading. Without it a
/// guest could pin host memory by making large calls and never reading
/// the replies.
const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;

/// Cap on the number of replies and uploads held at once.
const MAX_PENDING_BLOBS: usize = 256;

/// The release of the published rootfs images this backend is built
/// against. A snapshot (and the guest kernel's host contract) is tied to
/// the `hyperlight-unikraft` release, so the images must match it.
pub const IMAGE_RELEASE: &str = "v0.18.0";

/// The path where the `hluk` CLI caches the published rootfs image of
/// `runtime` (`python`, `python-shell`, `agent`, ...) for [`IMAGE_RELEASE`].
///
/// Mirrors the CLI's layout: `$HLUK_CACHE_DIR`, else `$XDG_CACHE_HOME/hluk`
/// or `~/.cache/hluk` on Unix, `%LOCALAPPDATA%\hluk\cache` on Windows.
/// Errors if the image has not been pulled; any `hluk run` of the image
/// pulls it.
pub fn cached_rootfs(runtime: &str) -> Result<PathBuf> {
    let cache = if let Some(dir) = std::env::var_os("HLUK_CACHE_DIR") {
        PathBuf::from(dir)
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join("hluk").join("cache"))
            .context("cannot find the hluk cache: set HLUK_CACHE_DIR or LOCALAPPDATA")?
    } else if let Some(dir) = std::env::var_os("XDG_CACHE_HOME").filter(|d| !d.is_empty()) {
        PathBuf::from(dir).join("hluk")
    } else {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".cache").join("hluk"))
            .context("cannot find the hluk cache: set HLUK_CACHE_DIR or HOME")?
    };
    let path = cache
        .join("rootfs/ghcr.io/hyperlight-dev/hyperlight-unikraft")
        .join(runtime)
        .join(format!("initrd-{IMAGE_RELEASE}.cpio"));
    anyhow::ensure!(
        path.is_file(),
        "no cached {runtime} rootfs at {}; pull it with `hluk run --runtime {runtime} --cold --exec pass` \
         (hluk {IMAGE_RELEASE}) or pass the CPIO path as the module path",
        path.display()
    );
    Ok(path)
}

// ---------------------------------------------------------------------------
// Guest
// ---------------------------------------------------------------------------

/// The hyperlight-unikraft guest. The sandbox's module path is the rootfs.
///
/// `SandboxConfig::heap_size` / `stack_size` do not apply: a unikernel has
/// one guest memory size, which defaults to the size the image was tested
/// with (1.5 GiB for `agent`, 256 MiB for `python`) and is set with
/// [`scratch_mb`](Self::scratch_mb).
///
/// `/input` and `/output` are host directories mounted at boot. `/output`
/// is read-write when its permissions allow creating and writing files,
/// else read-only when they allow reading, else absent; it cannot be
/// write-only. As in the other backends, its files persist from run to
/// run, and restoring a snapshot clears them. Files the guest has open are
/// interpreter state, though: they stay open across runs, and a restore
/// does not close them. Since the guest writes `/output` directly, it is
/// checked by the rules `CapFs` applies to writes made through it (the
/// filesystem limits, no symbolic or hard links, no special files, at most
/// 64 levels and 100,000 entries) after each run, and before each, as the
/// host may have changed it since. A run that breaks them fails, and
/// `/output` is cleared of every file and link and of the directories made
/// during the run; those it had when the run started are kept, emptied. A
/// run that would start on such things is refused, and they are left as
/// they are. The guest is (re)booted
/// only onto the directories the sandbox was built with: one replaced on
/// the host since then fails the boot an allowlist change makes, until it
/// is put back.
///
/// A run's exit status is the one a CPython process would exit with, 0 to
/// 255 (`sys.exit(-1)` is 255), so -1 always means the guest failed.
///
/// Requests and replies of any size cross the 64 KiB host call limit in
/// pieces; the host holds at most 64 MiB (and 256 pieces) of them unread
/// at a time, and a call past that fails in the guest.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unikraft {
    scratch_mb: Option<usize>,
    native_network: bool,
}

impl Unikraft {
    pub fn new() -> Self {
        Self::default()
    }

    /// Guest memory in MiB, overriding the image's tested default.
    pub fn scratch_mb(mut self, mb: usize) -> Self {
        self.scratch_mb = Some(mb);
        self
    }

    /// Also let guest sockets reach the hosts named by `allow_domain`, so
    /// libraries like `requests` and `urllib` work directly.
    ///
    /// Off by default because socket-level enforcement is coarser than the
    /// HTTP-level allowlist the other backends apply: it is by host name
    /// only, so the scheme, port, path prefix and method filters of an
    /// `allow_domain` entry are not enforced on these sockets (they still
    /// are for `http_get` / `http_post`). The names are resolved when the
    /// allowlist changes; one that does not resolve then is left out until
    /// it changes again, and the next run's stderr says so.
    pub fn native_network(mut self, on: bool) -> Self {
        self.native_network = on;
        self
    }
}

impl Guest for Unikraft {
    type Sandbox = UnikraftGuestSandbox;

    fn build(
        self,
        config: SandboxConfig,
        tools: ToolRegistry,
        network: Arc<Mutex<NetworkPermissions>>,
        fs: Arc<Mutex<CapFs>>,
    ) -> Result<UnikraftGuestSandbox> {
        UnikraftGuestSandbox::new(self, config, tools, network, fs)
    }
}

// ---------------------------------------------------------------------------
// Host state shared with the host functions
// ---------------------------------------------------------------------------

/// Data too large for one host call: replies waiting for the guest to read
/// them, and requests the guest is uploading. Emptied before each run.
#[derive(Default)]
struct Blobs {
    next_id: u64,
    pending: HashMap<u64, Vec<u8>>,
    /// Bytes held across `pending`.
    bytes: usize,
}

impl Blobs {
    /// Make room for `extra` more bytes, in a new blob if `new`.
    fn reserve(&self, extra: usize, new: bool) -> std::result::Result<(), String> {
        if new && self.pending.len() >= MAX_PENDING_BLOBS {
            return Err(format!(
                "the host holds {MAX_PENDING_BLOBS} unread replies or uploads already; \
                 read them before making more calls"
            ));
        }
        if self.bytes.saturating_add(extra) > MAX_PENDING_BYTES {
            return Err(format!(
                "the host would hold more than {MAX_PENDING_BYTES} bytes of unread replies \
                 and uploads; read earlier replies, or exchange less data per call"
            ));
        }
        Ok(())
    }

    fn insert(&mut self, data: Vec<u8>) -> std::result::Result<u64, String> {
        self.reserve(data.len(), true)?;
        let id = self.next_id;
        self.next_id += 1;
        self.bytes += data.len();
        self.pending.insert(id, data);
        Ok(id)
    }

    fn remove(&mut self, id: u64) -> Option<Vec<u8>> {
        let data = self.pending.remove(&id)?;
        self.bytes -= data.len();
        Some(data)
    }

    fn clear(&mut self) {
        self.pending.clear();
        self.bytes = 0;
    }
}

/// The upload a host call's arguments name, if they are `[{"upload": id}]`:
/// a request the guest sent in pieces, being too large for one host call.
/// No call's own arguments take that shape: they have two or four elements.
fn uploaded(args: &str) -> Option<u64> {
    // json.dumps spells it so; skip parsing every other call twice.
    if !args.starts_with("[{\"upload\"") {
        return None;
    }
    let serde_json::Value::Array(items) = serde_json::from_str(args).ok()? else {
        return None;
    };
    let [serde_json::Value::Object(fields)] = items.as_slice() else {
        return None;
    };
    if fields.len() != 1 {
        return None;
    }
    fields.get("upload")?.as_u64()
}

#[derive(Clone)]
struct HostState {
    tools: Arc<ToolRegistry>,
    network: Arc<Mutex<NetworkPermissions>>,
    blobs: Arc<Mutex<Blobs>>,
    /// What Python writes to `sys.stderr`, apart from standard output.
    /// Python's standard output is the guest's fd 1, with what everything
    /// else writes there, in order.
    stderr: Arc<Mutex<String>>,
}

impl HostState {
    /// Wrap a reply value in the envelope the prelude's `_hls_unwrap` reads.
    fn envelope(&self, value: &serde_json::Value) -> std::result::Result<String, String> {
        let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
        if text.len() <= INLINE_MAX {
            return Ok(format!("{{\"inline\":{text}}}"));
        }
        let mut blobs = self.blobs.lock().map_err(|_| "blob store poisoned")?;
        let len = text.len();
        let id = blobs.insert(text.into_bytes())?;
        Ok(serde_json::json!({ "blob": id, "len": len }).to_string())
    }

    /// Append a piece of an upload, `[id or null, text]`, starting a new
    /// one for `null`; returns the upload's id.
    fn upload(&self, args: &str) -> std::result::Result<String, String> {
        let (id, piece): (Option<u64>, String) =
            serde_json::from_str(args).map_err(|e| e.to_string())?;
        let mut blobs = self.blobs.lock().map_err(|_| "blob store poisoned")?;
        let id = match id {
            None => blobs.insert(piece.into_bytes())?,
            Some(id) => {
                blobs.reserve(piece.len(), false)?;
                let data = blobs
                    .pending
                    .get_mut(&id)
                    .ok_or_else(|| format!("no upload {id}"))?;
                data.extend_from_slice(piece.as_bytes());
                blobs.bytes += piece.len();
                id
            }
        };
        Ok(id.to_string())
    }

    /// The arguments of a host call, from an upload if the guest sent one.
    fn arguments<'a>(&self, args: &'a str) -> std::result::Result<Cow<'a, str>, String> {
        let Some(upload) = uploaded(args) else {
            return Ok(Cow::Borrowed(args));
        };
        let data = self
            .blobs
            .lock()
            .map_err(|_| "blob store poisoned")?
            .remove(upload)
            .ok_or_else(|| format!("no upload {upload}"))?;
        String::from_utf8(data)
            .map(Cow::Owned)
            .map_err(|_| "upload is not UTF-8".to_string())
    }

    fn blob_read(&self, args: &str) -> std::result::Result<String, String> {
        let (id, offset): (u64, usize) = serde_json::from_str(args).map_err(|e| e.to_string())?;
        let mut blobs = self.blobs.lock().map_err(|_| "blob store poisoned")?;
        let blob = blobs
            .pending
            .get(&id)
            .ok_or_else(|| format!("no host blob {id}"))?;
        let end = blob.len().min(offset.saturating_add(BLOB_CHUNK));
        let chunk = blob.get(offset..end).unwrap_or_default();
        let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
        // The guest reads front to back, so the last chunk frees the blob.
        if end >= blob.len() {
            blobs.remove(id);
        }
        serde_json::to_string(&encoded).map_err(|e| e.to_string())
    }

    fn call_tool(&self, args: &str) -> std::result::Result<String, String> {
        let args = self.arguments(args)?;
        let (name, kwargs): (String, serde_json::Value) =
            serde_json::from_str(&args).map_err(|e| e.to_string())?;
        let value = self
            .tools
            .dispatch(&name, kwargs)
            .map_err(|e| format!("{e:#}"))?;
        self.envelope(&value)
    }

    fn http(&self, args: &str) -> std::result::Result<String, String> {
        let args = self.arguments(args)?;
        let (method, url, headers, body): (
            String,
            String,
            HashMap<String, String>,
            Option<String>,
        ) = serde_json::from_str(&args).map_err(|e| e.to_string())?;
        let url = url::Url::parse(&url).map_err(|e| format!("invalid URL: {e}"))?;
        let method_enum: HttpMethod = method.parse().map_err(|e| format!("{e}"))?;
        {
            let network = self.network.lock().map_err(|_| "network mutex poisoned")?;
            if !network.is_allowed(&url, &method_enum) {
                return Err(format!("HTTP request denied for {method_enum} {url}"));
            }
        }
        if headers.len() > sandbox_http::MAX_RESPONSE_HEADER_COUNT {
            return Err("too many request headers".into());
        }
        let request = sandbox_http::HttpRequest {
            url,
            method: method_enum.to_string(),
            headers: headers.into_iter().collect(),
            body: sandbox_http::HttpRequest::body_from_bytes(body.map(String::into_bytes)),
        };
        let response = sandbox_http::send_http_request(request)
            .block_on()
            .map_err(|e| format!("{e:#}"))?;
        let body = match String::from_utf8(response.body) {
            Ok(body) => body,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        };
        self.envelope(&serde_json::json!({
            "status": response.status,
            "headers": response.headers,
            "body": body,
        }))
    }

    fn append(buffer: &Mutex<String>, args: &str) -> std::result::Result<String, String> {
        let [text]: [String; 1] = serde_json::from_str(args).map_err(|e| e.to_string())?;
        let mut buffer = buffer.lock().map_err(|_| "output buffer poisoned")?;
        push_capped(&mut buffer, &text);
        Ok(String::new())
    }

    /// Register every bridge host function on a hyperlight-unikraft builder.
    fn register(&self, builder: HlukBuilder) -> HlukBuilder {
        let s = self.clone();
        let builder = builder.host_function("sandbox.call_tool", move |a| s.call_tool(a));
        let s = self.clone();
        let builder = builder.host_function("sandbox.http", move |a| s.http(a));
        let s = self.clone();
        let builder = builder.host_function("sandbox.blob_read", move |a| s.blob_read(a));
        let s = self.clone();
        let builder = builder.host_function("sandbox.upload", move |a| s.upload(a));
        let err = self.stderr.clone();
        builder.host_function("sandbox.stderr", move |a| Self::append(&err, a))
    }

    fn take_stderr(&self) -> Result<String> {
        self.stderr
            .lock()
            .map(|mut b| std::mem::take(&mut *b))
            .map_err(|_| anyhow::anyhow!("output buffer poisoned"))
    }
}

/// Append `text`, cut at a character boundary once the buffer is full.
fn push_capped(buffer: &mut String, text: &str) {
    let room = MAX_OUTPUT_BYTES.saturating_sub(buffer.len());
    let mut take = text.len().min(room);
    while !text.is_char_boundary(take) {
        take -= 1;
    }
    buffer.push_str(&text[..take]);
}

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

pub struct UnikraftGuestSandbox {
    sandbox: AppSandbox,
    host: HostState,
    mounts: Vec<Mount>,
    native_network: bool,
    /// The hosts the guest's socket policy was built from, to notice an
    /// `allow_domain` made after boot.
    socket_hosts: Vec<String>,
    /// Said in the next run's stderr: which allowed hosts guest sockets
    /// cannot reach because they did not resolve.
    socket_notice: Option<String>,
    files: Arc<Mutex<CapFs>>,
    /// The directories `/output` had when the current run started, relative
    /// to it: kept when a failed check clears the run's output, so that
    /// what the host staged keeps its shape.
    kept_dirs: HashSet<PathBuf>,
}

impl UnikraftGuestSandbox {
    fn new(
        guest: Unikraft,
        config: SandboxConfig,
        tools: ToolRegistry,
        network: Arc<Mutex<NetworkPermissions>>,
        files: Arc<Mutex<CapFs>>,
    ) -> Result<Self> {
        // HTTP host calls run on the shared runtime; fail now, not mid-run.
        hyperlight_sandbox::runtime::RUNTIME
            .as_ref()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        anyhow::ensure!(
            !config.module_path.is_empty(),
            "the unikraft backend needs a rootfs image as its module path \
             (an initrd CPIO, e.g. from hyperlight_unikraft_sandbox::cached_rootfs(\"agent\"))"
        );
        let rootfs = PathBuf::from(&config.module_path);
        anyhow::ensure!(
            rootfs.is_file(),
            "rootfs image does not exist: {}",
            rootfs.display()
        );

        let host = HostState {
            tools: Arc::new(tools),
            network: network.clone(),
            blobs: Arc::default(),
            stderr: Arc::default(),
        };
        let mounts = mounts_for(&files)?;
        let socket_hosts = socket_hosts(guest.native_network, &network)?;
        let (policy, socket_notice) = socket_policy(&socket_hosts);

        let mut builder = HlukBuilder::from_initrd(&rootfs);
        if let Some(mb) = guest.scratch_mb {
            builder = builder.scratch_mb(mb);
        }
        check_mounts(&files, &mounts)?;
        let builder = configure(builder, &host, &mounts, policy);
        let mut sandbox = builder
            .boot()
            .with_context(|| format!("failed to boot {}", rootfs.display()))?;
        anyhow::ensure!(
            sandbox.has_driver(),
            "{} has no runtime driver to run code with; use a python, python-shell or agent image",
            rootfs.display()
        );
        sandbox.run(PRELUDE).map_err(|e| {
            anyhow::anyhow!(
                "failed to load the sandbox bridge into {} (only the python, python-shell and \
                 agent images are supported): {e}\n{}",
                rootfs.display(),
                sandbox.drain_output()
            )
        })?;
        sandbox.drain_output();

        Ok(Self {
            sandbox,
            host,
            mounts,
            native_network: guest.native_network,
            socket_hosts,
            socket_notice,
            files,
            // Taken before each run
            kept_dirs: HashSet::new(),
        })
    }

    /// Re-boot the guest under the current allowlist, keeping its state:
    /// the socket policy is fixed at boot, and a snapshot restores under a
    /// different one.
    fn sync_socket_policy(&mut self) -> Result<()> {
        let hosts = socket_hosts(self.native_network, &self.host.network)?;
        if hosts == self.socket_hosts {
            return Ok(());
        }
        // Resolved once per change of the allowlist, not every run: a name
        // that does not resolve leaves guest sockets unable to reach it, as
        // DNS would, rather than failing every run from then on.
        let (policy, notice) = socket_policy(&hosts);
        check_mounts(&self.files, &self.mounts)?;
        let snapshot = self.sandbox.snapshot()?;
        let builder = configure(
            HlukBuilder::from_snapshot(snapshot),
            &self.host,
            &self.mounts,
            policy,
        );
        self.sandbox = builder.boot()?;
        self.sandbox.drain_output();
        self.socket_hosts = hosts;
        self.socket_notice = notice;
        Ok(())
    }

    fn run_impl(&mut self, code: &str) -> Result<ExecutionResult> {
        {
            let mut files = self
                .files
                .lock()
                .map_err(|_| anyhow::anyhow!("filesystem mutex poisoned"))?;
            // What persists from the last run, and what the host has put
            // there since, by the rules a run's output is checked by: the
            // guest reaches `/output` itself, through its mount, so it must
            // not start on a link or a special file. The rest of
            // `prepare_for_run` is resetting `CapFs` handles, of which the
            // guest holds none.
            files
                .validate_output()
                .context("refusing to run on what /output holds")?;
            self.kept_dirs =
                output_dirs(&files).context("failed to list the directories in /output")?;
        }
        self.sync_socket_policy()?;
        self.host.take_stderr()?;
        self.host
            .blobs
            .lock()
            .map_err(|_| anyhow::anyhow!("blob store poisoned"))?
            .clear();

        let request = self
            .host
            .envelope(&serde_json::Value::String(code.to_owned()))
            .map_err(|e| anyhow::anyhow!(e))?;
        let outcome = self.sandbox.call(RUN_FUNCTION, &request);
        // The guest's fd 1 and 2: Python's standard output, and what C
        // extensions, child processes and the kernel write, each in the
        // order written. Standard error first: draining the interleaved
        // output drops it.
        let raw_stderr = self.sandbox.drain_stderr();
        let raw = self.sandbox.drain_output();
        let mut stderr = self.host.take_stderr()?;
        let mut stdout = String::new();
        // Without a clean split, standard output keeps both, interleaved,
        // and standard error still has its own.
        push_capped(
            &mut stdout,
            strip_stderr_lines(&raw, &raw_stderr)
                .as_deref()
                .unwrap_or(&raw),
        );
        push_capped(&mut stderr, &raw_stderr);

        if let Some(notice) = self.socket_notice.take() {
            stderr.insert_str(0, &notice);
        }

        // Nothing below returns early: the output limits are enforced, and
        // the output kept, whatever the guest returned.
        let mut exit_code = match outcome {
            Ok(result) => match result.trim().parse::<i64>() {
                Ok(status) => exit_status(status),
                Err(_) => {
                    push_capped(&mut stderr, &format!("unexpected run result {result:?}\n"));
                    1
                }
            },
            Err(error) => {
                // The guest itself failed (crashed, deadlocked, exited):
                // report it like the other backends report a trap.
                push_capped(&mut stderr, &format!("{error}\n"));
                -1
            }
        };
        if let Some(violation) = self.check_output_limits() {
            push_capped(&mut stderr, &format!("{violation}\n"));
            if exit_code == 0 {
                exit_code = 1;
            }
        }
        Ok(ExecutionResult {
            stdout,
            stderr,
            exit_code,
        })
    }

    /// The guest writes `/output` through hostfs, which `CapFs` does not
    /// see, so `CapFs` checks it once the run is over, by the rules it
    /// applies to writes made through it: the filesystem limits, no
    /// symbolic or hard links, no special files, and its traversal bounds
    /// (64 levels, 100,000 entries). A run that breaks them fails and
    /// `/output` is cleared, earlier runs' files with it: what the guest
    /// broke them with cannot be told from the rest.
    ///
    /// Returns what to say in the run's stderr, if anything.
    fn check_output_limits(&self) -> Option<String> {
        let Ok(mut files) = self.files.lock() else {
            return Some("[hyperlight-sandbox] /output could not be checked".into());
        };
        let error = files.validate_output().err()?;
        // Cleared by this backend, which no traversal limit stops, so
        // whatever tree the guest made goes; then `CapFs` resynchronises on
        // what is left.
        let cleared = clear_output(&files, &self.kept_dirs)
            .map_err(anyhow::Error::from)
            .and_then(|()| files.clear_output_files());
        Some(match cleared {
            Ok(()) => format!(
                "[hyperlight-sandbox] /output exceeded the filesystem limits ({error:#}); \
                 its files were removed"
            ),
            Err(clear_error) => format!(
                "[hyperlight-sandbox] /output exceeded the filesystem limits ({error:#}); \
                 removing its files failed: {clear_error:#}"
            ),
        })
    }
}

/// Deepest directory nesting `CapFs` walks in `/output`.
const MAX_OUTPUT_DEPTH: usize = 64;

/// The directories in `/output` now, relative to it, through the handle
/// `CapFs` opened on it.
fn output_dirs(files: &CapFs) -> Result<HashSet<PathBuf>> {
    let mut found = HashSet::new();
    let Some(root) = files.dir_by_guest_path("/output") else {
        return Ok(found);
    };
    // Within `CapFs`'s bounds: `validate_output` just checked the tree.
    let mut dirs = vec![(root.cap_std().try_clone()?, PathBuf::new())];
    while let Some((dir, at)) = dirs.pop() {
        for entry in dir.entries()? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let path = at.join(entry.file_name());
                if path.components().count() <= MAX_OUTPUT_DEPTH {
                    dirs.push((dir.open_dir(entry.file_name())?, path.clone()));
                    found.insert(path);
                }
            }
        }
    }
    Ok(found)
}

/// Remove what the guest left in `/output`: every file and link, and
/// every directory but `keep` (those the run started with), however
/// deep. Works through
/// the handle `CapFs` opened, so neither the process's working directory
/// nor a component of the path swapped since can redirect it, and follows
/// no symbolic links.
fn clear_output(files: &CapFs, keep: &HashSet<PathBuf>) -> std::io::Result<()> {
    match files.dir_by_guest_path("/output") {
        Some(root) => clear_dir(root.cap_std(), keep, Path::new("")),
        None => Ok(()),
    }
}

fn clear_dir(dir: &CapDir, keep: &HashSet<PathBuf>, at: &Path) -> std::io::Result<()> {
    for entry in dir.entries()? {
        let entry = entry?;
        let name = entry.file_name();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let path = at.join(&name);
            if keep.contains(&path) {
                clear_dir(&dir.open_dir(&name)?, keep, &path)?;
            } else {
                dir.remove_dir_all(&name)?;
            }
        } else if let Err(error) = dir.remove_file(&name) {
            // A directory symbolic link or junction (Windows) goes with
            // remove_dir; it does not touch the target.
            if !file_type.is_symlink() {
                return Err(error);
            }
            dir.remove_dir(&name)?;
        }
    }
    Ok(())
}

/// A run's exit status from the bridge, which reports it as a CPython
/// process would exit (0 to 255), so it never meets -1, a guest failure.
/// Anything else is not the bridge's doing and counts as a failure.
fn exit_status(status: i64) -> i32 {
    u8::try_from(status).map_or(1, i32::from)
}

/// Standard output alone, from the interleaved guest output and its
/// standard error.
///
/// hyperlight-unikraft captures standard error on its own but standard
/// output only interleaved with it. Taking out the standard error lines in
/// order recovers it whenever standard error was written in whole lines,
/// as kernel messages and most programs' errors are: a standard output
/// line identical to a pending standard error line can only trade places
/// with it, as a Python standard output line can with a native standard
/// error line written next to it. Otherwise `None`, and the caller keeps
/// the interleaved text.
fn strip_stderr_lines(output: &str, stderr: &str) -> Option<String> {
    if stderr.is_empty() {
        return Some(output.to_owned());
    }
    let mut pending = stderr.split_inclusive('\n').peekable();
    let mut stdout = String::with_capacity(output.len().saturating_sub(stderr.len()));
    for line in output.split_inclusive('\n') {
        if pending.peek() == Some(&line) {
            pending.next();
        } else {
            stdout.push_str(line);
        }
    }
    pending.peek().is_none().then_some(stdout)
}

/// `/input` read-only, and `/output` as close to its `CapFs` permissions as
/// a hostfs mount, read-only or read-write, can be: read-write when files
/// may be created and written, else read-only when it may be read, else
/// not mounted. Write-only cannot be had: the guest can read back what it
/// wrote.
fn mounts_for(files: &Mutex<CapFs>) -> Result<Vec<Mount>> {
    let files = files
        .lock()
        .map_err(|_| anyhow::anyhow!("filesystem mutex poisoned"))?;
    // Resolved now, as `CapFs` resolved them: a re-boot remounts them,
    // and a relative path would then follow the process's working
    // directory elsewhere.
    let absolute = |path: &Path| {
        std::fs::canonicalize(path).with_context(|| format!("cannot resolve {}", path.display()))
    };
    let mut mounts = Vec::new();
    if let Some(input) = files.input_path() {
        mounts.push(Mount::ro(absolute(input)?, "/input"));
    }
    if let (Some(output), Some(dir)) = (files.output_path(), files.dir_by_guest_path("/output")) {
        let output = absolute(output)?;
        if dir.perms().contains(DirPerms::MUTATE) && dir.file_perms().contains(FilePerms::WRITE) {
            mounts.push(Mount::rw(output, "/output"));
        } else if dir.perms().contains(DirPerms::READ) {
            mounts.push(Mount::ro(output, "/output"));
        }
    }
    Ok(mounts)
}

/// The host names guest sockets may reach: none unless native networking
/// is on, else those of every `allow_domain` entry.
fn socket_hosts(native: bool, network: &Mutex<NetworkPermissions>) -> Result<Vec<String>> {
    if !native {
        return Ok(Vec::new());
    }
    let network = network
        .lock()
        .map_err(|_| anyhow::anyhow!("network mutex poisoned"))?;
    let mut hosts: Vec<String> = network
        .permissions()
        .iter()
        .filter_map(|p| p.url.host().map(|host| allowlist_host(&host)))
        .collect();
    hosts.sort();
    hosts.dedup();
    Ok(hosts)
}

/// A URL's host as hyperlight-unikraft's allowlist takes it: a name, or an
/// address without the brackets a URL puts around IPv6.
fn allowlist_host(host: &url::Host<&str>) -> String {
    match host {
        url::Host::Domain(name) => (*name).to_owned(),
        url::Host::Ipv4(address) => address.to_string(),
        // As an IPv4 address when it is one: the allowlist compares
        // addresses in that form.
        url::Host::Ipv6(address) => address
            .to_ipv4_mapped()
            .map_or_else(|| address.to_string(), |v4| v4.to_string()),
    }
}

/// Refuse to mount a directory other than the one `CapFs` opened for it.
///
/// hostfs opens each mount by path when the guest boots and keeps that
/// directory until the next boot (a restore keeps it too), while `CapFs`
/// checks and clears the one it opened at build. So this is checked before
/// each boot, the first and the re-boot an allowlist change makes: were the
/// path replaced since the build (`rm -rf out && mkdir out`), the guest
/// would write a directory nothing checks. Between boots, replacing it
/// changes nothing for either side.
fn check_mounts(files: &Mutex<CapFs>, mounts: &[Mount]) -> Result<()> {
    let files = files
        .lock()
        .map_err(|_| anyhow::anyhow!("filesystem mutex poisoned"))?;
    for mount in mounts {
        let same = files
            .is_same_dir(&mount.guest_path, &mount.host_path)
            .with_context(|| {
                format!(
                    "cannot check that {} ({}) is the directory the sandbox was built with",
                    mount.guest_path,
                    mount.host_path.display()
                )
            })?;
        anyhow::ensure!(
            same,
            "{} ({}) was replaced since the sandbox was built; restore the original \
             directory or build a new sandbox",
            mount.guest_path,
            mount.host_path.display()
        );
    }
    Ok(())
}

/// The guest's socket policy for `hosts`, and a note naming those of them
/// that did not resolve, which the policy leaves out.
fn socket_policy(hosts: &[String]) -> (Option<AllowList>, Option<String>) {
    if hosts.is_empty() {
        return (None, None);
    }
    let mut reachable: Vec<&String> = hosts
        .iter()
        .filter(|host| AllowList::from_hosts(&[host.as_str()]).is_ok())
        .collect();
    // Resolved again as one list: drop any that stopped resolving since.
    let policy = loop {
        if reachable.is_empty() {
            break None;
        }
        match AllowList::from_hosts(&reachable) {
            Ok(allow) => break Some(allow),
            Err(e) => reachable.retain(|host| host.as_str() != e.host()),
        }
    };
    let unresolved: Vec<&str> = hosts
        .iter()
        .filter(|host| !reachable.contains(host))
        .map(String::as_str)
        .collect();
    let notice = (!unresolved.is_empty()).then(|| {
        format!(
            "[hyperlight-sandbox] native network: {} did not resolve; guest sockets cannot \
             reach it until the allowlist changes\n",
            unresolved.join(", ")
        )
    });
    (policy, notice)
}

fn configure(
    builder: HlukBuilder,
    host: &HostState,
    mounts: &[Mount],
    policy: Option<AllowList>,
) -> HlukBuilder {
    // Raw guest output belongs in the run's result, not in the embedding
    // process's stdout, and is held to the same cap as the rest of it.
    let mut builder = host
        .register(builder)
        .echo_output(false)
        .output_limit(MAX_OUTPUT_BYTES)
        .mounts(mounts.iter().cloned());
    if let Some(allow) = policy {
        builder = builder.network(NetworkPolicy::AllowList(allow));
    }
    builder
}

impl GuestSandbox for UnikraftGuestSandbox {
    type SnapshotData = HlukSnapshot;

    fn run(&mut self, code: &str) -> Result<ExecutionResult> {
        self.run_impl(code)
    }

    fn snapshot(&mut self) -> Result<Snapshot<HlukSnapshot>> {
        let snapshot = self
            .sandbox
            .snapshot()
            .map_err(|e| anyhow::anyhow!("snapshot failed: {e}"))?;
        Ok(Snapshot::new("hyperlight-unikraft", snapshot))
    }

    fn restore(&mut self, snapshot: &Snapshot<HlukSnapshot>) -> Result<()> {
        self.sandbox
            .restore(snapshot.snapshot().clone())
            .map_err(|e| anyhow::anyhow!("restore failed: {e}"))?;
        self.sandbox.drain_output();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{allowlist_host, exit_status, strip_stderr_lines, uploaded};

    #[test]
    fn exit_statuses_outside_a_byte_are_failures() {
        assert_eq!(exit_status(3), 3);
        assert_eq!(exit_status(255), 255);
        assert_eq!(exit_status(-1), 1);
        assert_eq!(exit_status(256), 1);
    }

    #[test]
    fn only_an_upload_marker_names_an_upload() {
        assert_eq!(uploaded(r#"[{"upload": 7}]"#), Some(7));
        assert_eq!(uploaded(r#"["echo", {"upload": 7}]"#), None);
        assert_eq!(uploaded(r#"[{"upload": 7, "x": 1}]"#), None);
        assert_eq!(uploaded(r#"[{"upload": "7"}]"#), None);
    }

    #[test]
    fn stderr_lines_are_taken_out_of_the_output() {
        let output = "a\n[1] ERR: x\nb\n[2] ERR: y\nc";
        let stderr = "[1] ERR: x\n[2] ERR: y\n";
        assert_eq!(
            strip_stderr_lines(output, stderr).as_deref(),
            Some("a\nb\nc")
        );
        assert_eq!(strip_stderr_lines("a\n", "").as_deref(), Some("a\n"));
    }

    #[test]
    fn stderr_not_in_whole_lines_leaves_the_output_alone() {
        // A partial stderr write joined a stdout line: no clean split.
        assert_eq!(strip_stderr_lines("out err\n", "err\n"), None);
    }

    #[test]
    fn allowlist_hosts_have_no_ipv6_brackets() {
        let host = |url: &str| allowlist_host(&url::Url::parse(url).unwrap().host().unwrap());
        assert_eq!(
            host("https://[2606:4700:4700::1111]/"),
            "2606:4700:4700::1111"
        );
        assert_eq!(host("http://[::ffff:1.2.3.4]/"), "1.2.3.4");
        assert_eq!(host("http://127.0.0.1:8080"), "127.0.0.1");
        assert_eq!(host("https://example.com"), "example.com");
    }
}
