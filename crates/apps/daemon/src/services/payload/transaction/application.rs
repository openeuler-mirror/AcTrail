use super::*;

impl PayloadTransactionContext<'_> {
    pub(super) fn prepare_application_events(
        &mut self,
        trace_id: TraceId,
        observed_at: SystemTime,
        process: ProcessIdentity,
        drafts: Vec<ApplicationEventDraft>,
        export_batch: &mut SemanticActionBatch,
    ) -> Result<Vec<PreparedApplicationEvent>, ControlError> {
        let mut prepared = Vec::with_capacity(drafts.len());
        for draft in drafts {
            let mut flags = EventFlags::clean();
            flags.metadata_partial = draft.metadata_partial;
            let event = DomainEvent::new(
                EventEnvelope {
                    event_id: self.next_event_id()?,
                    trace_id,
                    observed_at,
                    process: process.clone(),
                    collector: CollectorName::new(APPLICATION_PROTOCOL_COLLECTOR_NAME),
                    kind: EventKind::Application,
                    flags,
                },
                EventPayload::Application(draft.payload),
            );
            let event_actions = self.observe_semantic_actions_for_event(&event);
            export_batch.extend(event_actions.clone());
            prepared.push(PreparedApplicationEvent {
                event,
                semantic_actions: event_actions,
            });
        }
        Ok(prepared)
    }
}

pub(super) fn tls_summary_application_draft(
    segment: &PayloadSegment,
) -> Option<ApplicationEventDraft> {
    let hint = segment.protocol_hint.as_deref()?;
    let fields = parse_tls_summary_hint(hint)?;
    let reason = fields.get("reason").cloned().unwrap_or_default();
    let protocol = fields
        .get("protocol")
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let operation = match segment.direction {
        model_core::payload::PayloadDirection::Inbound => "download",
        model_core::payload::PayloadDirection::Outbound => "upload",
    };
    let mut metadata = BTreeMap::from([
        (
            "direction".to_string(),
            format!("{:?}", segment.direction).to_lowercase(),
        ),
        (
            "source_boundary".to_string(),
            format!("{:?}", segment.source_boundary),
        ),
        ("stream_key".to_string(), segment.stream_key.to_string()),
        ("payload_sequence".to_string(), segment.sequence.to_string()),
        (
            "payload_segment_id".to_string(),
            segment.segment_id.get().to_string(),
        ),
        (
            "payload.original_size".to_string(),
            segment.original_size.to_string(),
        ),
        (
            "payload.captured_size".to_string(),
            segment.captured_size.to_string(),
        ),
        ("payload.truncation".to_string(), "truncated".to_string()),
        ("payload.summary.reason".to_string(), reason.clone()),
        ("payload.summary.protocol".to_string(), protocol.clone()),
    ]);
    if segment.original_size > segment.captured_size {
        metadata.insert(
            "payload.omitted_size".to_string(),
            (segment.original_size - segment.captured_size).to_string(),
        );
    }
    Some(ApplicationEventDraft::partial(ApplicationPayload {
        protocol,
        operation: operation.to_string(),
        summary: format!("{operation} {} bytes ({reason})", segment.original_size),
        body: None,
        metadata,
    }))
}

fn parse_tls_summary_hint(hint: &str) -> Option<BTreeMap<String, String>> {
    let rest = hint.strip_prefix("tls-summary;")?;
    let mut fields = BTreeMap::new();
    for item in rest.split(';') {
        let Some((key, value)) = item.split_once('=') else {
            continue;
        };
        fields.insert(key.to_string(), value.to_string());
    }
    Some(fields)
}

pub(super) fn application_protocol_requested(
    trace_runtime: &TraceRuntime,
    trace_id: TraceId,
) -> Result<bool, ControlError> {
    let entry = trace_runtime
        .get_trace(trace_id)
        .ok_or_else(|| ControlError::new("payload_match", "payload trace does not exist"))?;
    Ok(entry
        .profile_snapshot
        .capability_requests
        .iter()
        .any(|request| {
            request.mode != RequestMode::Disabled
                && matches!(
                    request.capability,
                    Capability::NetApplicationPlaintextHttp | Capability::NetApplicationHttp2Frames
                )
        }))
}
