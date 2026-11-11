//! Btrfs ioctls and the command-line tools the backend relies on.

use std::ffi::{CString, OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use checkpoint::BackendError;

use crate::{backend_error, io_error};

const BTRFS_SUPER_MAGIC: libc::c_long = 0x9123_683e;
const SUBVOLUME_ROOT_INODE: u64 = 256;
const BTRFS_IOCTL_MAGIC: u64 = 0x94;
const BTRFS_SUBVOL_RDONLY: u64 = 1 << 1;
const BTRFS_SUBVOL_NAME_MAX: usize = 4039;
const BTRFS_UUID_SIZE: usize = 16;
const BTRFS_VOL_NAME_MAX: usize = 255;
const IOC_WRITE: u64 = 1;
const IOC_READ: u64 = 2;
const IOC_DIR_SHIFT: u64 = 30;
const IOC_SIZE_SHIFT: u64 = 16;
const IOC_TYPE_SHIFT: u64 = 8;

const fn ioctl_request<T>(direction: u64, number: u64) -> libc::c_ulong {
    ((direction << IOC_DIR_SHIFT)
        | ((std::mem::size_of::<T>() as u64) << IOC_SIZE_SHIFT)
        | (BTRFS_IOCTL_MAGIC << IOC_TYPE_SHIFT)
        | number) as libc::c_ulong
}

const BTRFS_IOC_SNAP_CREATE_V2: libc::c_ulong = ioctl_request::<BtrfsIoctlVolArgsV2>(IOC_WRITE, 23);
const BTRFS_IOC_SUBVOL_GETFLAGS: libc::c_ulong = ioctl_request::<u64>(IOC_READ, 25);
const BTRFS_IOC_GET_SUBVOL_INFO: libc::c_ulong =
    ioctl_request::<BtrfsIoctlGetSubvolInfoArgs>(IOC_READ, 60);
const BTRFS_IOC_GET_SUBVOL_ROOTREF: libc::c_ulong =
    ioctl_request::<BtrfsIoctlGetSubvolRootrefArgs>(IOC_READ | IOC_WRITE, 61);
const BTRFS_MAX_ROOTREF_BUFFER_NUM: usize = 255;

#[repr(C)]
#[derive(Clone, Copy)]
struct BtrfsRootref {
    treeid: u64,
    dirid: u64,
}

#[repr(C)]
struct BtrfsIoctlGetSubvolRootrefArgs {
    min_treeid: u64,
    rootref: [BtrfsRootref; BTRFS_MAX_ROOTREF_BUFFER_NUM],
    num_items: u8,
    align: [u8; 7],
}

// These layouts mirror linux/btrfs.h.
#[repr(C)]
struct BtrfsIoctlTimespec {
    sec: u64,
    nsec: u32,
    padding: u32,
}

#[repr(C)]
struct BtrfsIoctlGetSubvolInfoArgs {
    treeid: u64,
    name: [u8; BTRFS_VOL_NAME_MAX + 1],
    parent_id: u64,
    dirid: u64,
    generation: u64,
    flags: u64,
    uuid: [u8; BTRFS_UUID_SIZE],
    parent_uuid: [u8; BTRFS_UUID_SIZE],
    received_uuid: [u8; BTRFS_UUID_SIZE],
    ctransid: u64,
    otransid: u64,
    stransid: u64,
    rtransid: u64,
    ctime: BtrfsIoctlTimespec,
    otime: BtrfsIoctlTimespec,
    stime: BtrfsIoctlTimespec,
    rtime: BtrfsIoctlTimespec,
    reserved: [u64; 8],
}

#[repr(C)]
struct BtrfsIoctlVolArgsV2 {
    fd: i64,
    transid: u64,
    flags: u64,
    unused: [u64; 4],
    name: [u8; BTRFS_SUBVOL_NAME_MAX + 1],
}

pub(crate) fn require_btrfs(path: &Path) -> Result<(), BackendError> {
    let raw = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| backend_error("invalid_path", "path contains NUL"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: raw is a valid C string and stat is a writable statfs buffer.
    if unsafe { libc::statfs(raw.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io_error(path.display(), std::io::Error::last_os_error()));
    }
    // SAFETY: statfs returned 0, so it filled the buffer.
    let stat = unsafe { stat.assume_init() };
    if stat.f_type != BTRFS_SUPER_MAGIC {
        return Err(backend_error(
            "invalid_scope",
            format!("{} is not on a Btrfs filesystem", path.display()),
        ));
    }
    Ok(())
}

pub(crate) fn is_subvolume_root(path: &Path) -> std::io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(metadata.is_dir() && metadata.ino() == SUBVOLUME_ROOT_INODE)
}

pub(crate) fn require_subvolume_root(path: &Path) -> Result<(), BackendError> {
    if is_subvolume_root(path).map_err(|error| io_error(path.display(), error))? {
        Ok(())
    } else {
        Err(backend_error(
            "invalid_scope",
            format!("{} is not a Btrfs subvolume root", path.display()),
        ))
    }
}

/// Pins the connected directory so pathname replacements cannot redirect operations.
pub(crate) struct ScopeRoot {
    path: PathBuf,
    directory: File,
}

pub(crate) struct LockedScope<'a> {
    pub(crate) directory: &'a File,
    _lock: File,
}

