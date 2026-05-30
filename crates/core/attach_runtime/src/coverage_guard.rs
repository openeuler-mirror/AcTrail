//! Coverage-guard ownership during bootstrap race windows.

use std::time::SystemTime;

use collector_binding::CoverageGuardHandle;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapTimings {
    pub request_received_at: SystemTime,
    pub capability_checked_at: SystemTime,
    pub coverage_guard_installed_at: SystemTime,
    pub bootstrap_snapshot_started_at: SystemTime,
    pub bootstrap_snapshot_finished_at: SystemTime,
    pub trace_started_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageWindow {
    pub handle: CoverageGuardHandle,
    pub installed_at: SystemTime,
}
