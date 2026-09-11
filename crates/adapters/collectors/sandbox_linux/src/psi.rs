//! Fail-soft reading of Guest memory pressure-stall information.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sandbox_observation::{GuestBootId, GuestPressureSnapshot, PsiAverages};

use crate::SandboxLinuxError;
use crate::procfs::ProcfsReader;

/// Reads `/proc/pressure/memory` and reports Guest-wide PSI averages.
///
/// `open` returns `Ok(None)` when the running kernel has no PSI support
/// (`CONFIG_PSI` absent or disabled via `psi=0`), so callers can degrade
/// without failing the daemon. A real read/metadata error is returned as
/// `Err`.
pub struct PsiReader {
    path: PathBuf,
    boot_id: GuestBootId,
}

impl PsiReader {
    pub fn open(procfs_root: PathBuf) -> Result<Option<Self>, SandboxLinuxError> {
        let procfs = ProcfsReader::open(procfs_root)?;
        let path = procfs.root().join("pressure").join("memory");
        match fs::metadata(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(SandboxLinuxError::new(
                    "open_pressure",
                    format!("cannot inspect {}: {error}", path.display()),
                ));
            }
        }
        let boot_id = procfs.boot_id()?;
        Ok(Some(Self { path, boot_id }))
    }

    pub fn sample(&self) -> Result<GuestPressureSnapshot, SandboxLinuxError> {
        let raw = fs::read_to_string(&self.path).map_err(|error| {
            SandboxLinuxError::new(
                "read_pressure",
                format!("cannot read {}: {error}", self.path.display()),
            )
        })?;
        let (memory_some, memory_full) = parse_memory_pressure(&raw)?;
        let sampled_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| SandboxLinuxError::new("sample_clock", error.to_string()))?
            .as_millis()
            .try_into()
            .map_err(|error| {
                SandboxLinuxError::new("sample_clock", format!("timestamp overflow: {error}"))
            })?;
        Ok(GuestPressureSnapshot {
            guest_boot_id: self.boot_id,
            sampled_at_ms,
            memory_some,
            memory_full,
        })
    }
}

fn parse_memory_pressure(raw: &str) -> Result<(PsiAverages, PsiAverages), SandboxLinuxError> {
    let mut some = None;
    let mut full = None;
    for line in raw.lines() {
        let mut fields = line.split_whitespace();
        let Some(kind) = fields.next() else { continue };
        let mut avg10 = None;
        let mut avg60 = None;
        let mut avg300 = None;
        for field in fields {
            let Some((key, value)) = field.split_once('=') else {
                continue;
            };
            match key {
                "avg10" => avg10 = Some(parse_millipercent(value)?),
                "avg60" => avg60 = Some(parse_millipercent(value)?),
                "avg300" => avg300 = Some(parse_millipercent(value)?),
                _ => {}
            }
        }
        let averages = PsiAverages {
            avg10_millipercent: avg10.ok_or_else(|| malformed_pressure("avg10"))?,
            avg60_millipercent: avg60.ok_or_else(|| malformed_pressure("avg60"))?,
            avg300_millipercent: avg300.ok_or_else(|| malformed_pressure("avg300"))?,
        };
        match kind {
            "some" => some = Some(averages),
            "full" => full = Some(averages),
            _ => {}
        }
    }
    let some = some.ok_or_else(|| {
        SandboxLinuxError::new("parse_memory_pressure", "missing 'some' pressure line")
    })?;
    let full = full.ok_or_else(|| {
        SandboxLinuxError::new("parse_memory_pressure", "missing 'full' pressure line")
    })?;
    Ok((some, full))
}

fn parse_millipercent(raw: &str) -> Result<u32, SandboxLinuxError> {
    let (integer, fraction) = match raw.split_once('.') {
        Some((integer, fraction)) => (integer, fraction),
        None => (raw, ""),
    };
    if integer.is_empty() || !integer.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(malformed_value(raw));
    }
    if fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(malformed_value(raw));
    }
    let integer: u64 = integer.parse().map_err(|_| malformed_value(raw))?;
    let mut padded = fraction.to_string();
    while padded.len() < 3 {
        padded.push('0');
    }
    let fraction: u64 = if padded.is_empty() {
        0
    } else {
        padded.parse().map_err(|_| malformed_value(raw))?
    };
    let millipercent = integer
        .checked_mul(1000)
        .and_then(|value| value.checked_add(fraction))
        .ok_or_else(|| malformed_value(raw))?;
    u32::try_from(millipercent).map_err(|_| malformed_value(raw))
}

fn malformed_pressure(key: &str) -> SandboxLinuxError {
    SandboxLinuxError::new(
        "parse_memory_pressure",
        format!("missing pressure key {key}"),
    )
}

fn malformed_value(raw: &str) -> SandboxLinuxError {
    SandboxLinuxError::new(
        "parse_memory_pressure",
        format!("invalid pressure value {raw:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kernel_memory_pressure() {
        let raw = "some avg10=2.35 avg60=1.10 avg300=0.50 total=12345\n\
                   full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";
        let (some, full) = parse_memory_pressure(raw).expect("parse");
        assert_eq!(
            some,
            PsiAverages {
                avg10_millipercent: 2350,
                avg60_millipercent: 1100,
                avg300_millipercent: 500,
            }
        );
        assert_eq!(full, PsiAverages::default());
    }

    #[test]
    fn parses_integer_and_large_values() {
        let raw = "some avg10=0 avg60=12 avg300=1234.5 total=1\n\
                   full avg10=123.45 avg60=0.001 avg300=0.00 total=2\n";
        let (some, full) = parse_memory_pressure(raw).expect("parse");
        assert_eq!(some.avg10_millipercent, 0);
        assert_eq!(some.avg60_millipercent, 12_000);
        assert_eq!(some.avg300_millipercent, 1_234_500);
        assert_eq!(full.avg10_millipercent, 123_450);
        assert_eq!(full.avg60_millipercent, 1);
    }

    #[test]
    fn rejects_missing_keys() {
        let raw = "some avg10=0.00 avg60=0.00 total=1\n\
                   full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";
        assert!(parse_memory_pressure(raw).is_err());
    }

    #[test]
    fn rejects_missing_lines() {
        assert!(parse_memory_pressure("some avg10=0.00 avg60=0.00 avg300=0.00\n").is_err());
    }
}
