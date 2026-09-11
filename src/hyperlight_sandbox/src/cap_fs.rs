//! Capability-based filesystem shared across sandbox backends.
//!
//! Design and permission model (`DirPerms`, `FilePerms`, `Dir` wrapper)
//! inspired by [wasmtime's WASI filesystem implementation][wt].
//!
//! [wt]: https://github.com/bytecodealliance/wasmtime/blob/main/crates/wasi/src/filesystem.rs
//!
//! * **Input** — host-provided directory, exposed to the guest as a read-only
//!   WASI preopen (`DirPerms::READ`, `FilePerms::READ`).  The host populates
//!   the directory before creating the sandbox; the guest can only read.
//!
//! * **Output** — default temp directory or host-provided with explicit
//!   permissions, exposed as a writable WASI preopen.  Wiped clean after
//!   each run. Logical output size and file count are bounded by
//!   [`FilesystemLimits`] before host filesystem mutation.
//!
//! Snapshots only capture runtime state — input is immutable and output is
//! ephemeral, so no filesystem state needs to be saved or restored.

use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use cap_std::ambient_authority;
use cap_std::fs::Dir as CapDir;

/// First preopen fd (after 0–2 stdio).
const FIRST_PREOPEN_FD: u32 = 3;

// ---------------------------------------------------------------------------
// Filesystem errors
// ---------------------------------------------------------------------------

/// Errors returned by [`CapFs`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    BadDescriptor,
    NotPermitted,
    NoEntry,
    InvalidPath,
    FileTooLarge,
    QuotaExceeded,
    Overflow,
    Io(String),
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadDescriptor => f.write_str("bad file descriptor"),
            Self::NotPermitted => f.write_str("operation not permitted"),
            Self::NoEntry => f.write_str("file or directory not found"),
            Self::InvalidPath => f.write_str("invalid path"),
            Self::FileTooLarge => f.write_str("file exceeds logical size limit"),
            Self::QuotaExceeded => f.write_str("filesystem quota exceeded"),
            Self::Overflow => f.write_str("filesystem offset overflow"),
            Self::Io(message) => write!(f, "filesystem I/O error: {message}"),
        }
    }
}

impl std::error::Error for FsError {}

// ---------------------------------------------------------------------------
// Filesystem limits
// ---------------------------------------------------------------------------

pub const DEFAULT_MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;
pub const DEFAULT_MAX_TOTAL_SIZE: u64 = 20 * 1024 * 1024;
pub const DEFAULT_MAX_FILE_COUNT: usize = 20;

/// Logical resource limits for the writable filesystem.
///
/// Logical size includes sparse holes. These limits do not measure physical
/// blocks, filesystem metadata, compression, or deduplication; deployments
/// that require physical resource isolation should also use host filesystem or
/// container quotas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilesystemLimits {
    max_file_size: Option<u64>,
    max_total_size: Option<u64>,
    max_file_count: Option<usize>,
}

impl FilesystemLimits {
    pub fn new(max_file_size: u64, max_total_size: u64, max_file_count: usize) -> Result<Self> {
        if max_file_size > i64::MAX as u64 {
            anyhow::bail!("max_file_size must not exceed {}", i64::MAX);
        }
        Ok(Self {
            max_file_size: Some(max_file_size),
            max_total_size: Some(max_total_size),
            max_file_count: Some(max_file_count),
        })
    }

    pub const fn unlimited() -> Self {
        Self {
            max_file_size: None,
            max_total_size: None,
            max_file_count: None,
        }
    }

    pub const fn max_file_size(self) -> Option<u64> {
        self.max_file_size
    }

    pub const fn max_total_size(self) -> Option<u64> {
        self.max_total_size
    }

    pub const fn max_file_count(self) -> Option<usize> {
        self.max_file_count
    }

    fn is_unlimited(self) -> bool {
        self.max_file_size.is_none()
            && self.max_total_size.is_none()
            && self.max_file_count.is_none()
    }
}

impl Default for FilesystemLimits {
    fn default() -> Self {
        Self {
            max_file_size: Some(DEFAULT_MAX_FILE_SIZE),
            max_total_size: Some(DEFAULT_MAX_TOTAL_SIZE),
            max_file_count: Some(DEFAULT_MAX_FILE_COUNT),
        }
    }
}

// ---------------------------------------------------------------------------
// Metadata types
// ---------------------------------------------------------------------------

/// Type of a filesystem descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorType {
    Directory,
    RegularFile,
}

/// Metadata about a file or directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorStat {
    pub descriptor_type: DescriptorType,
    pub size: u64,
}

/// Effective permissions reported for a descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorFlags {
    pub read: bool,
    pub write: bool,
    pub mutate_directory: bool,
}

// ---------------------------------------------------------------------------
// Permission types (following wasmtime's cap-std pattern)
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DirPerms: u8 {
        const READ = 0b01;
        const MUTATE = 0b10;
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct FilePerms: u8 {
        const READ = 0b01;
        const WRITE = 0b10;
    }
}

bitflags::bitflags! {
    /// Flags controlling how [`CapFs::open_at`] opens a file.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct OpenFlags: u8 {
        /// Open an existing file without modification.
        const OPEN_EXISTING = 0;
        /// Create the file if it doesn't exist.
        const CREATE = 0b01;
        /// Truncate the file to zero length.
        const TRUNCATE = 0b10;
    }
}

// ---------------------------------------------------------------------------
// Capability-wrapped directory
// ---------------------------------------------------------------------------

/// A capability-wrapped directory handle with explicit permissions.
#[derive(Clone)]
pub struct Dir {
    dir: Arc<CapDir>,
    perms: DirPerms,
    file_perms: FilePerms,
}

impl Dir {
    pub fn new(dir: CapDir, perms: DirPerms, file_perms: FilePerms) -> Self {
        Self {
            dir: Arc::new(dir),
            perms,
            file_perms,
        }
    }

    pub fn cap_std(&self) -> &CapDir {
        &self.dir
    }

    pub fn perms(&self) -> DirPerms {
        self.perms
    }

    pub fn file_perms(&self) -> FilePerms {
        self.file_perms
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FileEntry {
    name: String,
    dir_fd: u32,
}

#[derive(Clone)]
struct StreamState {
    file_fd: u32,
    offset: u64,
    is_write: bool,
    append: bool,
}

#[derive(Clone)]
struct DirStreamState {
    entries: Vec<(String, bool)>,
    cursor: usize,
}

// ---------------------------------------------------------------------------
// CapFs
// ---------------------------------------------------------------------------

/// A preopened directory registered with the filesystem.
#[derive(Clone)]
struct PreopenEntry {
    dir: Dir,
    guest_path: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct FilesystemUsage {
    logical_bytes: u64,
    file_count: usize,
}

/// Capability-based virtual filesystem.
///
/// Input is read-only and shared across snapshots; output is ephemeral.
pub struct CapFs {
    // Preopened directories keyed by fd.
    preopen_dirs: HashMap<u32, PreopenEntry>,
    // The fd assigned to the output preopen (if configured).
    output_fd: Option<u32>,

    open_files: HashMap<u32, FileEntry>,
    streams: HashMap<u32, StreamState>,
    dir_streams: HashMap<u32, DirStreamState>,
    next_handle: u32,

    // Host filesystem path to the output directory.
    output_path: Option<PathBuf>,
    // Owns the output temp dir when using the default.
    _output_tmp: Option<tempfile::TempDir>,
    limits: FilesystemLimits,
    output_usage: FilesystemUsage,
    accounting_valid: bool,
}

impl Default for CapFs {
    fn default() -> Self {
        Self::new()
    }
}

impl CapFs {
    /// Create an empty filesystem with no preopens.
    ///
    /// Use [`with_input`], [`with_temp_output`], or [`with_output_dir`] to add
    /// directories. Without any preopens the guest sees no filesystem. Writable
    /// output uses [`FilesystemLimits::default`].
    pub fn new() -> Self {
        Self::with_limits(FilesystemLimits::default())
    }

    /// Create an empty filesystem with an explicit writable-output policy.
    pub fn with_limits(limits: FilesystemLimits) -> Self {
        Self {
            preopen_dirs: HashMap::new(),
            output_fd: None,
            open_files: HashMap::new(),
            streams: HashMap::new(),
            dir_streams: HashMap::new(),
            next_handle: FIRST_PREOPEN_FD,
            output_path: None,
            _output_tmp: None,
            limits,
            output_usage: FilesystemUsage::default(),
            accounting_valid: true,
        }
    }

    pub fn filesystem_limits(&self) -> FilesystemLimits {
        self.limits
    }

    pub fn set_filesystem_limits(&mut self, limits: FilesystemLimits) -> Result<()> {
        if self.output_fd.is_some() {
            anyhow::bail!("filesystem limits must be configured before the output directory");
        }
        self.limits = limits;
        Ok(())
    }

    /// Add a read-only input directory preopen (`/input`).
    pub fn with_input(mut self, input_path: impl AsRef<Path>) -> Result<Self> {
        let input_cap = CapDir::open_ambient_dir(input_path.as_ref(), ambient_authority())
            .with_context(|| {
                format!(
                    "failed to open input dir: {}",
                    input_path.as_ref().display()
                )
            })?;
        let fd = self
            .alloc_handle()
            .map_err(|err| anyhow::anyhow!("{err:?}"))?;
        self.preopen_dirs.insert(
            fd,
            PreopenEntry {
                dir: Dir::new(input_cap, DirPerms::READ, FilePerms::READ),
                guest_path: "/input".to_string(),
            },
        );
        Ok(self)
    }

    /// Add a writable output directory preopen (`/output`) backed by a temp dir.
    pub fn with_temp_output(mut self) -> Result<Self> {
        if self.output_fd.is_some() {
            anyhow::bail!("output directory is already configured");
        }
        let output_tmp = tempfile::tempdir().context("failed to create output temp dir")?;
        let output_cap = CapDir::open_ambient_dir(output_tmp.path(), ambient_authority())
            .context("failed to open output temp dir")?;
        let fd = self
            .alloc_handle()
            .map_err(|err| anyhow::anyhow!("{err:?}"))?;
        self.preopen_dirs.insert(
            fd,
            PreopenEntry {
                dir: Dir::new(
                    output_cap,
                    DirPerms::READ | DirPerms::MUTATE,
                    FilePerms::READ | FilePerms::WRITE,
                ),
                guest_path: "/output".to_string(),
            },
        );
        self.output_fd = Some(fd);
        self.output_path = Some(output_tmp.path().to_path_buf());
        self._output_tmp = Some(output_tmp);
        self.resynchronize_output()?;
        Ok(self)
    }

    /// Add a writable output directory preopen (`/output`) at a caller-provided path.
    pub fn with_output_dir(
        mut self,
        output_path: impl AsRef<Path>,
        dir_perms: DirPerms,
        file_perms: FilePerms,
    ) -> Result<Self> {
        if self.output_fd.is_some() {
            anyhow::bail!("output directory is already configured");
        }
        let output_cap = CapDir::open_ambient_dir(output_path.as_ref(), ambient_authority())
            .with_context(|| {
                format!(
                    "failed to open output dir: {}",
                    output_path.as_ref().display()
                )
            })?;
        let fd = self
            .alloc_handle()
            .map_err(|err| anyhow::anyhow!("{err:?}"))?;
        self.preopen_dirs.insert(
            fd,
            PreopenEntry {
                dir: Dir::new(output_cap, dir_perms, file_perms),
                guest_path: "/output".to_string(),
            },
        );
        self.output_fd = Some(fd);
        self.output_path = Some(output_path.as_ref().to_path_buf());
        self.resynchronize_output()?;
        Ok(self)
    }

    // -----------------------------------------------------------------------
    // Host-side output API
    // -----------------------------------------------------------------------

    pub fn write_output_path(&mut self, path: &str, data: Vec<u8>) -> Result<()> {
        self.ensure_accounting_valid()
            .map_err(|error| anyhow::anyhow!(error))?;
        let key = Self::normalize_path(path, "output")?;
        let output_fd = self
            .output_fd
            .ok_or_else(|| anyhow::anyhow!("no output directory configured"))?;
        let output = &self.preopen_dirs[&output_fd];
        if !output.dir.perms().contains(DirPerms::MUTATE) {
            anyhow::bail!("write permission denied on output directory");
        }
        if !output.dir.file_perms().contains(FilePerms::WRITE) {
            anyhow::bail!("write permission denied on output files");
        }
        let metadata = match output.dir.cap_std().symlink_metadata(&key) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "failed to inspect output file {key}: {error}"
                ));
            }
        };
        if metadata.as_ref().is_some_and(|meta| !meta.is_file()) {
            anyhow::bail!("output path is not a regular file: {key}");
        }
        let existed = metadata.as_ref().is_some_and(|meta| meta.is_file());
        let old_size = metadata.as_ref().map_or(0, |meta| meta.len());
        let new_size = u64::try_from(data.len()).map_err(|_| FsError::Overflow)?;
        self.validate_mutation(old_size, new_size, !existed)
            .map_err(|error| anyhow::anyhow!(error))?;

