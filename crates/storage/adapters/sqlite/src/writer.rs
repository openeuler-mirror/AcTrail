//! Single-writer boundary for serialized SQLite writes.

use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::payload::PayloadSegment;
use model_core::process::ProcessMembership;
use model_core::trace::{TraceHealth, TraceLifecycleState, TraceRecord};
use rusqlite::{Connection, params};
use store_write_contract::WriteError;
use store_write_contract::diagnostics::DiagnosticWriteStore;
use store_write_contract::events::EventWriteStore;
use store_write_contract::memberships::MembershipWriteStore;
use store_write_contract::payloads::PayloadWriteStore;
use store_write_contract::traces::TraceWriteStore;

use crate::SqliteStorage;
use crate::records::{
    EventMeta, PayloadSegmentMeta, StoredEventPayload, bool_to_i64, encode_diagnostic_kind,
    encode_diagnostic_severity, encode_event_kind, encode_event_payload,
    encode_exit_observation_source, encode_map, encode_membership_state, encode_policy_record,
    encode_tags, encode_time, encode_trace_health, encode_trace_lifecycle, payload_kind,
    take_shared_path,
};

impl TraceWriteStore for SqliteStorage {
    fn create_trace(&mut self, trace: TraceRecord) -> Result<(), WriteError> {
        let trace_id = trace.trace_id;
        let lifecycle_state = trace.lifecycle_state;
        let result = self.write_with_terminal_event_tail(
            trace_id.get(),
            trace_lifecycle_is_terminal(lifecycle_state),
            "create_trace",
            move |connection| {
                connection.prepare_cached(
                "INSERT INTO traces (
                    trace_id, otel_trace_id, alert_token, root_process_id, root_container_id, root_working_directory,
                    display_name, profile_name, tags, lifecycle_state, health, created_at,
                    started_at, completed_at, exited_at, failed_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                ON CONFLICT(trace_id) DO UPDATE SET
                    otel_trace_id = excluded.otel_trace_id,
                    alert_token = excluded.alert_token,
                    root_process_id = excluded.root_process_id,
                    root_container_id = excluded.root_container_id,
                    root_working_directory = excluded.root_working_directory,
                    display_name = excluded.display_name,
                    profile_name = excluded.profile_name,
                    tags = excluded.tags,
                    lifecycle_state = excluded.lifecycle_state,
                    health = excluded.health,
                    created_at = excluded.created_at,
                    started_at = excluded.started_at,
                    completed_at = excluded.completed_at,
                    exited_at = excluded.exited_at,
                    failed_at = excluded.failed_at",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    trace.trace_id.get(),
                    trace.otel_trace_id.as_bytes().as_slice(),
                    trace.alert_token.as_bytes().as_slice(),
                    trace.root_process_identity.get(),
                    trace.root_container_id.clone(),
                    trace.root_working_directory.clone(),
                    trace.display_name.to_string(),
                    trace.profile_name.to_string(),
                    encode_tags(&trace.tags),
                    encode_trace_lifecycle(trace.lifecycle_state),
                    encode_trace_health(trace.health),
                    encode_time(trace.timings.created_at),
                    trace.timings.started_at.map(encode_time),
                    trace.timings.completed_at.map(encode_time),
                    trace.timings.exited_at.map(encode_time),
                    trace.timings.failed_at.map(encode_time),
                ])
            })
            .map(|_| ())
            },
        );
        if result.is_ok() && trace_lifecycle_is_terminal(lifecycle_state) {
            self.event_payload_dictionary()
                .borrow_mut()
                .finish_trace(trace_id.get());
            self.event_path_dictionary()
                .borrow_mut()
                .finish_trace(trace_id.get());
        }
        result
    }

    fn update_trace_lifecycle(
        &mut self,
        trace_id: model_core::ids::TraceId,
        lifecycle_state: TraceLifecycleState,
    ) -> Result<(), WriteError> {
        let result = self.write_with_terminal_event_tail(
            trace_id.get(),
            trace_lifecycle_is_terminal(lifecycle_state),
            "update_trace_lifecycle",
            |connection| {
                connection
                    .prepare_cached("UPDATE traces SET lifecycle_state = ?2 WHERE trace_id = ?1")
                    .and_then(|mut statement| {
                        statement.execute(params![
                            trace_id.get(),
                            encode_trace_lifecycle(lifecycle_state)
                        ])
                    })
                    .map(|_| ())
            },
        );
        if result.is_ok() && trace_lifecycle_is_terminal(lifecycle_state) {
            self.event_payload_dictionary()
                .borrow_mut()
                .finish_trace(trace_id.get());
            self.event_path_dictionary()
                .borrow_mut()
                .finish_trace(trace_id.get());
        }
        result
    }

    fn update_trace_health(
        &mut self,
        trace_id: model_core::ids::TraceId,
        health: TraceHealth,
    ) -> Result<(), WriteError> {
        let connection = self.connection().borrow_mut();
        connection
            .prepare_cached("UPDATE traces SET health = ?2 WHERE trace_id = ?1")
            .and_then(|mut statement| {
                statement.execute(params![trace_id.get(), encode_trace_health(health)])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("update_trace_health", error.to_string()))
    }
}

