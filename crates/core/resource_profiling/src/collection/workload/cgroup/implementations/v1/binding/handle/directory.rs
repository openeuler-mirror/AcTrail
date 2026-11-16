use super::discovery::V1Discovery;
use crate::ProfilingError;
use std::{
    ffi::CString,
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::Path,
};
#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}
pub(super) struct V1Directory {
    file: File,
}
impl V1Directory {
    pub(super) fn open(mount: &Path, relative: &Path) -> Result<Self, ProfilingError> {
        let mount = CString::new(mount.as_os_str().as_bytes())
            .map_err(|_| V1Discovery::invalid("invalid mount path"))?;
        let fd = unsafe {
            libc::open(
                mount.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(V1Discovery::io(std::io::Error::last_os_error()));
        }
        let root = unsafe { File::from_raw_fd(fd) };
        let relative = if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        };
        let path = CString::new(relative.as_os_str().as_bytes())
            .map_err(|_| V1Discovery::invalid("invalid relative path"))?;
        // Linux openat2: BENEATH | NO_SYMLINKS, without a path-based fallback.
        let how = OpenHow {
            flags: (libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
            mode: 0,
            resolve: 0x08 | 0x04,
        };
        let fd = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                root.as_raw_fd(),
                path.as_ptr(),
                &how,
                std::mem::size_of::<OpenHow>(),
            )
        } as i32;
        if fd < 0 {
            return Err(V1Discovery::io(std::io::Error::last_os_error()));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } < 0 {
            return Err(V1Discovery::io(std::io::Error::last_os_error()));
        }
        if unsafe { stat.assume_init() }.f_type as u64 != 0x27e0eb {
            return Err(V1Discovery::invalid(
                "directory is not a cgroup v1 filesystem",
            ));
        }
        Ok(Self { file })
    }
    pub(super) fn identity(&self) -> Result<(u64, u64), ProfilingError> {
        let metadata = self.file.metadata().map_err(V1Discovery::io)?;
        Ok((metadata.dev(), metadata.ino()))
    }
    pub(super) fn read(&self, name: &str) -> Result<String, ProfilingError> {
        let name = CString::new(name).map_err(|_| V1Discovery::invalid("invalid counter name"))?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(V1Discovery::io(std::io::Error::last_os_error()));
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let mut text = String::new();
        file.read_to_string(&mut text).map_err(V1Discovery::io)?;
        Ok(text)
    }
}
