//! Bound libbpf diagnostics while retaining program names and verifier failures.

use std::env;
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

use libbpf_rs::PrintLevel;

use super::LoaderError;

// libbpf's print callback is process-wide. Publish its byte budget once per
// loader configuration, keeping environment parsing out of the callback.
static LOG_MAX_BYTES: AtomicUsize = AtomicUsize::new(LibbpfLogging::DEFAULT_LOG_MAX_BYTES);

pub(super) struct LibbpfLogging;

impl LibbpfLogging {
    const DEBUG_ENV: &str = "ACTRAIL_EBPF_LIBBPF_DEBUG";
    const MAX_BYTES_ENV: &str = "ACTRAIL_EBPF_LIBBPF_LOG_MAX_BYTES";
    const DEFAULT_LOG_MAX_BYTES: usize = 4096;

    pub(super) fn configure() -> Result<(), LoaderError> {
        let debug = Self::debug_enabled()?;
        let max_bytes = Self::max_bytes()?;
        LOG_MAX_BYTES.store(max_bytes, Ordering::Relaxed);
        if debug {
            libbpf_rs::set_print(Some((PrintLevel::Debug, Self::print_full)));
        } else {
            libbpf_rs::set_print(Some((PrintLevel::Info, Self::print_summary)));
        }
        Ok(())
    }

    fn debug_enabled() -> Result<bool, LoaderError> {
        let Some(value) = env::var_os(Self::DEBUG_ENV) else {
            return Ok(false);
        };
        let value = value.to_string_lossy().to_ascii_lowercase();
        match value.as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(LoaderError::new(
                "libbpf_debug_config",
                format!(
                    "{} must be one of 1,true,yes,on,0,false,no,off; got {value}",
                    Self::DEBUG_ENV
                ),
            )),
        }
    }

    fn max_bytes() -> Result<usize, LoaderError> {
        let Some(value) = env::var_os(Self::MAX_BYTES_ENV) else {
            return Ok(Self::DEFAULT_LOG_MAX_BYTES);
        };
        let value = value.to_string_lossy();
        value
            .parse::<usize>()
            .ok()
            .filter(|size| *size > 0)
            .ok_or_else(|| {
                LoaderError::new(
                    "libbpf_log_config",
                    format!(
                        "{} must be a positive byte count; got {value}",
                        Self::MAX_BYTES_ENV
                    ),
                )
            })
    }

    fn print_full(_level: PrintLevel, message: String) {
        let _ = io::stderr().lock().write_all(message.as_bytes());
    }

    fn print_summary(level: PrintLevel, message: String) {
        let max_bytes = LOG_MAX_BYTES.load(Ordering::Relaxed);
        if message.len() <= max_bytes {
            Self::print_full(level, message);
            return;
        }

        // Keep a short first line (the program name) and spend the remaining
        // budget on the tail, where the verifier records its rejection reason.
        let head_end = message.as_bytes()[..max_bytes / 2]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let mut tail_start = message.len() - (max_bytes - head_end);
        while !message.is_char_boundary(tail_start) {
            tail_start += 1;
        }

        let mut stderr = io::stderr().lock();
        let _ = stderr.write_all(&message.as_bytes()[..head_end]);
        let _ = writeln!(
            stderr,
            "libbpf: diagnostic truncated (omitted {} bytes; set {}=1 for full output)",
            tail_start - head_end,
            Self::DEBUG_ENV
        );
        let _ = stderr.write_all(&message.as_bytes()[tail_start..]);
    }
}
