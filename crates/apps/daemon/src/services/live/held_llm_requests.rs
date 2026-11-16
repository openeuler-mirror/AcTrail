//! Holds the responses to the LLM requests of watched Traces; checkpoint work takes the requests
//! after the drain and releases them.

use recording_runtime::SemanticActionBatch;
use semantic_action::{SemanticActionKind, attr_keys as attrs};

use crate::services::attach::{HeldLlmRequest, StorageAttachService, WatchedTraces};
use crate::services::tls_sync::TlsSyncService;

impl StorageAttachService {
    /// Holds the response to each LLM request of a watched Trace as the payload pipeline
    /// recognizes it. Nothing is released until the transaction flushes after recognition.
    pub(in crate::services) fn hold_llm_requests(
        batch: &SemanticActionBatch,
        tls_sync: &mut TlsSyncService,
        watched: &mut WatchedTraces,
    ) {
        for action in batch.actions() {
            if action.kind != SemanticActionKind::LlmRequest || !watched.contains(action.trace_id) {
                continue;
            }
            let attribute = |key: &str| action.attributes.get(key);
            let (Some(stream_key), Some(Ok(sequence_end))) = (
                attribute(attrs::payload::STREAM_KEY),
                attribute(attrs::payload::SEQUENCE_END).map(|value| value.parse()),
            ) else {
                continue;
            };
            let Some(hold) = tls_sync.hold_request(action.trace_id, stream_key, sequence_end)
            else {
                continue;
            };
            watched.push_llm_request(HeldLlmRequest {
                hold,
                trace_id: action.trace_id,
                request_action_id: action.action_id.clone(),
            });
        }
    }
}
