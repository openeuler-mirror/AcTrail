//! Independent SQLite storage for sandbox evidence with asynchronous bounded writes.

mod codec;
mod config;
mod reader;
mod schema;
mod status;
mod writer;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use sandbox_evidence_store::{
    SandboxEvidenceLifecyclePort, SandboxEvidenceReadPort, SandboxEvidenceShutdownError,
    SandboxEvidenceStatus, SandboxEvidenceStatusPort, SandboxEvidenceWritePort,
};

pub use config::{CURRENT_SCHEMA_VERSION, SandboxEvidenceSqliteConfig, SandboxEvidenceSynchronous};
pub use reader::SandboxEvidenceSqliteReader;

use status::StoreStatus;
use writer::SandboxEvidenceWriter;

pub struct SandboxEvidenceSqliteStore {
    writer: SandboxEvidenceWriter,
    reader: Arc<SandboxEvidenceSqliteReader>,
    status: Arc<StoreStatus>,
    shutdown_timeout: Duration,
}

impl SandboxEvidenceLifecyclePort for SandboxEvidenceSqliteStore {
    fn shutdown(&mut self, timeout: Duration) -> Result<(), SandboxEvidenceShutdownError> {
        self.shutdown_with_timeout(timeout)
    }
}

impl SandboxEvidenceSqliteStore {
    pub fn start(config: SandboxEvidenceSqliteConfig) -> Result<Self, String> {
        config.validate()?;
        prepare_parent_directory(&config.path, config.create_parent_directory)?;
        let status = Arc::new(StoreStatus::new(
            config.schema_version,
            config.writer_queue_capacity,
        ));
        let writer = SandboxEvidenceWriter::start(config.clone(), Arc::clone(&status))?;
        let reader = Arc::new(SandboxEvidenceSqliteReader::new(
            config.path.clone(),
            config.schema_version,
            config.busy_timeout,
            config.read_limit_max,
        ));
        Ok(Self {
            writer,
            reader,
            status,
            shutdown_timeout: config.shutdown_drain_timeout,
        })
    }

    pub fn write_port(&self) -> Arc<dyn SandboxEvidenceWritePort> {
        self.writer.port()
    }

    pub fn read_port(&self) -> Arc<dyn SandboxEvidenceReadPort> {
        self.reader.clone()
    }

    pub fn status_port(&self) -> Arc<dyn SandboxEvidenceStatusPort> {
        self.status.clone()
    }

    pub fn status(&self) -> SandboxEvidenceStatus {
        self.status.status()
    }

    pub fn shutdown(&mut self) -> Result<(), SandboxEvidenceShutdownError> {
        self.writer.shutdown(self.shutdown_timeout)
    }

    pub fn shutdown_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<(), SandboxEvidenceShutdownError> {
        self.writer.shutdown(timeout)
    }
}

impl Drop for SandboxEvidenceSqliteStore {
    fn drop(&mut self) {
        if let Err(error) = self.writer.shutdown(self.shutdown_timeout) {
            eprintln!("sandbox evidence store drop shutdown failed: {error}");
        }
    }
}

fn prepare_parent_directory(path: &Path, create: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "sandbox evidence path must have a parent directory".to_string())?;
    if parent.is_dir() {
        return Ok(());
    }
    if !create {
        return Err(format!(
            "sandbox evidence parent directory {} does not exist",
            parent.display()
        ));
    }
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create sandbox evidence directory {}: {error}",
            parent.display()
        )
    })
}
