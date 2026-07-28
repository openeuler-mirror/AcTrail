//! Semantic action attribute key names grouped by contract namespace.

pub mod actrail {
    pub const ACTION_FINALIZED_ON_TRACE_CLOSE: &str = "actrail.action.finalized_on_trace_close";
    pub const ACTION_VALID: &str = "actrail.action.valid";
    pub const ACTION_VALID_FALSE_MARKER: &str = "actrail.action.valid=false";
    pub const LINK_SOURCE: &str = "actrail.link.source";
    pub const LINK_VALID: &str = "actrail.link.valid";
    pub const LINK_VALID_FALSE_MARKER: &str = "actrail.link.valid=false";
}

pub mod agent {
    pub const IDENTITY_EVIDENCE_ACTION_ID: &str = "agent.identity.evidence_action_id";
    pub const IDENTITY_SOURCE: &str = "agent.identity.source";
    pub const IDENTITY_STATUS: &str = "agent.identity.status";
    pub const IDENTITY_STATUS_OBSERVED_MARKER: &str = "agent.identity.status=observed";
    pub const PERFORMED_ACTION_SEQUENCE: &str = "agent.performed_action.sequence";
}

pub mod agent_child {
    pub const COMMAND_LINE: &str = "agent.child.command_line";
    pub const EXECUTABLE: &str = "agent.child.executable";
    pub const PROCESS_ID: &str = "agent.child.process_id";
}

pub mod agent_invocation {
    pub const EVIDENCE_ACTION_ID: &str = "agent.invocation.evidence_action_id";
    pub const TRIGGER: &str = "agent.invocation.trigger";
}

pub mod command {
    pub const EXIT_CODE: &str = "command.exit_code";
    pub const FAILURE_KIND: &str = "command.failure.kind";
    pub const FAILURE_SUMMARY: &str = "command.failure.summary";
    pub const LINE: &str = "command.line";
}

pub mod enforcement {
    pub const BACKEND: &str = "enforcement.backend";
    pub const DECISION: &str = "enforcement.decision";
    pub const OPERATION: &str = "enforcement.operation";
    pub const RESULT: &str = "enforcement.result";
    pub const RULE_ID: &str = "enforcement.rule_id";
}

pub mod file {
    pub const BYTES_READ: &str = "file.bytes_read";
    pub const BYTES_WRITTEN: &str = "file.bytes_written";
    pub const CHANGE_KIND: &str = "file.change_kind";
    pub const FD: &str = "file.fd";
    pub const OPERATION: &str = "file.operation";
    pub const PATH: &str = "file.path";
    pub const READ_COUNT: &str = "file.read_count";
    pub const TTY: &str = "file.tty";
    pub const WRITE_COUNT: &str = "file.write_count";
}

pub mod file_bulk_read {
    pub const CLOSE_COUNT: &str = "file.bulk_read.close_count";
    pub const CHUNKING_SCHEME: &str = "file.bulk_read.chunking_scheme";
    pub const ERROR_COUNT: &str = "file.bulk_read.error_count";
    pub const ERROR_PATH_OVERFLOW: &str = "file.bulk_read.error_path_overflow";
    pub const ERROR_REASON_COUNTS: &str = "file.bulk_read.error_reason_counts";
    pub const ERROR_STORED_PATH_COUNT: &str = "file.bulk_read.error_stored_path_count";
    pub const ERROR_UNIQUE_PATH_COUNT: &str = "file.bulk_read.error_unique_path_count";
    pub const ERROR_UNIQUE_PATH_COUNT_STATE: &str = "file.bulk_read.error_unique_path_count_state";
    pub const FIRST_EVENT_ID: &str = "file.bulk_read.first_event_id";
    pub const LAST_EVENT_ID: &str = "file.bulk_read.last_event_id";
    pub const MODE: &str = "file.bulk_read.mode";
    pub const OPEN_COUNT: &str = "file.bulk_read.open_count";
    pub const PATH_OVERFLOW: &str = "file.bulk_read.path_overflow";
    pub const PATH_SET_ID: &str = "file.bulk_read.path_set_id";
    pub const PATH_SET_STATE: &str = "file.bulk_read.path_set_state";
    pub const READ_COUNT: &str = "file.bulk_read.read_count";
    pub const STORED_PATH_COUNT: &str = "file.bulk_read.stored_path_count";
    pub const UNIQUE_PATH_COUNT: &str = "file.bulk_read.unique_path_count";
    pub const UNIQUE_PATH_COUNT_STATE: &str = "file.bulk_read.unique_path_count_state";
}

