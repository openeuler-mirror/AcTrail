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
