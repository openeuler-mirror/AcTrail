// Tests for the corresponding production module.
use super::*;

impl TraceState {
    fn observe_llm_response(&mut self, action: &SemanticActionRecord, cap: usize) {
        let cfg = PluginConfig::default();
        let _ = self.observe_llm_response_at(action, &cfg, self.last_active_ms, cap);
    }
}

impl AlertData {
    fn total_count_for_test(&self) -> u64 {
        self.failure_count + self.success_count
    }
}

fn test_config() -> PluginConfig {
    PluginConfig::default()
}

fn record(
    trace: &mut TraceState,
    cfg: &PluginConfig,
    tool: &str,
    action_id: &str,
    outcome: Outcome,
    now_ms: u64,
) -> Option<AlertData> {
    trace.record_outcome(
        tool,
        "runtime_error",
        "2",
        outcome,
        action_id,
        &[action_id],
        now_ms,
        cfg,
        "exit code 2",
        "",
    )
}

include!("alerts.rs");
include!("attribution.rs");
include!("replay.rs");
include!("execution.rs");