        let mut file = match output.dir.cap_std().create(&key) {
            Ok(file) => file,
            Err(error) => {
                let reconcile_error = self.resynchronize_output().err();
                return Err(match reconcile_error {
                    Some(reconcile_error) => anyhow::anyhow!(
                        "failed to create output file {key}: {error}; failed to reconcile quota: {reconcile_error}"
                    ),
                    None => anyhow::anyhow!("failed to create output file {key}: {error}"),
                });
            }
        };
        if let Err(error) = file.write_all(&data) {
            let reconcile_error = self.resynchronize_output().err();
            return Err(match reconcile_error {
                Some(reconcile_error) => anyhow::anyhow!(
                    "failed to write output file {key}: {error}; failed to reconcile quota: {reconcile_error}"
                ),
                None => anyhow::anyhow!("failed to write output file {key}: {error}"),
            });
        }
        self.apply_mutation(old_size, new_size, !existed)
            .map_err(|error| anyhow::anyhow!(error))?;
        Ok(())
    }

    /// Maximum size (in bytes) for a single file read into memory.
    const MAX_FILE_READ_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB

    /// Read a file from a preopened directory by guest path (e.g. "/input/data.txt").
    pub fn read_guest_file(&self, guest_path: &str) -> Result<Vec<u8>> {
        // Determine which preopen and filename from the path.
        let trimmed = guest_path.trim().trim_start_matches('/');
        let (dir_name, file_name) = trimmed.split_once('/').ok_or_else(|| {
            anyhow::anyhow!("path must include a directory and filename: {guest_path}")
        })?;
        let preopen_path = format!("/{dir_name}");
        let dir = self
            .dir_by_guest_path(&preopen_path)
            .ok_or_else(|| anyhow::anyhow!("no preopen for {preopen_path}"))?;
        if !dir.perms().contains(DirPerms::READ) {
            anyhow::bail!("read permission denied on {preopen_path}");
        }
        if !dir.file_perms().contains(FilePerms::READ) {
            anyhow::bail!("read permission denied on files in {preopen_path}");
        }
        let file = dir
            .cap_std()
            .open(file_name)
            .map_err(|_| anyhow::anyhow!("file not found: {guest_path}"))?;
        let metadata = file
            .metadata()
            .with_context(|| format!("failed to stat: {guest_path}"))?;
        if metadata.len() > Self::MAX_FILE_READ_BYTES {
            anyhow::bail!(
                "file too large ({} bytes, max {} bytes): {guest_path}",
                metadata.len(),
                Self::MAX_FILE_READ_BYTES,
            );
        }
        let mut data = Vec::new();
        file.take(Self::MAX_FILE_READ_BYTES)
            .read_to_end(&mut data)
            .with_context(|| format!("failed to read: {guest_path}"))?;
        Ok(data)
    }

    /// Return the host filesystem path of the output directory, if configured.
    pub fn output_path(&self) -> Option<&Path> {
        self.output_path.as_deref()
    }

    /// Maximum number of output files returned by `get_output_files()`.
    const MAX_OUTPUT_FILE_COUNT: usize = 10_000;

    /// List filenames in the output directory (without reading file contents).
    pub fn get_output_files(&self) -> Vec<String> {
        let mut result = Vec::new();
        let Some(output_fd) = self.output_fd else {
            return result;
        };
        let Some(output) = self.preopen_dirs.get(&output_fd) else {
            return result;
        };
        let dir = output.dir.cap_std();
        if let Ok(entries) = dir.entries() {
            for entry in entries.flatten() {
                if result.len() >= Self::MAX_OUTPUT_FILE_COUNT {
                    log::warn!(
                        "get_output_files: truncated at {} entries",
                        Self::MAX_OUTPUT_FILE_COUNT
                    );
                    break;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                if meta.is_file() {
                    result.push(name);
                }
            }
        }
        result
    }

    /// Wipe output files and reset all handles/streams. Input is untouched.
    pub fn clear_output_files(&mut self) -> Result<()> {
        let Some(output_fd) = self.output_fd else {
            return Ok(());
        };
        let Some(output) = self.preopen_dirs.get(&output_fd).cloned() else {
            return Ok(());
        };
        self.reset_output_handles(output_fd);
        if let Err(error) = Self::remove_output_files(output.dir.cap_std()) {
            let reconcile_error = self.resynchronize_output().err();
            return Err(match reconcile_error {
                Some(reconcile_error) => anyhow::anyhow!(
                    "failed to clear output files: {error}; failed to reconcile quota: {reconcile_error}"
                ),
                None => error,
            });
        }
        self.resynchronize_output()?;
        Ok(())
    }

    /// Clear output directory. Input is host-managed and left untouched.
    pub fn clear(&mut self) -> Result<()> {
        self.clear_output_files()
    }

    /// Prepare the writable filesystem for a new guest execution.
    pub fn prepare_for_run(&mut self) -> Result<()> {
        self.clear_output_files()
    }

    // -----------------------------------------------------------------------
    // Descriptor queries
    // -----------------------------------------------------------------------

    pub fn get_dir(&self, fd: u32) -> Option<&Dir> {
        self.preopen_dirs.get(&fd).map(|e| &e.dir)
    }

    pub fn is_directory(&self, fd: u32) -> bool {
        self.preopen_dirs.contains_key(&fd)
    }

    /// Find a preopened directory by its guest-visible path.
    pub fn dir_by_guest_path(&self, guest_path: &str) -> Option<&Dir> {
        self.preopen_dirs
            .values()
            .find(|e| e.guest_path == guest_path)
            .map(|e| &e.dir)
    }

    pub fn is_file(&self, fd: u32) -> bool {
        self.open_files.contains_key(&fd)
    }

    pub fn file_size(&self, fd: u32) -> Option<u64> {
        let entry = self.open_files.get(&fd)?;
        let dir = self.get_dir(entry.dir_fd)?;
        Some(dir.cap_std().metadata(&entry.name).ok()?.len())
    }

    pub fn find_file_in_dir(&self, dir_fd: u32, name: &str) -> Option<u32> {
        for (&fd, entry) in &self.open_files {
            if entry.dir_fd == dir_fd && entry.name == name {
                return Some(fd);
            }
        }
        None
    }

    /// Return the parent directory fd for an open file.
    pub fn file_dir_fd(&self, fd: u32) -> Option<u32> {
        self.open_files.get(&fd).map(|e| e.dir_fd)
    }

    /// Check if a file's parent directory grants the given file permissions.
    pub fn file_has_perms(&self, fd: u32, perms: FilePerms) -> bool {
        self.open_files
            .get(&fd)
            .and_then(|e| self.get_dir(e.dir_fd))
            .is_some_and(|dir| dir.file_perms().contains(perms))
    }

    /// Get the type of a descriptor (directory or file).
    pub fn get_type(&self, fd: u32) -> Result<DescriptorType, FsError> {
        if self.is_directory(fd) {
            Ok(DescriptorType::Directory)
        } else if self.is_file(fd) {
            Ok(DescriptorType::RegularFile)
        } else {
            Err(FsError::BadDescriptor)
        }
    }

    /// Get metadata for an open descriptor.
    pub fn stat(&self, fd: u32) -> Result<DescriptorStat, FsError> {
        if self.is_directory(fd) {
            return Ok(DescriptorStat {
                descriptor_type: DescriptorType::Directory,
                size: 0,
            });
        }
        if let Some(size) = self.file_size(fd) {
            return Ok(DescriptorStat {
                descriptor_type: DescriptorType::RegularFile,
                size,
            });
        }
        Err(FsError::BadDescriptor)
    }

    /// Get metadata for a path relative to a directory descriptor.
    pub fn stat_at(&self, dir_fd: u32, path: &str) -> Result<DescriptorStat, FsError> {
        let dir = self.get_dir(dir_fd).ok_or(FsError::BadDescriptor)?;
        if !dir.perms().contains(DirPerms::READ) {
            return Err(FsError::NotPermitted);
        }
        let metadata = dir.cap_std().metadata(path).map_err(|_| FsError::NoEntry)?;
        let descriptor_type = if metadata.is_dir() {
            DescriptorType::Directory
        } else {
            DescriptorType::RegularFile
        };
        Ok(DescriptorStat {
            descriptor_type,
            size: metadata.len(),
        })
    }

    /// Get the effective permission flags for a descriptor.
    pub fn get_flags(&self, fd: u32) -> Result<DescriptorFlags, FsError> {
        if self.is_directory(fd) {
            let dir = self.get_dir(fd).ok_or(FsError::BadDescriptor)?;
            Ok(DescriptorFlags {
                read: dir.perms().contains(DirPerms::READ),
                write: dir.perms().contains(DirPerms::MUTATE),
                mutate_directory: dir.perms().contains(DirPerms::MUTATE),
            })
        } else if self.is_file(fd) {
            let dir_fd = self.file_dir_fd(fd).ok_or(FsError::BadDescriptor)?;
            let dir = self.get_dir(dir_fd).ok_or(FsError::BadDescriptor)?;
            Ok(DescriptorFlags {
                read: dir.file_perms().contains(FilePerms::READ),
                write: dir.file_perms().contains(FilePerms::WRITE),
                mutate_directory: false,
            })
        } else {
            Err(FsError::BadDescriptor)
        }
    }

    // -----------------------------------------------------------------------
    // File operations (cap-std backed)
    // -----------------------------------------------------------------------

    /// Maximum number of open file handles to prevent resource exhaustion.
    const MAX_OPEN_FILES: usize = 1024;

    /// Cap single read allocation to prevent guest-triggered OOM.
    const MAX_READ_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

    pub fn open_at(&mut self, dir_fd: u32, path: &str, flags: OpenFlags) -> Result<u32, FsError> {
        let dir = self
            .get_dir(dir_fd)
            .cloned()
            .ok_or(FsError::BadDescriptor)?;

        if path.is_empty()
            || path == "."
            || path == ".."
            || path.contains('/')
            || path.contains('\\')
            || path.contains('\0')
            || (cfg!(windows) && path.contains(':'))
        {
            return Err(FsError::InvalidPath);
        }

        let create = flags.contains(OpenFlags::CREATE);
        let truncate = flags.contains(OpenFlags::TRUNCATE);

        if (create || truncate) && !dir.perms().contains(DirPerms::MUTATE) {
            return Err(FsError::NotPermitted);
        }

        if let Some(fd) = self.find_file_in_dir(dir_fd, path) {
            if truncate {
                if self.output_fd == Some(dir_fd) {
                    self.ensure_accounting_valid()?;
                }
                let old_size = dir
                    .cap_std()
                    .metadata(path)
                    .map_err(|error| FsError::Io(error.to_string()))?
                    .len();
                if let Err(error) = dir.cap_std().create(path) {
                    if self.output_fd == Some(dir_fd) {
                        let _ = self.resynchronize_output();
                    }
                    return Err(FsError::Io(error.to_string()));
                }
                if self.output_fd == Some(dir_fd) {
                    self.apply_mutation(old_size, 0, false)?;
                }
            }
            return Ok(fd);
        }

        let exists = dir.cap_std().metadata(path).is_ok();
        if !exists && !create {
            return Err(FsError::NoEntry);
        }
        if self.open_files.len() >= Self::MAX_OPEN_FILES {
            return Err(FsError::Io("too many open files".into()));
        }
        let old_size = if exists {
            dir.cap_std()
                .metadata(path)
                .map_err(|error| FsError::Io(error.to_string()))?
                .len()
        } else {
            0
        };
        if self.output_fd == Some(dir_fd) {
            self.ensure_accounting_valid()?;
            self.validate_mutation(old_size, if truncate { 0 } else { old_size }, !exists)?;
        }
        if !exists || truncate {
            if let Err(error) = dir.cap_std().create(path) {
                if self.output_fd == Some(dir_fd) {
                    let _ = self.resynchronize_output();
                }
                return Err(FsError::Io(error.to_string()));
            }
            if self.output_fd == Some(dir_fd) {
                self.apply_mutation(old_size, 0, !exists)?;
            }
        }

        let fd = self.alloc_handle()?;
        self.open_files.insert(
            fd,
            FileEntry {
                name: path.to_string(),
                dir_fd,
            },
        );
        Ok(fd)
    }

    /// Close an open file handle, freeing the descriptor.
    pub fn close_file(&mut self, fd: u32) {
        self.open_files.remove(&fd);
        self.streams.retain(|_, stream| stream.file_fd != fd);
    }

    /// Close a stream handle, freeing the descriptor.
    pub fn close_stream(&mut self, stream_id: u32) {
        self.streams.remove(&stream_id);
    }

    /// Close a directory stream handle, freeing the descriptor.
    pub fn close_dir_stream(&mut self, stream_id: u32) {
        self.dir_streams.remove(&stream_id);
    }

    pub fn read_file(&self, fd: u32, offset: u64, len: u64) -> Result<(Vec<u8>, bool), FsError> {
        let entry = self.open_files.get(&fd).ok_or(FsError::BadDescriptor)?;
        let dir = self.get_dir(entry.dir_fd).ok_or(FsError::BadDescriptor)?;
        if !dir.file_perms().contains(FilePerms::READ) {
            return Err(FsError::NotPermitted);
        }
        let mut file = dir
            .cap_std()
            .open(&entry.name)
            .map_err(|e| FsError::Io(e.to_string()))?;
        let file_size = file
            .metadata()
            .map_err(|e| FsError::Io(e.to_string()))?
            .len();

        let start = offset.min(file_size);
        let remaining = file_size - start;
        let to_read = len.min(remaining).min(Self::MAX_READ_BYTES as u64) as usize;
        if to_read == 0 {
            return Ok((Vec::new(), true));
        }
        file.seek(SeekFrom::Start(start))
            .map_err(|e| FsError::Io(e.to_string()))?;
        let mut buf = vec![0u8; to_read];
        let n = file
            .read(&mut buf)
            .map_err(|e| FsError::Io(e.to_string()))?;
        buf.truncate(n);
        let eof = start + n as u64 >= file_size;
        Ok((buf, eof))
    }

    pub fn write_file(&mut self, fd: u32, offset: u64, buffer: &[u8]) -> Result<u64, FsError> {
        let entry = self
            .open_files
            .get(&fd)
            .cloned()
            .ok_or(FsError::BadDescriptor)?;
        let dir = self
            .get_dir(entry.dir_fd)
            .cloned()
            .ok_or(FsError::BadDescriptor)?;
        if !dir.file_perms().contains(FilePerms::WRITE) {
            return Err(FsError::NotPermitted);
        }
        let mut opts = cap_std::fs::OpenOptions::new();
        opts.read(true).write(true);
        let mut file = dir
            .cap_std()
            .open_with(&entry.name, &opts)
            .map_err(|e| FsError::Io(e.to_string()))?;
        if buffer.is_empty() {
            return Ok(0);
        }

        let file_size = file
            .metadata()
            .map_err(|e| FsError::Io(e.to_string()))?
            .len();
        let new_end = offset
            .checked_add(u64::try_from(buffer.len()).map_err(|_| FsError::Overflow)?)
            .ok_or(FsError::Overflow)?;
        let prospective_size = file_size.max(new_end);
        if self.output_fd == Some(entry.dir_fd) {
            self.ensure_accounting_valid()?;
            self.validate_mutation(file_size, prospective_size, false)?;
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| FsError::Io(e.to_string()))?;
        if let Err(error) = file.write_all(buffer) {
            if self.output_fd == Some(entry.dir_fd) {
                let _ = self.resynchronize_output();
            }
            return Err(FsError::Io(error.to_string()));
        }
        if self.output_fd == Some(entry.dir_fd) {
            self.apply_mutation(file_size, prospective_size, false)?;
        }
        u64::try_from(buffer.len()).map_err(|_| FsError::Overflow)
    }

    // -----------------------------------------------------------------------
    // Streams
    // -----------------------------------------------------------------------

    pub fn create_read_stream(&mut self, file_fd: u32, offset: u64) -> Result<u32, FsError> {
        if !self.is_file(file_fd) {
            return Err(FsError::BadDescriptor);
        }
        if !self.file_has_perms(file_fd, FilePerms::READ) {
            return Err(FsError::NotPermitted);
        }
        let id = self.alloc_handle()?;
        self.streams.insert(
            id,
            StreamState {
                file_fd,
                offset,
                is_write: false,
                append: false,
            },
        );
        Ok(id)
    }

    pub fn create_write_stream(&mut self, file_fd: u32, offset: u64) -> Result<u32, FsError> {
        if !self.is_file(file_fd) {
            return Err(FsError::BadDescriptor);
        }
        if !self.file_has_perms(file_fd, FilePerms::WRITE) {
            return Err(FsError::NotPermitted);
        }
        let id = self.alloc_handle()?;
        self.streams.insert(
            id,
            StreamState {
                file_fd,
                offset,
                is_write: true,
                append: false,
            },
        );
        Ok(id)
    }

    pub fn create_append_stream(&mut self, file_fd: u32) -> Result<u32, FsError> {
        if !self.is_file(file_fd) {
            return Err(FsError::BadDescriptor);
        }
        if !self.file_has_perms(file_fd, FilePerms::WRITE) {
            return Err(FsError::NotPermitted);
        }
        let id = self.alloc_handle()?;
        self.streams.insert(
            id,
            StreamState {
                file_fd,
                offset: 0,
                is_write: true,
                append: true,
            },
        );
        Ok(id)
    }

    pub fn stream_read(&mut self, stream_id: u32, len: u64) -> Result<Vec<u8>, FsError> {
        let stream = self.streams.get(&stream_id).ok_or(FsError::BadDescriptor)?;
        let file_fd = stream.file_fd;
        let offset = stream.offset;

        let entry = self
            .open_files
            .get(&file_fd)
            .ok_or(FsError::BadDescriptor)?;
        let dir = self.get_dir(entry.dir_fd).ok_or(FsError::BadDescriptor)?;
        let mut file = dir
            .cap_std()
            .open(&entry.name)
            .map_err(|e| FsError::Io(e.to_string()))?;
        let file_size = file
            .metadata()
            .map_err(|e| FsError::Io(e.to_string()))?
            .len();
        if offset >= file_size {
            return Err(FsError::Io("stream read past end of file".into()));
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| FsError::Io(e.to_string()))?;
        let remaining = file_size - offset;
        let to_read = len.min(remaining).min(Self::MAX_READ_BYTES as u64) as usize;
        let mut buf = vec![0u8; to_read];
        let n = file
            .read(&mut buf)
            .map_err(|e| FsError::Io(e.to_string()))?;
        buf.truncate(n);

        if buf.is_empty() {
            return Err(FsError::Io("stream read returned no data".into()));
        }
        let stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or(FsError::BadDescriptor)?;
        stream.offset = stream.offset.saturating_add(buf.len() as u64);
        Ok(buf)
    }

    pub fn stream_write(&mut self, stream_id: u32, buffer: &[u8]) -> Result<u64, FsError> {
        let stream = self.streams.get(&stream_id).ok_or(FsError::BadDescriptor)?;
        let file_fd = stream.file_fd;
        let offset = if stream.append {
            self.file_size(file_fd).ok_or(FsError::BadDescriptor)?
        } else {
            stream.offset
        };
        let written = self.write_file(file_fd, offset, buffer)?;
        let stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or(FsError::BadDescriptor)?;
        stream.offset = offset.checked_add(written).ok_or(FsError::Overflow)?;
        Ok(written)
    }

    pub fn has_stream(&self, id: u32) -> bool {
        self.streams.contains_key(&id)
    }
    pub fn is_write_stream(&self, id: u32) -> bool {
        self.streams.get(&id).is_some_and(|s| s.is_write)
    }

    // -----------------------------------------------------------------------
    // Directory streams
    // -----------------------------------------------------------------------

    /// Maximum number of entries materialized from a single directory listing.
    const MAX_DIR_ENTRIES: usize = 10_000;

    pub fn create_dir_stream(&mut self, dir_fd: u32) -> Result<u32, FsError> {
        let dir = self.get_dir(dir_fd).ok_or(FsError::BadDescriptor)?;
        if !dir.perms().contains(DirPerms::READ) {
            return Err(FsError::NotPermitted);
        }
        let mut file_entries = Vec::new();
        if let Ok(entries) = dir.cap_std().entries() {
            for entry in entries.flatten() {
                if file_entries.len() >= Self::MAX_DIR_ENTRIES {
                    return Err(FsError::Io("directory listing exceeds entry limit".into()));
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().is_ok_and(|ft| ft.is_dir());
                file_entries.push((name, is_dir));
            }
        }
        let id = self.alloc_handle()?;
        self.dir_streams.insert(
            id,
            DirStreamState {
                entries: file_entries,
                cursor: 0,
            },
        );
        Ok(id)
    }

    pub fn read_dir_entry(&mut self, stream_id: u32) -> Option<Option<(String, bool)>> {
        let stream = self.dir_streams.get_mut(&stream_id)?;
        if stream.cursor >= stream.entries.len() {
            return Some(None);
        }
        let entry = stream.entries[stream.cursor].clone();
        stream.cursor += 1;
        Some(Some(entry))
    }

    pub fn has_dir_stream(&self, id: u32) -> bool {
        self.dir_streams.contains_key(&id)
    }

    /// Return the WASI preopens — derived from the registered directories.
    pub fn preopens(&self) -> Vec<(u32, &str)> {
        self.preopen_dirs
            .iter()
            .map(|(&fd, e)| (fd, e.guest_path.as_str()))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    fn normalize_path(path: &str, root: &str) -> Result<String> {
        let trimmed = path.trim();
        let without_leading = trimmed.trim_start_matches('/');
        let Some(remainder) = without_leading.strip_prefix(root) else {
            return Err(anyhow::anyhow!("path must be rooted at /{root}: {trimmed}"));
        };
        let Some(normalized) = remainder.strip_prefix('/') else {
            return Err(anyhow::anyhow!(
                "path must include a file name under /{root}: {trimmed}"
            ));
        };
        if normalized.is_empty() {
            return Err(anyhow::anyhow!(
                "path must include a file name under /{root}: {trimmed}"
            ));
        }
        for component in normalized.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return Err(anyhow::anyhow!("path contains unsafe component: {trimmed}"));
            }
            if component.contains('\\') {
                return Err(anyhow::anyhow!(
                    "path contains unsupported separator: {trimmed}"
                ));
            }
            if cfg!(windows) && component.contains(':') {
                return Err(anyhow::anyhow!(
                    "path contains unsupported Windows stream separator: {trimmed}"
                ));
            }
        }
        Ok(normalized.to_string())
    }

    fn ensure_accounting_valid(&self) -> Result<(), FsError> {
        if self.accounting_valid {
            Ok(())
        } else {
            Err(FsError::Io(
                "filesystem quota accounting requires resynchronization".into(),
            ))
        }
    }

    fn validate_mutation(
        &self,
        old_size: u64,
        new_size: u64,
        creates_file: bool,
    ) -> Result<(), FsError> {
        self.ensure_accounting_valid()?;
        if self.limits.is_unlimited() {
            return Ok(());
        }
        if let Some(max_file_size) = self.limits.max_file_size
            && new_size > max_file_size
        {
            return Err(FsError::FileTooLarge);
        }
        let logical_bytes = self
            .output_usage
            .logical_bytes
            .checked_sub(old_size)
            .and_then(|current| current.checked_add(new_size))
            .ok_or(FsError::Overflow)?;
        if let Some(max_total_size) = self.limits.max_total_size
            && logical_bytes > max_total_size
        {
            return Err(FsError::QuotaExceeded);
        }
        let file_count = self
            .output_usage
            .file_count
            .checked_add(usize::from(creates_file))
            .ok_or(FsError::Overflow)?;
        if let Some(max_file_count) = self.limits.max_file_count
            && file_count > max_file_count
        {
            return Err(FsError::QuotaExceeded);
        }
        Ok(())
    }

    fn apply_mutation(
        &mut self,
        old_size: u64,
        new_size: u64,
        creates_file: bool,
    ) -> Result<(), FsError> {
        if self.limits.is_unlimited() {
            return Ok(());
        }
        let Some(logical_bytes) = self
            .output_usage
            .logical_bytes
            .checked_sub(old_size)
            .and_then(|current| current.checked_add(new_size))
        else {
            self.accounting_valid = false;
            return Err(FsError::Overflow);
        };
        self.output_usage.logical_bytes = logical_bytes;
        if creates_file {
            let Some(file_count) = self.output_usage.file_count.checked_add(1) else {
                self.accounting_valid = false;
                return Err(FsError::Overflow);
            };
            self.output_usage.file_count = file_count;
        }
        Ok(())
    }

    fn resynchronize_output(&mut self) -> Result<()> {
        let usage = match self.scan_output_usage() {
            Ok(usage) => usage,
            Err(error) => {
                self.accounting_valid = false;
                return Err(error);
            }
        };
        if let Err(error) = self.validate_scanned_usage(usage) {
            self.accounting_valid = false;
            return Err(anyhow::anyhow!(error));
        }
        self.output_usage = usage;
        self.accounting_valid = true;
        Ok(())
    }

    fn validate_scanned_usage(&self, usage: FilesystemUsage) -> Result<(), FsError> {
        if let Some(max_total_size) = self.limits.max_total_size
            && usage.logical_bytes > max_total_size
        {
            return Err(FsError::QuotaExceeded);
        }
        if let Some(max_file_count) = self.limits.max_file_count
            && usage.file_count > max_file_count
        {
            return Err(FsError::QuotaExceeded);
        }
        Ok(())
    }

    const MAX_SCAN_ENTRIES: usize = 100_000;
    const MAX_SCAN_DEPTH: usize = 64;

    fn scan_output_usage(&self) -> Result<FilesystemUsage> {
        let Some(output_fd) = self.output_fd else {
            return Ok(FilesystemUsage::default());
        };
        let output = self
            .preopen_dirs
            .get(&output_fd)
            .ok_or_else(|| anyhow::anyhow!("output directory capability is missing"))?;
        let mut usage = FilesystemUsage::default();
        let mut visited = 0usize;
        self.scan_output_dir(output.dir.cap_std(), 0, &mut visited, &mut usage)?;
        Ok(usage)
    }

    fn scan_output_dir(
        &self,
        dir: &CapDir,
        depth: usize,
        visited: &mut usize,
        usage: &mut FilesystemUsage,
    ) -> Result<()> {
        if depth > Self::MAX_SCAN_DEPTH {
            anyhow::bail!("output directory exceeds maximum traversal depth");
        }
        for entry in dir.entries().context("failed to scan output directory")? {
            let entry = entry.context("failed to read output directory entry")?;
            *visited = visited
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("output traversal counter overflow"))?;
            if *visited > Self::MAX_SCAN_ENTRIES {
                anyhow::bail!("output directory exceeds traversal entry limit");
            }
            let name = entry.file_name();
            let metadata = dir
                .symlink_metadata(&name)
                .context("failed to read output entry metadata")?;
            if metadata.file_type().is_symlink() {
                anyhow::bail!("symbolic links are not supported in output directories");
            }
            if metadata.is_dir() {
                let child = dir
                    .open_dir(&name)
                    .context("failed to open nested output directory")?;
                self.scan_output_dir(&child, depth + 1, visited, usage)?;
            } else if metadata.is_file() {
                let metadata = Self::validated_output_metadata(dir, &name, metadata)?;
                if let Some(max_file_size) = self.limits.max_file_size
                    && metadata.len() > max_file_size
                {
                    return Err(anyhow::anyhow!(FsError::FileTooLarge));
                }
                if !self.limits.is_unlimited() {
                    usage.logical_bytes = usage
                        .logical_bytes
                        .checked_add(metadata.len())
                        .ok_or(FsError::Overflow)?;
                    usage.file_count = usage.file_count.checked_add(1).ok_or(FsError::Overflow)?;
                }
            }
        }
        Ok(())
    }

    fn remove_output_files(root: &CapDir) -> Result<()> {
        let mut visited = 0usize;
        Self::remove_output_dir_files(root, 0, &mut visited)
    }

    fn remove_output_dir_files(dir: &CapDir, depth: usize, visited: &mut usize) -> Result<()> {
        if depth > Self::MAX_SCAN_DEPTH {
            anyhow::bail!("output directory exceeds maximum traversal depth");
        }
        for entry in dir.entries().context("failed to scan output directory")? {
            let entry = entry.context("failed to read output directory entry")?;
            *visited = visited
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("output traversal counter overflow"))?;
            if *visited > Self::MAX_SCAN_ENTRIES {
                anyhow::bail!("output directory exceeds traversal entry limit");
            }
            let file_type = entry
                .file_type()
                .context("failed to read output entry type")?;
            if file_type.is_dir() && !file_type.is_symlink() {
                let child = dir
                    .open_dir(entry.file_name())
                    .context("failed to open nested output directory")?;
                Self::remove_output_dir_files(&child, depth + 1, visited)?;
            } else {
                dir.remove_file(entry.file_name())
                    .context("failed to remove output file")?;
            }
        }
        Ok(())
    }

    #[cfg(windows)]
    fn validated_output_metadata(
        dir: &CapDir,
        name: &std::ffi::OsStr,
        _metadata: cap_std::fs::Metadata,
    ) -> Result<cap_std::fs::Metadata> {
        use std::mem::MaybeUninit;
        use std::os::windows::io::AsRawHandle;

        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };

        let file = dir
            .open(name)
            .context("failed to open output file for accounting")?;
        let metadata = file
            .metadata()
            .context("failed to read handle-based output metadata")?;
        let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        let result =
            unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, info.as_mut_ptr()) };
        if result == 0 {
            return Err(std::io::Error::last_os_error())
                .context("failed to read output file link count");
        }
        let info = unsafe { info.assume_init() };
        if info.nNumberOfLinks > 1 {
            anyhow::bail!("hard-linked files are not supported in output directories");
        }
        Ok(metadata)
    }

    #[cfg(not(windows))]
    fn validated_output_metadata(
        _dir: &CapDir,
        _name: &std::ffi::OsStr,
        metadata: cap_std::fs::Metadata,
    ) -> Result<cap_std::fs::Metadata> {
        #[cfg(unix)]
        {
            use cap_std::fs::MetadataExt;

            if metadata.nlink() > 1 {
                anyhow::bail!("hard-linked files are not supported in output directories");
            }
        }
        Ok(metadata)
    }

    fn reset_output_handles(&mut self, output_fd: u32) {
        self.open_files.retain(|_, entry| entry.dir_fd != output_fd);
        self.streams.clear();
        self.dir_streams.clear();
        let min_handle = self
            .preopen_dirs
            .keys()
            .copied()
            .max()
            .map(|handle| handle + 1)
            .unwrap_or(FIRST_PREOPEN_FD);
        self.next_handle = self
            .open_files
            .keys()
            .copied()
            .max()
            .map(|handle| handle + 1)
            .unwrap_or(min_handle)
            .max(min_handle);
    }

    fn alloc_handle(&mut self) -> Result<u32, FsError> {
        let id = self.next_handle;
        self.next_handle = self
            .next_handle
            .checked_add(1)
            .ok_or_else(|| FsError::Io("file descriptor handle space exhausted".into()))?;
        Ok(id)
    }
}

