//! File-system workload helpers for live eBPF verification.

use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::args::{MmapWorkloadConfig, WorkloadConfig};

pub(super) fn run_file_mutation_workload(config: &WorkloadConfig) -> Result<(), String> {
    create_directory(&config.mkdir_path, config.directory_mode)?;
    create_directory(&config.rmdir_path, config.directory_mode)?;
    remove_directory(&config.rmdir_path)?;

    std::fs::write(&config.rename_source_path, config.file_message.as_bytes())
        .map_err(|error| error.to_string())?;
    rename_path(&config.rename_source_path, &config.rename_target_path)?;

    std::fs::write(&config.unlink_path, config.file_message.as_bytes())
        .map_err(|error| error.to_string())?;
    unlink_path(&config.unlink_path)?;

    let truncate_source = config
        .file_message
        .as_bytes()
        .iter()
        .chain(config.file_message.as_bytes())
        .copied()
        .collect::<Vec<_>>();
    std::fs::write(&config.truncate_path, truncate_source).map_err(|error| error.to_string())?;
    let _truncate_file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&config.truncate_path)
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) struct FileEndpoint {
    file: std::fs::File,
}

impl FileEndpoint {
    pub(super) fn create(path: &Path) -> Result<Self, String> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        Ok(Self { file })
    }

    pub(super) fn roundtrip(&mut self, message: &[u8]) -> Result<(), String> {
        self.file
            .write_all(message)
            .map_err(|error| error.to_string())?;
        self.file.flush().map_err(|error| error.to_string())?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        let mut observed = vec![0; message.len()];
        self.file
            .read_exact(&mut observed)
            .map_err(|error| error.to_string())?;
        if observed != message {
            return Err("file observed unexpected payload".to_string());
        }
        Ok(())
    }
}

pub(super) struct MmapSharedFile {
    _file: std::fs::File,
}

impl MmapSharedFile {
    pub(super) fn create(config: &MmapWorkloadConfig) -> Result<Self, String> {
        let length = usize::try_from(config.length).map_err(|error| error.to_string())?;
        let offset = libc::off_t::try_from(config.offset).map_err(|error| error.to_string())?;
        let message = config.message.as_bytes();
        if length == 0 {
            return Err("mmap length must be positive".to_string());
        }
        if message.len() > length {
            return Err(format!(
                "mmap message length {} exceeds configured mapping length {}",
                message.len(),
                length
            ));
        }
        let file_length = config
            .offset
            .checked_add(config.length)
            .ok_or_else(|| "mmap offset plus length overflowed".to_string())?;

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&config.path)
            .map_err(|error| error.to_string())?;
        file.set_len(file_length)
            .map_err(|error| error.to_string())?;

        let mapping = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                offset,
            )
        };
        if mapping == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error().to_string());
        }

        let write_result = unsafe {
            std::ptr::copy_nonoverlapping(message.as_ptr(), mapping.cast::<u8>(), message.len());
            libc::msync(mapping, length, libc::MS_SYNC)
        };
        let sync_error = if write_result == 0 {
            None
        } else {
            Some(std::io::Error::last_os_error().to_string())
        };
        let unmap_result = unsafe { libc::munmap(mapping, length) };
        if unmap_result != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if let Some(error) = sync_error {
            return Err(error);
        }

        let mut observed = vec![0; message.len()];
        let mut observed_file =
            std::fs::File::open(&config.path).map_err(|error| error.to_string())?;
        observed_file
            .seek(SeekFrom::Start(config.offset))
            .map_err(|error| error.to_string())?;
        observed_file
            .read_exact(&mut observed)
            .map_err(|error| error.to_string())?;
        if observed != message {
            return Err("mmap file observed unexpected payload".to_string());
        }
        Ok(Self { _file: file })
    }
}

fn create_directory(path: &Path, mode: u32) -> Result<(), String> {
    let raw_path = cstring_path(path)?;
    let result = unsafe { libc::mkdirat(libc::AT_FDCWD, raw_path.as_ptr(), mode) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

fn remove_directory(path: &Path) -> Result<(), String> {
    let raw_path = cstring_path(path)?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_unlinkat,
            libc::AT_FDCWD,
            raw_path.as_ptr(),
            libc::AT_REMOVEDIR,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

fn rename_path(source: &Path, target: &Path) -> Result<(), String> {
    let raw_source = cstring_path(source)?;
    let raw_target = cstring_path(target)?;
    let result = unsafe {
        libc::renameat(
            libc::AT_FDCWD,
            raw_source.as_ptr(),
            libc::AT_FDCWD,
            raw_target.as_ptr(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

fn unlink_path(path: &Path) -> Result<(), String> {
    let raw_path = cstring_path(path)?;
    let result = unsafe { libc::syscall(libc::SYS_unlinkat, libc::AT_FDCWD, raw_path.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

fn cstring_path(path: &Path) -> Result<std::ffi::CString, String> {
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("path contains NUL byte: {}", path.display()))
}