impl MembershipWriteStore for SqliteStorage {
    fn upsert_membership(&mut self, membership: ProcessMembership) -> Result<(), WriteError> {
        let connection = self.connection().borrow_mut();
        connection
            .prepare_cached(
                "INSERT INTO memberships (
                    trace_id, process_id, inherited_from_process_id, observed_at,
                    capture_enabled, propagation_enabled, membership_state, exit_code,
                    exit_observed_at, exit_observation_source
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                ON CONFLICT(trace_id, process_id) DO UPDATE SET
                    inherited_from_process_id = excluded.inherited_from_process_id,
                    observed_at = excluded.observed_at,
                    capture_enabled = excluded.capture_enabled,
                    propagation_enabled = excluded.propagation_enabled,
                    membership_state = excluded.membership_state,
                    exit_code = excluded.exit_code,
                    exit_observed_at = excluded.exit_observed_at,
                    exit_observation_source = excluded.exit_observation_source
                WHERE memberships.inherited_from_process_id IS NOT excluded.inherited_from_process_id
                   OR memberships.observed_at IS NOT excluded.observed_at
                   OR memberships.capture_enabled IS NOT excluded.capture_enabled
                   OR memberships.propagation_enabled IS NOT excluded.propagation_enabled
                   OR memberships.membership_state IS NOT excluded.membership_state
                   OR memberships.exit_code IS NOT excluded.exit_code
                   OR memberships.exit_observed_at IS NOT excluded.exit_observed_at
                   OR memberships.exit_observation_source IS NOT excluded.exit_observation_source",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    membership.trace_id.get(),
                    membership.identity.get(),
                    membership.inherited_from.map(|identity| identity.get()),
                    membership.observed_at.map(encode_time),
                    bool_to_i64(membership.capture_enabled),
                    bool_to_i64(membership.propagation_enabled),
                    encode_membership_state(membership.state),
                    membership.exit_status.as_ref().and_then(|value| value.code),
                    membership
                        .exit_status
                        .as_ref()
                        .map(|value| encode_time(value.observed_at)),
                    membership
                        .exit_status
                        .as_ref()
                        .and_then(|value| value.source)
                        .map(encode_exit_observation_source),
                ])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("upsert_membership", error.to_string()))
    }
}

impl EventWriteStore for SqliteStorage {
    fn append_event(&mut self, event: DomainEvent) -> Result<(), WriteError> {
        self.append_event_atomically(event, |_| Ok(()))
    }
}

