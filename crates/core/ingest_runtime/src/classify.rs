//! Provider-label classification flow ownership.

use model_core::event::{
    DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, LabelPayload,
};
use model_core::ids::EventId;
use provider_evidence::EvidenceBundle;
use provider_label::{ProviderClassifier, ProviderLabelRecord};

pub fn classify_event<C: ProviderClassifier + ?Sized>(
    classifier: &C,
    event: &DomainEvent,
    label_event_id: EventId,
) -> Option<DomainEvent> {
    let evidence = evidence_from_event(event);
    let label = classifier.classify(&evidence);
    if label.confidence_millis.is_none() {
        return None;
    }

    Some(label_event(event, label, evidence, label_event_id))
}

#[allow(dead_code)]
pub fn provider_label(
    event: &DomainEvent,
    classifier: &impl ProviderClassifier,
) -> ProviderLabelRecord {
    classifier.classify(&evidence_from_event(event))
}

fn evidence_from_event(event: &DomainEvent) -> EvidenceBundle {
    let mut evidence = EvidenceBundle::new();
    if let EventPayload::Net(payload) = &event.payload {
        if let Some(local) = &payload.local {
            evidence.insert("local", local);
        }
        if let Some(remote) = &payload.remote {
            evidence.insert("remote", remote);
        }
        evidence.insert("transport", &payload.transport);
        if let Some(size) = payload.size {
            evidence.insert("size", size.to_string());
        }
        if let Some(result) = payload.result {
            evidence.insert("result", result.to_string());
        }
        if let Some(operation) = payload.metadata.get("operation") {
            evidence.insert("operation", operation);
        }
        if let Some(direction) = payload.metadata.get("direction") {
            evidence.insert("direction", direction);
        }
    }
    evidence
}

fn label_event(
    source: &DomainEvent,
    label: ProviderLabelRecord,
    evidence: EvidenceBundle,
    event_id: EventId,
) -> DomainEvent {
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id: source.envelope.trace_id,
            observed_at: source.envelope.observed_at,
            process: source.envelope.process.clone(),
            collector: source.envelope.collector.clone(),
            kind: EventKind::Label,
            flags: EventFlags::clean(),
        },
        EventPayload::Label(LabelPayload {
            provider: label.provider,
            confidence_millis: label.confidence_millis,
            evidence: evidence.fields().clone(),
        }),
    )
}