pub mod fs_enumerate {
    pub const CHUNKING_SCHEME: &str = "fs.enumerate.chunking_scheme";
    pub const CLOSE_COUNT: &str = "fs.enumerate.close_count";
    pub const ERROR_COUNT: &str = "fs.enumerate.error_count";
    pub const ERROR_PATH_OVERFLOW: &str = "fs.enumerate.error_path_overflow";
    pub const ERROR_REASON_COUNTS: &str = "fs.enumerate.error_reason_counts";
    pub const ERROR_STORED_PATH_COUNT: &str = "fs.enumerate.error_stored_path_count";
    pub const ERROR_UNIQUE_PATH_COUNT: &str = "fs.enumerate.error_unique_path_count";
    pub const ERROR_UNIQUE_PATH_COUNT_STATE: &str = "fs.enumerate.error_unique_path_count_state";
    pub const FIRST_EVENT_ID: &str = "fs.enumerate.first_event_id";
    pub const LAST_EVENT_ID: &str = "fs.enumerate.last_event_id";
    pub const OPEN_COUNT: &str = "fs.enumerate.open_count";
    pub const PATH_OVERFLOW: &str = "fs.enumerate.path_overflow";
    pub const PATH_SET_ID: &str = "fs.enumerate.path_set_id";
    pub const PATH_SET_STATE: &str = "fs.enumerate.path_set_state";
    pub const STORED_PATH_COUNT: &str = "fs.enumerate.stored_path_count";
    pub const UNIQUE_PATH_COUNT: &str = "fs.enumerate.unique_path_count";
    pub const UNIQUE_PATH_COUNT_STATE: &str = "fs.enumerate.unique_path_count_state";
}

pub mod file_tty {
    pub const CLOSE_COUNT: &str = "file.tty.close_count";
    pub const ERROR_COUNT: &str = "file.tty.error_count";
    pub const EVENT_COUNT: &str = "file.tty.event_count";
    pub const FIRST_EVENT_ID: &str = "file.tty.first_event_id";
    pub const LAST_EVENT_ID: &str = "file.tty.last_event_id";
    pub const OPEN_COUNT: &str = "file.tty.open_count";
    pub const READ_COUNT: &str = "file.tty.read_count";
    pub const WRITE_COUNT: &str = "file.tty.write_count";
}

pub mod http {
    pub const OPERATION: &str = "http.operation";
}

pub mod http_request {
    pub const BODY_CONTAINS_NUL: &str = "http.request.body_contains_nul";
    pub const BODY_JSON: &str = "http.request.body_json";
    pub const BODY_JSON_STATE: &str = "http.request.body_json_state";
    pub const BODY_TEXT: &str = "http.request.body_text";
    pub const HEADERS_ENCODING: &str = "http.request.headers_encoding";
    pub const HEADERS_HPACK_BASE64: &str = "http.request.headers_hpack_base64";
    pub const HEADERS_TEXT: &str = "http.request.headers_text";
    pub const METHOD: &str = "http.request.method";
    pub const PROTOCOL: &str = "http.request.protocol";
    pub const STREAM_ID: &str = "http.request.stream_id";
}

pub mod http_response {
    pub const BODY_FORMAT: &str = "http.response.body_format";
    pub const BODY_JSON: &str = "http.response.body_json";
    pub const BODY_JSON_STATE: &str = "http.response.body_json_state";
    pub const BODY_TEXT: &str = "http.response.body_text";
    pub const HEADERS_ENCODING: &str = "http.response.headers_encoding";
    pub const HEADERS_HPACK_BASE64: &str = "http.response.headers_hpack_base64";
    pub const HEADERS_TEXT: &str = "http.response.headers_text";
    pub const PROTOCOL: &str = "http.response.protocol";
    pub const REASON: &str = "http.response.reason";
    pub const REQUEST_ACTION_ID: &str = "http.request.action_id";
    pub const STATUS_CODE: &str = "http.response.status_code";
    pub const STREAM_ID: &str = "http.response.stream_id";
}

pub mod invocation {
    pub const KIND: &str = "invocation.kind";
}

pub mod llm_call {
    pub const HTTP_RESPONSE_ACTION_ID: &str = "llm.call.http_response_action_id";
    pub const MODEL: &str = "llm.call.model";
    pub const REQUEST_ACTION_ID: &str = "llm.call.request_action_id";
    pub const RESPONSE_ACTION_ID: &str = "llm.call.response_action_id";
}

pub mod llm_request {
    pub const BODY_JSON: &str = "llm.request.body_json";
    pub const BODY_TEXT: &str = "llm.request.body_text";
    pub const BLOCK_COUNT: &str = "llm.request.block_count";
    pub const CANONICAL_BODY_BYTES: &str = "llm.request.canonical_body_bytes";
    pub const CANONICAL_BODY_HASH: &str = "llm.request.canonical_body_hash";
    pub const CLASSIFIER_ID: &str = "llm.request.classifier_id";
    pub const CONTENT_FORMAT_VERSION: &str = "llm.request.content_format_version";
    pub const CONTENT_STATE: &str = "llm.request.content_state";
    pub const MESSAGE_PREVIEW: &str = "llm.request.message_preview";
    pub const MODEL: &str = "llm.request.model";
    pub const PAYLOAD_BYTES: &str = "llm.request.payload_bytes";
    pub const PAYLOAD_TEXT: &str = "llm.request.payload_text";
    pub const PROTOCOL_ID: &str = "llm.request.protocol_id";
    pub const RAW_PAYLOAD_BYTES: &str = "llm.request.raw_payload_bytes";
}

