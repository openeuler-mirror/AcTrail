// Tests for the corresponding production module.
use semantic_action::SemanticActionKind;

use super::parse_otel_http_plugin_config;

#[test]
fn official_template_enables_tool_graph_actions_and_parses() {
    let raw = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../examples/plugins/builtin/otel-http/otel-http.config.toml"
    ));

    let config = parse_otel_http_plugin_config(raw).expect("official template parses");

    assert!(config.action_kinds.enabled(SemanticActionKind::LlmToolCall));
    assert!(
        config
            .action_kinds
            .enabled(SemanticActionKind::LlmToolResult)
    );
    assert!(
        config
            .action_kinds
            .enabled(SemanticActionKind::AgentInvocation)
    );
    assert!(
        !config
            .action_kinds
            .enabled(SemanticActionKind::AgentContextCompact),
        "the OTLP/HTTP allow-list stays opt-in for context compaction"
    );
}
