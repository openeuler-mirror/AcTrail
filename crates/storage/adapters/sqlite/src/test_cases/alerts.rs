use std::time::UNIX_EPOCH;

use alert_contract::{
    AlertDefinition, AlertDefinitionStore, AlertDraft, AlertListLimit, AlertReadStore,
    AlertSeverity, AlertSubmitOutcome, AlertWriteStore,
};
use model_core::ids::{ProfileName, TraceId, TraceName};
use model_core::process::ProcessIdentity;
use model_core::trace::{TraceAlertToken, TraceRecord};
use store_write_contract::traces::TraceWriteStore;

use crate::SqliteStorage;

#[test]
fn same_definition_alerts_without_key_are_each_stored() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(7);
    let token = TraceAlertToken::new([9; 32]);
    storage
        .create_trace(TraceRecord::new(
            trace_id,
            token.clone(),
            ProcessIdentity::new(200),
            TraceName::new("alerts"),
            ProfileName::new("snapshot"),
            UNIX_EPOCH,
        ))
        .expect("create trace");

    storage
        .register_alert_definition(&AlertDefinition {
            producer_plugin_id: "plugin-a".to_string(),
            definition_key: "llm-request-growth".to_string(),
            kind: "llm.request.growth".to_string(),
            title: "Request growth".to_string(),
            severity: AlertSeverity::High,
            payload_schema_id: "llm-growth.payload.v1.schema.json".to_string(),
        })
        .expect("register definition");

    for _ in 0..2 {
        let outcome = storage
            .submit_alert(
                trace_id,
                &token,
                "plugin-a",
                &AlertDraft {
                    definition_key: "llm-request-growth".to_string(),
                    payload_json: r#"{"observed_bytes":42}"#.to_string(),
                    deduplication_key: None,
                },
                UNIX_EPOCH,
            )
            .expect("submit alert");
        assert!(matches!(outcome, AlertSubmitOutcome::Stored(_)));
    }

    let alerts = storage
        .trace_alerts(trace_id, AlertListLimit::new(10).expect("positive limit"))
        .expect("list trace alerts");
    assert_eq!(
        alerts.len(),
        2,
        "same-type alert events must each be stored when no deduplication key is supplied"
    );
}

#[test]
fn plugin_supplied_deduplication_key_still_suppresses_repeat() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(8);
    let token = TraceAlertToken::new([10; 32]);
    storage
        .create_trace(TraceRecord::new(
            trace_id,
            token.clone(),
            ProcessIdentity::new(200),
            TraceName::new("alerts"),
            ProfileName::new("snapshot"),
            UNIX_EPOCH,
        ))
        .expect("create trace");

    storage
        .register_alert_definition(&AlertDefinition {
            producer_plugin_id: "plugin-a".to_string(),
            definition_key: "llm-request-growth".to_string(),
            kind: "llm.request.growth".to_string(),
            title: "Request growth".to_string(),
            severity: AlertSeverity::High,
            payload_schema_id: "llm-growth.payload.v1.schema.json".to_string(),
        })
        .expect("register definition");

    let draft = || AlertDraft {
        definition_key: "llm-request-growth".to_string(),
        payload_json: r#"{"observed_bytes":42}"#.to_string(),
        deduplication_key: Some("idempotency-boundary".to_string()),
    };
    let first = storage
        .submit_alert(trace_id, &token, "plugin-a", &draft(), UNIX_EPOCH)
        .expect("submit first alert");
    assert!(matches!(first, AlertSubmitOutcome::Stored(_)));
    let second = storage
        .submit_alert(trace_id, &token, "plugin-a", &draft(), UNIX_EPOCH)
        .expect("submit second alert");
    assert!(matches!(second, AlertSubmitOutcome::DuplicateSuppressed));

    let alerts = storage
        .trace_alerts(trace_id, AlertListLimit::new(10).expect("positive limit"))
        .expect("list trace alerts");
    assert_eq!(alerts.len(), 1);
}
