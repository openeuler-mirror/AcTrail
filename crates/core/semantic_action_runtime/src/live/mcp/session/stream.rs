use model_core::payload::{PayloadRedactionState, PayloadSegment, PayloadSegmentId};

use super::{McpConfirmedStdioSession, McpStdioCandidate};
use crate::live::mcp::framing::McpJsonRpcFramer;
use crate::live::mcp::model::{McpBufferedStdioMessage, McpStdioStream};

pub(super) enum McpCandidateSegmentOutcome {
    Pending,
    Confirmed,
    StreamDiscarded(&'static str),
}

impl McpStdioCandidate {
    pub(super) fn new(max_bytes: usize) -> Self {
        Self {
            stdin: McpJsonRpcFramer::new(max_bytes),
            stdout: McpJsonRpcFramer::new(max_bytes),
            buffered_messages: Vec::new(),
            observed_bytes: 0,
            client_jsonrpc_observed: false,
        }
    }

    pub(super) fn accepts_stream(&self, stream: McpStdioStream) -> bool {
        stream == McpStdioStream::Stdin
            || (stream == McpStdioStream::Stdout && self.client_jsonrpc_observed)
    }

    pub(super) fn observe_segment(
        &mut self,
        segment: &PayloadSegment,
        stream: McpStdioStream,
        retain_evidence: bool,
    ) -> Result<McpCandidateSegmentOutcome, &'static str> {
        let direction = stream
            .message_direction()
            .ok_or("invalid_candidate_stream")?;
        let observation = match stream {
            McpStdioStream::Stdin => self.stdin.observe_candidate(segment, retain_evidence),
            McpStdioStream::Stdout => self.stdout.observe_candidate(segment, retain_evidence),
            McpStdioStream::Stderr | McpStdioStream::Unknown => {
                return Err("invalid_candidate_stream");
            }
        };
        let mut confirmed = false;
        for message in observation.messages {
            let buffered = McpBufferedStdioMessage {
                direction,
                server_process: segment.process.clone(),
                stream_key: segment.stream_key.as_str().to_string(),
                message,
            };
            if direction == crate::live::mcp::model::McpMessageDirection::ClientToServer {
                self.client_jsonrpc_observed = true;
            }
            confirmed |= buffered.is_tools_call_admission();
            self.buffered_messages.push(buffered);
        }
        if confirmed {
            return Ok(McpCandidateSegmentOutcome::Confirmed);
        }
        match observation.rejection {
            Some("candidate_truncated") if stream == McpStdioStream::Stdout => Ok(
                McpCandidateSegmentOutcome::StreamDiscarded("candidate_truncated"),
            ),
            Some(rejection) => Err(rejection),
            None => Ok(McpCandidateSegmentOutcome::Pending),
        }
    }

    pub(super) fn confirm(
        mut self,
        parse_buffer_max_bytes: usize,
    ) -> (McpConfirmedStdioSession, Vec<McpBufferedStdioMessage>) {
        self.stdin.set_max_buffer_bytes(parse_buffer_max_bytes);
        self.stdout.set_max_buffer_bytes(parse_buffer_max_bytes);
        (
            McpConfirmedStdioSession {
                stdin: self.stdin,
                stdout: self.stdout,
            },
            self.buffered_messages,
        )
    }

    pub(super) fn stdin_payload_draft(&self, segment: &PayloadSegment) -> Option<PayloadSegment> {
        let segment_id = self
            .stdin
            .first_segment_id()
            .unwrap_or_else(|| segment.segment_id.get());
        let size = u64::try_from(self.stdin.buffer_bytes().len()).ok()?;
        Some(PayloadSegment {
            segment_id: PayloadSegmentId::new(segment_id),
            trace_id: segment.trace_id,
            observed_at: segment.observed_at,
            process: segment.process.clone(),
            source_boundary: segment.source_boundary,
            content_state: segment.content_state,
            direction: segment.direction,
            stream_key: segment.stream_key.clone(),
            sequence: segment.sequence,
            original_size: size,
            captured_size: size,
            operation_id: segment.operation_id,
            operation_offset: 0,
            operation_original_size: size,
            operation_captured_size: size,
            operation_completion_state: segment.operation_completion_state,
            truncation: segment.truncation,
            redaction: PayloadRedactionState::NotRequired,
            library: segment.library.clone(),
            symbol: segment.symbol.clone(),
            protocol_hint: segment.protocol_hint.clone(),
            bytes: self.stdin.buffer_bytes().to_vec(),
        })
    }
}

impl McpConfirmedStdioSession {
    pub(super) fn observe_segment(
        &mut self,
        segment: &PayloadSegment,
        stream: McpStdioStream,
        retain_evidence: bool,
    ) -> (Vec<McpBufferedStdioMessage>, Option<&'static str>) {
        if stream.expected_payload_direction() != Some(segment.direction) {
            return (Vec::new(), Some("stdio_direction_mismatch"));
        }
        let direction = stream
            .message_direction()
            .expect("confirmed stdio stream must have a message direction");
        let observation = match stream {
            McpStdioStream::Stdin => self.stdin.observe_confirmed(segment, retain_evidence),
            McpStdioStream::Stdout => self.stdout.observe_confirmed(segment, retain_evidence),
            McpStdioStream::Stderr | McpStdioStream::Unknown => {
                return (Vec::new(), None);
            }
        };
        let messages = observation
            .messages
            .into_iter()
            .map(|message| McpBufferedStdioMessage {
                direction,
                server_process: segment.process.clone(),
                stream_key: segment.stream_key.as_str().to_string(),
                message,
            })
            .collect();
        (messages, observation.discarded_reason)
    }
}
