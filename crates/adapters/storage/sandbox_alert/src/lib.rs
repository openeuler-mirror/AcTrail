//! Independent SQLite storage for typed sandbox alerts with bounded asynchronous writes.

mod codec;
mod config;
mod reader;
mod schema;
mod status;
mod writer;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use sandbox_alert_store::{
    SandboxAlertCommitPort, SandboxAlertLifecyclePort, SandboxAlertReadPort,
    SandboxAlertShutdownError, SandboxAlertStatus, SandboxAlertStatusPort, SandboxAlertWritePort,
};

pub use config::{CURRENT_SCHEMA_VERSION, SandboxAlertSqliteConfig, SandboxAlertSynchronous};
pub use reader::SandboxAlertSqliteReader;

use status::StoreStatus;
use writer::SandboxAlertWriter;

pub struct SandboxAlertSqliteStore {
    writer: SandboxAlertWriter,
    reader: Arc<SandboxAlertSqliteReader>,
    status: Arc<StoreStatus>,
    shutdown_timeout: Duration,
}

impl SandboxAlertLifecyclePort for SandboxAlertSqliteStore {
    fn shutdown(&mut self, timeout: Duration) -> Result<(), SandboxAlertShutdownError> {
        self.shutdown_with_timeout(timeout)
    }
}

impl SandboxAlertSqliteStore {
    pub fn start(
        config: SandboxAlertSqliteConfig,
        commit_port: Arc<dyn SandboxAlertCommitPort>,
    ) -> Result<Self, String> {
        config.validate()?;
        prepare_parent_directory(&config.path, config.create_parent_directory)?;
        let status = Arc::new(StoreStatus::new(
            config.schema_version,
            config.writer_queue_capacity,
        ));
        let writer = SandboxAlertWriter::start(config.clone(), Arc::clone(&status), commit_port)?;
        let reader = Arc::new(SandboxAlertSqliteReader::new(
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

    pub fn write_port(&self) -> Arc<dyn SandboxAlertWritePort> {
        self.writer.port()
    }

    pub fn read_port(&self) -> Arc<dyn SandboxAlertReadPort> {
        self.reader.clone()
    }

    pub fn status_port(&self) -> Arc<dyn SandboxAlertStatusPort> {
        self.status.clone()
    }

    pub fn status(&self) -> SandboxAlertStatus {
        self.status.status()
    }

    pub fn shutdown(&mut self) -> Result<(), SandboxAlertShutdownError> {
        self.writer.shutdown(self.shutdown_timeout)
    }

    pub fn shutdown_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<(), SandboxAlertShutdownError> {
        self.writer.shutdown(timeout)
    }
}

impl Drop for SandboxAlertSqliteStore {
    fn drop(&mut self) {
        let _ = self.writer.shutdown(self.shutdown_timeout);
    }
}

fn prepare_parent_directory(path: &Path, create: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "sandbox alert path must have a parent directory".to_string())?;
    if parent.is_dir() {
        return Ok(());
    }
    if !create {
        return Err(format!(
            "sandbox alert parent directory {} does not exist",
            parent.display()
        ));
    }
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create sandbox alert directory {}: {error}",
            parent.display()
        )
    })
}
