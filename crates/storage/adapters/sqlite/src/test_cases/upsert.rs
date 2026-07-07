use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadRedactionState,
    PayloadSegment, PayloadSegmentId, PayloadSourceBoundary, PayloadStreamKey,
    PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use model_core::trace::{TraceHealth, TraceLifecycleState};
use semantic_action::{
    FilePathSetState, FilePathSetWrite, LlmRequestBlock, LlmRequestBlockRef,
    LlmRequestContentWrite, LlmRequestManifest, SemanticAction, SemanticActionCompleteness,
    SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence, SemanticActionLinkRole,
    SemanticActionReadStore, SemanticActionStatus, SemanticActionWriteStore, SemanticEvidence,
    SemanticEvidenceKind, attr_keys as attrs, file_path_set_identity_for_paths,
};
use sha2::{Digest, Sha256};
use store_retention_contract::cleanup::RetentionStore;
use store_retention_contract::tombstone::TraceTombstone;
use store_write_contract::payloads::PayloadWriteStore;

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
fn semantic_action_attributes_are_stored_as_compressed_cold_fields() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(1);
    let long_value = "contains_llm_call;".repeat(512);

    let mut parent = action("parent", SemanticActionStatus::Success);
    parent
        .attributes
        .insert("large".to_string(), long_value.clone());
    let mut child = action("child", SemanticActionStatus::Success);
    child
        .attributes
        .insert("large".to_string(), long_value.clone());
    storage
        .upsert_semantic_action(parent)
        .expect("write parent action");
    storage
        .upsert_semantic_action(child)
        .expect("write child action");
    storage
        .upsert_semantic_action_link(SemanticActionLink {
            trace_id,
            parent_action_id: "parent".to_string(),
            child_action_id: "child".to_string(),
            role: SemanticActionLinkRole::LlmCallResponse,
            confidence: SemanticActionLinkConfidence::Observed,
            valid: true,
            evidence: Vec::new(),
            attributes: BTreeMap::from([("large".to_string(), long_value.clone())]),
        })
        .expect("write compressed link attributes");

    let connection = storage.connection().borrow();
    let (hot_attributes, action_uncompressed, action_payload) = connection
        .query_row(
            "SELECT action.attributes,
                    cold.uncompressed_bytes,
                    length(cold.payload)
             FROM semantic_actions action
             JOIN semantic_action_ids ids
               ON ids.action_key = action.action_key
             JOIN semantic_action_cold_fields cold
               ON cold.owner_key = action.action_key
             WHERE ids.action_id = 'parent'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .expect("read compressed action attributes");
    assert_eq!(hot_attributes, "");
    assert!(action_payload < action_uncompressed);

    let (link_hot_attributes, link_uncompressed, link_payload) = connection
        .query_row(
            "SELECT link.attributes,
                    cold.uncompressed_bytes,
                    length(cold.payload)
             FROM semantic_action_links link
             JOIN semantic_action_link_cold_fields cold
               ON cold.trace_id = link.trace_id
              AND cold.parent_action_key = link.parent_action_key
              AND cold.child_action_key = link.child_action_key
              AND cold.role_code = link.role_code",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .expect("read compressed link attributes");
    assert_eq!(link_hot_attributes, "");
    assert!(link_payload < link_uncompressed);
    drop(connection);

    let actions = storage
        .list_semantic_actions(trace_id)
        .expect("read semantic actions");
    let parent = actions
        .iter()
        .find(|action| action.action_id == "parent")
        .expect("parent action should be present");
    assert_eq!(parent.attributes["large"], long_value);
    let links = storage
        .list_semantic_action_links(trace_id)
        .expect("read semantic action links");
    assert_eq!(links[0].attributes["large"], long_value);
}

#[test]
fn purge_trace_removes_semantic_action_interning_and_cold_fields() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(1);
    let long_value = "contains_llm_call;".repeat(512);

    let mut parent = action("parent", SemanticActionStatus::Success);
    parent
        .attributes
        .insert("large".to_string(), long_value.clone());
    let mut child = action("child", SemanticActionStatus::Success);
    child
        .attributes
        .insert("large".to_string(), long_value.clone());
    storage
        .upsert_semantic_action(parent)
        .expect("write parent action");
    storage
        .upsert_semantic_action(child)
        .expect("write child action");
    storage
        .upsert_semantic_action_link(SemanticActionLink {
            trace_id,
            parent_action_id: "parent".to_string(),
            child_action_id: "child".to_string(),
            role: SemanticActionLinkRole::LlmCallResponse,
            confidence: SemanticActionLinkConfidence::Observed,
            valid: true,
            evidence: Vec::new(),
            attributes: BTreeMap::from([("large".to_string(), long_value)]),
        })
        .expect("write link attributes");

    assert!(row_count(&storage, "semantic_action_ids") > 0);
    assert!(row_count(&storage, "semantic_action_cold_fields") > 0);
    assert!(row_count(&storage, "semantic_action_link_cold_fields") > 0);

    storage
        .purge_trace(
            trace_id,
            TraceTombstone {
                trace_id,
                lifecycle_state: TraceLifecycleState::Completed,
                health: TraceHealth::Degraded,
                cleaned_at: UNIX_EPOCH,
                cleanup_reason: "test".to_string(),
            },
        )
        .expect("purge trace");

    assert_eq!(row_count(&storage, "semantic_action_cold_fields"), 0);
    assert_eq!(row_count(&storage, "semantic_action_link_cold_fields"), 0);
    assert_eq!(row_count(&storage, "semantic_action_ids"), 0);
    assert_eq!(row_count(&storage, "semantic_actions"), 0);
    assert_eq!(row_count(&storage, "semantic_action_links"), 0);
}

