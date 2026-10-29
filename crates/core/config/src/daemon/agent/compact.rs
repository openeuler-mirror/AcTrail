//! context compaction pool and evidence limits.

use std::time::Duration;

pub const DEFAULT_LLM_CONTEXT_COMPACT_MAX_ACTIVE_SCOPES_PER_TRACE: u32 = 8;
pub const DEFAULT_LLM_CONTEXT_COMPACT_MAX_OPEN_REQUESTS_PER_SCOPE: u32 = 10;
pub const DEFAULT_LLM_CONTEXT_COMPACT_COMPACTION_CANDIDATES_MAX_ENTRIES_PER_SCOPE: u32 = 2;
pub const DEFAULT_LLM_CONTEXT_COMPACT_COMPACTION_CANDIDATES_TTL: Duration =
    Duration::from_secs(30 * 60);
pub const DEFAULT_LLM_CONTEXT_COMPACT_MAX_ARTIFACT_BYTES: u64 = 128 * 1024;
pub const DEFAULT_LLM_CONTEXT_COMPACT_MAX_CONTEXT_ITEMS_TO_SCAN: u32 = 128;
pub const DEFAULT_LLM_CONTEXT_COMPACT_MAX_CONTEXT_SCAN_BYTES: u64 = 512 * 1024;
pub const DEFAULT_LLM_CONTEXT_COMPACT_MAX_HISTORY_SOURCE_BYTES: u64 = 512 * 1024;
pub const DEFAULT_LLM_CONTEXT_COMPACT_MIN_ARTIFACT_FLOW_BYTES: u32 = 256;
pub const DEFAULT_LLM_CONTEXT_COMPACT_MIN_ARTIFACT_COVERAGE_PERCENT: u8 = 50;
pub const DEFAULT_LLM_CONTEXT_COMPACT_MAX_FINGERPRINT_POSITIONS: u32 = 16;
pub const DEFAULT_LLM_CONTEXT_COMPACT_PENDING_ADOPTIONS_MAX_ENTRIES_PER_SCOPE: u32 = 2;
pub const DEFAULT_LLM_CONTEXT_COMPACT_PENDING_ADOPTIONS_TTL: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LlmContextCompactConfig {
    pub enabled: bool,
    pub inference_enabled: bool,
    /// Admit inferred implementations only on Root, ForkRoot or DuplicateRoot.
    /// Disable to allow compact requests that append to an existing trajectory.
    pub inferred_candidate_root_only: bool,
    pub max_active_scopes_per_trace: u32,
    pub max_open_requests_per_scope: u32,
    /// Completed calls that may have produced a compacted context.
    pub compaction_candidates: LlmContextCompactPoolConfig,
    /// Requests waiting to confirm adoption of a compaction candidate.
    pub pending_adoptions: LlmContextCompactPoolConfig,
    pub max_artifact_bytes: u64,
    pub max_context_items_to_scan: u32,
    pub max_context_scan_bytes: u64,
    /// Maximum decoded bytes of the entire request JSON input retained for deferred history.
    /// Defaults to 524288; must be positive and twice its value must fit usize.
    /// History snapshots use twice this limit; carriers use max_context_scan_bytes.
    /// Bounds the lifetime extension of the JSON source;
    /// the input byte count is a memory proxy, not the size of the parsed JSON DOM heap.
    pub max_history_source_bytes: u64,
    pub min_artifact_flow_bytes: u32,
    pub min_artifact_coverage_percent: u8,
    pub max_fingerprint_positions: u32,
}

impl Default for LlmContextCompactConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            inference_enabled: true,
            inferred_candidate_root_only: true,
            max_active_scopes_per_trace: DEFAULT_LLM_CONTEXT_COMPACT_MAX_ACTIVE_SCOPES_PER_TRACE,
            max_open_requests_per_scope: DEFAULT_LLM_CONTEXT_COMPACT_MAX_OPEN_REQUESTS_PER_SCOPE,
            compaction_candidates: LlmContextCompactPoolConfig {
                max_entries_per_scope:
                    DEFAULT_LLM_CONTEXT_COMPACT_COMPACTION_CANDIDATES_MAX_ENTRIES_PER_SCOPE,
                ttl: DEFAULT_LLM_CONTEXT_COMPACT_COMPACTION_CANDIDATES_TTL,
            },
            pending_adoptions: LlmContextCompactPoolConfig {
                max_entries_per_scope:
                    DEFAULT_LLM_CONTEXT_COMPACT_PENDING_ADOPTIONS_MAX_ENTRIES_PER_SCOPE,
                ttl: DEFAULT_LLM_CONTEXT_COMPACT_PENDING_ADOPTIONS_TTL,
            },
            max_artifact_bytes: DEFAULT_LLM_CONTEXT_COMPACT_MAX_ARTIFACT_BYTES,
            max_context_items_to_scan: DEFAULT_LLM_CONTEXT_COMPACT_MAX_CONTEXT_ITEMS_TO_SCAN,
            max_context_scan_bytes: DEFAULT_LLM_CONTEXT_COMPACT_MAX_CONTEXT_SCAN_BYTES,
            max_history_source_bytes: DEFAULT_LLM_CONTEXT_COMPACT_MAX_HISTORY_SOURCE_BYTES,
            min_artifact_flow_bytes: DEFAULT_LLM_CONTEXT_COMPACT_MIN_ARTIFACT_FLOW_BYTES,
            min_artifact_coverage_percent:
                DEFAULT_LLM_CONTEXT_COMPACT_MIN_ARTIFACT_COVERAGE_PERCENT,
            max_fingerprint_positions: DEFAULT_LLM_CONTEXT_COMPACT_MAX_FINGERPRINT_POSITIONS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LlmContextCompactPoolConfig {
    pub max_entries_per_scope: u32,
    pub ttl: Duration,
}
