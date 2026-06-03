//! CLI and runtime configuration values.

use std::ffi::OsString;
use std::path::PathBuf;

use tls_payload_core::RewriteRule;
pub(crate) use tls_payload_sync::{EventFilter, RedactionMode};
use tls_probe_point_finder::fast::{ArchFilter, ProviderFilter, SourceFilter};

pub(crate) const DEFAULT_MATCH_LIMIT: usize = 8;
pub(crate) const DEFAULT_MAX_PAYLOAD_BYTES: usize = 262_144;

#[derive(Clone, Debug)]
pub(crate) struct ProbeConfig {
    pub(crate) command: Vec<OsString>,
    pub(crate) arch: ArchFilter,
    pub(crate) provider: ProviderFilter,
    pub(crate) source: SourceFilter,
    pub(crate) match_limit: usize,
    pub(crate) libraries: Vec<PathBuf>,
    pub(crate) library_search_dirs: Vec<PathBuf>,
    pub(crate) rules: Vec<RewriteRule>,
    pub(crate) max_payload_bytes: usize,
    pub(crate) redaction: RedactionMode,
    pub(crate) events: EventFilter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReportEvent {
    Target,
    Payload,
    Decision,
}

pub(crate) fn event_filter_from_events(events: &[ReportEvent]) -> EventFilter {
    if events.is_empty() {
        return EventFilter::all();
    }
    EventFilter {
        target: events.contains(&ReportEvent::Target),
        payload: events.contains(&ReportEvent::Payload),
        decision: events.contains(&ReportEvent::Decision),
    }
}