impl SqliteStorage {
    pub(crate) fn append_event_atomically(
        &mut self,
        event: DomainEvent,
        side_effect: impl FnOnce(&Connection) -> Result<(), rusqlite::Error>,
    ) -> Result<(), WriteError> {
        if !self.connection().borrow().is_autocommit() {
            self.append_event_record(event)?;
            return side_effect(&self.connection().borrow())
                .map_err(|error| WriteError::new("append_event_side_effect", error.to_string()));
        }
        self.connection()
            .borrow_mut()
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| WriteError::new("begin_event_append", error.to_string()))?;
        let block_begin = self
            .event_record_blocks()
            .borrow_mut()
            .begin_transaction(&self.connection().borrow());
        if let Err(error) = block_begin {
            let rollback_result = self.connection().borrow_mut().execute_batch("ROLLBACK");
            if rollback_result.is_err() {
                self.event_record_blocks().borrow_mut().poison_transaction();
            }
            return Err(error);
        }
        self.event_payload_dictionary()
            .borrow_mut()
            .begin_transaction();
        self.event_path_dictionary()
            .borrow_mut()
            .begin_transaction();
        let append_result = self
            .append_event_record(event)
            .and_then(|()| {
                side_effect(&self.connection().borrow())
                    .map_err(|error| WriteError::new("append_event_side_effect", error.to_string()))
            })
            .and_then(|()| {
                self.event_record_blocks()
                    .borrow()
                    .persist_transaction_state(&self.connection().borrow())
            });
        match append_result {
            Ok(()) => {
                let release_result = self
                    .connection()
                    .borrow_mut()
                    .execute_batch("COMMIT")
                    .map_err(|error| WriteError::new("commit_event_append", error.to_string()));
                if release_result.is_ok() {
                    self.event_payload_dictionary()
                        .borrow_mut()
                        .commit_transaction();
                    self.event_path_dictionary()
                        .borrow_mut()
                        .commit_transaction();
                    self.event_record_blocks().borrow_mut().commit_transaction();
                } else {
                    let rollback_result = self.connection().borrow_mut().execute_batch("ROLLBACK");
                    self.event_payload_dictionary()
                        .borrow_mut()
                        .rollback_transaction();
                    self.event_path_dictionary()
                        .borrow_mut()
                        .rollback_transaction();
                    if rollback_result.is_ok() {
                        self.event_record_blocks()
                            .borrow_mut()
                            .rollback_transaction();
                    } else {
                        self.event_record_blocks().borrow_mut().poison_transaction();
                    }
                }
                release_result
            }
            Err(append_error) => {
                let rollback_result = self.connection().borrow_mut().execute_batch("ROLLBACK");
                self.event_payload_dictionary()
                    .borrow_mut()
                    .rollback_transaction();
                self.event_path_dictionary()
                    .borrow_mut()
                    .rollback_transaction();
                if rollback_result.is_ok() {
                    self.event_record_blocks()
                        .borrow_mut()
                        .rollback_transaction();
                } else {
                    self.event_record_blocks().borrow_mut().poison_transaction();
                }
                match rollback_result {
                    Ok(()) => Err(append_error),
                    Err(rollback_error) => Err(WriteError::new(
                        "rollback_event_append",
                        format!(
                            "{}: {}; rollback failed: {rollback_error}",
                            append_error.stage, append_error.message
                        ),
                    )),
                }
            }
        }
    }

    fn write_with_terminal_event_tail(
        &mut self,
        trace_id: u64,
        terminal: bool,
        stage: &'static str,
        write: impl FnOnce(&rusqlite::Connection) -> Result<(), rusqlite::Error>,
    ) -> Result<(), WriteError> {
        let has_pending = terminal
            && self
                .event_record_blocks()
                .borrow()
                .has_pending_trace(&self.connection().borrow(), trace_id)?;
        if !has_pending {
            write(&self.connection().borrow())
                .map_err(|error| WriteError::new(stage, error.to_string()))?;
            if terminal && !self.connection().borrow().is_autocommit() {
                self.event_record_blocks()
                    .borrow_mut()
                    .mark_terminal_trace(trace_id)?;
            }
            return Ok(());
        }
        if !self.connection().borrow().is_autocommit() {
            write(&self.connection().borrow())
                .map_err(|error| WriteError::new(stage, error.to_string()))?;
            let mut blocks = self.event_record_blocks().borrow_mut();
            blocks.mark_terminal_trace(trace_id)?;
            return blocks.flush_trace(&self.connection().borrow(), trace_id);
        }

        self.connection()
            .borrow_mut()
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| WriteError::new("begin_terminal_trace", error.to_string()))?;
        let block_begin = self
            .event_record_blocks()
            .borrow_mut()
            .begin_transaction(&self.connection().borrow());
        if let Err(error) = block_begin {
            let rollback_result = self.connection().borrow_mut().execute_batch("ROLLBACK");
            if rollback_result.is_err() {
                self.event_record_blocks().borrow_mut().poison_transaction();
            }
            return Err(error);
        }
        let result = write(&self.connection().borrow())
            .map_err(|error| WriteError::new(stage, error.to_string()))
            .and_then(|()| {
                let mut blocks = self.event_record_blocks().borrow_mut();
                blocks.mark_terminal_trace(trace_id)?;
                blocks.flush_trace(&self.connection().borrow(), trace_id)
            });
        match result {
            Ok(()) => {
                let commit_result = self
                    .connection()
                    .borrow_mut()
                    .execute_batch("COMMIT")
                    .map_err(|error| WriteError::new("commit_terminal_trace", error.to_string()));
                if commit_result.is_ok() {
                    self.event_record_blocks().borrow_mut().commit_transaction();
                } else {
                    let rollback_result = self.connection().borrow_mut().execute_batch("ROLLBACK");
                    if rollback_result.is_ok() {
                        self.event_record_blocks()
                            .borrow_mut()
                            .rollback_transaction();
                    } else {
                        self.event_record_blocks().borrow_mut().poison_transaction();
                    }
                }
                commit_result
            }
            Err(error) => {
                let rollback_result = self.connection().borrow_mut().execute_batch("ROLLBACK");
                if rollback_result.is_ok() {
                    self.event_record_blocks()
                        .borrow_mut()
                        .rollback_transaction();
                    Err(error)
                } else {
                    self.event_record_blocks().borrow_mut().poison_transaction();
                    Err(WriteError::new(
                        "rollback_terminal_trace",
                        format!(
                            "{}: {}; rollback failed: {}",
                            error.stage,
                            error.message,
                            rollback_result.expect_err("checked rollback failure")
                        ),
                    ))
                }
            }
        }
    }

    fn append_event_record(&mut self, mut event: DomainEvent) -> Result<(), WriteError> {
        if payload_kind(&event.payload) != event.envelope.kind {
            return Err(WriteError::new(
                "event_payload_kind",
                "event envelope kind does not match payload variant",
            ));
        }
        self.event_record_blocks()
            .borrow_mut()
            .observe_event_id(&self.connection().borrow(), event.envelope.event_id.get())?;
        let connection_handle = self.connection().clone();
        let connection = connection_handle.borrow_mut();
        let payload_path_id = take_shared_path(&mut event.payload)
            .map(|path| {
                self.event_path_dictionary().borrow_mut().intern(
                    &connection,
                    event.envelope.trace_id.get(),
                    &path,
                )
            })
            .transpose()?;
        if self.event_record_blocks().borrow().enabled() {
            return self.event_record_blocks().borrow_mut().append(
                &connection,
                event,
                payload_path_id,
            );
        }
        let encoded = encode_event_payload(&mut event.payload)
            .map_err(|error| WriteError::new("encode_event_payload", error.to_string()))?;
        let (policy_redactions, policy_truncations) = encode_policy_record(&event.policy);
        let has_policy_details = event.policy.note.is_some()
            || !policy_redactions.is_empty()
            || !policy_truncations.is_empty();
        let event_meta = EventMeta::encode(
            &event.envelope.collector,
            &event.envelope.flags,
            event.policy.verdict,
            !encoded.blocks.is_empty(),
            has_policy_details,
        )
        .ok_or_else(|| {
            WriteError::new(
                "encode_event_meta",
                format!("unknown collector {}", event.envelope.collector.as_str()),
            )
        })?;
        let dictionary_handle = self.event_payload_dictionary().clone();
        let stored_payload = dictionary_handle.borrow_mut().intern(
            &connection,
            event.envelope.trace_id.get(),
            encoded.fields,
        )?;
        let (payload_id, payload_inline) = match stored_payload {
            StoredEventPayload::Dictionary(payload_id) => (Some(payload_id), None),
            StoredEventPayload::Inline(payload) => (None, Some(payload)),
        };
        connection
            .prepare_cached(
                "INSERT INTO events (
                    event_id, trace_id, observed_at, process_id, event_meta,
                    kind_code,
                    payload_path_id, payload_id, payload_inline
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    event.envelope.event_id.get(),
                    event.envelope.trace_id.get(),
                    encode_time(event.envelope.observed_at),
                    event.envelope.process.get(),
                    event_meta.code(),
                    encode_event_kind(event.envelope.kind),
                    payload_path_id,
                    payload_id,
                    payload_inline,
                ])
            })
            .map_err(|error| WriteError::new("append_event", error.to_string()))?;
        for (block_order, block) in encoded.blocks.iter().enumerate() {
            let compressed = zstd::stream::encode_all(
                block.bytes.as_slice(),
                self.cold_field_compression.zstd_level,
            )
            .map_err(|error| WriteError::new("encode_event_payload_block", error.to_string()))?;
            connection
                .execute(
                    "INSERT INTO event_payload_blocks (event_id, block_order, kind, encoded_bytes)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        event.envelope.event_id.get(),
                        block_order,
                        block.kind.to_i64(),
                        compressed,
                    ],
                )
                .map_err(|error| {
                    WriteError::new("insert_event_payload_block", error.to_string())
                })?;
        }
        if has_policy_details {
            connection
                .execute(
                    "INSERT INTO event_policy_details (event_id, note, redactions, truncations)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        event.envelope.event_id.get(),
                        event.policy.note,
                        policy_redactions,
                        policy_truncations,
                    ],
                )
                .map_err(|error| {
                    WriteError::new("insert_event_policy_details", error.to_string())
                })?;
        }
        Ok(())
    }
}

