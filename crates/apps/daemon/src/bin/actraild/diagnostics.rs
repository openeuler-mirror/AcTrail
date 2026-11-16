//! Operator guidance and bounded evidence for a failed background startup.

use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::num::NonZeroU64;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use config_core::daemon::OperatorConfig;

pub(super) struct DaemonDiagnostics;

impl DaemonDiagnostics {
    pub(super) fn load(path: &Path) -> Result<OperatorConfig, String> {
        OperatorConfig::load(path).map_err(|error| format!(
            "load daemon configuration {} failed: {error}; check --config and file permissions, or initialize a configuration with actrailctl --config <path> init",
            path.display()
        ))
    }

    pub(super) fn operation(operation: &str, config: &OperatorConfig, error: String) -> String {
        format!(
            "daemon {operation} failed: {error}; {}",
            Self::guidance(config)
        )
    }

    pub(super) fn foreground_failure(config: &OperatorConfig) {
        // Keep guidance out of the cause returned to main. A supervising start
        // reads that final cause and supplies its own single guidance block.
        tracing::error!("daemon run failed; {}", Self::guidance(config));
    }

    fn guidance(config: &OperatorConfig) -> String {
        let hand = if config.hand_observation.enabled {
            format!(
                "; Hand observation listen_addr={}",
                config.hand_observation.listen_addr
            )
        } else {
            String::new()
        };
        format!(
            "control socket={}{}; check endpoint ownership, configured addresses and permissions; inspect log {}",
            config.socket_path.display(),
            hand,
            config.log_path.display()
        )
    }
}

pub(super) struct StartupLog {
    initial: Metadata,
    max_bytes: NonZeroU64,
}

impl StartupLog {
    const DEFAULT_MAX_BYTES: NonZeroU64 =
        NonZeroU64::new(16 * 1024).expect("nonzero startup log default");

    pub(super) fn max_bytes() -> Result<NonZeroU64, String> {
        const VARIABLE: &str = "ACTRAIL_DAEMON_STARTUP_LOG_MAX_BYTES";
        match std::env::var(VARIABLE) {
            Ok(value) => value.parse::<NonZeroU64>().map_err(|error| format!(
                "{VARIABLE}={value:?} must be a positive integer ({error}); set a positive byte limit or unset {VARIABLE} to use {} bytes",
                Self::DEFAULT_MAX_BYTES
            )),
            Err(std::env::VarError::NotPresent) => Ok(Self::DEFAULT_MAX_BYTES),
            Err(error) => Err(format!(
                "read {VARIABLE}: {error}; set a positive byte limit or unset {VARIABLE} to use {} bytes",
                Self::DEFAULT_MAX_BYTES
            )),
        }
    }

    pub(super) fn capture(log: &File, max_bytes: NonZeroU64) -> Result<Self, String> {
        let initial = log
            .metadata()
            .map_err(|error| format!("inspect startup log: {error}"))?;
        Ok(Self { initial, max_bytes })
    }

    pub(super) fn failure(&self, path: &Path, error: String) -> String {
        let last_line = self
            .last_line(path)
            .unwrap_or_else(|error| format!("startup log unavailable: {error}"));
        format!("{error}; last startup output: {last_line}")
    }

    fn last_line(&self, path: &Path) -> Result<String, String> {
        let mut file =
            File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
        let current = file
            .metadata()
            .map_err(|error| format!("inspect {}: {error}", path.display()))?;
        if current.dev() != self.initial.dev() || current.ino() != self.initial.ino() {
            return Err(format!("log {} changed during startup", path.display()));
        }
        if current.len() <= self.initial.len() {
            return Ok("no output recorded by this startup".to_string());
        }
        let start = self
            .initial
            .len()
            .max(current.len().saturating_sub(self.max_bytes.get()));
        file.seek(SeekFrom::Start(start))
            .map_err(|error| format!("seek startup log: {error}"))?;
        let mut bytes = Vec::new();
        file.take(self.max_bytes.get())
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read startup log: {error}"))?;
        Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .rev()
            .find(|line| !line.is_empty())
            .unwrap_or("startup output exceeds the diagnostic window; inspect the log file")
            .to_string())
    }
}
