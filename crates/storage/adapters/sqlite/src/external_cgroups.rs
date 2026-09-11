//! Durable runtime-owned cgroup binding lifecycle.

use std::path::PathBuf;
use std::time::SystemTime;

use model_core::container::{ContainerRuntime, NormalizedContainerId};
use model_core::event::DomainEvent;
use model_core::external_cgroup::{
    ExternalBindingStaleReason, ExternalBindingState, ExternalCgroupBinding, HostBootId,
};
use model_core::ids::{EventId, TraceId};
use rusqlite::{Error as SqlError, OptionalExtension, Row, params};

use crate::SqliteStorage;
use crate::records::{decode_time, encode_time};
use crate::writer::append_event_to_connection;

impl SqliteStorage {
    pub fn create_external_cgroup_binding(
        &mut self,
        binding: &ExternalCgroupBinding,
    ) -> Result<(), SqlError> {
        validate_binding(binding)?;
        let connection = self.connection().borrow_mut();
        let container_id = binding.container_id.to_string();
        let relative_path = binding.relative_path.to_string_lossy().into_owned();
        let cgroup_device = binding.cgroup_device.to_be_bytes();
        let cgroup_inode = binding.cgroup_inode.to_be_bytes();
        let host_boot_id = binding.host_boot_id.as_bytes();
        match connection.execute(
            "INSERT INTO trace_external_cgroup_bindings (
                trace_id, runtime, container_id, relative_path,
                cgroup_device_be, cgroup_inode_be, host_boot_id,
                lifecycle_state, stale_reason, consecutive_failures,
                created_at, updated_at, last_good_at, closed_at, final_event_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                binding.trace_id.get(),
                binding.runtime.as_str(),
                container_id,
                relative_path,
                cgroup_device.as_slice(),
                cgroup_inode.as_slice(),
                host_boot_id.as_slice(),
                binding.lifecycle_state.as_storage_str(),
                binding.stale_reason.map(|reason| reason.as_storage_str()),
                binding.consecutive_failures,
                encode_time(binding.created_at),
                encode_time(binding.updated_at),
                binding.last_good_at.map(encode_time),
                binding.closed_at.map(encode_time),
                binding.final_event_id.map(EventId::get),
            ],
        ) {
            Ok(_) => Ok(()),
            Err(error) if is_constraint_violation(&error) => {
                drop(connection);
                if self.get_external_cgroup_binding(binding.trace_id)?.as_ref() == Some(binding) {
                    Ok(())
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }

    pub fn get_external_cgroup_binding(
        &self,
        trace_id: TraceId,
    ) -> Result<Option<ExternalCgroupBinding>, SqlError> {
        self.connection()
            .borrow()
            .query_row(
                &format!("{} WHERE trace_id = ?1", external_binding_select()),
                [trace_id.get()],
                decode_binding,
            )
            .optional()
    }

    pub fn list_live_external_cgroup_bindings(
        &self,
    ) -> Result<Vec<ExternalCgroupBinding>, SqlError> {
        let connection = self.connection().borrow();
        let mut statement = connection.prepare(&format!(
            "{} WHERE lifecycle_state IN ('active', 'stale') ORDER BY trace_id",
            external_binding_select()
        ))?;
        statement
            .query_map([], decode_binding)?
            .collect::<Result<Vec<_>, _>>()
    }

    pub fn record_external_cgroup_success(
        &mut self,
        trace_id: TraceId,
        observed_at: SystemTime,
    ) -> Result<(), SqlError> {
        require_one(self.connection().borrow_mut().execute(
            "UPDATE trace_external_cgroup_bindings
             SET consecutive_failures = 0, last_good_at = ?2, updated_at = ?2
             WHERE trace_id = ?1 AND lifecycle_state = 'active'",
            params![trace_id.get(), encode_time(observed_at)],
        )?)
    }

    pub fn record_external_cgroup_failure(
        &mut self,
        trace_id: TraceId,
        observed_at: SystemTime,
    ) -> Result<u32, SqlError> {
        require_one(self.connection().borrow_mut().execute(
            "UPDATE trace_external_cgroup_bindings
             SET consecutive_failures = consecutive_failures + 1, updated_at = ?2
             WHERE trace_id = ?1 AND lifecycle_state = 'active'
                   AND consecutive_failures < 4294967295",
            params![trace_id.get(), encode_time(observed_at)],
        )?)?;
        self.connection().borrow().query_row(
            "SELECT consecutive_failures FROM trace_external_cgroup_bindings WHERE trace_id = ?1",
            [trace_id.get()],
            |row| row.get(0),
        )
    }

    pub fn mark_external_cgroup_stale(
        &mut self,
        trace_id: TraceId,
        reason: ExternalBindingStaleReason,
        updated_at: SystemTime,
    ) -> Result<(), SqlError> {
        let connection = self.connection().borrow_mut();
        let changed = connection.execute(
            "UPDATE trace_external_cgroup_bindings
             SET lifecycle_state = 'stale', stale_reason = ?2, updated_at = ?3
             WHERE trace_id = ?1 AND lifecycle_state = 'active'",
            params![
                trace_id.get(),
                reason.as_storage_str(),
                encode_time(updated_at)
            ],
        )?;
        if changed == 1 {
            return Ok(());
        }
        let current = connection
            .query_row(
                "SELECT lifecycle_state, stale_reason
                 FROM trace_external_cgroup_bindings WHERE trace_id = ?1",
                [trace_id.get()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        match current {
            Some((state, current_reason))
                if state == "stale"
                    && current_reason.as_deref() == Some(reason.as_storage_str()) =>
            {
                Ok(())
            }
            _ => Err(SqlError::InvalidQuery),
        }
    }

    pub fn append_final_event_and_close_external_binding(
        &mut self,
        event: DomainEvent,
        closed_at: SystemTime,
    ) -> Result<EventId, SqlError> {
        let trace_id = event.envelope.trace_id;
        let proposed_event_id = event.envelope.event_id;
        let zstd_level = self.cold_field_compression.zstd_level;
        let mut connection = self.connection().borrow_mut();
        let transaction = connection.transaction()?;
        let current = transaction
            .query_row(
                "SELECT lifecycle_state, final_event_id
                 FROM trace_external_cgroup_bindings WHERE trace_id = ?1",
                [trace_id.get()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<u64>>(1)?)),
            )
            .optional()?;
        match current {
            Some((state, Some(event_id))) if state == "closed" => {
                transaction.commit()?;
                return Ok(EventId::new(event_id));
            }
            Some((state, None)) if matches!(state.as_str(), "active" | "stale") => {}
            _ => return Err(SqlError::InvalidQuery),
        }
        append_event_to_connection(&transaction, zstd_level, event)
            .map_err(|_| SqlError::InvalidQuery)?;
        require_one(transaction.execute(
            "UPDATE trace_external_cgroup_bindings
             SET lifecycle_state = 'closed', closed_at = ?2,
                 final_event_id = ?3, updated_at = ?2
             WHERE trace_id = ?1 AND lifecycle_state IN ('active', 'stale')
                   AND final_event_id IS NULL",
            params![
                trace_id.get(),
                encode_time(closed_at),
                proposed_event_id.get()
            ],
        )?)?;
        transaction.commit()?;
        Ok(proposed_event_id)
    }
}

fn external_binding_select() -> &'static str {
    "SELECT trace_id, runtime, container_id, relative_path,
            cgroup_device_be, cgroup_inode_be, host_boot_id,
            lifecycle_state, stale_reason, consecutive_failures,
            created_at, updated_at, last_good_at, closed_at, final_event_id
     FROM trace_external_cgroup_bindings"
}

fn decode_binding(row: &Row<'_>) -> Result<ExternalCgroupBinding, SqlError> {
    let runtime = ContainerRuntime::from_storage_str(&row.get::<_, String>("runtime")?)
        .ok_or(SqlError::InvalidQuery)?;
    let container_id =
        NormalizedContainerId::from_lower_hex(&row.get::<_, String>("container_id")?)
            .map_err(|_| SqlError::InvalidQuery)?;
    let lifecycle_state =
        ExternalBindingState::from_storage_str(&row.get::<_, String>("lifecycle_state")?)
            .ok_or(SqlError::InvalidQuery)?;
    let stale_reason = row
        .get::<_, Option<String>>("stale_reason")?
        .map(|raw| ExternalBindingStaleReason::from_storage_str(&raw).ok_or(SqlError::InvalidQuery))
        .transpose()?;
    let host_boot_id = HostBootId::from_bytes(decode_array(row.get("host_boot_id")?)?);
    let binding = ExternalCgroupBinding {
        trace_id: TraceId::new(row.get("trace_id")?),
        runtime,
        container_id,
        relative_path: PathBuf::from(row.get::<_, String>("relative_path")?),
        cgroup_device: u64::from_be_bytes(decode_array(row.get("cgroup_device_be")?)?),
        cgroup_inode: u64::from_be_bytes(decode_array(row.get("cgroup_inode_be")?)?),
        host_boot_id,
        lifecycle_state,
        stale_reason,
        consecutive_failures: row.get("consecutive_failures")?,
        created_at: decode_time(row.get("created_at")?),
        updated_at: decode_time(row.get("updated_at")?),
        last_good_at: row.get::<_, Option<i64>>("last_good_at")?.map(decode_time),
        closed_at: row.get::<_, Option<i64>>("closed_at")?.map(decode_time),
        final_event_id: row
            .get::<_, Option<u64>>("final_event_id")?
            .map(EventId::new),
    };
    validate_binding(&binding)?;
    Ok(binding)
}

fn decode_array<const N: usize>(bytes: Vec<u8>) -> Result<[u8; N], SqlError> {
    bytes.try_into().map_err(|_| SqlError::InvalidQuery)
}

fn validate_binding(binding: &ExternalCgroupBinding) -> Result<(), SqlError> {
    let valid_runtime = !matches!(binding.runtime, ContainerRuntime::Unknown);
    let valid = valid_runtime
        && match binding.lifecycle_state {
            ExternalBindingState::Active => {
                binding.stale_reason.is_none()
                    && binding.closed_at.is_none()
                    && binding.final_event_id.is_none()
            }
            ExternalBindingState::Stale => {
                binding.stale_reason.is_some()
                    && binding.closed_at.is_none()
                    && binding.final_event_id.is_none()
            }
            ExternalBindingState::Closed => {
                binding.closed_at.is_some() && binding.final_event_id.is_some()
            }
        };
    if valid {
        Ok(())
    } else {
        Err(SqlError::InvalidQuery)
    }
}

fn require_one(changed: usize) -> Result<(), SqlError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(SqlError::QueryReturnedNoRows)
    }
}