impl ScopeRoot {
    pub(crate) fn open(path: PathBuf) -> Result<Self, BackendError> {
        let directory = open_directory(&path).map_err(|error| io_error(path.display(), error))?;
        Ok(Self { path, directory })
    }

    pub(crate) fn access_path(&self) -> PathBuf {
        directory_path(&self.directory).join(".")
    }

    pub(crate) fn nested_subvolume(&self) -> Result<Option<u64>, BackendError> {
        first_nested_subvolume(&self.directory)
    }

    /// Lock daemon-owned metadata, not the live directory an Agent may have locked before
    /// being paused. Each operation opens its own lock fd so dropping the guard releases it.
    pub(crate) fn lock(&self, lock_dir: &Path) -> Result<LockedScope<'_>, BackendError> {
        let uuid = subvolume_uuid(&self.directory)?;
        let lock_path = lock_dir.join(format!("{uuid}.lock"));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&lock_path)
            .map_err(|error| io_error(lock_path.display(), error))?;
        // SAFETY: lock owns the descriptor until the returned guard is dropped.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io_error(
                lock_path.display(),
                std::io::Error::last_os_error(),
            ));
        }
        let changed = || {
            backend_error(
                "scope_changed",
                format!(
                    "protected subvolume {} was moved, removed or replaced",
                    self.path.display()
                ),
            )
        };
        let current = open_directory(&self.path).map_err(|error| {
            backend_error(
                "scope_changed",
                format!(
                    "cannot open protected subvolume {} at its original path: {error}",
                    self.path.display()
                ),
            )
        })?;
        let expected = self
            .directory
            .metadata()
            .map_err(|error| io_error(self.path.display(), error))?;
        let actual = current
            .metadata()
            .map_err(|error| io_error(self.path.display(), error))?;
        let current_path = fs::read_link(directory_path(&current))
            .map_err(|error| io_error(self.path.display(), error))?;
        if (actual.dev(), actual.ino()) != (expected.dev(), expected.ino())
            || current_path != self.path
        {
            return Err(changed());
        }
        Ok(LockedScope {
            directory: &self.directory,
            _lock: lock,
        })
    }
}

fn open_directory(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

/// A child command must address the daemon's fd, since its own CLOEXEC fds are closed at exec.
pub(crate) fn directory_path(directory: &File) -> PathBuf {
    PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        directory.as_raw_fd()
    ))
}

pub(crate) fn subvolume_uuid(subvolume: &File) -> Result<String, BackendError> {
    let mut args = zeroed_get_subvolume_info_args();
    // SAFETY: args has the kernel UAPI layout and both it and the fd remain valid for the call.
    if unsafe { libc::ioctl(subvolume.as_raw_fd(), BTRFS_IOC_GET_SUBVOL_INFO, &mut args) } < 0 {
        return Err(ioctl_error(
            "inspect Btrfs subvolume",
            &directory_path(subvolume),
        ));
    }
    Ok(format_uuid(args.uuid))
}

/// Creates a read-only snapshot of `source` at `destination` and returns the snapshot UUID.
pub(crate) fn create_read_only_snapshot(
    source: &File,
    destination: &Path,
) -> Result<String, BackendError> {
    let parent = destination.parent().ok_or_else(|| {
        backend_error(
            "invalid_path",
            format!("{} has no parent", destination.display()),
        )
    })?;
    let name = snapshot_name(destination)?;
    let parent_dir = File::open(parent).map_err(|error| io_error(parent.display(), error))?;
    let mut args = zeroed_snapshot_args();
    args.fd = i64::from(source.as_raw_fd());
    args.flags = BTRFS_SUBVOL_RDONLY;
    args.name[..name.len()].copy_from_slice(name);
    // SAFETY: args has the kernel UAPI layout and the source/parent fds remain open for the call.
    if unsafe { libc::ioctl(parent_dir.as_raw_fd(), BTRFS_IOC_SNAP_CREATE_V2, &mut args) } < 0 {
        return Err(ioctl_error("create read-only Btrfs snapshot", destination));
    }
    let snapshot =
        File::open(destination).map_err(|error| io_error(destination.display(), error))?;
    subvolume_uuid(&snapshot)
}

pub(crate) fn require_read_only_snapshot(path: &Path) -> Result<(), BackendError> {
    require_subvolume_root(path)?;
    let subvolume = File::open(path).map_err(|error| io_error(path.display(), error))?;
    let mut flags = 0_u64;
    // SAFETY: flags is the u64 output buffer required by BTRFS_IOC_SUBVOL_GETFLAGS.
    if unsafe { libc::ioctl(subvolume.as_raw_fd(), BTRFS_IOC_SUBVOL_GETFLAGS, &mut flags) } < 0 {
        return Err(ioctl_error("read Btrfs subvolume flags", path));
    }
    if flags & BTRFS_SUBVOL_RDONLY == 0 {
        return Err(backend_error(
            "btrfs",
            format!("snapshot {} is writable", path.display()),
        ));
    }
    Ok(())
}

