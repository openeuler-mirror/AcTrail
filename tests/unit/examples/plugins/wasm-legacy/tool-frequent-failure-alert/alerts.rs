// Existing alerts tests included in the parent test module.
#[test]
fn three_failures_trigger_alert_and_reset() {
    let mut trace = TraceState::new(1000);
    let cfg = test_config();
    let mut alert = None;
    for i in 0..3 {
        alert = record(
            &mut trace,
            &cfg,
            "bash",
            &format!("f{i}"),
            Outcome::Failure,
            1000 + i,
        );
    }
    let alert = alert.expect("3 failures must alert");
    assert_eq!(alert.failure_count, 3);
    assert_eq!(alert.total_count_for_test(), 3);
    assert_eq!(alert.evidence_action_ids.len(), 3);
    // 窗口已重置：继续失败属于新一轮
    assert!(record(&mut trace, &cfg, "bash", "f3", Outcome::Failure, 1010).is_none());
}

#[test]
fn success_counts_in_rate_denominator() {
    let mut trace = TraceState::new(1000);
    let mut cfg = test_config();
    cfg.alert.trigger_mode = TriggerMode::CountAndRate;
    cfg.alert.min_failure_rate = 0.5;
    // 交错事件，第 3 次失败时已有 4 次成功 → rate = 3/7 ≈ 0.43 < 0.5，不告警
    let events: &[(Outcome, &str, u64)] = &[
        (Outcome::Failure, "f0", 1000),
        (Outcome::Success, "s0", 1001),
        (Outcome::Success, "s1", 1002),
        (Outcome::Success, "s2", 1003),
        (Outcome::Failure, "f1", 1004),
        (Outcome::Success, "s3", 1005),
        (Outcome::Failure, "f2", 1006),
    ];
    for (outcome, id, t) in events {
        assert!(
            record(&mut trace, &cfg, "bash", id, *outcome, *t).is_none(),
            "rate 0.43 must not alert yet"
        );
    }
    // 第 4 次失败后 4F/4S → rate = 0.5 → 告警
    let alert = record(&mut trace, &cfg, "bash", "f3", Outcome::Failure, 1007)
        .expect("rate 0.5 must alert");
    assert_eq!(alert.failure_count, 4);
    assert_eq!(alert.success_count, 4);
}

#[test]
fn low_rate_does_not_alert_in_and_mode() {
    let mut trace = TraceState::new(1000);
    let mut cfg = test_config();
    cfg.alert.trigger_mode = TriggerMode::CountAndRate;
    cfg.alert.min_failure_rate = 0.5;
    let events: &[(Outcome, &str, u64)] = &[
        (Outcome::Failure, "f0", 1000),
        (Outcome::Success, "s0", 1001),
        (Outcome::Success, "s1", 1002),
        (Outcome::Success, "s2", 1003),
        (Outcome::Failure, "f1", 1004),
        (Outcome::Success, "s3", 1005),
        (Outcome::Success, "s4", 1006),
        (Outcome::Failure, "f2", 1007),
    ];
    for (outcome, id, t) in events {
        assert!(
            record(&mut trace, &cfg, "bash", id, *outcome, *t).is_none(),
            "rate 0.375 must not alert"
        );
    }
    // 3F/5S 未告警；第 4 次失败后 4F/5S ≈ 0.44 < 0.5 仍不告警
    assert!(record(&mut trace, &cfg, "bash", "f3", Outcome::Failure, 1008).is_none());
}

#[test]
fn cooldown_suppresses_after_reset() {
    let mut trace = TraceState::new(1000);
    let cfg = test_config();
    let mut alert = None;
    for i in 0..3 {
        alert = record(
            &mut trace,
            &cfg,
            "bash",
            &format!("f{i}"),
            Outcome::Failure,
            1000 + i,
        );
    }
    assert!(alert.is_some(), "first burst alerts");
    // 冷却期内（告警于 t=1002 触发，60s 冷却）不重复告警
    assert!(
        record(
            &mut trace,
            &cfg,
            "bash",
            "f3",
            Outcome::Failure,
            1002 + 30_000
        )
        .is_none()
    );
    // 冷却结束后，新的窗口需要重新累积 3 次失败
    let mut alert = None;
    for i in 4..7 {
        alert = record(
            &mut trace,
            &cfg,
            "bash",
            &format!("f{i}"),
            Outcome::Failure,
            1002 + 60_001 + (i - 4),
        );
    }
    assert!(alert.is_some(), "fresh burst after cooldown alerts");
}