fn is_constraint_violation(error: &SqlError) -> bool {
    matches!(
        error,
        SqlError::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use model_core::event::{
        DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, LossPayload,
    };
    use model_core::ids::CollectorName;
    use model_core::process::ProcessIdentity;

    use super::*;

    fn binding(trace_id: u64) -> ExternalCgroupBinding {
        ExternalCgroupBinding::active(
            TraceId::new(trace_id),
            ContainerRuntime::Docker,
            NormalizedContainerId::from_lower_hex(&"ab".repeat(32)).unwrap(),
            format!("system.slice/docker-{}.scope", "ab".repeat(32)),
            u64::MAX,
            (i64::MAX as u64) + 19,
            HostBootId::from_bytes([0x5a; 16]),
            UNIX_EPOCH + Duration::from_secs(10),
        )
    }

    fn final_event(trace_id: TraceId, event_id: u64) -> DomainEvent {
        DomainEvent::new(
            EventEnvelope {
                event_id: EventId::new(event_id),
                trace_id,
                observed_at: UNIX_EPOCH + Duration::from_secs(40),
                process: ProcessIdentity::new(3),
                collector: CollectorName::new("resource-metrics"),
                kind: EventKind::Loss,
                flags: EventFlags::clean(),
            },
            EventPayload::Loss(LossPayload {
                reason: "external binding closed".to_string(),
                fatal: false,
            }),
        )
    }

    #[test]
    fn lifecycle_round_trips_unsigned_identity_and_failure_state() {
        let mut storage = SqliteStorage::open_in_memory().unwrap();
        let expected = binding(41);
        storage.create_external_cgroup_binding(&expected).unwrap();
        storage.create_external_cgroup_binding(&expected).unwrap();
        assert_eq!(
            storage
                .get_external_cgroup_binding(expected.trace_id)
                .unwrap(),
            Some(expected.clone())
        );

        let failure_time = UNIX_EPOCH + Duration::from_secs(20);
        assert_eq!(
            storage
                .record_external_cgroup_failure(expected.trace_id, failure_time)
                .unwrap(),
            1
        );
        assert_eq!(
            storage
                .record_external_cgroup_failure(expected.trace_id, failure_time)
                .unwrap(),
            2
        );
        let success_time = UNIX_EPOCH + Duration::from_secs(30);
        storage
            .record_external_cgroup_success(expected.trace_id, success_time)
            .unwrap();
        let stored = storage
            .get_external_cgroup_binding(expected.trace_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.consecutive_failures, 0);
        assert_eq!(stored.last_good_at, Some(success_time));
        assert_eq!(stored.cgroup_device, u64::MAX);
        assert_eq!(stored.cgroup_inode, (i64::MAX as u64) + 19);
    }

    #[test]
    fn stale_is_one_way_and_final_event_close_is_idempotent() {
        let mut storage = SqliteStorage::open_in_memory().unwrap();
        let expected = binding(42);
        storage.create_external_cgroup_binding(&expected).unwrap();
        let stale_at = UNIX_EPOCH + Duration::from_secs(25);
        storage
            .mark_external_cgroup_stale(
                expected.trace_id,
                ExternalBindingStaleReason::RootMoved,
                stale_at,
            )
            .unwrap();
        storage
            .mark_external_cgroup_stale(
                expected.trace_id,
                ExternalBindingStaleReason::RootMoved,
                stale_at,
            )
            .unwrap();
        assert!(
            storage
                .record_external_cgroup_success(expected.trace_id, stale_at)
                .is_err()
        );

        let first = final_event(expected.trace_id, 900);
        let closed_at = UNIX_EPOCH + Duration::from_secs(50);
        assert_eq!(
            storage
                .append_final_event_and_close_external_binding(first, closed_at)
                .unwrap(),
            EventId::new(900)
        );
        assert_eq!(
            storage
                .append_final_event_and_close_external_binding(
                    final_event(expected.trace_id, 901),
                    closed_at,
                )
                .unwrap(),
            EventId::new(900)
        );
        let event_count: u64 = storage
            .connection()
            .borrow()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE trace_id = ?1",
                [expected.trace_id.get()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 1);
        let stored = storage
            .get_external_cgroup_binding(expected.trace_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.lifecycle_state, ExternalBindingState::Closed);
        assert_eq!(stored.final_event_id, Some(EventId::new(900)));
    }

    #[test]
    fn conflicting_create_and_invalid_transition_are_rejected() {
        let mut storage = SqliteStorage::open_in_memory().unwrap();
        let expected = binding(43);
        storage.create_external_cgroup_binding(&expected).unwrap();
        let mut conflicting = expected.clone();
        conflicting.cgroup_inode = 7;
        assert!(
            storage
                .create_external_cgroup_binding(&conflicting)
                .is_err()
        );
        assert!(
            storage
                .mark_external_cgroup_stale(
                    expected.trace_id,
                    ExternalBindingStaleReason::PermissionDenied,
                    UNIX_EPOCH + Duration::from_secs(30),
                )
                .is_ok()
        );
        assert!(
            storage
                .mark_external_cgroup_stale(
                    expected.trace_id,
                    ExternalBindingStaleReason::BoundaryMissing,
                    UNIX_EPOCH + Duration::from_secs(31),
                )
                .is_err()
        );
    }
}
