//! Output shaping for low-level eBPF inspection results.

use model_core::ids::TraceId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveVerificationReport {
    pub trace_id: TraceId,
    pub process_events: Vec<String>,
    pub file_events: Vec<String>,
    pub net_events: Vec<String>,
    pub ipc_events: Vec<String>,
    pub resource_events: Vec<String>,
    pub provider_events: Vec<String>,
    pub stdio_payloads: Vec<String>,
}

pub fn format_live_verification_report(report: &LiveVerificationReport) -> String {
    format!(
        "live verification passed\ntrace_id={}\nprocess_events={}\nfile_events={}\nnet_events={}\nipc_events={}\nresource_events={}\nprovider_events={}\nstdio_payloads={}",
        report.trace_id,
        report.process_events.join(","),
        report.file_events.join(","),
        report.net_events.join(","),
        report.ipc_events.join(","),
        report.resource_events.join(","),
        report.provider_events.join(","),
        report.stdio_payloads.join(","),
    )
}
