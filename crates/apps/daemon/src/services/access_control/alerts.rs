use std::collections::{BTreeMap, btree_map::Entry};
use std::time::{Duration, Instant, SystemTime};

use access_control::lsm::LsmAuditEvent;
use config_core::daemon::LsmConfig;
use model_core::ids::TraceId;
use trace_runtime::registry::TraceRuntime;

use crate::services::alert_ingress::{
    AlertIngress, BoundaryAlertAggregation, CommandExecutionBoundaryAlert,
    CommandExecutionDenySource, FileAccessBoundaryAlert, FileAccessDenySource,
};

/// The daemon's sole aggregation state. Admission credits last the entire window,
/// including after trace cleanup, so trace churn cannot bypass the output bound.
pub(in crate::services) struct LsmBoundaryAlerts {
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
    domain: u32,
    owner: String,
    rule_id: String,
    revision: u64,
}

struct PendingDenials {
    count: u64,
    first: SystemTime,
    last: SystemTime,
    resource: String,
    process_id: u64,
}

impl LsmBoundaryAlerts {
    pub(in crate::services) fn new(config: &LsmConfig) -> Self {
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

    pub(in crate::services) fn record(
        &mut self,
        audit: &LsmAuditEvent,
        process_id: u64,
        observed_at: SystemTime,
        ingress: &AlertIngress,
        traces: &TraceRuntime,
    ) {
        if audit.decision != 1
            || audit.error != 0
            || audit.rule_id.is_empty()
            || !(1..=4).contains(&audit.domain)
        {
            return;
        }
        self.flush_due(ingress, traces);
        self.deadline
            .get_or_insert_with(|| Instant::now() + self.window);
        let key = RuleKey {
            trace_id: TraceId::new(audit.trace_id),
            domain: audit.domain,
            owner: audit.owner.clone(),
            rule_id: audit.rule_id.clone(),
            revision: audit.revision,
        };
        match self.groups.entry(key) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().record(audit, process_id, observed_at);
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
                    resource: audit.resource.clone(),
                    process_id,
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
                "LSM boundary alert aggregation capacity exceeded; new groups suppressed"
            );
        }
        self.deadline = None;
        self.admitted = 0;
        self.overflow = 0;
    }
}

impl PendingDenials {
    fn record(&mut self, audit: &LsmAuditEvent, process_id: u64, observed_at: SystemTime) {
        if self.count == 0 {
            self.first = observed_at;
            self.last = observed_at;
            self.resource.clone_from(&audit.resource);
            self.process_id = process_id;
        } else {
            self.first = self.first.min(observed_at);
            self.last = self.last.max(observed_at);
        }
        self.count = self.count.saturating_add(1);
    }

    fn submit(&mut self, key: &RuleKey, ingress: &AlertIngress, traces: &TraceRuntime) {
        if self.count == 0 {
            return;
        }
        let Some(trace) = traces.get_trace(key.trace_id) else {
            tracing::warn!(trace_id = %key.trace_id, received_denials = self.count,
                "LSM boundary alert lost its trace before queue admission");
            return;
        };
        let aggregation = BoundaryAlertAggregation {
            backend: "bpf-lsm",
            rule_revision: key.revision,
            received_denials: self.count,
            first_observed_at: self.first,
            last_observed_at: self.last,
        };
        let token = trace.trace.alert_token.clone();
        let result = if key.domain == 4 {
            ingress.submit_command_execution_boundary_alert(
                key.trace_id,
                token,
                CommandExecutionBoundaryAlert::new(
                    "exec".into(),
                    self.resource.clone(),
                    key.rule_id.clone(),
                    key.owner.clone(),
                    self.process_id,
                    CommandExecutionDenySource::FastPathDeny,
                    None,
                    None,
                )
                .with_lsm_aggregation(aggregation),
            )
        } else {
            let operation = match key.domain {
                1 => "open",
                2 => "mkdir",
                3 => "rmdir",
                _ => unreachable!("validated LSM domain"),
            };
            ingress.submit_file_access_boundary_alert(
                key.trace_id,
                token,
                FileAccessBoundaryAlert::new(
                    operation.into(),
                    self.resource.clone(),
                    String::new(),
                    key.rule_id.clone(),
                    key.owner.clone(),
                    self.process_id,
                    FileAccessDenySource::FastPathDeny,
                    None,
                    None,
                )
                .with_lsm_aggregation(aggregation),
            )
        };
        match result {
            Ok(()) => self.count = 0,
            Err(error) => tracing::warn!(trace_id = %key.trace_id, received_denials = self.count,
                error.code = %error.code, error.message = %error.message,
                "LSM boundary alert queue admission failed"),
        }
    }
}
