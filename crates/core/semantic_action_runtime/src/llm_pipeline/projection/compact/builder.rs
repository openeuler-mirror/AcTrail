use std::collections::BTreeMap;

use model_core::diagnostics::LlmPipelineDiagnostic;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionChange, SemanticActionCompleteness, SemanticActionIdentity,
    SemanticActionKind, SemanticActionStatus, SemanticActionUpdate, SemanticAttributeSupplement,
    SemanticEvidence, attr_keys as attrs, evidence_roles,
};

use super::state::{CompactConfirmation, CompactScopeKey, CompactionCandidate, PredecessorHead};
use crate::llm_pipeline::projection::links::LlmContextCompactLink;

pub(in crate::llm_pipeline) struct CompactProjectionBatch {
    pub(in crate::llm_pipeline) actions: Vec<SemanticAction>,
    pub(in crate::llm_pipeline) updates: Vec<SemanticActionUpdate>,
    pub(in crate::llm_pipeline) updated_actions: Vec<SemanticAction>,
    pub(in crate::llm_pipeline) implementation_links: Vec<LlmContextCompactLink>,
    pub(in crate::llm_pipeline) diagnostics: Vec<LlmPipelineDiagnostic>,
    pub(in crate::llm_pipeline) confirmed_adoptions: Vec<ConfirmedAdoption>,
    pub(super) work: CompactMatchWork,
}

pub(in crate::llm_pipeline) struct ConfirmedAdoption {
    pub(in crate::llm_pipeline) trace_id: TraceId,
    pub(in crate::llm_pipeline) process: ProcessIdentity,
    pub(in crate::llm_pipeline) classifier_id: String,
    pub(in crate::llm_pipeline) action_id: String,
}

impl ConfirmedAdoption {
    pub(super) fn new(scope: &CompactScopeKey, action_id: &str) -> Self {
        Self {
            trace_id: scope.trace_id,
            process: scope.process,
            classifier_id: scope.classifier_id.clone(),
            action_id: action_id.to_string(),
        }
    }
}

#[derive(Default)]
pub(super) struct CompactMatchWork {
    pub(super) initializations: usize,
    pub(super) pair_checks: usize,
    pub(super) content_checks: usize,
}

const DETECTOR_ID: &str = "artifact-flow-v10";

pub(super) struct CompactActionBuilder;

pub(super) struct CompactProjection {
    action: SemanticAction,
    update: Option<SemanticActionUpdate>,
    link: LlmContextCompactLink,
}

impl CompactProjectionBatch {
    pub(in crate::llm_pipeline) fn empty() -> Self {
        Self {
            actions: Vec::new(),
            updates: Vec::new(),
            updated_actions: Vec::new(),
            implementation_links: Vec::new(),
            diagnostics: Vec::new(),
            confirmed_adoptions: Vec::new(),
            work: CompactMatchWork::default(),
        }
    }

    pub(super) fn report_work(&self) {
        if self.work.initializations > 0 || self.work.pair_checks > 0 {
            tracing::debug!(target: "actrail::context_compact",
                initializations = self.work.initializations,
                pair_checks = self.work.pair_checks,
                content_checks = self.work.content_checks,
                "context compaction matching work");
        }
    }

    pub(super) fn push(&mut self, projection: CompactProjection) {
        self.implementation_links.push(projection.link);
        if let Some(update) = projection.update {
            self.updates.push(update);
            self.updated_actions.push(projection.action);
        } else {
            self.actions.push(projection.action);
        }
    }
}