/// Deletes a snapshot subvolume; a snapshot that no longer exists counts as deleted.
pub(crate) fn delete_subvolume(btrfs_path: &Path, path: &Path) -> Result<(), BackendError> {
    if fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    run_command(
        btrfs_path,
        &[
            OsString::from("subvolume"),
            OsString::from("delete"),
            OsString::from("--"),
            path.as_os_str().to_os_string(),
        ],
    )
    .map(|_| ())
}

/// Returns the tree ID of one subvolume directly below `subvolume`, which must be a subvolume
/// root. A deeper subvolume always has such an ancestor.
fn first_nested_subvolume(subvolume: &File) -> Result<Option<u64>, BackendError> {
    let mut args = BtrfsIoctlGetSubvolRootrefArgs {
        min_treeid: 0,
        rootref: [BtrfsRootref {
            treeid: 0,
            dirid: 0,
        }; BTRFS_MAX_ROOTREF_BUFFER_NUM],
        num_items: 0,
        align: [0; 7],
    };
    // SAFETY: args has the kernel UAPI layout and both it and the fd remain valid for the call.
    if unsafe { libc::ioctl(subvolume.as_raw_fd(), BTRFS_IOC_GET_SUBVOL_ROOTREF, &mut args) } < 0
        // EOVERFLOW only says the filled buffer did not hold every child.
        && std::io::Error::last_os_error().raw_os_error() != Some(libc::EOVERFLOW)
    {
        return Err(ioctl_error(
            "list nested Btrfs subvolumes of",
            &directory_path(subvolume),
        ));
    }
    Ok((args.num_items > 0).then_some(args.rootref[0].treeid))
}

pub(crate) fn filesystem_uuid(findmnt_path: &Path, path: &Path) -> Result<String, BackendError> {
    let output = run_command(
        findmnt_path,
        &[
            OsString::from("-n"),
            OsString::from("-o"),
            OsString::from("UUID"),
            OsString::from("--target"),
            path.as_os_str().to_os_string(),
        ],
    )?;
    let uuid = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_ascii_lowercase();
    if uuid.is_empty() || uuid.contains(char::is_whitespace) {
        return Err(backend_error(
            "btrfs",
            format!("findmnt returned no filesystem UUID for {}", path.display()),
        ));
    }
    Ok(uuid)
}

pub(crate) fn run_command(program: &Path, args: &[OsString]) -> Result<Output, BackendError> {
    let output = Command::new(program).args(args).output().map_err(|error| {
        backend_error(
            "command",
            format!("failed to start {}: {error}", program.display()),
        )
    })?;
    if !output.status.success() {
        return Err(backend_error(
            "command",
            format!(
                "{} exited with {}: {}",
                program.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(output)
}

fn snapshot_name(destination: &Path) -> Result<&[u8], BackendError> {
    let name = destination
        .file_name()
        .map(OsStr::as_bytes)
        .unwrap_or_default();
    if name.is_empty() || name.len() > BTRFS_SUBVOL_NAME_MAX || name.contains(&0) {
        return Err(backend_error(
            "invalid_path",
            format!("snapshot name must contain 1 to {BTRFS_SUBVOL_NAME_MAX} non-NUL bytes"),
        ));
    }
    Ok(name)
}

fn format_uuid(uuid: [u8; BTRFS_UUID_SIZE]) -> String {
    use std::fmt::Write as _;

    let mut formatted = String::with_capacity(36);
    for (index, byte) in uuid.into_iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            formatted.push('-');
        }
        write!(formatted, "{byte:02x}").expect("writing to a String cannot fail");
    }
    formatted
}

fn ioctl_error(operation: &str, path: &Path) -> BackendError {
    backend_error(
        "btrfs",
        format!(
            "failed to {operation} {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ),
    )
}

fn zeroed_timespec() -> BtrfsIoctlTimespec {
    BtrfsIoctlTimespec {
        sec: 0,
        nsec: 0,
        padding: 0,
    }
}

fn zeroed_get_subvolume_info_args() -> BtrfsIoctlGetSubvolInfoArgs {
    BtrfsIoctlGetSubvolInfoArgs {
        treeid: 0,
        name: [0; BTRFS_VOL_NAME_MAX + 1],
        parent_id: 0,
        dirid: 0,
        generation: 0,
        flags: 0,
        uuid: [0; BTRFS_UUID_SIZE],
        parent_uuid: [0; BTRFS_UUID_SIZE],
        received_uuid: [0; BTRFS_UUID_SIZE],
        ctransid: 0,
        otransid: 0,
        stransid: 0,
        rtransid: 0,
        ctime: zeroed_timespec(),
        otime: zeroed_timespec(),
        stime: zeroed_timespec(),
        rtime: zeroed_timespec(),
        reserved: [0; 8],
    }
}

fn zeroed_snapshot_args() -> BtrfsIoctlVolArgsV2 {
    BtrfsIoctlVolArgsV2 {
        fd: 0,
        transid: 0,
        flags: 0,
        unused: [0; 4],
        name: [0; BTRFS_SUBVOL_NAME_MAX + 1],
    }
}
