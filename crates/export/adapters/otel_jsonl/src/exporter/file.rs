//! JSONL file exporter implementation.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};

use export_core::{BestEffortSink, ExportError};

use crate::config::FileExporterConfig;

const EXPORTER_NAME: &str = "otel_live_jsonl";

pub(super) struct FileExporterSink {
    writer: BufWriter<File>,
    flush_every_lines: usize,
    pending_flush: usize,
}

impl FileExporterSink {
    pub(super) fn open(config: FileExporterConfig) -> Result<Self, ExportError> {
        let flush_every_lines = usize::try_from(config.flush_every_spans).map_err(|error| {
            ExportError::new(EXPORTER_NAME, format!("flush span count overflow: {error}"))
        })?;
        Self::create_parent_directory(&config)?;
        let mut options = OpenOptions::new();
        options.write(true);
        if config.overwrite_enabled {
            options.create(true).truncate(true);
        } else {
            options.create_new(true);
        }
        let file = options.open(&config.path).map_err(|error| {
            ExportError::new(
                EXPORTER_NAME,
                format!("open {} failed: {error}", config.path.display()),
            )
        })?;
        Ok(Self {
            writer: BufWriter::new(file),
            flush_every_lines,
            pending_flush: usize::default(),
        })
    }

    fn create_parent_directory(config: &FileExporterConfig) -> Result<(), ExportError> {
        let Some(parent) = config
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        else {
            return Ok(());
        };
        fs::create_dir_all(parent).map_err(|error| {
            ExportError::new(
                EXPORTER_NAME,
                format!(
                    "create live OTEL output directory {} failed: {error}",
                    parent.display(),
                ),
            )
        })
    }
}

impl BestEffortSink<String> for FileExporterSink {
    fn deliver(&mut self, line: String) -> Result<u64, String> {
        writeln!(self.writer, "{line}").map_err(|error| error.to_string())?;
        self.pending_flush = self.pending_flush.saturating_add(1);
        if self.pending_flush < self.flush_every_lines {
            return Ok(u64::default());
        }
        self.writer.flush().map_err(|error| error.to_string())?;
        let delivered = u64::try_from(self.pending_flush).unwrap_or(u64::MAX);
        self.pending_flush = usize::default();
        Ok(delivered)
    }

    fn finish(&mut self) -> Result<u64, String> {
        self.writer.flush().map_err(|error| error.to_string())?;
        let delivered = u64::try_from(self.pending_flush).unwrap_or(u64::MAX);
        self.pending_flush = usize::default();
        Ok(delivered)
    }
}
