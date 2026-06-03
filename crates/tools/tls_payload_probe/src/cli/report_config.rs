//! Reporter configuration shared by CLI argument parsing and formatting.

use clap::ValueEnum;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReporterConfig {
    pub(crate) redaction: RedactionMode,
    pub(crate) events: EventFilter,
    pub(crate) ring_stats: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RedactionMode {
    Redact,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ReportEvent {
    Payload,
    Http,
    Sse,
    Llm,
    Target,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EventFilter {
    payload: bool,
    http: bool,
    sse: bool,
    llm: bool,
    target: bool,
}

impl EventFilter {
    pub(crate) fn from_choices(choices: &[ReportEvent]) -> Self {
        if choices.is_empty() {
            return Self::all();
        }

        let mut filter = Self::none();
        for choice in choices {
            match choice {
                ReportEvent::Payload => filter.payload = true,
                ReportEvent::Http => filter.http = true,
                ReportEvent::Sse => filter.sse = true,
                ReportEvent::Llm => filter.llm = true,
                ReportEvent::Target => filter.target = true,
            }
        }
        filter
    }

    pub(crate) fn payload(self) -> bool {
        self.payload
    }

    pub(crate) fn http(self) -> bool {
        self.http
    }

    pub(crate) fn sse(self) -> bool {
        self.sse
    }

    pub(crate) fn llm(self) -> bool {
        self.llm
    }

    pub(crate) fn target(self) -> bool {
        self.target
    }

    fn all() -> Self {
        Self {
            payload: true,
            http: true,
            sse: true,
            llm: true,
            target: true,
        }
    }

    fn none() -> Self {
        Self {
            payload: false,
            http: false,
            sse: false,
            llm: false,
            target: false,
        }
    }
}
