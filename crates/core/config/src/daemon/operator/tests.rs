use super::super::{
    ExistingContainerCgroups, LlmRequestBodyExportRetention, LlmRequestContentRetention,
    LlmToolResultContentExportRetention, ResourceMetricsMode,
};
use super::{OperatorConfig, launch_seccomp_requirements};
use crate::daemon::{CommandControlConfig, PayloadSocketCaptureBackend};

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

#[test]
fn resource_metrics_cgroup_settings_parse_and_round_trip() {
    let config = OperatorConfig::init()
        .expect("default operator config initializes")
        .patch(
            "[resource_metrics]\n\
             mode = \"cgroup-v2\"\n\
             existing_container_cgroups = \"require\"\n\
             external_cgroup_failure_threshold = 5\n\
             cgroup_root = \"/sys/fs/cgroup/actrail-test\"\n\
             finalization_timeout_ms = 45000\n\
             orphan_limit = 77\n",
        )
        .expect("cgroup resource settings parse");

    assert_eq!(config.resource_metrics.mode, ResourceMetricsMode::CgroupV2);
    assert_eq!(
        config.resource_metrics.existing_container_cgroups,
        ExistingContainerCgroups::Require
    );
    assert_eq!(config.resource_metrics.external_cgroup_failure_threshold, 5);
    assert_eq!(
        config.resource_metrics.cgroup_root.to_string_lossy(),
        "/sys/fs/cgroup/actrail-test"
    );
    assert_eq!(config.resource_metrics.finalization_timeout_ms, 45_000);
    assert_eq!(config.resource_metrics.orphan_limit, 77);

    let rendered = config.dump().expect("operator config renders");
    let reparsed = OperatorConfig::parse(&rendered).expect("rendered config parses");
    assert_eq!(reparsed.resource_metrics, config.resource_metrics);
}

#[test]
fn resource_metrics_default_remains_procfs() {
    let config = OperatorConfig::init().expect("default operator config initializes");
    assert_eq!(config.resource_metrics.mode, ResourceMetricsMode::Procfs);
    assert_eq!(
        config.resource_metrics.existing_container_cgroups,
        ExistingContainerCgroups::Disabled
    );
    assert_eq!(config.resource_metrics.external_cgroup_failure_threshold, 3);
}

#[test]
fn procfs_rejects_existing_container_cgroups() {
    let error = OperatorConfig::init()
        .expect("default operator config initializes")
        .patch("[resource_metrics]\nmode = \"procfs\"\nexisting_container_cgroups = \"prefer\"\n")
        .expect_err("procfs plus external cgroups must fail validation");
    assert!(
        error
            .to_string()
            .contains("must be disabled when mode=procfs")
    );
}

#[test]
fn charged_memory_alert_round_trips_and_rejects_zero() {
    let config = OperatorConfig::init()
        .unwrap()
        .patch("[resource_metrics]\nmemory_alert_current_bytes = \"1048576\"\n")
        .unwrap();
    assert_eq!(
        config.resource_metrics.memory_alert_current_bytes,
        Some(1048576)
    );
    assert!(
        OperatorConfig::init()
            .unwrap()
            .patch("[resource_metrics]\nmemory_alert_current_bytes = \"0\"\n")
            .is_err()
    );
}
