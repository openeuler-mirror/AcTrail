//! Policy decisions projected by the plugin host.

use model_core::policy::{PolicyRecord, PolicyVerdict};
use policy_evaluate_contract::decision::PolicyDecision;

pub fn merge_decisions(decisions: impl IntoIterator<Item = PolicyDecision>) -> PolicyDecision {
    let mut merged = PolicyRecord::allow();

    for decision in decisions {
        merged.redactions.extend(decision.record.redactions);
        merged.truncations.extend(decision.record.truncations);
        if merged.note.is_none() {
            merged.note = decision.record.note;
        }
        merged.verdict = merge_verdict(merged.verdict, decision.record.verdict);
    }

    PolicyDecision { record: merged }
}

fn merge_verdict(current: PolicyVerdict, next: PolicyVerdict) -> PolicyVerdict {
    use PolicyVerdict::{Allow, Drop, Fatal, Redact};

    match (current, next) {
        (Fatal, _) | (_, Fatal) => Fatal,
        (Drop, _) | (_, Drop) => Drop,
        (Redact, _) | (_, Redact) => Redact,
        _ => Allow,
    }
}
