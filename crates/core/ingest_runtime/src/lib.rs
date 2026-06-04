//! Ingest-flow orchestration skeleton over observation contracts.

pub mod classify;
pub mod diagnostics;
pub mod match_event;
pub mod normalize;
pub mod policy_gate;

use collector_event::RawCollectorEvent;
use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::ids::{DiagnosticId, EventId, TraceId};
use model_core::process::ProcessIdentity;
use policy_evaluate_contract::evaluate::PolicyEvaluator;
use provider_label::ProviderClassifier;

use crate::classify::classify_event;
use crate::diagnostics::{identity_mismatch_diagnostic, policy_diagnostic};
use crate::normalize::normalize_event;
use crate::policy_gate::apply_policy;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestMatch {
    pub trace_id: TraceId,
    pub process: ProcessIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestOutcome {
    pub events: Vec<DomainEvent>,
    pub diagnostics: Vec<DiagnosticRecord>,
}

pub struct IngestPipeline<'a, P> {
    policy_evaluator: P,
    classifier: &'a dyn ProviderClassifier,
}

impl<'a, P> IngestPipeline<'a, P>
where
    P: PolicyEvaluator,
{
    pub fn new(policy_evaluator: P, classifier: &'a dyn ProviderClassifier) -> Self {
        Self {
            policy_evaluator,
            classifier,
        }
    }

    pub fn process(
        &self,
        raw_event: RawCollectorEvent,
        matched: Option<IngestMatch>,
        event_id: EventId,
        label_event_id: Option<EventId>,
        diagnostic_id: DiagnosticId,
    ) -> IngestOutcome {
        let Some(matched) = matched else {
            return IngestOutcome {
                events: Vec::new(),
                diagnostics: vec![identity_mismatch_diagnostic(
                    diagnostic_id,
                    raw_event.envelope.process,
                )],
            };
        };

        let mut raw_event = raw_event;
        raw_event.envelope.process = matched.process.clone();
        let mut event = normalize_event(raw_event, matched.trace_id, event_id);
        let policy_decision = apply_policy(&self.policy_evaluator, matched.trace_id, &event);
        let diagnostics = policy_diagnostic(
            diagnostic_id,
            matched.trace_id,
            &matched.process,
            &policy_decision,
        );

        if matches!(
            policy_decision.record.verdict,
            model_core::policy::PolicyVerdict::Drop | model_core::policy::PolicyVerdict::Fatal
        ) {
            return IngestOutcome {
                events: Vec::new(),
                diagnostics,
            };
        }

        event = event.with_policy(policy_decision.record);
        let mut events = vec![event];
        if let Some(label_event_id) = label_event_id {
            if let Some(label_event) = classify_event(self.classifier, &events[0], label_event_id) {
                events.push(label_event);
            }
        }

        IngestOutcome {
            events,
            diagnostics,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::SystemTime;

    use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
    use model_core::ids::{CollectorName, DiagnosticId, EventId, TraceId};
    use model_core::process::ProcessIdentity;
    use policy_evaluate_contract::decision::PolicyDecision;
    use policy_evaluate_contract::evaluate::{PolicyEvaluator, PolicyInput};
    use provider_evidence::EvidenceBundle;
    use provider_label::{ProviderClassifier, ProviderLabelRecord};

    use super::{IngestMatch, IngestPipeline};

    const PID: u32 = 100;
    const RAW_START_TICKS: u64 = 10;
    const RAW_GENERATION: u64 = RAW_START_TICKS;
    const MATCHED_START_TICKS: u64 = 20;
    const MATCHED_GENERATION: u64 = MATCHED_START_TICKS;
    const TRACE_ID: u64 = 7;
    const EVENT_ID: u64 = 1;
    const DIAGNOSTIC_ID: u64 = 1;

    struct AllowPolicy;

    impl PolicyEvaluator for AllowPolicy {
        fn evaluate(&self, _input: &PolicyInput) -> PolicyDecision {
            PolicyDecision::allow()
        }
    }

    struct UnknownProvider;

    impl ProviderClassifier for UnknownProvider {
        fn classify(&self, _evidence: &EvidenceBundle) -> ProviderLabelRecord {
            ProviderLabelRecord::unknown("unknown")
        }
    }

    #[test]
    fn matched_process_identity_is_used_for_persisted_event() {
        let raw_process = ProcessIdentity::new(PID, RAW_START_TICKS, RAW_GENERATION);
        let matched_process = ProcessIdentity::new(PID, MATCHED_START_TICKS, MATCHED_GENERATION);
        let raw_event = RawCollectorEvent {
            envelope: RawEventEnvelope {
                observed_at: SystemTime::UNIX_EPOCH,
                process: raw_process,
                collector: CollectorName::new("process-seccomp"),
            },
            payload: RawObservationPayload::Process {
                operation: "exec".to_string(),
                parent: None,
                metadata: BTreeMap::new(),
            },
        };

        let outcome = IngestPipeline::new(AllowPolicy, &UnknownProvider).process(
            raw_event,
            Some(IngestMatch {
                trace_id: TraceId::new(TRACE_ID),
                process: matched_process.clone(),
            }),
            EventId::new(EVENT_ID),
            None,
            DiagnosticId::new(DIAGNOSTIC_ID),
        );

        assert_eq!(outcome.events.len(), 1);
        assert_eq!(outcome.events[0].envelope.process, matched_process);
    }
}
