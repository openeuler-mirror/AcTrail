//! Permission ceilings at managed file lifecycle boundaries.

use std::fs::{self, DirBuilder, File, Metadata, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Existing access is intersected with 0640/0750. New files also honor umask.
#[derive(Clone, Copy, Default)]
pub struct FilePermissions;

impl FilePermissions {
    /// Read configuration without permission changes or blocking on special files.
    pub fn read_only(&self, path: &Path) -> io::Result<String> {
        let target = fs::canonicalize(path).map_err(|error| Self::at(path, error))?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&target)
            .map_err(|error| Self::at(path, error))?;
        if !file
            .metadata()
            .map_err(|error| Self::at(path, error))?
            .is_file()
        {
            return Err(Self::at(
                path,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "expected a regular configuration file",
                ),
            ));
        }
        let mut raw = String::new();
        file.read_to_string(&mut raw)
            .map_err(|error| Self::at(path, error))?;
        Ok(raw)
    }

    /// The configured immediate parent is dedicated; existing ancestors remain external.
    pub fn prepare_owned_parent(&self, path: &Path) -> io::Result<()> {
        let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) else {
            return Ok(());
        };
        if parent == Path::new(".") {
            return Ok(());
        }
        DirBuilder::new()
            .recursive(true)
            .mode(0o750)
            .create(parent)
            .map_err(|error| Self::at(parent, error))?;
        let actual = fs::canonicalize(parent).map_err(|error| Self::at(parent, error))?;
        if self.shared_directory(&actual) {
            return Ok(());
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&actual)
            .map_err(|error| Self::at(&actual, error))?;
        self.restrict(&directory, 0o750)
            .map_err(|error| Self::at(&actual, error))
    }

    /// Restrict a config/rules file at its runtime load boundary, following a legal alias.
    pub fn prepare_existing(&self, path: &Path) -> io::Result<()> {
        let target = fs::canonicalize(path).map_err(|error| Self::at(path, error))?;
        self.prepare_owned_parent(&target)?;
        self.restrict_existing_file(&target)
    }

    /// Read a regular rules file through the same descriptor used to restrict it.
    pub fn read_restricted(&self, path: &Path) -> io::Result<String> {
        let target = fs::canonicalize(path).map_err(|error| Self::at(path, error))?;
        self.prepare_owned_parent(&target)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&target)
            .map_err(|error| Self::at(path, error))?;
        self.regular_metadata(&file)
            .map_err(|error| Self::at(path, error))?;
        self.restrict(&file, 0o640)
            .map_err(|error| Self::at(path, error))?;
        let mut raw = String::new();
        file.read_to_string(&mut raw)
            .map_err(|error| Self::at(path, error))?;
        Ok(raw)
    }

    pub fn create_new(&self, path: &Path) -> io::Result<File> {
        self.open(path, OpenOptions::new().write(true).create_new(true))
    }

    pub fn append(&self, path: &Path) -> io::Result<File> {
        self.open(path, OpenOptions::new().append(true).create(true))
    }

    fn open(&self, path: &Path, options: &mut OpenOptions) -> io::Result<File> {
        self.prepare_owned_parent(path)?;
        let file = options
            .mode(0o640)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|error| Self::at(path, error))?;
        self.regular_metadata(&file)
            .map_err(|error| Self::at(path, error))?;
        self.restrict(&file, 0o640)
            .map_err(|error| Self::at(path, error))?;
        Ok(file)
    }

    /// O_PATH cannot release a SQLite connection's process-wide POSIX locks on close.
    pub fn restrict_existing_file(&self, path: &Path) -> io::Result<()> {
        self.restrict_file(path, false)
    }

    fn restrict_file(&self, path: &Path, database: bool) -> io::Result<()> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|error| Self::at(path, error))?;
        let metadata = self
            .regular_metadata(&file)
            .map_err(|error| Self::at(path, error))?;
        let desired = metadata.mode() & 0o640;
        if database && desired == 0 {
            return Err(Self::at(
                path,
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "SQLite requires nonzero access permissions; zero would create sidecars with default permissions",
                ),
            ));
        }
        if metadata.mode() & 0o7777 != desired {
            fs::set_permissions(
                format!("/proc/self/fd/{}", file.as_raw_fd()),
                Permissions::from_mode(desired),
            )
            .map_err(|error| Self::at(path, error))?;
        }
        Ok(())
    }

    /// SQLite inherits the prepared main database's mode for future sidecars.
    pub fn prepare_database(&self, path: &Path) -> io::Result<()> {
        if path == Path::new(":memory:") {
            return Ok(());
        }
        self.prepare_owned_parent(path)?;
        match self.restrict_file(path, true) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => match self.create_new(path) {
                Ok(_) => self.restrict_file(path, true)?,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    self.restrict_file(path, true)?
                }
                Err(error) => return Err(error),
            },
            Err(error) => return Err(error),
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            match self.restrict_existing_file(Path::new(&name)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Publish complete content atomically, preserving an existing target's owner and access.
    pub fn atomic_write(
        &self,
        path: &Path,
        overwrite: bool,
        write: impl FnOnce(&mut File) -> io::Result<()>,
    ) -> io::Result<()> {
        self.prepare_owned_parent(path)?;
        let target = if overwrite {
            self.target(path)?
        } else {
            path.to_path_buf()
        };
        self.prepare_owned_parent(&target)?;
        let original = if overwrite {
            match OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&target)
            {
                Ok(file) => Some(
                    self.regular_metadata(&file)
                        .map_err(|error| Self::at(&target, error))?,
                ),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(Self::at(&target, error)),
            }
        } else {
            None
        };
        let parent = target
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::Builder::new()
            .prefix(".actrail-write-")
            .permissions(Permissions::from_mode(if original.is_some() {
                0o600
            } else {
                0o640
            }))
            .tempfile_in(parent)
            .map_err(|error| Self::at(&target, error))?;
        if let Some(original) = original {
            self.preserve_metadata(temporary.as_file(), &original)
                .map_err(|error| Self::at(&target, error))?;
        }
        write(temporary.as_file_mut())
            .and_then(|()| temporary.flush())
            .map_err(|error| Self::at(&target, error))?;
        let result = if overwrite {
            temporary.persist(&target)
        } else {
            temporary.persist_noclobber(&target)
        };
        result.map_err(|error| Self::at(&target, error.error))?;
        Ok(())
    }

    pub fn preserve_metadata(&self, file: &File, original: &Metadata) -> io::Result<()> {
        let current = file.metadata()?;
        if (current.uid(), current.gid()) != (original.uid(), original.gid()) {
            // SAFETY: the descriptor is owned by file and both IDs came from metadata.
            if unsafe { libc::fchown(file.as_raw_fd(), original.uid(), original.gid()) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        file.set_permissions(Permissions::from_mode(original.mode() & 0o640))
    }

    fn target(&self, path: &Path) -> io::Result<PathBuf> {
        let mut target = path.to_path_buf();
        loop {
            match fs::canonicalize(&target) {
                Ok(path) => return Ok(path),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match fs::read_link(&target) {
                        Ok(link) => target = target.parent().unwrap_or(Path::new(".")).join(link),
                        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(target),
                        Err(error) => return Err(Self::at(&target, error)),
                    }
                }
                Err(error) => return Err(Self::at(&target, error)),
            }
        }
    }

    fn regular_metadata(&self, file: &File) -> io::Result<Metadata> {
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a regular file with one link",
            ));
        }
        Ok(metadata)
    }

    fn restrict(&self, file: &File, ceiling: u32) -> io::Result<()> {
        let current = file.metadata()?.mode();
        let desired = current & ceiling;
        if current & 0o7777 != desired {
            file.set_permissions(Permissions::from_mode(desired))?;
        }
        Ok(())
    }

    fn shared_directory(&self, path: &Path) -> bool {
        if [
            "/",
            "/tmp",
            "/var/tmp",
            "/etc",
            "/run",
            "/var",
            "/var/lib",
            "/var/log",
            "/var/cache",
            "/var/spool",
            "/usr",
            "/usr/bin",
            "/usr/sbin",
            "/usr/lib",
            "/usr/lib64",
            "/usr/libexec",
            "/usr/local",
            "/usr/local/bin",
            "/usr/local/sbin",
            "/usr/local/lib",
            "/usr/local/lib64",
            "/usr/local/libexec",
            "/usr/local/share",
            "/usr/share",
            "/bin",
            "/sbin",
            "/lib",
            "/lib64",
            "/opt",
            "/srv",
            "/mnt",
            "/media",
            "/home",
            "/root",
            "/dev",
            "/dev/shm",
            "/proc",
            "/sys",
        ]
        .iter()
        .any(|root| path == Path::new(root))
            || path.parent() == Some(Path::new("/home"))
        {
            return true;
        }
        [
            std::env::var_os("HOME").map(PathBuf::from),
            Some(std::env::temp_dir()),
        ]
        .into_iter()
        .flatten()
        .filter_map(|path| fs::canonicalize(path).ok())
        .any(|root| root == path)
    }

    fn at(path: &Path, error: io::Error) -> io::Error {
        io::Error::new(error.kind(), format!("{}: {error}", path.display()))
    }
}
