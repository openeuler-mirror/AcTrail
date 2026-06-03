//! Assembly data model.

use std::collections::BTreeMap;

use crate::capture::{CaptureConfig, CaptureDirection, CaptureEvent};

use super::decode::HttpDecodeConfig;

const HEADER_CONTENT_TYPE: &str = "content-type";
const CONTENT_TYPE_EVENT_STREAM: &str = "text/event-stream";

#[derive(Clone, Copy, Debug)]
pub(super) struct AssemblyConfig {
    pub(super) max_buffer_bytes: usize,
    pub(super) decode: HttpDecodeConfig,
}

impl From<&CaptureConfig> for AssemblyConfig {
    fn from(config: &CaptureConfig) -> Self {
        Self {
            max_buffer_bytes: config.assemble_buffer_bytes,
            decode: HttpDecodeConfig::from(config),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct AssemblyKey {
    pub(super) pid: u32,
    pub(super) stream_key: u64,
    pub(super) direction: CaptureDirection,
}

impl AssemblyKey {
    pub(super) fn from_event(event: &CaptureEvent) -> Self {
        Self {
            pid: event.pid,
            stream_key: event.stream_key,
            direction: event.direction,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HttpAssemblyOutput {
    Message(AssembledHttp),
    BodyFragment(HttpBodyFragment),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AssembledHttp {
    pub(crate) pid: u32,
    pub(crate) stream_key: u64,
    pub(crate) direction: CaptureDirection,
    pub(crate) first_line: String,
    pub(crate) headers: Vec<HttpHeader>,
    pub(crate) body: HttpBody,
}

impl AssembledHttp {
    pub(crate) fn is_event_stream(&self) -> bool {
        headers_are_event_stream(&self.headers)
    }

    pub(crate) fn body_text(&self) -> Option<&str> {
        self.body.text()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpHeader {
    pub(crate) name: String,
    pub(crate) value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HttpBody {
    Empty,
    Text {
        bytes: usize,
        text: String,
    },
    Binary {
        bytes: usize,
    },
    DecodedText {
        encoding: String,
        compressed_bytes: usize,
        decoded_bytes: usize,
        text: String,
    },
    DecodedBinary {
        encoding: String,
        compressed_bytes: usize,
        decoded_bytes: usize,
    },
    DecodeSkipped {
        encoding: String,
        compressed_bytes: usize,
        limit_bytes: usize,
    },
    DecodeFailed {
        encoding: String,
        compressed_bytes: usize,
        error: String,
    },
    Partial {
        buffered_bytes: usize,
        reason: String,
    },
    PartialText {
        bytes: usize,
        buffered_bytes: usize,
        reason: String,
        text: String,
    },
    PartialDecodedText {
        encoding: String,
        compressed_bytes: usize,
        decoded_bytes: usize,
        buffered_bytes: usize,
        reason: String,
        text: String,
    },
    Streamed {
        bytes: usize,
    },
}

impl HttpBody {
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Self::Text { text, .. }
            | Self::DecodedText { text, .. }
            | Self::PartialText { text, .. }
            | Self::PartialDecodedText { text, .. } => Some(text),
            Self::Empty
            | Self::Binary { .. }
            | Self::DecodedBinary { .. }
            | Self::DecodeSkipped { .. }
            | Self::DecodeFailed { .. }
            | Self::Partial { .. }
            | Self::Streamed { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpBodyFragment {
    pub(crate) pid: u32,
    pub(crate) stream_key: u64,
    pub(crate) direction: CaptureDirection,
    pub(crate) first_line: String,
    pub(crate) headers: Vec<HttpHeader>,
    pub(crate) body: HttpBodyFragmentBody,
}

impl HttpBodyFragment {
    pub(crate) fn is_event_stream(&self) -> bool {
        headers_are_event_stream(&self.headers)
    }

    pub(crate) fn body_text(&self) -> Option<&str> {
        match &self.body {
            HttpBodyFragmentBody::Text { text, .. } => Some(text),
            HttpBodyFragmentBody::Binary { .. } => None,
        }
    }

    pub(crate) fn body_bytes(&self) -> &[u8] {
        match &self.body {
            HttpBodyFragmentBody::Text { data, .. } | HttpBodyFragmentBody::Binary { data, .. } => {
                data
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HttpBodyFragmentBody {
    Text {
        bytes: usize,
        text: String,
        data: Vec<u8>,
    },
    Binary {
        bytes: usize,
        data: Vec<u8>,
    },
}

pub(super) struct ParsedHeaders {
    pub(super) first_line: String,
    pub(super) headers: Vec<HttpHeader>,
    pub(super) fields: BTreeMap<String, String>,
}

fn headers_are_event_stream(headers: &[HttpHeader]) -> bool {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(HEADER_CONTENT_TYPE))
        .map(|header| {
            header
                .value
                .split(';')
                .next()
                .unwrap_or(&header.value)
                .trim()
                .eq_ignore_ascii_case(CONTENT_TYPE_EVENT_STREAM)
        })
        .unwrap_or(false)
}
