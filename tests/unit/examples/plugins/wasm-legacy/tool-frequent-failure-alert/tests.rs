// Tests for the corresponding production module.
use super::*;

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