#[test]
fn purge_trace_removes_payload_segments() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(1);
    storage
        .append_payload_segment(payload_segment(trace_id))
        .expect("write payload segment");

    assert_eq!(row_count(&storage, "payload_segments"), 1);

    storage
        .purge_trace(
            trace_id,
            TraceTombstone {
                trace_id,
                lifecycle_state: TraceLifecycleState::Completed,
                health: TraceHealth::Clean,
                cleaned_at: UNIX_EPOCH,
                cleanup_reason: "test".to_string(),
            },
        )
        .expect("purge trace");

    assert_eq!(row_count(&storage, "payload_segments"), 0);
}

#[test]
fn file_path_sets_page_paths_and_reuse_identical_chunks() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(1);
    let paths = vec![
        "/tmp/a".to_string(),
        "/tmp/b".to_string(),
        "/tmp/c".to_string(),
    ];
    let identity = file_path_set_identity_for_paths(
        FilePathSetState::Complete,
        "path-id-v1:chunk-max=2",
        paths.iter().map(String::as_str),
    );
    let first = FilePathSetWrite {
        trace_id,
        action_id: "action-1".to_string(),
        path_set_id: identity.path_set_id.clone(),
        state: FilePathSetState::Complete,
        unique_path_count: 3,
        stored_path_count: 3,
        chunking_scheme: "path-id-v1:chunk-max=2".to_string(),
        chunk_max_paths: 2,
        paths,
    };
    let mut second = first.clone();
    second.action_id = "action-2".to_string();

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
            .query_row("SELECT COUNT(*) FROM file_path_sets", [], |row| row
                .get::<_, i64>(0))
            .expect("read file_path_sets count"),
        1
    );
    assert_eq!(
        storage
            .connection()
            .borrow()
            .query_row(
                "SELECT COUNT(*) FROM file_path_set_action_refs",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .expect("read file_path_set_action_refs count"),
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
        2
    );

    let page = storage
        .file_path_set_paths_page(trace_id, "action-2", 1, 2)
        .expect("read file path set page")
        .expect("path set should exist");
    assert_eq!(page.path_set_id, identity.path_set_id);
    assert_eq!(page.total_count, 3);
    assert_eq!(page.paths.len(), 2);
    assert_eq!(page.paths[0].path, "/tmp/b");
    assert_eq!(page.paths[1].path, "/tmp/c");
}

