use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::ConfigStore;

pub(super) struct FileConfigStore {
    path: PathBuf,
}

impl FileConfigStore {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl ConfigStore for FileConfigStore {
    fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<String, String> {
        fs::read_to_string(&self.path)
            .map_err(|error| format!("read config {} failed: {error}", self.path.display()))
    }

    fn replace(&self, text: &str) -> Result<(), String> {
        let target = fs::canonicalize(&self.path)
            .map_err(|error| format!("resolve config {} failed: {error}", self.path.display()))?;
        // Check the target's write permission without truncating it. Resolving the
        // target also keeps a configured symlink intact during atomic replacement.
        let original = OpenOptions::new()
            .write(true)
            .open(&target)
            .map_err(|error| {
                format!(
                    "open config {} for writing failed: {error}",
                    target.display()
                )
            })?;
        let permissions = original
            .metadata()
            .map_err(|error| format!("read config permissions failed: {error}"))?
            .permissions();
        let parent = target
            .parent()
            .ok_or("config path has no parent directory")?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".actrail-config-")
            .tempfile_in(parent)
            .map_err(|error| {
                format!(
                    "create config temporary file in {} failed: {error}",
                    parent.display()
                )
            })?;
        temporary
            .write_all(text.as_bytes())
            .and_then(|()| temporary.flush())
            .and_then(|()| temporary.as_file().set_permissions(permissions))
            .map_err(|error| format!("write config temporary file failed: {error}"))?;
        temporary.persist(&target).map_err(|error| {
            format!(
                "replace config {} failed: {}",
                target.display(),
                error.error
            )
        })?;
        Ok(())
    }
}