// No Clone — snapshots don't need filesystem state. Input is immutable
// (shared via Arc) and output is ephemeral (wiped each run).

#[cfg(test)]
mod tests {
    use super::*;

    fn test_fs() -> (CapFs, tempfile::TempDir, tempfile::TempDir) {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let fs = CapFs::new()
            .with_input(input.path())
            .unwrap()
            .with_output_dir(
                output.path(),
                DirPerms::READ | DirPerms::MUTATE,
                FilePerms::READ | FilePerms::WRITE,
            )
            .unwrap();
        (fs, input, output)
    }

    fn limited_output_fs(
        max_file_size: u64,
        max_total_size: u64,
        max_file_count: usize,
    ) -> (CapFs, tempfile::TempDir) {
        let output = tempfile::tempdir().unwrap();
        let limits = FilesystemLimits::new(max_file_size, max_total_size, max_file_count).unwrap();
        let fs = CapFs::with_limits(limits)
            .with_output_dir(
                output.path(),
                DirPerms::READ | DirPerms::MUTATE,
                FilePerms::READ | FilePerms::WRITE,
            )
            .unwrap();
        (fs, output)
    }

    /// Helper: write a file directly into the input dir (simulates host setup).
    fn host_write_input(input: &tempfile::TempDir, name: &str, data: &[u8]) {
        std::fs::write(input.path().join(name), data).unwrap();
    }