#[test]
fn llm_request_content_reconstructs_body_and_reuses_blocks() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(1);
    for action_id in ["request-1", "request-2"] {
        let mut request = action(action_id, SemanticActionStatus::Success);
        request.kind = SemanticActionKind::LlmRequest;
        storage
            .upsert_semantic_action(request)
            .expect("write request action");
    }

    storage
        .upsert_llm_request_contents(&[
            llm_request_content(trace_id, "request-1"),
            llm_request_content(trace_id, "request-2"),
        ])
        .expect("write LLM request contents");

    assert_eq!(
        storage
            .connection()
            .borrow()
            .query_row("SELECT COUNT(*) FROM llm_request_blocks", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("read llm_request_blocks count"),
        1
    );
    assert_eq!(
        storage
            .connection()
            .borrow()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('llm_request_block_refs')
                 WHERE name IN ('trace_id', 'action_id', 'block_hash', 'block_kind')",
                [],
                |row| row.get::<_, i64>(0)
            )
            .expect("read compact ref schema"),
        0
    );
    let page = storage
        .llm_request_content_page(trace_id, "request-2", 4096)
        .expect("read LLM request content")
        .expect("content should exist");
    assert_eq!(
        page.body_json,
        r#"{"messages":[{"content":"hello","role":"user"}],"model":"m"}"#
    );
    assert!(!page.truncated);
}

#[test]
fn semantic_action_lists_load_action_and_link_evidence() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(1);
    let mut parent = action("parent", SemanticActionStatus::Success);
    parent.evidence.push(evidence(10));
    parent.evidence.push(evidence(11));
    let mut child = action("child", SemanticActionStatus::Success);
    child.evidence.push(evidence(12));
    storage
        .upsert_semantic_action(parent)
        .expect("write parent action");
    storage
        .upsert_semantic_action(child)
        .expect("write child action");

    storage
        .upsert_semantic_action_link(SemanticActionLink {
            trace_id,
            parent_action_id: "parent".to_string(),
            child_action_id: "child".to_string(),
            role: SemanticActionLinkRole::LlmCallResponse,
            confidence: SemanticActionLinkConfidence::Observed,
            valid: true,
            evidence: vec![evidence(20), evidence(21)],
            attributes: BTreeMap::new(),
        })
        .expect("write action link");

    let actions = storage
        .list_semantic_actions(trace_id)
        .expect("list semantic actions");
    let evidence_by_action = actions
        .iter()
        .map(|action| {
            (
                action.action_id.as_str(),
                action
                    .evidence
                    .iter()
                    .map(|evidence| evidence.id)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(evidence_by_action["parent"], vec![10, 11]);
    assert_eq!(evidence_by_action["child"], vec![12]);

    let links = storage
        .list_semantic_action_links(trace_id)
        .expect("list semantic action links");
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0]
            .evidence
            .iter()
            .map(|evidence| evidence.id)
            .collect::<Vec<_>>(),
        vec![20, 21]
    );
}

#[test]
fn mcp_action_inherits_existing_same_process_parent_attrs() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(1);

    let mut command = action("command", SemanticActionStatus::Success);
    command.kind = SemanticActionKind::CommandInvocation;
    command.attributes.extend(parent_attrs());
    storage
        .upsert_semantic_action(command)
        .expect("write command action");

    let mut mcp = action("mcp", SemanticActionStatus::Success);
    mcp.kind = SemanticActionKind::McpToolCall;
    mcp.attributes
        .insert(attrs::mcp::TOOL_NAME.to_string(), "emit_probe".to_string());
    storage
        .upsert_semantic_action(mcp)
        .expect("write mcp action");

    let actions = storage
        .list_semantic_actions(trace_id)
        .expect("list semantic actions");
    let mcp = actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::McpToolCall)
        .expect("mcp action should exist");
    assert_eq!(
        mcp.attributes
            .get(attrs::process_parent::PID)
            .map(String::as_str),
        Some("200")
    );
    assert_eq!(
        mcp.attributes
            .get(attrs::mcp::CLIENT_PID)
            .map(String::as_str),
        Some("200")
    );
}