impl CompactActionBuilder {
    pub(super) fn prepare(
        candidate: &CompactionCandidate,
        adoption_request: Option<&SemanticAction>,
        confirmation: CompactConfirmation,
        predecessor: Option<&PredecessorHead>,
    ) -> Option<CompactProjection> {
        let adoption_request = match &confirmation {
            CompactConfirmation::OpaqueArtifact { .. } => Some(adoption_request?),
            _ => adoption_request,
        };
        let is_adoption_supplement =
            matches!(&confirmation, CompactConfirmation::OpaqueArtifact { .. });
        let action = Self::build(candidate, adoption_request, confirmation, predecessor);
        let update = if is_adoption_supplement {
            Some(Self::adoption_update(&action, adoption_request?)?)
        } else {
            None
        };
        let link = LlmContextCompactLink {
            compact: action.clone(),
            implementation_call: candidate.implementation.call.clone(),
        };
        Some(CompactProjection {
            action,
            update,
            link,
        })
    }
    fn adoption_update(
        action: &SemanticAction,
        request: &SemanticAction,
    ) -> Option<SemanticActionUpdate> {
        let fields = [
            attrs::agent_context_compact::POST_CONTEXT_REQUEST_ACTION_ID,
            attrs::agent_context_compact::POST_CONTEXT_ITEM_COUNT,
            attrs::agent_context_compact::CONTEXT_ITEM_UNIT,
            attrs::agent_context_compact::SUMMARY_CONTENT_STATE,
            attrs::agent_context_compact::POST_CONTEXT_ARTIFACT_PATH,
            attrs::agent_context_compact::OPAQUE_ARTIFACT_BYTES,
        ];
        let expected = [
            attrs::agent_context_compact::IMPLEMENTATION_LLM_CALL_ACTION_ID,
            attrs::agent_context_compact::EVIDENCE_MODE,
        ];
        let expected_attributes = expected
            .into_iter()
            .map(|key| Some((key.to_string(), action.attributes.get(key)?.clone())))
            .collect::<Option<BTreeMap<_, _>>>()?;
        let attributes = fields
            .into_iter()
            .map(|key| Some((key.to_string(), action.attributes.get(key)?.clone())))
            .collect::<Option<BTreeMap<_, _>>>()?;
        Some(SemanticActionUpdate {
            action_id: action.action_id.clone(),
            trace_id: action.trace_id,
            kind: action.kind,
            process: action.process,
            change: SemanticActionChange::SupplementAttributes {
                supplement: SemanticAttributeSupplement {
                    expected_attributes,
                    attributes,
                    required_action: Some(SemanticActionIdentity {
                        action_id: request.action_id.clone(),
                        trace_id: request.trace_id,
                        kind: request.kind,
                        process: request.process,
                    }),
                },
            },
            evidence: action
                .evidence
                .iter()
                .filter(|evidence| {
                    evidence.role == evidence_roles::agent_context_compact::POST_CONTEXT_PAYLOAD
                })
                .cloned()
                .collect(),
        })
    }