const fn trace_lifecycle_is_terminal(state: TraceLifecycleState) -> bool {
    matches!(
        state,
        TraceLifecycleState::Completed | TraceLifecycleState::Exited | TraceLifecycleState::Failed
    )
}

impl PayloadWriteStore for SqliteStorage {
    fn append_payload_segment(&mut self, segment: PayloadSegment) -> Result<(), WriteError> {
        let connection = self.connection().borrow_mut();
        connection
            .prepare_cached(
                "INSERT OR REPLACE INTO payload_segments (
                    segment_id, trace_id, observed_at, process_id, segment_meta,
                    stream_key, sequence,
                    original_size, captured_size, operation_id, operation_offset,
                    operation_original_size, operation_captured_size,
                    library, symbol, protocol_hint, bytes
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )
            .and_then(|mut statement| {
                let segment_meta = PayloadSegmentMeta::from_segment(&segment);
                statement.execute(params![
                    segment.segment_id.get(),
                    segment.trace_id.get(),
                    encode_time(segment.observed_at),
                    segment.process.get(),
                    segment_meta.code(),
                    segment.stream_key.to_string(),
                    segment.sequence,
                    segment.original_size,
                    segment.captured_size,
                    segment.operation_id,
                    segment.operation_offset,
                    segment.operation_original_size,
                    segment.operation_captured_size,
                    segment.library,
                    segment.symbol,
                    segment.protocol_hint,
                    segment.bytes,
                ])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("append_payload_segment", error.to_string()))
    }
}

impl DiagnosticWriteStore for SqliteStorage {
    fn append_diagnostic(&mut self, diagnostic: DiagnosticRecord) -> Result<(), WriteError> {
        let connection = self.connection().borrow_mut();
        connection
            .prepare_cached(
                "INSERT OR REPLACE INTO diagnostics (
                    diagnostic_id, trace_id, process_id, kind, severity, emitted_at, message, metadata
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    diagnostic.diagnostic_id.get(),
                    diagnostic.trace_id.map(|value| value.get()),
                    diagnostic.process.map(|value| value.get()),
                    encode_diagnostic_kind(diagnostic.kind),
                    encode_diagnostic_severity(diagnostic.severity),
                    encode_time(diagnostic.emitted_at),
                    diagnostic.message,
                    encode_map(&diagnostic.metadata),
                ])
            })
            .map(|_| ())
            .map_err(|error| WriteError::new("append_diagnostic", error.to_string()))
    }
}
