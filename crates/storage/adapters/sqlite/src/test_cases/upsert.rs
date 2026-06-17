use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    FilePathSetState, FilePathSetWrite, SemanticAction, SemanticActionCompleteness,
    SemanticActionKind, SemanticActionReadStore, SemanticActionStatus, SemanticActionWriteStore,
    SemanticEvidence, SemanticEvidenceKind,
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

#[test]
fn file_path_sets_page_paths_and_reuse_identical_chunks() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(1);
    let first = FilePathSetWrite {
        trace_id,
        action_id: "action-1".to_string(),
        path_set_id: "set-1".to_string(),
        state: FilePathSetState::Complete,
        unique_path_count: 3,
        stored_path_count: 3,
        chunking_scheme: "path-id-v1:chunk-max=2".to_string(),
        chunk_max_paths: 2,
        paths: vec![
            "/tmp/a".to_string(),
            "/tmp/b".to_string(),
            "/tmp/c".to_string(),
        ],
    };
    let mut second = first.clone();
    second.action_id = "action-2".to_string();
    second.path_set_id = "set-2".to_string();

    storage
        .upsert_file_path_sets(&[first, second])
        .expect("write file path sets");

    assert_eq!(
        storage
            .connection()
            .borrow()
            .query_row("SELECT COUNT(*) FROM file_paths", [], |row| row
                .get::<_, i64>(0))
            .expect("read file_paths count"),
        3
    );
    assert_eq!(
        storage
            .connection()
            .borrow()
            .query_row("SELECT COUNT(*) FROM file_path_set_chunks", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("read file_path_set_chunks count"),
        2
    );
    assert_eq!(
        storage
            .connection()
            .borrow()
            .query_row("SELECT COUNT(*) FROM file_path_set_chunk_refs", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("read file_path_set_chunk_refs count"),
        4
    );

    let page = storage
        .file_path_set_paths_page(trace_id, "action-2", 1, 2)
        .expect("read file path set page")
        .expect("path set should exist");
    assert_eq!(page.path_set_id, "set-2");
    assert_eq!(page.total_count, 3);
    assert_eq!(page.paths.len(), 2);
    assert_eq!(page.paths[0].path, "/tmp/b");
    assert_eq!(page.paths[1].path, "/tmp/c");
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