    fn build(
        candidate: &CompactionCandidate,
        adoption_request: Option<&SemanticAction>,
        confirmation: CompactConfirmation,
        predecessor: Option<&PredecessorHead>,
    ) -> SemanticAction {
        let mut attributes = BTreeMap::from([
            (
                attrs::agent_context_compact::DETECTOR_ID.to_string(),
                DETECTOR_ID.to_string(),
            ),
            (
                attrs::agent_context_compact::IMPLEMENTATION_LLM_CALL_ACTION_ID.to_string(),
                candidate.implementation.call.action_id.clone(),
            ),
            (
                attrs::agent_context_compact::SOURCE.to_string(),
                candidate
                    .intent
                    .as_ref()
                    .map_or("unknown", |intent| intent.source)
                    .to_string(),
            ),
        ]);
        if let Some(adoption_request) = adoption_request {
            attributes.insert(
                attrs::agent_context_compact::POST_CONTEXT_REQUEST_ACTION_ID.to_string(),
                adoption_request.action_id.clone(),
            );
        }
        if let Some(predecessor) = predecessor {
            attributes.insert(
                attrs::agent_context_compact::PRE_CONTEXT_REQUEST_ACTION_ID.to_string(),
                predecessor.request.action_id.clone(),
            );
        }
        let item_unit = match confirmation {
            CompactConfirmation::ExplicitProtocol => {
                attributes.insert(
                    attrs::agent_context_compact::EVIDENCE_MODE.to_string(),
                    attrs::agent_context_compact::EVIDENCE_MODE_PROTOCOL_EXPLICIT_COMPACTION
                        .to_string(),
                );
                candidate.implementation.shape.item_unit
            }
            CompactConfirmation::ArtifactFlow {
                adoption_shape,
                flow,
            } => {
                attributes.insert(
                    attrs::agent_context_compact::EVIDENCE_MODE.to_string(),
                    if flow.partial_summary {
                        attrs::agent_context_compact::EVIDENCE_MODE_ARTIFACT_FLOW_PARTIAL_SUMMARY
                    } else {
                        attrs::agent_context_compact::EVIDENCE_MODE_ARTIFACT_FLOW
                    }
                    .to_string(),
                );
                attributes.insert(
                    attrs::agent_context_compact::SUMMARY_CONTENT_STATE.to_string(),
                    "plaintext".to_string(),
                );
                attributes.insert(
                    attrs::agent_context_compact::POST_CONTEXT_ITEM_COUNT.to_string(),
                    adoption_shape.item_count.to_string(),
                );
                attributes.insert(
                    attrs::agent_context_compact::SUMMARY_CHARS.to_string(),
                    flow.chars.to_string(),
                );
                attributes.insert(
                    attrs::agent_context_compact::SUMMARY_BYTES.to_string(),
                    flow.bytes.to_string(),
                );
                adoption_shape.item_unit
            }
            CompactConfirmation::OpaqueArtifact { adoption } => {
                attributes.insert(
                    attrs::agent_context_compact::EVIDENCE_MODE.to_string(),
                    attrs::agent_context_compact::EVIDENCE_MODE_PROTOCOL_EXPLICIT_COMPACTION
                        .to_string(),
                );
                attributes.insert(
                    attrs::agent_context_compact::SUMMARY_CONTENT_STATE.to_string(),
                    "opaque".to_string(),
                );
                attributes.insert(
                    attrs::agent_context_compact::POST_CONTEXT_ITEM_COUNT.to_string(),
                    adoption.item_count.to_string(),
                );
                attributes.insert(
                    attrs::agent_context_compact::POST_CONTEXT_ARTIFACT_PATH.to_string(),
                    adoption.artifact_path,
                );
                attributes.insert(
                    attrs::agent_context_compact::OPAQUE_ARTIFACT_BYTES.to_string(),
                    adoption.artifact_bytes.to_string(),
                );
                adoption.item_unit
            }
        };
        attributes.insert(
            attrs::agent_context_compact::CONTEXT_ITEM_UNIT.to_string(),
            item_unit.to_string(),
        );
        if let Some(predecessor) = predecessor
            && predecessor.shape.item_unit == item_unit
        {
            attributes.insert(
                attrs::agent_context_compact::PRE_CONTEXT_ITEM_COUNT.to_string(),
                predecessor.shape.item_count.to_string(),
            );
        }
        SemanticAction {
            action_id: SemanticAction::context_compact_action_id(
                &candidate.implementation.call.action_id,
            ),
            trace_id: candidate.implementation.call.trace_id,
            kind: SemanticActionKind::AgentContextCompact,
            title: "context compaction".to_string(),
            start_time: candidate.implementation.call.start_time,
            end_time: candidate.implementation.response_end_time,
            process: candidate.implementation.call.process,
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            attributes,
            evidence: Self::evidence(candidate, adoption_request, predecessor),
        }
    }

    fn evidence(
        candidate: &CompactionCandidate,
        adoption_request: Option<&SemanticAction>,
        predecessor: Option<&PredecessorHead>,
    ) -> Vec<SemanticEvidence> {
        let mut evidence = Vec::new();
        if let Some(predecessor) = predecessor {
            Self::append_evidence(
                &mut evidence,
                &predecessor.request_evidence,
                evidence_roles::agent_context_compact::PRE_CONTEXT_PAYLOAD,
            );
        }
        Self::append_evidence(
            &mut evidence,
            &candidate.implementation.response_evidence,
            evidence_roles::agent_context_compact::RESPONSE_PAYLOAD,
        );
        if let Some(adoption_request) = adoption_request {
            Self::append_evidence(
                &mut evidence,
                &adoption_request.evidence,
                evidence_roles::agent_context_compact::POST_CONTEXT_PAYLOAD,
            );
        }
        evidence
    }

    fn append_evidence(
        target: &mut Vec<SemanticEvidence>,
        source: &[SemanticEvidence],
        role: &str,
    ) {
        target.extend(source.iter().cloned().map(|mut evidence| {
            evidence.role = role.to_string();
            evidence
        }));
    }
}
