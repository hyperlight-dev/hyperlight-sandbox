//! Filesystem trait implementations.
#![allow(unused_variables)]

use hyperlight_common::resource::BorrowedResourceGuard;
use hyperlight_host::HyperlightError;
use hyperlight_sandbox::FsError;
use wasi::clocks::wall_clock;
use wasi::filesystem::types as fs_types;

use crate::HostState;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;
use crate::wasi_impl::types::stream::Stream;

type HlResult<T> = Result<T, HyperlightError>;

impl From<FsError> for fs_types::ErrorCode {
    fn from(e: FsError) -> Self {
        match e {
            FsError::BadDescriptor => fs_types::ErrorCode::BadDescriptor,
            FsError::NotPermitted => fs_types::ErrorCode::NotPermitted,
            FsError::NoEntry => fs_types::ErrorCode::NoEntry,
            FsError::InvalidPath => fs_types::ErrorCode::NoEntry,
            FsError::Io(_) => fs_types::ErrorCode::Io,
        }
    }
}

// ---------------------------------------------------------------------------
// DirectoryEntryStream
// ---------------------------------------------------------------------------

impl fs_types::DirectoryEntryStream for HostState {
    type T = u32;
    fn read_directory_entry(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<Option<fs_types::DirectoryEntry>, fs_types::ErrorCode>> {
        let stream_id = *self_;
        let Ok(mut fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        if fs.has_dir_stream(stream_id) {
            match fs.read_dir_entry(stream_id) {
                Some(Some((name, is_dir))) => {
                    let dtype = if is_dir {
                        fs_types::DescriptorType::Directory
                    } else {
                        fs_types::DescriptorType::RegularFile
                    };
                    return Ok(Ok(Some(fs_types::DirectoryEntry {
                        r#type: dtype,
                        name,
                    })));
                }
                Some(None) => return Ok(Ok(None)),
                None => {}
            }
        }
        Ok(Ok(None))
    }
}

// ---------------------------------------------------------------------------
// Descriptor
// ---------------------------------------------------------------------------

impl fs_types::Descriptor<wall_clock::Datetime, u32, Resource<Stream>, Resource<Stream>>
    for HostState
{
    type T = u32;

    fn read_via_stream(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        offset: fs_types::Filesize,
    ) -> HlResult<Result<Resource<Stream>, fs_types::ErrorCode>> {
        let fd = *self_;
        let Ok(mut fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        Ok(fs
            .create_read_stream(fd, offset)
            .map(|stream_id| Resource::new(Stream::from_cap_fs(stream_id, self.fs.clone())))
            .map_err(Into::into))
    }
    fn write_via_stream(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        offset: fs_types::Filesize,
    ) -> HlResult<Result<Resource<Stream>, fs_types::ErrorCode>> {
        let fd = *self_;
        let Ok(mut fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        Ok(fs
            .create_write_stream(fd, offset)
            .map(|stream_id| Resource::new(Stream::from_cap_fs(stream_id, self.fs.clone())))
            .map_err(Into::into))
    }
    fn append_via_stream(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<Resource<Stream>, fs_types::ErrorCode>> {
        let fd = *self_;
        let Ok(mut fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        Ok(fs
            .create_append_stream(fd)
            .map(|stream_id| Resource::new(Stream::from_cap_fs(stream_id, self.fs.clone())))
            .map_err(Into::into))
    }
    fn advise(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        offset: fs_types::Filesize,
        length: fs_types::Filesize,
        advice: fs_types::Advice,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Ok(()))
    }
    fn sync_data(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn get_type(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<fs_types::DescriptorType, fs_types::ErrorCode>> {
        let fd = *self_;
        let Ok(fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        Ok(fs
            .get_type(fd)
            .map(|t| match t {
                hyperlight_sandbox::DescriptorType::Directory => {
                    fs_types::DescriptorType::Directory
                }
                hyperlight_sandbox::DescriptorType::RegularFile => {
                    fs_types::DescriptorType::RegularFile
                }
            })
            .map_err(Into::into))
    }
    fn set_size(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        size: fs_types::Filesize,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn set_times(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        data_access_timestamp: fs_types::NewTimestamp<wall_clock::Datetime>,
        data_modification_timestamp: fs_types::NewTimestamp<wall_clock::Datetime>,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn read(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        length: fs_types::Filesize,
        offset: fs_types::Filesize,
    ) -> HlResult<Result<(Vec<u8>, bool), fs_types::ErrorCode>> {
        let fd = *self_;
        let Ok(fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        Ok(fs.read_file(fd, offset, length).map_err(Into::into))
    }
    fn write(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        buffer: Vec<u8>,
        offset: fs_types::Filesize,
    ) -> HlResult<Result<fs_types::Filesize, fs_types::ErrorCode>> {
        let fd = *self_;
        let Ok(mut fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        Ok(fs.write_file(fd, offset, &buffer).map_err(Into::into))
    }
    fn read_directory(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<u32, fs_types::ErrorCode>> {
        let fd = *self_;
        let Ok(mut fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        Ok(fs.create_dir_stream(fd).map_err(Into::into))
    }
    fn sync(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn create_directory_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        path: String,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn stat(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<fs_types::DescriptorStat<wall_clock::Datetime>, fs_types::ErrorCode>> {
        let fd = *self_;
        let Ok(fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        Ok(fs
            .stat(fd)
            .map(|s| fs_types::DescriptorStat {
                r#type: match s.descriptor_type {
                    hyperlight_sandbox::DescriptorType::Directory => {
                        fs_types::DescriptorType::Directory
                    }
                    hyperlight_sandbox::DescriptorType::RegularFile => {
                        fs_types::DescriptorType::RegularFile
                    }
                },
                r#link_count: 1,
                r#size: s.size,
                r#data_access_timestamp: None,
                r#data_modification_timestamp: None,
                r#status_change_timestamp: None,
            })
            .map_err(Into::into))
    }
    fn readlink_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        path: String,
    ) -> HlResult<Result<String, fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::NoEntry))
    }
    fn remove_directory_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        path: String,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn rename_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        old_path: String,
        new_descriptor: BorrowedResourceGuard<u32>,
        new_path: String,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn symlink_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        old_path: String,
        new_path: String,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn unlink_file_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        path: String,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn is_same_object(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        other: BorrowedResourceGuard<u32>,
    ) -> HlResult<bool> {
        Ok(false)
    }
    fn metadata_hash(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<fs_types::MetadataHashValue, fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn get_flags(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
    ) -> HlResult<Result<fs_types::DescriptorFlags, fs_types::ErrorCode>> {
        let fd = *self_;
        let Ok(fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        Ok(fs
            .get_flags(fd)
            .map(|f| fs_types::DescriptorFlags {
                r#read: f.read,
                r#write: f.write,
                r#file_integrity_sync: false,
                r#data_integrity_sync: false,
                r#requested_write_sync: false,
                r#mutate_directory: f.mutate_directory,
            })
            .map_err(Into::into))
    }
    fn stat_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        path_flags: fs_types::PathFlags,
        path: String,
    ) -> HlResult<Result<fs_types::DescriptorStat<wall_clock::Datetime>, fs_types::ErrorCode>> {
        let dir_fd = *self_;
        let Ok(fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };
        Ok(fs
            .stat_at(dir_fd, &path)
            .map(|s| fs_types::DescriptorStat {
                r#type: match s.descriptor_type {
                    hyperlight_sandbox::DescriptorType::Directory => {
                        fs_types::DescriptorType::Directory
                    }
                    hyperlight_sandbox::DescriptorType::RegularFile => {
                        fs_types::DescriptorType::RegularFile
                    }
                },
                r#link_count: 1,
                r#size: s.size,
                r#data_access_timestamp: None,
                r#data_modification_timestamp: None,
                r#status_change_timestamp: None,
            })
            .map_err(Into::into))
    }
    fn set_times_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        path_flags: fs_types::PathFlags,
        path: String,
        data_access_timestamp: fs_types::NewTimestamp<wall_clock::Datetime>,
        data_modification_timestamp: fs_types::NewTimestamp<wall_clock::Datetime>,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn link_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        old_path_flags: fs_types::PathFlags,
        old_path: String,
        new_descriptor: BorrowedResourceGuard<u32>,
        new_path: String,
    ) -> HlResult<Result<(), fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
    fn open_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        path_flags: fs_types::PathFlags,
        path: String,
        open_flags: fs_types::OpenFlags,
        flags: fs_types::DescriptorFlags,
    ) -> HlResult<Result<u32, fs_types::ErrorCode>> {
        let dir_fd = *self_;
        let Ok(mut fs) = self.fs.lock() else {
            return Ok(Err(fs_types::ErrorCode::Io));
        };

        let mut flags = hyperlight_sandbox::OpenFlags::empty();
        if open_flags.r#create {
            flags |= hyperlight_sandbox::OpenFlags::CREATE;
        }
        if open_flags.r#truncate {
            flags |= hyperlight_sandbox::OpenFlags::TRUNCATE;
        }
        Ok(fs.open_at(dir_fd, &path, flags).map_err(Into::into))
    }
    fn metadata_hash_at(
        &mut self,
        self_: BorrowedResourceGuard<u32>,
        path_flags: fs_types::PathFlags,
        path: String,
    ) -> HlResult<Result<fs_types::MetadataHashValue, fs_types::ErrorCode>> {
        Ok(Err(fs_types::ErrorCode::Unsupported))
    }
}

impl
    wasi::filesystem::Types<wall_clock::Datetime, anyhow::Error, Resource<Stream>, Resource<Stream>>
    for HostState
{
    fn filesystem_error_code(
        &mut self,
        err: BorrowedResourceGuard<anyhow::Error>,
    ) -> HlResult<Option<fs_types::ErrorCode>> {
        Ok(None)
    }
}

impl wasi::filesystem::Preopens<u32> for HostState {
    fn get_directories(&mut self) -> HlResult<Vec<(u32, String)>> {
        let Ok(fs) = self.fs.lock() else {
            return Ok(vec![]);
        };
        Ok(fs
            .preopens()
            .into_iter()
            .map(|(fd, name)| (fd, name.to_string()))
            .collect())
    }
}
