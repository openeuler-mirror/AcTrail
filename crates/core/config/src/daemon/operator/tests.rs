use super::super::{
    LlmRequestBodyExportRetention, LlmRequestContentRetention, LlmToolResultContentExportRetention,
};
use super::OperatorConfig;

fn parse_l0_llm_call(patch: &str) -> Result<OperatorConfig, String> {
    OperatorConfig::init()
        .expect("default operator config initializes")
        .patch(&format!("[semantic_retention.l0_llm_call]\n{patch}"))
}

#[test]
fn request_body_export_is_off_by_default() {
    let raw = OperatorConfig::default_hierarchical_template()
        .expect("default operator config template renders");

    assert!(raw.contains("request_body_export = \"none\""));
    let config = OperatorConfig::parse(&raw).expect("default template parses");
    assert_eq!(
        config.semantic_retention.l0_llm_call.request_body_export,
        LlmRequestBodyExportRetention::None
    );
}

#[test]
fn request_body_export_limit_defaults_to_the_web_body_view_limit() {
    let raw = OperatorConfig::default_hierarchical_template()
        .expect("default operator config template renders");
    let config = OperatorConfig::parse(&raw).expect("default template parses");

    assert_eq!(
        config
            .semantic_retention
            .l0_llm_call
            .request_body_export_max_bytes,
        128 * 1024
    );
}

#[test]
fn request_body_export_can_be_enabled_with_a_custom_limit() {
    let config = parse_l0_llm_call(
        "request_body_export = \"canonical_json\"\nrequest_body_export_max_bytes = 1048576\n",
    )
    .expect("explicit body export parses");

    assert_eq!(
        config.semantic_retention.l0_llm_call.request_body_export,
        LlmRequestBodyExportRetention::CanonicalJson
    );
    assert_eq!(
        config
            .semantic_retention
            .l0_llm_call
            .request_body_export_max_bytes,
        1_048_576
    );
    assert_eq!(
        config.semantic_retention.l0_llm_call.request_content,
        LlmRequestContentRetention::CanonicalBlocks
    );
}

#[test]
fn request_body_export_without_canonical_block_retention_fails_validation() {
    for request_content in ["none", "shape"] {
        let error = parse_l0_llm_call(&format!(
            "request_content = \"{request_content}\"\n\
             request_body_export = \"canonical_json\"\n"
        ))
        .expect_err("contradictory retention and export should fail validation");

        assert!(
            error.contains("semantic_retention.l0_llm_call.request_body_export"),
            "error should name the offending setting, got: {error}"
        );
        assert!(
            error.contains("request_content"),
            "error should name the conflicting setting, got: {error}"
        );
    }
}

#[test]
fn request_body_export_limit_of_zero_fails_validation() {
    let error = parse_l0_llm_call("request_body_export_max_bytes = 0\n")
        .expect_err("a zero body ceiling should fail validation");

    assert!(error.contains("semantic_retention.l0_llm_call.request_body_export_max_bytes"));
    assert!(error.contains("must be positive"));
}

#[test]
fn request_body_export_survives_a_serialization_round_trip() {
    let config = parse_l0_llm_call(
        "request_body_export = \"canonical_json\"\nrequest_body_export_max_bytes = 4096\n",
    )
    .expect("explicit body export parses");

    let rendered = config.dump().expect("operator config renders");
    let reparsed = OperatorConfig::parse(&rendered).expect("rendered config parses");

    assert_eq!(reparsed.semantic_retention, config.semantic_retention);
}

#[test]
fn tool_result_content_export_is_off_by_default_and_can_be_enabled() {
    let defaults = OperatorConfig::init().expect("default operator config initializes");
    assert_eq!(
        defaults
            .semantic_retention
            .l0_llm_call
            .tool_result_content_export,
        LlmToolResultContentExportRetention::None
    );

    let config = parse_l0_llm_call(
        "tool_result_content_export = \"canonical_json\"\n\
         tool_result_content_export_max_bytes = 4096\n",
    )
    .expect("explicit tool result export parses");
    assert_eq!(
        config
            .semantic_retention
            .l0_llm_call
            .tool_result_content_export,
        LlmToolResultContentExportRetention::CanonicalJson
    );
    assert_eq!(
        config
            .semantic_retention
            .l0_llm_call
            .tool_result_content_export_max_bytes,
        4096
    );
}

#[test]
fn logical_agent_tool_names_have_safe_defaults_and_round_trip() {
    let config = OperatorConfig::init().expect("default operator config initializes");
    assert!(
        config
            .agent_invocation
            .tool_names
            .iter()
            .any(|name| name == "Agent")
    );
    assert!(
        config
            .agent_invocation
            .tool_names
            .iter()
            .any(|name| name == "task")
    );

    let rendered = config.dump().expect("operator config renders");
    let reparsed = OperatorConfig::parse(&rendered).expect("rendered config parses");
    assert_eq!(reparsed.agent_invocation, config.agent_invocation);
}
