//! File access for an ELF and its dependencies.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::ElfImage;
use crate::{ToolError, ToolResult};

/// Opens runtime paths in the filesystem used by the target process.
pub trait ProbeFileSource {
    fn open(&self, path: &Path) -> io::Result<File>;
}

impl ElfImage {
    pub(crate) fn parse_related(&self, path: &Path) -> ToolResult<Self> {
        Self::parse_with_cache(
            path,
            self.scan_mode,
            self.scan_chunk_bytes,
            self.analysis_cache.as_ref().map(Rc::clone),
            self.file_source.as_ref().map(Rc::clone),
        )
    }

    pub(crate) fn related_path(&self, path: &Path) -> ToolResult<PathBuf> {
        match &self.file_source {
            Some(source) => {
                source.open(path).map_err(|error| {
                    ToolError::new(format!(
                        "cannot open dependency {}: {error}",
                        path.display()
                    ))
                })?;
                Ok(path.to_path_buf())
            }
            None => std::fs::canonicalize(path).map_err(|error| {
                ToolError::new(format!("cannot resolve {}: {error}", path.display()))
            }),
        }
    }

    pub(crate) fn related_exists(&self, path: &Path) -> bool {
        match &self.file_source {
            Some(source) => source.open(path).is_ok(),
            None => path.exists(),
        }
    }

    pub(crate) fn uses_target_filesystem(&self) -> bool {
        self.file_source.is_some()
    }

    pub(crate) fn file_key(&self) -> (u64, u64) {
        self.file_key
    }
}