pub mod llm_response {
    pub const ACTION_ID: &str = "llm.response.action_id";
    pub const BODY_FORMAT: &str = "llm.response.body_format";
    pub const CACHED_PROMPT_TOKENS: &str = "llm.response.cached_prompt_tokens";
    pub const CHUNK_COUNT: &str = "llm.response.chunk_count";
    pub const COMPLETION_TOKENS: &str = "llm.response.completion_tokens";
    pub const CONTENT_TEXT: &str = "llm.response.content_text";
    pub const DONE: &str = "llm.response.done";
    pub const FINISH_REASON: &str = "llm.response.finish_reason";
    pub const MODEL: &str = "llm.response.model";
    pub const OUTPUT_TEXT: &str = "llm.response.output_text";
    pub const PAYLOAD_BYTES: &str = "llm.response.payload_bytes";
    pub const PAYLOAD_TEXT: &str = "llm.response.payload_text";
    pub const PROMPT_CACHE_HIT_TOKENS: &str = "llm.response.prompt_cache_hit_tokens";
    pub const PROMPT_CACHE_MISS_TOKENS: &str = "llm.response.prompt_cache_miss_tokens";
    pub const PROMPT_TOKENS: &str = "llm.response.prompt_tokens";
    pub const PROVIDER_ID: &str = "llm.response.provider_id";
    pub const RAW_PAYLOAD_BYTES: &str = "llm.response.raw_payload_bytes";
    pub const REASONING_TEXT: &str = "llm.response.reasoning_text";
    pub const REASONING_TOKENS: &str = "llm.response.reasoning_tokens";
    pub const SSE_EVENTS_JSON: &str = "llm.response.sse_events_json";
    pub const STREAM: &str = "llm.response.stream";
    pub const TOOL_CALLS_JSON: &str = "llm.response.tool_calls_json";
    pub const TOTAL_TOKENS: &str = "llm.response.total_tokens";
}

pub mod network {
    pub const PROTOCOL_NAME: &str = "network.protocol.name";
    pub const PROTOCOL_VERSION: &str = "network.protocol.version";
}

pub mod payload {
    pub const LIBRARY: &str = "payload.library";
    pub const OPERATION_ID: &str = "payload.operation_id";
    pub const OPERATION_IDS: &str = "payload.operation_ids";
    pub const SEGMENT_COUNT: &str = "payload.segment_count";
    pub const SEQUENCE: &str = "payload.sequence";
    pub const SEQUENCE_END: &str = "payload.sequence_end";
    pub const SEQUENCE_START: &str = "payload.sequence_start";
    pub const SOURCE_BOUNDARY: &str = "payload.source_boundary";
    pub const STREAM_KEY: &str = "payload.stream_key";
    pub const SYMBOL: &str = "payload.symbol";
}

pub mod payload_aggregate {
    pub const FIRST_SEGMENT_ID: &str = "payload.aggregate.first_segment_id";
    pub const LAST_SEGMENT_ID: &str = "payload.aggregate.last_segment_id";
}

pub mod process {
    pub const EXECUTABLE: &str = "process.executable";
    pub const EXIT_CODE: &str = "process.exit_code";
    pub const FAILURE_KIND: &str = "process.failure.kind";
    pub const FAILURE_SUMMARY: &str = "process.failure.summary";
    pub const OPERATION: &str = "process.operation";
}

pub mod process_parent {
    pub const ID: &str = "process.parent.id";
    pub const IDENTITY_STATE: &str = "process.parent.identity_state";
    pub const IDENTITY_STATE_CONFLICT_MARKER: &str = "process.parent.identity_state=conflict";
}

pub mod server {
    pub const ADDRESS: &str = "server.address";
}

pub mod sse {
    pub const CONTENT_DELTA_COUNT: &str = "sse.content_delta_count";
    pub const DATA_JSON_STATE: &str = "sse.data_json_state";
    pub const DONE: &str = "sse.done";
    pub const EVENT_COUNT: &str = "sse.event_count";
    pub const EVENTS_JSON: &str = "sse.events_json";
    pub const REASONING_DELTA_COUNT: &str = "sse.reasoning_delta_count";
    pub const STREAM_ACTION_ID: &str = "sse.stream.action_id";
    pub const TOOL_DELTA_COUNT: &str = "sse.tool_delta_count";
}

pub mod syscall {
    pub const RESULT: &str = "syscall.result";
}

pub mod url {
    pub const PATH: &str = "url.path";
    pub const SCHEME: &str = "url.scheme";
}
