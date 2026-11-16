//! Durable runtime-owned cgroup binding lifecycle.

use std::path::PathBuf;
use std::time::SystemTime;

use model_core::container::{ContainerRuntime, NormalizedContainerId};
use model_core::ids::TraceId;
use resource_profiling::model::cgroup_v2_binding::CgroupV2Binding;
use resource_profiling::model::cgroup_v2_binding::CgroupV2BindingStaleReason;
use resource_profiling::model::cgroup_v2_binding::CgroupV2BindingState;
use resource_profiling::model::cgroup_v2_binding::HostBootId;
use rusqlite::{Error as SqlError, OptionalExtension, Row, params};

use crate::SqliteStorage;
use crate::records::{decode_time, encode_time};

impl SqliteStorage {
    pub fn create_cgroup_v2_binding(&mut self, binding: &CgroupV2Binding) -> Result<(), SqlError> {
        validate_binding(binding)?;
        let connection = self.connection().borrow_mut();
        let container_id = binding.container_id.to_string();
        let relative_path = binding.relative_path.to_string_lossy().into_owned();
        let cgroup_device = binding.cgroup_device.to_be_bytes();
        let cgroup_inode = binding.cgroup_inode.to_be_bytes();
        let host_boot_id = binding.host_boot_id.as_bytes();
        match connection.execute(
            "INSERT INTO trace_external_cgroup_v2_bindings (
                trace_id, runtime, container_id, relative_path,
                cgroup_device_be, cgroup_inode_be, host_boot_id,
                lifecycle_state, stale_reason, consecutive_failures,
                created_at, updated_at, last_good_at, closed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
            ],
        ) {
            Ok(_) => Ok(()),
            Err(error) if is_constraint_violation(&error) => {
                drop(connection);
                if self.get_cgroup_v2_binding(binding.trace_id)?.as_ref() == Some(binding) {
                    Ok(())
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }

    pub fn get_cgroup_v2_binding(
        &self,
        trace_id: TraceId,
    ) -> Result<Option<CgroupV2Binding>, SqlError> {
        self.connection()
            .borrow()
            .query_row(
                &format!("{} WHERE trace_id = ?1", v2_binding_select()),
                [trace_id.get()],
                decode_binding,
            )
            .optional()
    }

    pub fn list_live_cgroup_v2_bindings(&self) -> Result<Vec<CgroupV2Binding>, SqlError> {
        let connection = self.connection().borrow();
        let mut statement = connection.prepare(&format!(
            "{} WHERE lifecycle_state IN ('active', 'stale') ORDER BY trace_id",
            v2_binding_select()
        ))?;
        statement
            .query_map([], decode_binding)?
            .collect::<Result<Vec<_>, _>>()
    }

    pub fn record_cgroup_v2_binding_success(
        &mut self,
        trace_id: TraceId,
        observed_at: SystemTime,
    ) -> Result<(), SqlError> {
        require_one(self.connection().borrow_mut().execute(
            "UPDATE trace_external_cgroup_v2_bindings
             SET consecutive_failures = 0, last_good_at = ?2, updated_at = ?2
             WHERE trace_id = ?1 AND lifecycle_state = 'active'",
            params![trace_id.get(), encode_time(observed_at)],
        )?)
    }

    pub fn record_cgroup_v2_binding_failure(
        &mut self,
        trace_id: TraceId,
        observed_at: SystemTime,
    ) -> Result<u32, SqlError> {
        require_one(self.connection().borrow_mut().execute(
            "UPDATE trace_external_cgroup_v2_bindings
             SET consecutive_failures = consecutive_failures + 1, updated_at = ?2
             WHERE trace_id = ?1 AND lifecycle_state = 'active'
                   AND consecutive_failures < 4294967295",
            params![trace_id.get(), encode_time(observed_at)],
        )?)?;
        self.connection().borrow().query_row(
            "SELECT consecutive_failures FROM trace_external_cgroup_v2_bindings WHERE trace_id = ?1",
            [trace_id.get()],
            |row| row.get(0),
        )
    }

    pub fn mark_cgroup_v2_binding_stale(
        &mut self,
        trace_id: TraceId,
        reason: CgroupV2BindingStaleReason,
        updated_at: SystemTime,
    ) -> Result<(), SqlError> {
        let connection = self.connection().borrow_mut();
        let changed = connection.execute(
            "UPDATE trace_external_cgroup_v2_bindings
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
                 FROM trace_external_cgroup_v2_bindings WHERE trace_id = ?1",
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

    pub fn close_cgroup_v2_binding(
        &mut self,
        trace_id: TraceId,
        at: SystemTime,
    ) -> Result<(), SqlError> {
        let binding = self
            .get_cgroup_v2_binding(trace_id)?
            .ok_or(SqlError::QueryReturnedNoRows)?;
        if binding.lifecycle_state == CgroupV2BindingState::Closed {
            return Ok(());
        }
        require_one(self.connection().borrow().execute("UPDATE trace_external_cgroup_v2_bindings SET lifecycle_state='closed',closed_at=?2,updated_at=?2 WHERE trace_id=?1 AND lifecycle_state IN ('active','stale')",params![trace_id.get(),encode_time(at)])?)
    }
}

fn v2_binding_select() -> &'static str {
    "SELECT trace_id, runtime, container_id, relative_path,
            cgroup_device_be, cgroup_inode_be, host_boot_id,
            lifecycle_state, stale_reason, consecutive_failures,
            created_at, updated_at, last_good_at, closed_at
     FROM trace_external_cgroup_v2_bindings"
}

fn decode_binding(row: &Row<'_>) -> Result<CgroupV2Binding, SqlError> {
    let runtime = ContainerRuntime::from_storage_str(&row.get::<_, String>("runtime")?)
        .ok_or(SqlError::InvalidQuery)?;
    let container_id =
        NormalizedContainerId::from_lower_hex(&row.get::<_, String>("container_id")?)
            .map_err(|_| SqlError::InvalidQuery)?;
    let lifecycle_state =
        CgroupV2BindingState::from_storage_str(&row.get::<_, String>("lifecycle_state")?)
            .ok_or(SqlError::InvalidQuery)?;
    let stale_reason = row
        .get::<_, Option<String>>("stale_reason")?
        .map(|raw| CgroupV2BindingStaleReason::from_storage_str(&raw).ok_or(SqlError::InvalidQuery))
        .transpose()?;
    let host_boot_id = HostBootId::from_bytes(decode_array(row.get("host_boot_id")?)?);
    let binding = CgroupV2Binding {
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
    };
    validate_binding(&binding)?;
    Ok(binding)
}

fn decode_array<const N: usize>(bytes: Vec<u8>) -> Result<[u8; N], SqlError> {
    bytes.try_into().map_err(|_| SqlError::InvalidQuery)
}

fn validate_binding(binding: &CgroupV2Binding) -> Result<(), SqlError> {
    let valid_runtime = !matches!(binding.runtime, ContainerRuntime::Unknown);
    let valid = valid_runtime
        && match binding.lifecycle_state {
            CgroupV2BindingState::Active => {
                binding.stale_reason.is_none() && binding.closed_at.is_none()
            }
            CgroupV2BindingState::Stale => {
                binding.stale_reason.is_some() && binding.closed_at.is_none()
            }
            CgroupV2BindingState::Closed => binding.closed_at.is_some(),
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
