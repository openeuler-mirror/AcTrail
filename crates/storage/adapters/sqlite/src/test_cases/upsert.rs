use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticActionWriteStore, SemanticEvidence, SemanticEvidenceKind,
};

use crate::SqliteStorage;

#[test]
fn action_upsert_skips_unchanged_and_splits_row_from_evidence_writes() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    install_write_audit(&storage);

    let mut base = action("action", SemanticActionStatus::InProgress);
    base.evidence.push(evidence(1));
    storage
        .upsert_semantic_action(base.clone())
        .expect("write initial action");

    clear_write_audit(&storage);
    storage
        .upsert_semantic_action(base.clone())
        .expect("repeat identical action");
    assert_eq!(audit_count(&storage, "semantic_actions"), 0);
    assert_eq!(audit_count(&storage, "semantic_action_evidence"), 0);

    clear_write_audit(&storage);
    let mut completed = base.clone();
    completed.status = SemanticActionStatus::Success;
    completed.end_time = Some(UNIX_EPOCH + Duration::from_millis(20));
    storage
        .upsert_semantic_action(completed)
        .expect("write row-only action update");
    assert_eq!(audit_count(&storage, "semantic_actions"), 1);
    assert_eq!(audit_count(&storage, "semantic_action_evidence"), 0);

    clear_write_audit(&storage);
    let mut with_new_evidence = base;
    with_new_evidence.evidence.push(evidence(2));
    storage
        .upsert_semantic_action(with_new_evidence)
        .expect("write evidence-only action update");
    assert_eq!(audit_count(&storage, "semantic_actions"), 0);
    assert_eq!(audit_count(&storage, "semantic_action_evidence"), 3);
}

fn install_write_audit(storage: &SqliteStorage) {
    storage
        .connection()
        .borrow()
        .execute_batch(
            "CREATE TEMP TABLE write_audit (
                table_name TEXT NOT NULL,
                operation TEXT NOT NULL
            );
            CREATE TEMP TRIGGER semantic_actions_insert_audit
            AFTER INSERT ON semantic_actions
            BEGIN
                INSERT INTO write_audit(table_name, operation)
                VALUES ('semantic_actions', 'insert');
            END;
            CREATE TEMP TRIGGER semantic_action_evidence_insert_audit
            AFTER INSERT ON semantic_action_evidence
            BEGIN
                INSERT INTO write_audit(table_name, operation)
                VALUES ('semantic_action_evidence', 'insert');
            END;
            CREATE TEMP TRIGGER semantic_action_evidence_delete_audit
            AFTER DELETE ON semantic_action_evidence
            BEGIN
                INSERT INTO write_audit(table_name, operation)
                VALUES ('semantic_action_evidence', 'delete');
            END;",
        )
        .expect("install sqlite write audit triggers");
}

fn clear_write_audit(storage: &SqliteStorage) {
    storage
        .connection()
        .borrow()
        .execute("DELETE FROM write_audit", [])
        .expect("clear write audit");
}

fn audit_count(storage: &SqliteStorage, table_name: &str) -> i64 {
    storage
        .connection()
        .borrow()
        .query_row(
            "SELECT COUNT(*) FROM write_audit WHERE table_name = ?1",
            [table_name],
            |row| row.get(0),
        )
        .expect("read write audit count")
}

fn action(action_id: &str, status: SemanticActionStatus) -> SemanticAction {
    SemanticAction {
        action_id: action_id.to_string(),
        trace_id: TraceId::new(1),
        kind: SemanticActionKind::LlmCall,
        title: "LLM call".to_string(),
        start_time: UNIX_EPOCH + Duration::from_millis(10),
        end_time: None,
        process: ProcessIdentity::new(100, 1, 1),
        status,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes: BTreeMap::from([("provider".to_string(), "test".to_string())]),
        evidence: Vec::new(),
    }
}

fn evidence(id: u64) -> SemanticEvidence {
    SemanticEvidence {
        kind: SemanticEvidenceKind::PayloadSegment,
        id,
        role: "llm.payload".to_string(),
    }
}
