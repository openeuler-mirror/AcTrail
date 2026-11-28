use std::io::Write;
use std::path::Path;

pub(super) struct OperatorConfigFile<'a> {
    path: &'a Path,
}

impl<'a> OperatorConfigFile<'a> {
    pub(super) fn new(path: &'a Path) -> Self {
        Self { path }
    }

    pub(super) fn write(&self, overwrite: bool, raw: &str) -> Result<(), String> {
        file_permissions::FilePermissions
            .atomic_write(self.path, overwrite, |file| file.write_all(raw.as_bytes()))
            .map_err(|error| format!("write config {}: {error}", self.path.display()))
    }
}