    fn host_write_output(output: &tempfile::TempDir, name: &str, data: &[u8]) {
        std::fs::write(output.path().join(name), data).unwrap();
    }

    /// Look up the fd for a preopen by guest path.
    fn preopen_fd(fs: &CapFs, path: &str) -> u32 {
        fs.preopens()
            .into_iter()
            .find(|(_, p)| *p == path)
            .unwrap()
            .0
    }

    #[test]
    fn input_is_readonly_preopen() {
        let (fs, _i, _o) = test_fs();
        let dir = fs.dir_by_guest_path("/input").unwrap();
        assert!(dir.perms().contains(DirPerms::READ));
        assert!(!dir.perms().contains(DirPerms::MUTATE));
        assert!(dir.file_perms().contains(FilePerms::READ));
        assert!(!dir.file_perms().contains(FilePerms::WRITE));
    }

    #[test]
    fn guest_reads_host_provided_input() {
        let (mut fs, input, _o) = test_fs();
        host_write_input(&input, "test.txt", b"hello world");

        let input_fd = preopen_fd(&fs, "/input");
        let fd = fs
            .open_at(input_fd, "test.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        let (data, _) = fs.read_file(fd, 0, 100).unwrap();
        assert_eq!(data, b"hello world");
    }

    #[test]
    fn output_write_and_collect() {
        let (mut fs, _i, _o) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs.open_at(output_fd, "out.txt", OpenFlags::CREATE).unwrap();
        fs.write_file(fd, 0, b"result").unwrap();
        let files = fs.get_output_files();
        assert!(files.contains(&"out.txt".to_string()));
        let output_dir = fs.output_path().unwrap();
        assert_eq!(
            std::fs::read(output_dir.join("out.txt")).unwrap(),
            b"result"
        );
    }

    #[test]
    fn write_file_enforces_logical_file_size_before_mutation() {
        let (mut fs, output) = limited_output_fs(8, 32, 4);
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "bounded.bin", OpenFlags::CREATE)
            .unwrap();

        fs.write_file(fd, 7, b"x").unwrap();
        assert_eq!(
            std::fs::metadata(output.path().join("bounded.bin"))
                .unwrap()
                .len(),
            8
        );

        assert_eq!(fs.write_file(fd, 8, b"x"), Err(FsError::FileTooLarge));
        assert_eq!(
            std::fs::metadata(output.path().join("bounded.bin"))
                .unwrap()
                .len(),
            8
        );
    }