#[test]
fn parent_action_backfills_existing_same_process_mcp_action() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(1);

    let mut mcp = action("mcp", SemanticActionStatus::Success);
    mcp.kind = SemanticActionKind::McpToolCall;
    mcp.attributes
        .insert(attrs::mcp::TOOL_NAME.to_string(), "emit_probe".to_string());
    storage
        .upsert_semantic_action(mcp)
        .expect("write mcp action");

    let mut command = action("command", SemanticActionStatus::Success);
    command.kind = SemanticActionKind::CommandInvocation;
    command.attributes.extend(parent_attrs());
    storage
        .upsert_semantic_action(command)
        .expect("write command action");

    let actions = storage
        .list_semantic_actions(trace_id)
        .expect("list semantic actions");
    let mcp = actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::McpToolCall)
        .expect("mcp action should exist");
    assert_eq!(
        mcp.attributes
            .get(attrs::process_parent::PID)
            .map(String::as_str),
        Some("200")
    );
    assert_eq!(
        mcp.attributes
            .get(attrs::mcp::CLIENT_PID)
            .map(String::as_str),
        Some("200")
    );
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

fn row_count(storage: &SqliteStorage, table_name: &str) -> i64 {
    let query = format!("SELECT COUNT(*) FROM {table_name}");
    storage
        .connection()
        .borrow()
        .query_row(&query, [], |row| row.get(0))
        .expect("read sqlite table row count")
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

fn payload_segment(trace_id: TraceId) -> PayloadSegment {
    PayloadSegment {
        segment_id: PayloadSegmentId::new(1),
        trace_id,
        observed_at: UNIX_EPOCH + Duration::from_millis(10),
        process: ProcessIdentity::new(100, 1, 1),
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction: PayloadDirection::Outbound,
        stream_key: PayloadStreamKey::new("stream"),
        sequence: 1,
        original_size: 5,
        captured_size: 5,
        operation_id: 1,
        operation_offset: 0,
        operation_original_size: 5,
        operation_captured_size: 5,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        redaction: PayloadRedactionState::NotRequired,
        library: "test".to_string(),
        symbol: "SSL_write".to_string(),
        protocol_hint: None,
        bytes: b"hello".to_vec(),
    }
}

fn parent_attrs() -> BTreeMap<String, String> {
    BTreeMap::from([
        (attrs::process_parent::PID.to_string(), "200".to_string()),
        (
            attrs::process_parent::GENERATION.to_string(),
            "2".to_string(),
        ),
        (
            attrs::process_parent::START_TIME_TICKS.to_string(),
            "2".to_string(),
        ),
        (
            attrs::process_parent::PID_NAMESPACE.to_string(),
            "pid:[parent]".to_string(),
        ),
        (
            attrs::process_parent::IDENTITY_STATE.to_string(),
            "observed".to_string(),
        ),
    ])
}

fn llm_request_content(trace_id: TraceId, action_id: &str) -> LlmRequestContentWrite {
    let body_json = r#"{"messages":[{"content":"hello","role":"user"}],"model":"m"}"#;
    let block_json = r#"{"content":"hello","role":"user"}"#;
    let block_hash = sha256_hex(block_json.as_bytes());
    LlmRequestContentWrite {
        manifest: LlmRequestManifest {
            trace_id,
            action_id: action_id.to_string(),
            format_version: 1,
            canonical_body_hash: sha256_hex(body_json.as_bytes()),
            canonical_body_bytes: body_json.len() as u64,
            skeleton_json: r#"{"messages":[{"$actrail_llm_block":0}],"model":"m"}"#.to_string(),
        },
        block_refs: vec![LlmRequestBlockRef {
            trace_id,
            action_id: action_id.to_string(),
            ordinal: 0,
            block_hash: block_hash.clone(),
        }],
        blocks: vec![LlmRequestBlock {
            trace_id,
            block_hash,
            uncompressed_bytes: block_json.len() as u64,
            encoded_bytes: block_json.as_bytes().to_vec(),
        }],
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::from("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to string cannot fail");
    }
    output
}
