use super::*;
use crate::daemon::LlmContextCompactPoolConfig;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::daemon::operator::document) struct LlmContextCompactDocument {
    pub enabled: bool,
    pub inference_enabled: bool,
    pub inferred_candidate_root_only: bool,
    pub max_active_scopes_per_trace: u32,
    pub max_open_requests_per_scope: u32,
    pub compaction_candidates: LlmContextCompactPoolDocument,
    pub pending_adoptions: LlmContextCompactPoolDocument,
    pub max_artifact_bytes: u64,
    pub max_context_items_to_scan: u32,
    pub max_context_scan_bytes: u64,
    pub max_history_source_bytes: u64,
    pub min_artifact_flow_bytes: u32,
    pub min_artifact_coverage_percent: u8,
    pub max_fingerprint_positions: u32,
}

impl Default for LlmContextCompactDocument {
    fn default() -> Self {
        Self::from_config(&LlmContextCompactConfig::default())
    }
}

impl LlmContextCompactDocument {
    pub(super) fn from_config(config: &LlmContextCompactConfig) -> Self {
        Self {
            enabled: config.enabled,
            inference_enabled: config.inference_enabled,
            inferred_candidate_root_only: config.inferred_candidate_root_only,
            max_active_scopes_per_trace: config.max_active_scopes_per_trace,
            max_open_requests_per_scope: config.max_open_requests_per_scope,
            compaction_candidates: LlmContextCompactPoolDocument::from_config(
                config.compaction_candidates,
            ),
            pending_adoptions: LlmContextCompactPoolDocument::from_config(config.pending_adoptions),
            max_artifact_bytes: config.max_artifact_bytes,
            max_context_items_to_scan: config.max_context_items_to_scan,
            max_context_scan_bytes: config.max_context_scan_bytes,
            max_history_source_bytes: config.max_history_source_bytes,
            min_artifact_flow_bytes: config.min_artifact_flow_bytes,
            min_artifact_coverage_percent: config.min_artifact_coverage_percent,
            max_fingerprint_positions: config.max_fingerprint_positions,
        }
    }

    pub(super) fn to_config(&self) -> Result<LlmContextCompactConfig, String> {
        let max_artifact_bytes = require_positive_u64(
            "semantic_retention.l0_llm_call.context_compact.max_artifact_bytes",
            self.max_artifact_bytes,
        )?;
        let max_context_scan_bytes = require_positive_u64(
            "semantic_retention.l0_llm_call.context_compact.max_context_scan_bytes",
            self.max_context_scan_bytes,
        )?;
        let max_history_source_bytes = require_positive_u64(
            "semantic_retention.l0_llm_call.context_compact.max_history_source_bytes",
            self.max_history_source_bytes,
        )?;
        let min_artifact_flow_bytes = require_positive_u32(
            "semantic_retention.l0_llm_call.context_compact.min_artifact_flow_bytes",
            self.min_artifact_flow_bytes,
        )?;
        if u64::from(min_artifact_flow_bytes) > max_artifact_bytes
            || max_artifact_bytes > max_context_scan_bytes
        {
            return Err(
                "semantic_retention.l0_llm_call.context_compact requires min_artifact_flow_bytes <= max_artifact_bytes <= max_context_scan_bytes"
                    .to_string(),
            );
        }
        if !(1..=100).contains(&self.min_artifact_coverage_percent) {
            return Err(
                "semantic_retention.l0_llm_call.context_compact.min_artifact_coverage_percent must be in 1..=100"
                    .to_string(),
            );
        }
        usize::try_from(max_artifact_bytes).map_err(|error| {
            format!(
                "semantic_retention.l0_llm_call.context_compact.max_artifact_bytes must fit usize: {error}"
            )
        })?;
        usize::try_from(max_context_scan_bytes).map_err(|error| {
            format!(
                "semantic_retention.l0_llm_call.context_compact.max_context_scan_bytes must fit usize: {error}"
            )
        })?;
        let history_source_bytes = usize::try_from(max_history_source_bytes).map_err(|error| {
            format!(
                "semantic_retention.l0_llm_call.context_compact.max_history_source_bytes must fit usize: {error}"
            )
        })?;
        history_source_bytes.checked_mul(2).ok_or_else(|| {
            "semantic_retention.l0_llm_call.context_compact requires 2 * max_history_source_bytes to fit usize"
                .to_string()
        })?;
        Ok(LlmContextCompactConfig {
            enabled: self.enabled,
            inference_enabled: self.inference_enabled,
            inferred_candidate_root_only: self.inferred_candidate_root_only,
            max_active_scopes_per_trace: require_positive_u32(
                "semantic_retention.l0_llm_call.context_compact.max_active_scopes_per_trace",
                self.max_active_scopes_per_trace,
            )?,
            max_open_requests_per_scope: require_positive_u32(
                "semantic_retention.l0_llm_call.context_compact.max_open_requests_per_scope",
                self.max_open_requests_per_scope,
            )?,
            compaction_candidates: self.compaction_candidates.to_config(
                "semantic_retention.l0_llm_call.context_compact.compaction_candidates.max_entries_per_scope",
                "semantic_retention.l0_llm_call.context_compact.compaction_candidates.ttl",
                LlmContextCompactConfig::default().compaction_candidates,
            )?,
            pending_adoptions: self.pending_adoptions.to_config(
                "semantic_retention.l0_llm_call.context_compact.pending_adoptions.max_entries_per_scope",
                "semantic_retention.l0_llm_call.context_compact.pending_adoptions.ttl",
                LlmContextCompactConfig::default().pending_adoptions,
            )?,
            max_artifact_bytes,
            max_context_items_to_scan: require_positive_u32(
                "semantic_retention.l0_llm_call.context_compact.max_context_items_to_scan",
                self.max_context_items_to_scan,
            )?,
            max_context_scan_bytes,
            max_history_source_bytes,
            min_artifact_flow_bytes,
            min_artifact_coverage_percent: self.min_artifact_coverage_percent,
            max_fingerprint_positions: require_positive_u32(
                "semantic_retention.l0_llm_call.context_compact.max_fingerprint_positions",
                self.max_fingerprint_positions,
            )?,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::daemon::operator::document) struct LlmContextCompactPoolDocument {
    pub max_entries_per_scope: Option<u32>,
    pub ttl: Option<String>,
}

impl LlmContextCompactPoolDocument {
    fn from_config(config: LlmContextCompactPoolConfig) -> Self {
        Self {
            max_entries_per_scope: Some(config.max_entries_per_scope),
            ttl: Some(duration_as_string(config.ttl)),
        }
    }

    fn to_config(
        &self,
        capacity_path: &'static str,
        ttl_path: &'static str,
        defaults: LlmContextCompactPoolConfig,
    ) -> Result<LlmContextCompactPoolConfig, String> {
        Ok(LlmContextCompactPoolConfig {
            max_entries_per_scope: require_positive_u32(
                capacity_path,
                self.max_entries_per_scope
                    .unwrap_or(defaults.max_entries_per_scope),
            )?,
            ttl: self
                .ttl
                .as_deref()
                .map(|ttl| parse_required_duration(ttl_path, ttl))
                .transpose()?
                .unwrap_or(defaults.ttl),
        })
    }
}
