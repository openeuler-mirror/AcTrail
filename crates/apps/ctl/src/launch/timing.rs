//! Env-gated launch timing diagnostics.

use std::ffi::OsStr;
use std::time::Instant;

const ENV_LAUNCH_TIMING: &str = "ACTRAIL_LAUNCH_TIMING";

pub(super) struct LaunchTiming {
    enabled: bool,
    started_at: Instant,
    last_at: Instant,
}

impl LaunchTiming {
    pub(super) fn from_env() -> Self {
        let enabled = std::env::var_os(ENV_LAUNCH_TIMING)
            .as_deref()
            .is_some_and(enabled_value);
        let now = Instant::now();
        Self {
            enabled,
            started_at: now,
            last_at: now,
        }
    }

    pub(super) fn mark(&mut self, stage: &str) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        let delta = now.duration_since(self.last_at);
        let total = now.duration_since(self.started_at);
        self.last_at = now;
        eprintln!(
            "[actrailctl launch timing] +{:.3}ms total={:.3}ms stage={stage}",
            delta.as_secs_f64() * 1000.0,
            total.as_secs_f64() * 1000.0
        );
    }

    pub(super) fn mark_detail(&mut self, stage: &str, detail: impl std::fmt::Display) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        let delta = now.duration_since(self.last_at);
        let total = now.duration_since(self.started_at);
        self.last_at = now;
        eprintln!(
            "[actrailctl launch timing] +{:.3}ms total={:.3}ms stage={stage} {detail}",
            delta.as_secs_f64() * 1000.0,
            total.as_secs_f64() * 1000.0
        );
    }
}

fn enabled_value(value: &OsStr) -> bool {
    let value = value.to_string_lossy();
    let normalized = value.trim().to_ascii_lowercase();
    !matches!(normalized.as_str(), "" | "0" | "false" | "off" | "no")
}
