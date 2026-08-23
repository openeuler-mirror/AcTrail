use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{AlertForwardingConfig, AlertForwardingConfigError};

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub trait AlertForwardingConfigOwner: Send + Sync {
    fn load(&self) -> Result<AlertForwardingConfig, AlertForwardingConfigOwnerError>;

    fn persist(
        &self,
        config: &AlertForwardingConfig,
    ) -> Result<(), AlertForwardingConfigOwnerError>;
}

pub struct AlertForwardingConfigFileOwner {
    path: PathBuf,
}

impl AlertForwardingConfigFileOwner {
    pub fn new(path: PathBuf) -> Result<Self, AlertForwardingConfigOwnerError> {
        if !path.is_absolute() {
            return Err(AlertForwardingConfigOwnerError::InvalidPath(
                "alert forwarding config path must be absolute".to_string(),
            ));
        }
        if path.file_name().is_none() {
            return Err(AlertForwardingConfigOwnerError::InvalidPath(
                "alert forwarding config path must name a file".to_string(),
            ));
        }
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn temporary_path(&self) -> Result<PathBuf, AlertForwardingConfigOwnerError> {
        let file_name = self.path.file_name().ok_or_else(|| {
            AlertForwardingConfigOwnerError::InvalidPath(
                "alert forwarding config path must name a file".to_string(),
            )
        })?;
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Ok(self.path.with_file_name(format!(
            ".{}.tmp.{}.{}",
            file_name.to_string_lossy(),
            std::process::id(),
            sequence
        )))
    }

    fn write_replacement(
        &self,
        temporary_path: &Path,
        raw: &[u8],
    ) -> Result<File, AlertForwardingConfigOwnerError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(temporary_path)
            .map_err(|error| {
                AlertForwardingConfigOwnerError::io("create", temporary_path, error)
            })?;
        file.write_all(raw)
            .map_err(|error| AlertForwardingConfigOwnerError::io("write", temporary_path, error))?;
        file.sync_all()
            .map_err(|error| AlertForwardingConfigOwnerError::io("sync", temporary_path, error))?;
        Ok(file)
    }
}

impl AlertForwardingConfigOwner for AlertForwardingConfigFileOwner {
    fn load(&self) -> Result<AlertForwardingConfig, AlertForwardingConfigOwnerError> {
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|error| AlertForwardingConfigOwnerError::io("read", &self.path, error))?;
        AlertForwardingConfig::from_json(&raw).map_err(AlertForwardingConfigOwnerError::Config)
    }

    fn persist(
        &self,
        config: &AlertForwardingConfig,
    ) -> Result<(), AlertForwardingConfigOwnerError> {
        config
            .validate()
            .map_err(AlertForwardingConfigOwnerError::Config)?;
        let raw = config
            .to_json()
            .map_err(AlertForwardingConfigOwnerError::Config)?;
        let parent = self.path.parent().ok_or_else(|| {
            AlertForwardingConfigOwnerError::InvalidPath(
                "alert forwarding config path has no parent".to_string(),
            )
        })?;
        std::fs::create_dir_all(parent)
            .map_err(|error| AlertForwardingConfigOwnerError::io("create parent", parent, error))?;
        let temporary_path = self.temporary_path()?;
        let write_result = self.write_replacement(&temporary_path, raw.as_bytes());
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(error);
        }
        if let Err(error) = std::fs::rename(&temporary_path, &self.path) {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(AlertForwardingConfigOwnerError::io(
                "replace", &self.path, error,
            ));
        }
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| AlertForwardingConfigOwnerError::io("sync parent", parent, error))
    }
}

#[derive(Debug)]
pub enum AlertForwardingConfigOwnerError {
    InvalidPath(String),
    Config(AlertForwardingConfigError),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl AlertForwardingConfigOwnerError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }

    pub(crate) fn is_not_found(&self) -> bool {
        matches!(self, Self::Io { source, .. } if source.kind() == io::ErrorKind::NotFound)
    }
}

impl fmt::Display for AlertForwardingConfigOwnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(message) => formatter.write_str(message),
            Self::Config(error) => write!(formatter, "{error}"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "{operation} alert forwarding config {}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for AlertForwardingConfigOwnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::InvalidPath(_) => None,
        }
    }
}
