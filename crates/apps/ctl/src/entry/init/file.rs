use std::fs::{self, File, Metadata, OpenOptions, Permissions};
use std::io::{ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub(super) struct ConfigOutput<'a> {
    path: &'a Path,
    overwrite: bool,
}

impl<'a> ConfigOutput<'a> {
    pub(super) fn new(path: &'a Path, overwrite: bool) -> Self {
        Self { path, overwrite }
    }

    pub(super) fn write(&self, raw: &str) -> Result<(), String> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                format!("create config directory {}: {error}", parent.display())
            })?;
        }
        let target = self.target()?;
        let original = if self.overwrite {
            match OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&target)
            {
                Ok(file) => {
                    let metadata = file.metadata().map_err(|error| {
                        format!("read config metadata {}: {error}", target.display())
                    })?;
                    if !metadata.is_file() {
                        return Err(format!(
                            "config target must be a regular file: {}",
                            target.display()
                        ));
                    }
                    Some(metadata)
                }
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => return Err(format!("open config {}: {error}", target.display())),
            }
        } else {
            None
        };
        let parent = target
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut builder = tempfile::Builder::new();
        builder.prefix(".actrail-config-");
        if original.is_none() {
            // Set the creation mode through open(2), so the process umask still applies.
            builder.permissions(Permissions::from_mode(0o666));
        }
        let mut temporary = builder.tempfile_in(parent).map_err(|error| {
            format!(
                "create config temporary file in {}: {error}",
                parent.display()
            )
        })?;
        temporary
            .write_all(raw.as_bytes())
            .and_then(|()| temporary.flush())
            .map_err(|error| format!("write config temporary file: {error}"))?;
        if let Some(metadata) = original {
            self.preserve_permissions(temporary.as_file(), &metadata)?;
        }
        let result = if self.overwrite {
            temporary.persist(&target)
        } else {
            temporary.persist_noclobber(&target)
        };
        result.map_err(|error| format!("publish config {}: {}", target.display(), error.error))?;
        Ok(())
    }

    fn target(&self) -> Result<PathBuf, String> {
        let mut target = self.path.to_path_buf();
        if !self.overwrite {
            return Ok(target);
        }
        loop {
            match fs::canonicalize(&target) {
                Ok(resolved) => return Ok(resolved),
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    // Follow a dangling final link to create its referent. Each
                    // canonicalize also lets the OS reject cyclic symlink chains.
                    match fs::read_link(&target) {
                        Ok(link) => {
                            target = target.parent().unwrap_or_else(|| Path::new(".")).join(link);
                        }
                        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(target),
                        Err(error) => {
                            return Err(format!("resolve config {}: {error}", target.display()));
                        }
                    }
                }
                Err(error) => {
                    return Err(format!("resolve config {}: {error}", target.display()));
                }
            }
        }
    }

    fn preserve_permissions(&self, file: &File, original: &Metadata) -> Result<(), String> {
        let current = file
            .metadata()
            .map_err(|error| format!("read config temporary file metadata: {error}"))?;
        if current.uid() != original.uid() || current.gid() != original.gid() {
            // SAFETY: file owns a valid descriptor; the IDs come from the original file.
            if unsafe { libc::fchown(file.as_raw_fd(), original.uid(), original.gid()) } != 0 {
                return Err(format!(
                    "preserve config ownership {}: {}",
                    self.path.display(),
                    std::io::Error::last_os_error()
                ));
            }
        }
        // chown can clear mode bits, so permissions are applied afterwards.
        file.set_permissions(original.permissions())
            .map_err(|error| {
                format!(
                    "preserve config permissions {}: {error}",
                    self.path.display()
                )
            })
    }
}
