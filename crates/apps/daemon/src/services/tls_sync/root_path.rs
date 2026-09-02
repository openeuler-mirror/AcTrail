//! Peer mount-namespace path resolution for TLS sync plan lookup.

use std::ffi::CString;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use control_contract::reply::ControlError;

#[derive(Debug)]
pub(super) struct PeerRoot {
    peer_pid: u32,
    root: File,
    source: PathBuf,
}

#[derive(Debug)]
pub(super) struct PeerRootResolver {
    peer_pid: u32,
    root: Result<PeerRoot, String>,
    mount_namespace: Result<String, String>,
}

#[derive(Debug)]
pub(super) struct PeerRootHandle {
    peer_pid: u32,
    root: File,
}

#[derive(Debug)]
pub(super) struct PinnedPeerPath {
    file: File,
}

impl PeerRootResolver {
    pub(super) fn new(peer_pid: u32) -> Self {
        let mount_namespace = mount_namespace(peer_pid);
        let root = open_peer_root(peer_pid);
        Self {
            peer_pid,
            root,
            mount_namespace,
        }
    }

    pub(super) fn duplicate(&mut self) -> Result<PeerRootHandle, String> {
        let current_namespace = mount_namespace(self.peer_pid);
        if current_namespace != self.mount_namespace {
            tracing::info!(
                target: "actrail::tls_sync",
                peer_pid = self.peer_pid,
                previous_mount_namespace = ?self.mount_namespace,
                current_mount_namespace = ?current_namespace,
                "refreshing TLS-sync peer root after mount namespace change"
            );
            self.mount_namespace = current_namespace;
            self.root = open_peer_root(self.peer_pid);
        }
        self.root
            .as_ref()
            .map_err(Clone::clone)?
            .duplicate()
            .map_err(|error| error.message)
    }
}

impl PeerRoot {
    fn open(peer_pid: u32) -> Result<Self, ControlError> {
        let source = Path::new("/proc").join(peer_pid.to_string()).join("root");
        let root = open_root_directory(&source).map_err(|error| {
            ControlError::new(
                "tls_sync_plan_root",
                format!("open peer root {}: {error}", source.display()),
            )
        })?;
        Ok(Self {
            peer_pid,
            root,
            source,
        })
    }

    pub(super) fn duplicate(&self) -> Result<PeerRootHandle, ControlError> {
        let fd = duplicate_fd(self.root.as_raw_fd()).map_err(|error| {
            ControlError::new(
                "tls_sync_plan_root",
                format!("duplicate peer root {}: {error}", self.source.display()),
            )
        })?;
        let root = unsafe { File::from_raw_fd(fd) };
        Ok(PeerRootHandle {
            peer_pid: self.peer_pid,
            root,
        })
    }
}

impl PeerRootHandle {
    pub(super) fn probe_path_for(&self, runtime_path: &Path) -> Result<PathBuf, String> {
        let relative = safe_relative_path(runtime_path)?;
        let _keep_root_alive = self.root.as_raw_fd();
        Ok(Path::new("/proc")
            .join(self.peer_pid.to_string())
            .join("root")
            .join(relative))
    }

    pub(super) fn pin_path(&self, runtime_path: &Path) -> Result<PinnedPeerPath, String> {
        let relative = safe_relative_path(runtime_path)?;
        let file = open_path_in_root(self.root.as_raw_fd(), &relative).map_err(|error| {
            format!(
                "open runtime path {} in peer root: {error}",
                runtime_path.display()
            )
        })?;
        Ok(PinnedPeerPath { file })
    }
}

impl PinnedPeerPath {
    pub(super) fn path(&self) -> PathBuf {
        Path::new("/proc")
            .join("self")
            .join("fd")
            .join(self.file.as_raw_fd().to_string())
    }
}

fn safe_relative_path(runtime_path: &Path) -> Result<PathBuf, String> {
    if !runtime_path.is_absolute() {
        return Err(format!(
            "TLS plan lookup path must be absolute: {}",
            runtime_path.display()
        ));
    }
    let mut relative = PathBuf::new();
    for component in runtime_path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(value) => relative.push(value),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(format!(
                    "TLS plan lookup path contains an unsafe component: {}",
                    runtime_path.display()
                ));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err("TLS plan lookup path cannot name the runtime root".to_string());
    }
    Ok(relative)
}

fn open_peer_root(peer_pid: u32) -> Result<PeerRoot, String> {
    PeerRoot::open(peer_pid).map_err(|error| error.message)
}

fn mount_namespace(peer_pid: u32) -> Result<String, String> {
    let path = Path::new("/proc")
        .join(peer_pid.to_string())
        .join("ns")
        .join("mnt");
    std::fs::read_link(&path)
        .map(|namespace| namespace.display().to_string())
        .map_err(|error| format!("read peer mount namespace {}: {error}", path.display()))
}

fn duplicate_fd(fd: RawFd) -> std::io::Result<RawFd> {
    let duplicated = unsafe { libc::dup(fd) };
    if duplicated < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(duplicated)
    }
}

fn open_root_directory(path: &Path) -> std::io::Result<File> {
    let raw = CString::new(path.as_os_str().as_bytes())?;
    let fd = unsafe {
        libc::open(
            raw.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

fn open_path_in_root(root_fd: RawFd, path: &Path) -> std::io::Result<File> {
    const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
    const RESOLVE_IN_ROOT: u64 = 0x10;

    let raw = CString::new(path.as_os_str().as_bytes())?;
    let how = OpenHow {
        flags: u64::try_from(libc::O_PATH | libc::O_CLOEXEC).expect("open flags fit u64"),
        mode: 0,
        resolve: RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS,
    };
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root_fd,
            raw.as_ptr(),
            &how as *const OpenHow,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd as RawFd) })
    }
}
