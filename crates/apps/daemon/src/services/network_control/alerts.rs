//! Bounded summaries of seccomp network rule denials, owned by the daemon thread.

use std::collections::{BTreeMap, btree_map::Entry};
use std::time::{Duration, Instant, SystemTime};

use access_control::policy::network::StoredNetworkRule;
use config_core::daemon::NetworkControlConfig;
use model_core::ids::TraceId;
use trace_runtime::registry::TraceRuntime;

use super::request::NetworkConnectContext;
use crate::services::alert_ingress::{
    AlertIngress, BoundaryAlertAggregation, NetworkConnectionBoundaryAlert,
    NetworkConnectionDenySource,
};

pub(in crate::services) struct NetworkBoundaryAlerts {
    window: Duration,
    max_groups: u32,
    deadline: Option<Instant>,
    admitted: u32,
    overflow: u64,
    groups: BTreeMap<RuleKey, PendingDenials>,
}

#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct RuleKey {
    trace_id: TraceId,
    owner: String,
    rule_id: String,
    revision: u64,
    source: NetworkConnectionDenySource,
    decider: Option<String>,
}

struct PendingDenials {
    count: u64,
    first: SystemTime,
    last: SystemTime,
    sample: NetworkConnectionBoundaryAlert,
}

impl NetworkBoundaryAlerts {
    pub(in crate::services) fn new(config: &NetworkControlConfig) -> Self {
        Self {
            window: Duration::from_millis(u64::from(config.alert_window_ms)),
            max_groups: config.alert_max_groups,
            deadline: None,
            admitted: 0,
            overflow: 0,
            groups: BTreeMap::new(),
        }
    }

    pub(in crate::services) fn poll_timeout(&self) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn record(
        &mut self,
        context: &NetworkConnectContext,
        rule: &StoredNetworkRule,
        source: NetworkConnectionDenySource,
        decider: Option<&str>,
        reason: Option<&str>,
        ingress: &AlertIngress,
        traces: &TraceRuntime,
    ) {
        self.flush_due(ingress, traces);
        self.deadline
            .get_or_insert_with(|| Instant::now() + self.window);
        let key = RuleKey {
            trace_id: context.trace_id(),
            owner: rule.owner_instance_id.clone(),
            rule_id: rule.rule_id.clone(),
            revision: rule.rule_revision,
            source,
            decider: decider.map(str::to_string),
        };
        let observed_at = SystemTime::now();
        let sample = || {
            NetworkConnectionBoundaryAlert::new(
                context.endpoint().to_string(),
                rule.rule_id.clone(),
                rule.owner_instance_id.clone(),
                context.process().get(),
                source,
                decider.map(str::to_string),
                reason.map(str::to_string),
            )
        };
        match self.groups.entry(key) {
            Entry::Occupied(mut entry) => {
                let pending = entry.get_mut();
                if pending.count == 0 {
                    pending.sample = sample();
                    pending.first = observed_at;
                    pending.last = observed_at;
                } else {
                    pending.first = pending.first.min(observed_at);
                    pending.last = pending.last.max(observed_at);
                }
                pending.count = pending.count.saturating_add(1);
            }
            Entry::Vacant(entry) => {
                if self.admitted >= self.max_groups {
                    self.overflow = self.overflow.saturating_add(1);
                    return;
                }
                self.admitted += 1;
                let mut pending = PendingDenials {
                    count: 1,
                    first: observed_at,
                    last: observed_at,
                    sample: sample(),
                };
                pending.submit(entry.key(), ingress, traces);
                entry.insert(pending);
            }
        }
    }

    pub(in crate::services) fn flush_due(&mut self, ingress: &AlertIngress, traces: &TraceRuntime) {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.finish_all(ingress, traces);
        }
    }

    pub(in crate::services) fn finish_trace(
        &mut self,
        trace_id: TraceId,
        ingress: &AlertIngress,
        traces: &TraceRuntime,
    ) {
        self.groups.retain(|key, pending| {
            if key.trace_id != trace_id {
                return true;
            }
            pending.submit(key, ingress, traces);
            false
        });
        // Admission credits survive trace cleanup until the window expires.
    }

    pub(in crate::services) fn finish_all(
        &mut self,
        ingress: &AlertIngress,
        traces: &TraceRuntime,
    ) {
        for (key, mut pending) in std::mem::take(&mut self.groups) {
            pending.submit(&key, ingress, traces);
        }
        if self.overflow != 0 {
            tracing::warn!(
                received_denials = self.overflow,
                max_groups = self.max_groups,
                "network boundary alert aggregation capacity exceeded; new groups suppressed"
            );
        }
        self.deadline = None;
        self.admitted = 0;
        self.overflow = 0;
    }
}

impl PendingDenials {
    fn submit(&mut self, key: &RuleKey, ingress: &AlertIngress, traces: &TraceRuntime) {
        if self.count == 0 {
            return;
        }
        let Some(trace) = traces.get_trace(key.trace_id) else {
            tracing::warn!(trace_id = %key.trace_id, received_denials = self.count,
                "network boundary alert lost its trace before queue admission");
            return;
        };
        let aggregation = BoundaryAlertAggregation {
            backend: "seccomp-user-notify",
            rule_revision: key.revision,
            received_denials: self.count,
            first_observed_at: self.first,
            last_observed_at: self.last,
        };
        match ingress.submit_network_connection_boundary_alert(
            key.trace_id,
            trace.trace.alert_token.clone(),
            self.sample.clone().with_aggregation(aggregation),
        ) {
            Ok(()) => self.count = 0,
            Err(error) => tracing::warn!(trace_id = %key.trace_id, received_denials = self.count,
                error.code = %error.code, error.message = %error.message,
                "network boundary alert queue admission failed"),
        }
    }
}
