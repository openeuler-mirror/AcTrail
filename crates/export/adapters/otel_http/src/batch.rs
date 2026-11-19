//! Encoded records and transport batch assembly.

use export_core::{ExportError, SemanticActionExportAdapter, SemanticActionExportRecord};

use crate::config::OtelEncoding;
use crate::live_http::OTEL_HTTP_EXPORTER_NAME;

/// One record encoded in the route's configured wire format.
pub(super) enum EncodedRecord {
    /// A `{"resourceSpans":[...]}` JSON document.
    Json(String),
    /// A single `ExportTraceServiceRequest` protobuf message; concatenating
    /// several yields one valid merged request.
    Proto(Vec<u8>),
}

pub(super) struct OtelHttpSemanticActionAdapter {
    pub(super) encoding: OtelEncoding,
}

impl SemanticActionExportAdapter for OtelHttpSemanticActionAdapter {
    type Message = EncodedRecord;

    fn name(&self) -> &'static str {
        OTEL_HTTP_EXPORTER_NAME
    }

    fn encode(&self, record: SemanticActionExportRecord<'_>) -> Result<Self::Message, ExportError> {
        Ok(match self.encoding {
            OtelEncoding::Json => EncodedRecord::Json(export_otel_codec::render_otlp_json_line(
                record.trace,
                record.action,
                record.links,
            )),
            OtelEncoding::Protobuf => {
                EncodedRecord::Proto(export_otel_codec::render_otlp_protobuf_line(
                    record.trace,
                    record.action,
                    record.links,
                ))
            }
        })
    }
}

/// The buffered batch, in whichever wire format the route emits.
pub(super) enum PendingBatch {
    /// Accumulated `resourceSpans` array elements.
    Json(Vec<serde_json::Value>),
    /// Concatenated `ExportTraceServiceRequest` bytes. `record_ends` holds the
    /// end offset of each appended record, which is both the span count and the
    /// only way to cut the concatenation back apart on a 413.
    Proto {
        bytes: Vec<u8>,
        record_ends: Vec<usize>,
    },
}

impl PendingBatch {
    pub(super) fn new(encoding: OtelEncoding) -> Self {
        match encoding {
            OtelEncoding::Json => Self::Json(Vec::new()),
            OtelEncoding::Protobuf => Self::Proto {
                bytes: Vec::new(),
                record_ends: Vec::new(),
            },
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of buffered spans (drives the count-based flush).
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Json(spans) => spans.len(),
            Self::Proto { record_ends, .. } => record_ends.len(),
        }
    }

    /// Move the second half of the batch into a new batch, leaving the first
    /// half here. `None` when a single record cannot be divided any further.
    pub(super) fn split_off_half(&mut self) -> Option<Self> {
        if self.len() < 2 {
            return None;
        }
        match self {
            Self::Json(spans) => Some(Self::Json(spans.split_off(spans.len() / 2))),
            Self::Proto { bytes, record_ends } => {
                let middle = record_ends.len() / 2;
                let cut = record_ends[middle - 1];
                let tail_bytes = bytes.split_off(cut);
                let tail_ends = record_ends
                    .split_off(middle)
                    .into_iter()
                    .map(|end| end - cut)
                    .collect();
                Some(Self::Proto {
                    bytes: tail_bytes,
                    record_ends: tail_ends,
                })
            }
        }
    }

    /// Fold one encoded record into the batch. A record whose format does not
    /// match the batch, or a JSON record without `resourceSpans`, is dropped
    /// loudly rather than corrupting the batch.
    pub(super) fn append(&mut self, record: EncodedRecord) {
        match (self, record) {
            (Self::Json(spans), EncodedRecord::Json(line)) => {
                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(mut document) => match document
                        .get_mut("resourceSpans")
                        .and_then(serde_json::Value::as_array_mut)
                    {
                        Some(resource_spans) => spans.append(resource_spans),
                        None => {
                            eprintln!(
                                "{OTEL_HTTP_EXPORTER_NAME}: dropped record without resourceSpans"
                            )
                        }
                    },
                    Err(error) => {
                        eprintln!("{OTEL_HTTP_EXPORTER_NAME}: dropped undecodable record: {error}")
                    }
                }
            }
            (Self::Proto { bytes, record_ends }, EncodedRecord::Proto(record_bytes)) => {
                bytes.extend_from_slice(&record_bytes);
                record_ends.push(bytes.len());
            }
            _ => eprintln!("{OTEL_HTTP_EXPORTER_NAME}: dropped record with mismatched encoding"),
        }
    }

    /// Serialize the batch into the POST body bytes.
    pub(super) fn body(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Json(spans) => {
                let body = serde_json::json!({ "resourceSpans": spans });
                serde_json::to_vec(&body).map_err(|error| error.to_string())
            }
            Self::Proto { bytes, .. } => Ok(bytes.clone()),
        }
    }
}
