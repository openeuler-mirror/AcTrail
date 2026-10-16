use crate::{
    SqliteStorage,
    records::{decode_time, encode_time},
};
use model_core::host_identity::HostBootId;
use model_core::ids::TraceId;
use resource_profiling::model::cgroup_v1_binding::CgroupV1Binding;
use rusqlite::{OptionalExtension, Row, params};
use std::time::SystemTime;
use storage_core::{CgroupV1BindingStore, StorageError};
struct BindingRow;
impl BindingRow {
    fn error(error: impl std::fmt::Display) -> StorageError {
        StorageError::new("cgroup_v1_storage", error.to_string())
    }
    fn decode(row: &Row<'_>) -> Result<CgroupV1Binding, rusqlite::Error> {
        Ok(CgroupV1Binding {
            trace_id: TraceId::new(row.get("trace_id")?),
            host_boot_id: HostBootId::from_bytes(row.get("host_boot_id")?),
            cpuacct: serde_json::from_str(&row.get::<_, String>("cpuacct")?)
                .map_err(|_| rusqlite::Error::InvalidQuery)?,
            memory: serde_json::from_str(&row.get::<_, String>("memory")?)
                .map_err(|_| rusqlite::Error::InvalidQuery)?,
            created_at: decode_time(row.get("created_at")?),
            stale_reason: row.get("stale_reason")?,
            closed_at: row.get::<_, Option<i64>>("closed_at")?.map(decode_time),
        })
    }
}
impl CgroupV1BindingStore for SqliteStorage {
    fn create_cgroup_v1_binding(&mut self, binding: &CgroupV1Binding) -> Result<(), StorageError> {
        if let Some(current) = self.cgroup_v1_binding(binding.trace_id)? {
            return if current == *binding {
                Ok(())
            } else {
                Err(BindingRow::error("conflicting v1 binding"))
            };
        }
        if binding.closed_at.is_some() {
            return Err(BindingRow::error("admission binding already closed"));
        }
        self.connection().borrow().execute("INSERT INTO trace_cgroup_v1_bindings (trace_id,host_boot_id,cpuacct,memory,created_at,stale_reason) VALUES (?1,?2,?3,?4,?5,?6)",
            params![binding.trace_id.get(),binding.host_boot_id.as_bytes().as_slice(),
                serde_json::to_string(&binding.cpuacct).map_err(BindingRow::error)?,serde_json::to_string(&binding.memory).map_err(BindingRow::error)?,encode_time(binding.created_at),binding.stale_reason]).map_err(BindingRow::error)?;
        Ok(())
    }
    fn cgroup_v1_binding(&self, id: TraceId) -> Result<Option<CgroupV1Binding>, StorageError> {
        self.connection()
            .borrow()
            .query_row(
                "SELECT * FROM trace_cgroup_v1_bindings WHERE trace_id=?1",
                [id.get()],
                BindingRow::decode,
            )
            .optional()
            .map_err(BindingRow::error)
    }
    fn live_cgroup_v1_bindings(&self) -> Result<Vec<CgroupV1Binding>, StorageError> {
        let connection = self.connection().borrow();
        let mut statement = connection
            .prepare("SELECT * FROM trace_cgroup_v1_bindings WHERE closed_at IS NULL")
            .map_err(BindingRow::error)?;
        statement
            .query_map([], BindingRow::decode)
            .map_err(BindingRow::error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(BindingRow::error)
    }
    fn mark_cgroup_v1_stale(&mut self, id: TraceId, reason: &str) -> Result<(), StorageError> {
        let changed=self.connection().borrow().execute("UPDATE trace_cgroup_v1_bindings SET stale_reason=COALESCE(stale_reason,?2) WHERE trace_id=?1 AND closed_at IS NULL",params![id.get(),reason]).map_err(BindingRow::error)?;
        if changed != 1 {
            return Err(BindingRow::error("live v1 binding missing"));
        }
        Ok(())
    }

    fn close_cgroup_v1_binding(&mut self, id: TraceId, at: SystemTime) -> Result<(), StorageError> {
        let current = self
            .cgroup_v1_binding(id)?
            .ok_or_else(|| BindingRow::error("v1 binding missing"))?;
        if current.closed_at.is_some() {
            return Ok(());
        }
        let changed=self.connection().borrow().execute("UPDATE trace_cgroup_v1_bindings SET closed_at=?2 WHERE trace_id=?1 AND closed_at IS NULL",params![id.get(),encode_time(at)]).map_err(BindingRow::error)?;
        if changed != 1 {
            return Err(BindingRow::error("v1 binding close failed"));
        }
        Ok(())
    }
}
