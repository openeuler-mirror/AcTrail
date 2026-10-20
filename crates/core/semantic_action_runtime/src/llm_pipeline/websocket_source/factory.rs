//! Stateless projection of the transport owner's native message identity.
use crate::llm_pipeline::projection::ProjectionBatch;
use crate::llm_pipeline::transport::websocket::{
    WebSocketAbortReason, WebSocketMessageEvent, WebSocketMessageHead,
};
use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::PayloadDirection;
use semantic_action::attr_keys::{payload, websocket};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkOrigin, SemanticActionLinkRole, SemanticActionStatus,
};
use std::collections::BTreeMap;

pub(in crate::llm_pipeline) struct WebSocketSourceFactory {
    retain: bool,
}

impl WebSocketSourceFactory {
    pub(in crate::llm_pipeline) fn new(config: &SemanticRetentionConfig) -> Self {
        Self {
            retain: config.projection_enabled
                && config.l2_websocket.enabled
                && config.l2_websocket.message_summary,
        }
    }

    pub(in crate::llm_pipeline) fn observe(
        &self,
        event: &WebSocketMessageEvent,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        if !self.retain {
            return output;
        }
        match event {
            WebSocketMessageEvent::Start(head) => output.actions.push(Self::source(head)),
            WebSocketMessageEvent::Complete {
                head,
                evidence,
                observed_at,
                ..
            } => {
                let mut action = Self::source(head);
                action.end_time = Some(*observed_at);
                action.status = SemanticActionStatus::Success;
                action.completeness = SemanticActionCompleteness::Complete;
                action.evidence.clone_from(evidence);
                // The coordinator derives the persisted lifecycle update from this
                // ordered source snapshot; updated_actions alone is a live view.
                output.actions.push(action);
            }
            WebSocketMessageEvent::Abort {
                head,
                evidence,
                observed_at,
                reason,
            } => {
                let mut action = Self::source(head);
                action.end_time = Some(*observed_at);
                action.status = SemanticActionStatus::Unknown;
                action.completeness =
                    if matches!(reason, WebSocketAbortReason::CapturePolicyLimited) {
                        SemanticActionCompleteness::CaptureLimited
                    } else {
                        SemanticActionCompleteness::Partial
                    };
                action.attributes.insert(
                    "websocket.message.incomplete_reason".into(),
                    format!("{reason:?}"),
                );
                action.evidence.clone_from(evidence);
                output.actions.push(action);
            }
        }
        output
    }

    pub(in crate::llm_pipeline) fn bind(
        &self,
        llm_action_id: &str,
        head: &WebSocketMessageHead,
        role: SemanticActionLinkRole,
    ) -> ProjectionBatch {
        let mut output = ProjectionBatch::default();
        if !self.retain
            || !matches!(
                role,
                SemanticActionLinkRole::LlmRequestWebSocketMessage
                    | SemanticActionLinkRole::LlmResponseWebSocketMessage
            )
        {
            return output;
        }
        output.links.push(SemanticActionLink {
            trace_id: head.identity.trace_id,
            parent_action_id: llm_action_id.to_owned(),
            child_action_id: head.message_id.clone(),
            role,
            origin: SemanticActionLinkOrigin::Observed,
            valid: true,
            evidence: head.evidence.clone(),
            attributes: Self::attributes(head),
        });
        output
    }

    fn source(head: &WebSocketMessageHead) -> SemanticAction {
        SemanticAction {
            action_id: head.message_id.clone(),
            trace_id: head.identity.trace_id,
            kind: SemanticActionKind::WebSocketMessage,
            title: format!(
                "WebSocket {} {} message",
                match head.direction {
                    PayloadDirection::Outbound => "outbound",
                    PayloadDirection::Inbound => "inbound",
                },
                if head.opcode == 1 { "text" } else { "binary" }
            ),
            start_time: head.start_at,
            end_time: None,
            process: head.identity.process,
            status: SemanticActionStatus::InProgress,
            completeness: SemanticActionCompleteness::Partial,
            attributes: Self::attributes(head),
            evidence: head.evidence.clone(),
        }
    }

    fn attributes(head: &WebSocketMessageHead) -> BTreeMap<String, String> {
        BTreeMap::from([
            (websocket::MESSAGE_ID.into(), head.message_id.clone()),
            (websocket::CONNECTION_ID.into(), head.connection_id.clone()),
            (
                payload::STREAM_KEY.into(),
                head.identity.stream_key.to_string(),
            ),
            (
                payload::SOURCE_BOUNDARY.into(),
                format!("{:?}", head.identity.source_boundary),
            ),
            (
                "direction".into(),
                match head.direction {
                    PayloadDirection::Outbound => "outbound",
                    PayloadDirection::Inbound => "inbound",
                }
                .into(),
            ),
            (websocket::OPCODE.into(), head.opcode.to_string()),
        ])
    }
}