#[test]
fn indeterminate_never_alerts_as_failure() {
    let mut trace = TraceState::new(1000);
    let cfg = test_config();
    for i in 0..10 {
        assert!(
            record(
                &mut trace,
                &cfg,
                "bash",
                &format!("u{i}"),
                Outcome::Indeterminate,
                1000 + i,
            )
            .is_none(),
            "indeterminate must not fabricate a failure alert"
        );
    }
}

#[test]
fn desensitization_sanitizes_secrets() {
    let cfg = DesensitizationConfig {
        mode: DesensitizationMode::Sanitized,
        summary_max_chars: 200,
        redact_keywords: vec!["sk-".to_string(), "token".to_string(), "secret".to_string()],
    };
    let raw = "failed sk-abcdef123456 token=xyz path /home/user/secret.txt";
    let out = cfg.desensitize(raw);
    assert!(!out.contains("sk-"), "sk- prefix must be redacted");
    assert!(!out.contains("token"), "token must be redacted");
    assert!(!out.contains("secret"), "secret must be redacted");

    let category_only = DesensitizationConfig {
        mode: DesensitizationMode::CategoryOnly,
        ..DesensitizationConfig::default()
    };
    assert_eq!(category_only.desensitize(raw), "");
}

#[test]
fn redact_long_hex_and_base64_runs() {
    let hex = "0123456789abcdef0123456789abcdef";
    let out = redact_secret_runs(hex);
    assert_eq!(out, "***");
    let short = "abc123";
    assert_eq!(redact_secret_runs(short), short);
    let b64 = "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo=";
    assert_eq!(redact_secret_runs(b64), "***");
}

#[test]
fn pattern_matching_supports_suffix_wildcard() {
    assert!(pattern_matches("bash", "bash"));
    assert!(pattern_matches("bash", "ba*"));
    assert!(!pattern_matches("bash", "read"));
    assert!(!pattern_matches("bash", "b*zz"));
}

#[test]
fn dedup_key_stays_bounded() {
    let short = dedup_key_for("bash", "runtime_error", 1786089600000);
    assert!(short.len() <= 256);
    assert!(short.contains("freq:"));
    let long_tool = "x".repeat(500);
    let hashed = dedup_key_for(&long_tool, "runtime_error", 1786089600000);
    assert!(hashed.len() <= 256);
    assert!(hashed.starts_with("freq-hash:"));
}

#[test]
fn iso8601_formatting() {
    assert_eq!(
        epoch_ms_to_iso8601(1786089600000),
        "2026-08-07T08:00:00.000Z"
    );
    assert_eq!(epoch_ms_to_iso8601(0), "1970-01-01T00:00:00.000Z");
}

#[test]
fn e2e_config_parses_and_validates() {
    let path = concat!(
        "../../../../tests/e2e/regression/tool_frequent_failure_alert/",
        "tool-frequent-failure-alert.e2e.config.json"
    );
    let raw = std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    let parsed = serde_json::from_str::<PluginConfig>(&raw)
        .unwrap_or_else(|error| panic!("parse {path}: {error}"));
    parsed
        .validate()
        .unwrap_or_else(|error| panic!("validate {path}: {error}"));
    assert_eq!(parsed.reporting.mode, "database");
}

fn action_record(kind: &str, attrs: &[(&str, &str)]) -> SemanticActionRecord {
    SemanticActionRecord {
        action_id: format!("action-{}", kind),
        trace_id: "trace-test".to_string(),
        kind: kind.to_string(),
        status: "success".to_string(),
        completeness: "complete".to_string(),
        file_change: None,
        attributes: attrs
            .iter()
            .map(|(k, v)| actrail::plugin::types::AttributePair {
                key: k.to_string(),
                value: v.to_string(),
            })
            .collect(),
    }
}
