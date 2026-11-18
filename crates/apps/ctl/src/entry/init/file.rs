use std::io::Write;
use std::path::Path;

pub(super) struct ConfigOutput<'a> {
    path: &'a Path,
    overwrite: bool,
}

impl<'a> ConfigOutput<'a> {
    pub(super) fn new(path: &'a Path, overwrite: bool) -> Self {
        Self { path, overwrite }
    }

    pub(super) fn write(&self, raw: &str) -> Result<(), String> {
        file_permissions::FilePermissions
            .atomic_write(self.path, self.overwrite, |file| {
                file.write_all(raw.as_bytes())
            })
            .map_err(|error| format!("write config {}: {error}", self.path.display()))
    }
}
