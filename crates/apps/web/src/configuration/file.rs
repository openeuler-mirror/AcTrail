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
        file_permissions::FilePermissions
            .read_only(&self.path)
            .map_err(|error| format!("read config {} failed: {error}", self.path.display()))
    }

    fn replace(&self, text: &str) -> Result<(), String> {
        file_permissions::FilePermissions
            .atomic_write(&self.path, true, |file| file.write_all(text.as_bytes()))
            .map_err(|error| format!("replace config {} failed: {error}", self.path.display()))
    }
}