    #[test]
    fn write_file_rejects_offset_overflow() {
        let (mut fs, output) = limited_output_fs(8, 32, 4);
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "overflow.bin", OpenFlags::CREATE)
            .unwrap();

        assert_eq!(fs.write_file(fd, u64::MAX, b"x"), Err(FsError::Overflow));
        assert_eq!(
            std::fs::metadata(output.path().join("overflow.bin"))
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn empty_write_is_a_noop() {
        let (mut fs, output) = limited_output_fs(8, 32, 4);
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "empty.bin", OpenFlags::CREATE)
            .unwrap();
        let stream = fs.create_write_stream(fd, u64::MAX).unwrap();

        assert_eq!(fs.stream_write(stream, b"").unwrap(), 0);
        assert_eq!(
            std::fs::metadata(output.path().join("empty.bin"))
                .unwrap()
                .len(),
            0
        );
        assert_eq!(fs.streams[&stream].offset, u64::MAX);
        assert_eq!(fs.output_usage.logical_bytes, 0);
    }

    #[test]
    fn cumulative_quota_counts_growth_not_rewrites() {
        let (mut fs, output) = limited_output_fs(8, 10, 4);
        let output_fd = preopen_fd(&fs, "/output");
        let first = fs
            .open_at(output_fd, "first.bin", OpenFlags::CREATE)
            .unwrap();
        let second = fs
            .open_at(output_fd, "second.bin", OpenFlags::CREATE)
            .unwrap();

        fs.write_file(first, 0, b"12345678").unwrap();
        fs.write_file(first, 0, b"abcdefgh").unwrap();
        fs.write_file(second, 0, b"12").unwrap();
        assert_eq!(fs.write_file(second, 2, b"3"), Err(FsError::QuotaExceeded));

        assert_eq!(
            std::fs::read(output.path().join("first.bin")).unwrap(),
            b"abcdefgh"
        );
        assert_eq!(
            std::fs::read(output.path().join("second.bin")).unwrap(),
            b"12"
        );
        assert_eq!(fs.output_usage.logical_bytes, 10);
    }

