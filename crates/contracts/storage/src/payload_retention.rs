//! Write-side payload byte budgets, independently applied to each source.

use model_core::payload::PayloadSourceBoundary;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadRetentionLimits {
    pub tls: u64,
    pub socket: u64,
    pub stdio: u64,
}

impl PayloadRetentionLimits {
    pub fn for_source(self, source: PayloadSourceBoundary) -> u64 {
        match source {
            PayloadSourceBoundary::TlsUserSpace => self.tls,
            PayloadSourceBoundary::Syscall => self.socket,
            PayloadSourceBoundary::Stdio => self.stdio,
        }
    }
}