    #[test]
    fn file_count_quota_survives_descriptor_close() {
        let (mut fs, _output) = limited_output_fs(8, 32, 1);
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "first.bin", OpenFlags::CREATE)
            .unwrap();
        fs.close_file(fd);

        assert_eq!(
            fs.open_at(output_fd, "second.bin", OpenFlags::CREATE),
            Err(FsError::QuotaExceeded)
        );
    }

    #[test]
    fn interleaved_append_streams_use_current_eof() {
        let (mut fs, output) = limited_output_fs(8, 32, 4);
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "append.bin", OpenFlags::CREATE)
            .unwrap();
        let first = fs.create_append_stream(fd).unwrap();
        let second = fs.create_append_stream(fd).unwrap();

        fs.stream_write(first, b"ab").unwrap();
        fs.stream_write(second, b"cd").unwrap();

        assert_eq!(
            std::fs::read(output.path().join("append.bin")).unwrap(),
            b"abcd"
        );
        assert_eq!(fs.output_usage.logical_bytes, 4);
    }

    #[test]
    fn host_write_replaces_file_and_releases_quota() {
        let (mut fs, _output) = limited_output_fs(8, 8, 2);
        fs.write_output_path("/output/value.bin", b"12345678".to_vec())
            .unwrap();
        fs.write_output_path("/output/value.bin", b"12".to_vec())
            .unwrap();
        fs.write_output_path("/output/other.bin", b"123456".to_vec())
            .unwrap();
        assert_eq!(fs.output_usage.logical_bytes, 8);
    }

    #[test]
    fn truncate_releases_logical_byte_quota() {
        let (mut fs, _output) = limited_output_fs(8, 8, 2);
        let output_fd = preopen_fd(&fs, "/output");
        let first = fs
            .open_at(output_fd, "first.bin", OpenFlags::CREATE)
            .unwrap();
        fs.write_file(first, 0, b"12345678").unwrap();

        fs.open_at(output_fd, "first.bin", OpenFlags::TRUNCATE)
            .unwrap();
        let second = fs
            .open_at(output_fd, "second.bin", OpenFlags::CREATE)
            .unwrap();
        fs.write_file(second, 0, b"12345678").unwrap();

        assert_eq!(fs.output_usage.logical_bytes, 8);
    }

    #[test]
    fn existing_output_over_limit_is_rejected() {
        let output = tempfile::tempdir().unwrap();
        host_write_output(&output, "large.bin", b"123456789");
        let limits = FilesystemLimits::new(8, 32, 4).unwrap();

        assert!(
            CapFs::with_limits(limits)
                .with_output_dir(
                    output.path(),
                    DirPerms::READ | DirPerms::MUTATE,
                    FilePerms::READ | FilePerms::WRITE,
                )
                .is_err()
        );
    }

    #[test]
    fn unlimited_limits_allow_large_logical_offsets() {
        let output = tempfile::tempdir().unwrap();
        let mut fs = CapFs::with_limits(FilesystemLimits::unlimited())
            .with_output_dir(
                output.path(),
                DirPerms::READ | DirPerms::MUTATE,
                FilePerms::READ | FilePerms::WRITE,
            )
            .unwrap();
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "sparse.bin", OpenFlags::CREATE)
            .unwrap();

        fs.write_file(fd, 1024, b"x").unwrap();

        assert_eq!(
            std::fs::metadata(output.path().join("sparse.bin"))
                .unwrap()
                .len(),
            1025
        );
    }

    #[test]
    fn zero_limits_disallow_output_files() {
        let (mut fs, _output) = limited_output_fs(0, 0, 0);
        let output_fd = preopen_fd(&fs, "/output");

        assert_eq!(
            fs.open_at(output_fd, "blocked.bin", OpenFlags::CREATE),
            Err(FsError::QuotaExceeded)
        );
    }

    #[test]
    fn repeated_output_registration_is_rejected() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let fs = CapFs::new()
            .with_output_dir(
                first.path(),
                DirPerms::READ | DirPerms::MUTATE,
                FilePerms::READ | FilePerms::WRITE,
            )
            .unwrap();

        assert!(
            fs.with_output_dir(
                second.path(),
                DirPerms::READ | DirPerms::MUTATE,
                FilePerms::READ | FilePerms::WRITE,
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_output_is_rejected() {
        let output = tempfile::tempdir().unwrap();
        host_write_output(&output, "first.bin", b"data");
        std::fs::hard_link(
            output.path().join("first.bin"),
            output.path().join("second.bin"),
        )
        .unwrap();

        assert!(
            CapFs::new()
                .with_output_dir(
                    output.path(),
                    DirPerms::READ | DirPerms::MUTATE,
                    FilePerms::READ | FilePerms::WRITE,
                )
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_output_is_rejected_with_unlimited_limits() {
        let output = tempfile::tempdir().unwrap();
        host_write_output(&output, "first.bin", b"data");
        std::fs::hard_link(
            output.path().join("first.bin"),
            output.path().join("second.bin"),
        )
        .unwrap();

        assert!(
            CapFs::with_limits(FilesystemLimits::unlimited())
                .with_output_dir(
                    output.path(),
                    DirPerms::READ | DirPerms::MUTATE,
                    FilePerms::READ | FilePerms::WRITE,
                )
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_linked_output_is_rejected_with_unlimited_limits() {
        use std::os::unix::fs::symlink;

        let output = tempfile::tempdir().unwrap();
        host_write_output(&output, "target.bin", b"data");
        symlink(
            output.path().join("target.bin"),
            output.path().join("link.bin"),
        )
        .unwrap();

        assert!(
            CapFs::with_limits(FilesystemLimits::unlimited())
                .with_output_dir(
                    output.path(),
                    DirPerms::READ | DirPerms::MUTATE,
                    FilePerms::READ | FilePerms::WRITE,
                )
                .is_err()
        );
    }

    #[test]
    fn failed_create_does_not_consume_handle() {
        let (mut fs, output) = limited_output_fs(8, 32, 4);
        std::fs::create_dir(output.path().join("directory")).unwrap();
        let output_fd = preopen_fd(&fs, "/output");
        let next_handle = fs.next_handle;

        assert!(
            fs.open_at(output_fd, "directory", OpenFlags::TRUNCATE)
                .is_err()
        );
        assert_eq!(fs.next_handle, next_handle);
    }

    #[test]
    fn host_write_rejects_directory_without_invalidating_accounting() {
        let (mut fs, output) = limited_output_fs(8, 32, 4);
        std::fs::create_dir(output.path().join("directory")).unwrap();

        assert!(
            fs.write_output_path("/output/directory", b"data".to_vec())
                .is_err()
        );
        assert!(fs.accounting_valid);
        assert_eq!(fs.output_usage.logical_bytes, 0);
        assert_eq!(fs.output_usage.file_count, 0);
    }

    #[cfg(unix)]
    #[test]
    fn successful_reconciliation_survives_cleanup_failure() {
        use std::os::unix::fs::PermissionsExt;

        let (mut fs, output) = limited_output_fs(8, 32, 4);
        host_write_output(&output, "locked.bin", b"data");
        let original_permissions = std::fs::metadata(output.path()).unwrap().permissions();
        std::fs::set_permissions(output.path(), std::fs::Permissions::from_mode(0o555)).unwrap();

        let result = fs.clear_output_files();

        std::fs::set_permissions(output.path(), original_permissions).unwrap();
        assert!(result.is_err());
        assert!(fs.accounting_valid);
        assert_eq!(fs.output_usage.logical_bytes, 4);
        assert_eq!(fs.output_usage.file_count, 1);
    }

    #[test]
    fn closing_file_invalidates_dependent_streams() {
        let (mut fs, _output) = limited_output_fs(8, 32, 4);
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "stream.bin", OpenFlags::CREATE)
            .unwrap();
        let stream = fs.create_write_stream(fd, 0).unwrap();

        fs.close_file(fd);

        assert!(!fs.has_stream(stream));
        assert_eq!(fs.stream_write(stream, b"x"), Err(FsError::BadDescriptor));
    }

    #[test]
    fn clear_output_removes_nested_files_and_resets_usage() {
        let (mut fs, output) = limited_output_fs(8, 32, 4);
        std::fs::create_dir(output.path().join("nested")).unwrap();
        fs.write_output_path("/output/nested/value.bin", b"1234".to_vec())
            .unwrap();

        fs.clear_output_files().unwrap();

        assert!(!output.path().join("nested/value.bin").exists());
        assert!(output.path().join("nested").is_dir());
        assert_eq!(fs.output_usage.logical_bytes, 0);
        assert_eq!(fs.output_usage.file_count, 0);
    }

    #[test]
    fn clear_output_preserves_input() {
        let (mut fs, input, _o) = test_fs();
        host_write_input(&input, "keep.txt", b"input");
        fs.write_output_path("/output/gone.txt", b"output".to_vec())
            .unwrap();

        fs.clear_output_files().unwrap();

        let input_fd = preopen_fd(&fs, "/input");
        let fd = fs
            .open_at(input_fd, "keep.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        let (data, _) = fs.read_file(fd, 0, 100).unwrap();
        assert_eq!(data, b"input");
        assert!(fs.get_output_files().is_empty());
    }

    #[test]
    fn default_temp_output() {
        let input = tempfile::tempdir().unwrap();
        let mut fs = CapFs::new()
            .with_input(input.path())
            .unwrap()
            .with_temp_output()
            .unwrap();

        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "test.txt", OpenFlags::CREATE)
            .unwrap();
        fs.write_file(fd, 0, b"works").unwrap();
        let files = fs.get_output_files();
        assert!(files.contains(&"test.txt".to_string()));
        let output_dir = fs.output_path().unwrap();
        assert_eq!(
            std::fs::read(output_dir.join("test.txt")).unwrap(),
            b"works"
        );
    }

    #[test]
    fn no_input_dir() {
        let fs = CapFs::new();
        assert!(fs.dir_by_guest_path("/input").is_none());
        assert!(fs.dir_by_guest_path("/output").is_none());
        assert_eq!(fs.preopens().len(), 0);
    }

    #[test]
    fn stream_file_backed() {
        let (mut fs, _i, _o) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs.open_at(output_fd, "s.txt", OpenFlags::CREATE).unwrap();
        let ws = fs.create_write_stream(fd, 0).unwrap();
        fs.stream_write(ws, b"hello ").unwrap();
        fs.stream_write(ws, b"world").unwrap();
        let rs = fs.create_read_stream(fd, 0).unwrap();
        assert_eq!(fs.stream_read(rs, 100).unwrap(), b"hello world");
    }

    #[test]
    fn open_at_rejects_bad_paths() {
        let (mut fs, _i, _o) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");

        // Path traversal
        assert_eq!(
            fs.open_at(output_fd, "../x", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            fs.open_at(output_fd, "../../etc/passwd", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );

        // Nested paths (only single-segment names allowed)
        assert_eq!(
            fs.open_at(output_fd, "a/b", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            fs.open_at(output_fd, "dir/file.txt", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );

        // Backslash separators
        assert_eq!(
            fs.open_at(output_fd, "a\\b", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            fs.open_at(output_fd, "..\\x", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );

        // Special names
        assert_eq!(
            fs.open_at(output_fd, ".", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            fs.open_at(output_fd, "..", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            fs.open_at(output_fd, "", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );

        // Null bytes and whitespace tricks
        assert_eq!(
            fs.open_at(output_fd, "\0", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            fs.open_at(output_fd, "file\0.txt", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );

        // Leading/trailing slashes
        assert_eq!(
            fs.open_at(output_fd, "/absolute", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            fs.open_at(output_fd, "trailing/", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );

        // Invalid descriptor
        assert_eq!(
            fs.open_at(999, "file.txt", OpenFlags::CREATE),
            Err(FsError::BadDescriptor)
        );

        // Valid names should work
        assert!(fs.open_at(output_fd, "file.txt", OpenFlags::CREATE).is_ok());
        assert!(fs.open_at(output_fd, ".hidden", OpenFlags::CREATE).is_ok());
        assert!(
            fs.open_at(output_fd, "file-name_v2.tar.gz", OpenFlags::CREATE)
                .is_ok()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_reject_alternate_data_streams() {
        let (mut fs, _input, _output) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");

        assert_eq!(
            fs.open_at(output_fd, "file.txt:stream", OpenFlags::CREATE),
            Err(FsError::InvalidPath)
        );
        assert!(
            fs.write_output_path("/output/file.txt:stream", b"data".to_vec())
                .is_err()
        );
    }

    #[test]
    fn input_is_readonly_no_create() {
        let (mut fs, _i, _o) = test_fs();
        let input_fd = preopen_fd(&fs, "/input");
        assert_eq!(
            fs.open_at(input_fd, "new.txt", OpenFlags::CREATE),
            Err(FsError::NotPermitted)
        );
    }

    #[test]
    fn input_is_readonly_no_truncate() {
        let (mut fs, input, _o) = test_fs();
        host_write_input(&input, "existing.txt", b"data");
        let input_fd = preopen_fd(&fs, "/input");
        let fd = fs
            .open_at(input_fd, "existing.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        assert!(fs.read_file(fd, 0, 100).is_ok());
        assert_eq!(
            fs.open_at(input_fd, "existing.txt", OpenFlags::TRUNCATE),
            Err(FsError::NotPermitted)
        );
    }

    #[test]
    fn input_file_perms_are_read_only() {
        let (mut fs, input, _o) = test_fs();
        host_write_input(&input, "readonly.txt", b"original");
        let input_fd = preopen_fd(&fs, "/input");
        let fd = fs
            .open_at(input_fd, "readonly.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        assert!(fs.file_has_perms(fd, FilePerms::READ));
        assert!(!fs.file_has_perms(fd, FilePerms::WRITE));
    }

    #[test]
    fn output_allows_read_and_write() {
        let (mut fs, _i, _o) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs.open_at(output_fd, "rw.txt", OpenFlags::CREATE).unwrap();
        assert!(fs.file_has_perms(fd, FilePerms::READ));
        assert!(fs.file_has_perms(fd, FilePerms::WRITE));
        fs.write_file(fd, 0, b"hello").unwrap();
        let (data, _) = fs.read_file(fd, 0, 100).unwrap();
        assert_eq!(data, b"hello");
    }

    #[test]
    fn dir_perms_correct() {
        let (fs, _i, _o) = test_fs();
        let input = fs.get_dir(preopen_fd(&fs, "/input")).unwrap();
        assert!(input.perms().contains(DirPerms::READ));
        assert!(!input.perms().contains(DirPerms::MUTATE));

        let output = fs.get_dir(preopen_fd(&fs, "/output")).unwrap();
        assert!(output.perms().contains(DirPerms::READ));
        assert!(output.perms().contains(DirPerms::MUTATE));
    }

    #[test]
    fn custom_output_perms_respected() {
        let output = tempfile::tempdir().unwrap();
        let fs = CapFs::new()
            .with_output_dir(output.path(), DirPerms::MUTATE, FilePerms::WRITE)
            .unwrap();
        let output_fd = preopen_fd(&fs, "/output");
        let dir = fs.get_dir(output_fd).unwrap();
        assert!(!dir.perms().contains(DirPerms::READ));
        assert!(dir.perms().contains(DirPerms::MUTATE));
        assert!(!dir.file_perms().contains(FilePerms::READ));
        assert!(dir.file_perms().contains(FilePerms::WRITE));
    }

    // ── Metadata tests ─────────────────────────────────────────────

    #[test]
    fn get_type_directory() {
        let (fs, _i, _o) = test_fs();
        let input_fd = preopen_fd(&fs, "/input");
        let output_fd = preopen_fd(&fs, "/output");
        assert_eq!(fs.get_type(input_fd).unwrap(), DescriptorType::Directory);
        assert_eq!(fs.get_type(output_fd).unwrap(), DescriptorType::Directory);
    }

    #[test]
    fn get_type_file() {
        let (mut fs, input, _o) = test_fs();
        host_write_input(&input, "test.txt", b"data");
        let input_fd = preopen_fd(&fs, "/input");
        let fd = fs
            .open_at(input_fd, "test.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        assert_eq!(fs.get_type(fd).unwrap(), DescriptorType::RegularFile);
    }

    #[test]
    fn get_type_invalid_fd() {
        let (fs, _i, _o) = test_fs();
        assert_eq!(fs.get_type(999), Err(FsError::BadDescriptor));
    }

    #[test]
    fn stat_directory() {
        let (fs, _i, _o) = test_fs();
        let input_fd = preopen_fd(&fs, "/input");
        let s = fs.stat(input_fd).unwrap();
        assert_eq!(s.descriptor_type, DescriptorType::Directory);
        assert_eq!(s.size, 0);
    }

    #[test]
    fn stat_file_reports_size() {
        let (mut fs, _i, _o) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "data.bin", OpenFlags::CREATE)
            .unwrap();
        fs.write_file(fd, 0, b"hello").unwrap();
        let s = fs.stat(fd).unwrap();
        assert_eq!(s.descriptor_type, DescriptorType::RegularFile);
        assert_eq!(s.size, 5);
    }

    #[test]
    fn stat_invalid_fd() {
        let (fs, _i, _o) = test_fs();
        assert_eq!(fs.stat(999), Err(FsError::BadDescriptor));
    }

    #[test]
    fn stat_at_existing_file() {
        let (fs, input, _o) = test_fs();
        host_write_input(&input, "readme.md", b"# Hello");
        let input_fd = preopen_fd(&fs, "/input");
        let s = fs.stat_at(input_fd, "readme.md").unwrap();
        assert_eq!(s.descriptor_type, DescriptorType::RegularFile);
        assert_eq!(s.size, 7);
    }

    #[test]
    fn stat_at_missing_file() {
        let (fs, _i, _o) = test_fs();
        let input_fd = preopen_fd(&fs, "/input");
        assert_eq!(fs.stat_at(input_fd, "nope.txt"), Err(FsError::NoEntry));
    }

    #[test]
    fn stat_at_requires_read_perm() {
        let output = tempfile::tempdir().unwrap();
        std::fs::write(output.path().join("file.txt"), b"data").unwrap();
        let fs = CapFs::new()
            .with_output_dir(output.path(), DirPerms::MUTATE, FilePerms::WRITE)
            .unwrap();
        let output_fd = preopen_fd(&fs, "/output");
        assert_eq!(
            fs.stat_at(output_fd, "file.txt"),
            Err(FsError::NotPermitted)
        );
    }

    #[test]
    fn stat_at_invalid_fd() {
        let (fs, _i, _o) = test_fs();
        assert_eq!(fs.stat_at(999, "file.txt"), Err(FsError::BadDescriptor));
    }

    #[test]
    fn get_flags_input_directory() {
        let (fs, _i, _o) = test_fs();
        let input_fd = preopen_fd(&fs, "/input");
        let f = fs.get_flags(input_fd).unwrap();
        assert!(f.read);
        assert!(!f.write);
        assert!(!f.mutate_directory);
    }

    #[test]
    fn get_flags_output_directory() {
        let (fs, _i, _o) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");
        let f = fs.get_flags(output_fd).unwrap();
        assert!(f.read);
        assert!(f.write);
        assert!(f.mutate_directory);
    }

    #[test]
    fn get_flags_input_file() {
        let (mut fs, input, _o) = test_fs();
        host_write_input(&input, "x.txt", b"data");
        let input_fd = preopen_fd(&fs, "/input");
        let fd = fs
            .open_at(input_fd, "x.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        let f = fs.get_flags(fd).unwrap();
        assert!(f.read);
        assert!(!f.write);
        assert!(!f.mutate_directory);
    }

    #[test]
    fn get_flags_output_file() {
        let (mut fs, _i, _o) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs.open_at(output_fd, "out.txt", OpenFlags::CREATE).unwrap();
        let f = fs.get_flags(fd).unwrap();
        assert!(f.read);
        assert!(f.write);
        assert!(!f.mutate_directory);
    }

    #[test]
    fn get_flags_invalid_fd() {
        let (fs, _i, _o) = test_fs();
        assert_eq!(fs.get_flags(999), Err(FsError::BadDescriptor));
    }

    // ── Permission denial tests ─────────────────────────────────────

    #[test]
    fn read_file_denied_without_read_perm() {
        let output = tempfile::tempdir().unwrap();
        std::fs::write(output.path().join("secret.txt"), b"data").unwrap();
        let mut fs = CapFs::new()
            .with_output_dir(
                output.path(),
                DirPerms::READ | DirPerms::MUTATE,
                FilePerms::WRITE,
            )
            .unwrap();
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "secret.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        assert_eq!(fs.read_file(fd, 0, 100), Err(FsError::NotPermitted));
    }

    #[test]
    fn write_file_denied_without_write_perm() {
        let (mut fs, input, _o) = test_fs();
        host_write_input(&input, "ro.txt", b"data");
        let input_fd = preopen_fd(&fs, "/input");
        let fd = fs
            .open_at(input_fd, "ro.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        assert_eq!(fs.write_file(fd, 0, b"hacked"), Err(FsError::NotPermitted));
    }

    #[test]
    fn create_read_stream_denied_without_read_perm() {
        let output = tempfile::tempdir().unwrap();
        std::fs::write(output.path().join("f.txt"), b"data").unwrap();
        let mut fs = CapFs::new()
            .with_output_dir(
                output.path(),
                DirPerms::READ | DirPerms::MUTATE,
                FilePerms::WRITE,
            )
            .unwrap();
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs
            .open_at(output_fd, "f.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        assert_eq!(fs.create_read_stream(fd, 0), Err(FsError::NotPermitted));
    }

    #[test]
    fn create_write_stream_denied_without_write_perm() {
        let (mut fs, input, _o) = test_fs();
        host_write_input(&input, "ro.txt", b"data");
        let input_fd = preopen_fd(&fs, "/input");
        let fd = fs
            .open_at(input_fd, "ro.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        assert_eq!(fs.create_write_stream(fd, 0), Err(FsError::NotPermitted));
    }

    #[test]
    fn create_append_stream_denied_without_write_perm() {
        let (mut fs, input, _o) = test_fs();
        host_write_input(&input, "ro.txt", b"data");
        let input_fd = preopen_fd(&fs, "/input");
        let fd = fs
            .open_at(input_fd, "ro.txt", OpenFlags::OPEN_EXISTING)
            .unwrap();
        assert_eq!(fs.create_append_stream(fd), Err(FsError::NotPermitted));
    }

    #[test]
    fn create_dir_stream_denied_without_read_perm() {
        let output = tempfile::tempdir().unwrap();
        let mut fs = CapFs::new()
            .with_output_dir(output.path(), DirPerms::MUTATE, FilePerms::WRITE)
            .unwrap();
        let output_fd = preopen_fd(&fs, "/output");
        assert_eq!(fs.create_dir_stream(output_fd), Err(FsError::NotPermitted));
    }

    #[test]
    fn write_output_path_denied_without_write_perm() {
        let output = tempfile::tempdir().unwrap();
        let mut fs = CapFs::new()
            .with_output_dir(output.path(), DirPerms::READ, FilePerms::READ)
            .unwrap();
        assert!(
            fs.write_output_path("/output/test.txt", b"data".to_vec())
                .is_err()
        );
    }

    #[test]
    fn read_guest_file_works() {
        let (fs, input, _o) = test_fs();
        host_write_input(&input, "hello.txt", b"world");
        let data = fs.read_guest_file("/input/hello.txt").unwrap();
        assert_eq!(data, b"world");
    }

    #[test]
    fn read_guest_file_denied_without_read_perm() {
        let output = tempfile::tempdir().unwrap();
        std::fs::write(output.path().join("f.txt"), b"data").unwrap();
        let fs = CapFs::new()
            .with_output_dir(output.path(), DirPerms::MUTATE, FilePerms::WRITE)
            .unwrap();
        assert!(fs.read_guest_file("/output/f.txt").is_err());
    }

    #[test]
    fn read_guest_file_missing() {
        let (fs, _i, _o) = test_fs();
        assert!(fs.read_guest_file("/input/nope.txt").is_err());
    }

    #[test]
    fn read_guest_file_outside_preopens() {
        let (fs, _i, _o) = test_fs();
        assert!(fs.read_guest_file("/etc/passwd").is_err());
        assert!(fs.read_guest_file("/tmp/something").is_err());
        assert!(fs.read_guest_file("/secret/data.txt").is_err());
    }

    #[test]
    fn read_guest_file_traversal() {
        let (fs, _i, _o) = test_fs();
        assert!(fs.read_guest_file("/input/../etc/passwd").is_err());
        assert!(fs.read_guest_file("/input/../../root/.ssh/id_rsa").is_err());
    }

    #[test]
    fn open_at_on_nonexistent_preopen() {
        let (mut fs, _i, _o) = test_fs();
        // Try to use a made-up fd that isn't a preopen
        assert_eq!(
            fs.open_at(99, "file.txt", OpenFlags::CREATE),
            Err(FsError::BadDescriptor)
        );
        assert_eq!(
            fs.open_at(0, "file.txt", OpenFlags::CREATE),
            Err(FsError::BadDescriptor)
        );
        assert_eq!(
            fs.open_at(1, "file.txt", OpenFlags::CREATE),
            Err(FsError::BadDescriptor)
        );
        assert_eq!(
            fs.open_at(2, "file.txt", OpenFlags::CREATE),
            Err(FsError::BadDescriptor)
        );
    }

    #[test]
    fn write_output_path_outside_output() {
        let (mut fs, _i, _o) = test_fs();
        // Must be rooted at /output
        assert!(
            fs.write_output_path("/input/sneaky.txt", b"data".to_vec())
                .is_err()
        );
        assert!(
            fs.write_output_path("/etc/passwd", b"data".to_vec())
                .is_err()
        );
        assert!(
            fs.write_output_path("/output/../etc/passwd", b"data".to_vec())
                .is_err()
        );
    }

    // ── Directory stream tests ──────────────────────────────────────

    #[test]
    fn dir_stream_lists_files() {
        let (mut fs, _i, _o) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");
        fs.open_at(output_fd, "a.txt", OpenFlags::CREATE).unwrap();
        fs.open_at(output_fd, "b.txt", OpenFlags::CREATE).unwrap();

        let stream = fs.create_dir_stream(output_fd).unwrap();
        let mut names = Vec::new();
        while let Some(Some((name, _is_dir))) = fs.read_dir_entry(stream) {
            names.push(name);
        }
        names.sort();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
        // End of stream
        assert_eq!(fs.read_dir_entry(stream), Some(None));
    }

    #[test]
    fn dir_stream_empty_dir() {
        let (mut fs, _i, _o) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");
        let stream = fs.create_dir_stream(output_fd).unwrap();
        assert_eq!(fs.read_dir_entry(stream), Some(None));
    }

    #[test]
    fn dir_stream_includes_names_with_http_prefix() {
        let (mut fs, _i, output) = test_fs();
        host_write_output(&output, "__http_cache.txt", b"data");

        let output_fd = preopen_fd(&fs, "/output");
        let stream = fs.create_dir_stream(output_fd).unwrap();
        let mut names = Vec::new();
        while let Some(Some((name, _is_dir))) = fs.read_dir_entry(stream) {
            names.push(name);
        }

        assert!(names.contains(&"__http_cache.txt".to_string()));
    }

    #[test]
    fn dir_stream_errors_when_directory_exceeds_limit() {
        let (mut fs, _i, output) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");

        for i in 0..=CapFs::MAX_DIR_ENTRIES {
            host_write_output(&output, &format!("file-{i}.txt"), b"");
        }

        assert!(matches!(
            fs.create_dir_stream(output_fd),
            Err(FsError::Io(_))
        ));
    }

    // ── Append stream test ──────────────────────────────────────────

    #[test]
    fn append_stream_appends() {
        let (mut fs, _i, _o) = test_fs();
        let output_fd = preopen_fd(&fs, "/output");
        let fd = fs.open_at(output_fd, "log.txt", OpenFlags::CREATE).unwrap();
        fs.write_file(fd, 0, b"first").unwrap();

        let as_ = fs.create_append_stream(fd).unwrap();
        fs.stream_write(as_, b" second").unwrap();

        let (data, _) = fs.read_file(fd, 0, 100).unwrap();
        assert_eq!(data, b"first second");
    }

    #[test]
    fn alloc_handle_overflow_returns_error() {
        let mut fs = CapFs::new();
        fs.next_handle = u32::MAX;

        assert_eq!(
            fs.alloc_handle(),
            Err(FsError::Io("file descriptor handle space exhausted".into()))
        );
    }
}
