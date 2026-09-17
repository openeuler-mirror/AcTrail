//! Durable trace resource-scope registry.

use std::path::PathBuf;
use std::time::SystemTime;

use model_core::event::ResourceAccountingMethod;
use model_core::ids::{EventId, TraceId};
use model_core::resource_scope::{ResourceScopeLifecycleState, TraceResourceScope};
use rusqlite::{Error as SqlError, OptionalExtension, Row, params};

use crate::SqliteStorage;
use crate::records::{decode_time, encode_time};

impl SqliteStorage {
    pub fn create_resource_scope(&mut self, scope: &TraceResourceScope) -> Result<(), SqlError> {
        let connection = self.connection().borrow_mut();
        match connection.execute(
            "INSERT INTO trace_resource_scopes (
                trace_id, nonce, relative_path, accounting_method, lifecycle_state,
                created_at, final_event_id, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                scope.trace_id.get(),
                scope.nonce,
                scope.relative_path.to_string_lossy(),
                encode_accounting_method(scope.accounting_method),
                scope.lifecycle_state.as_storage_str(),
                encode_time(scope.created_at),
                scope.final_event_id.map(EventId::get),
                encode_time(scope.updated_at),
            ],
        ) {
            Ok(_) => Ok(()),
            Err(error) if is_constraint_violation(&error) => {
                drop(connection);
                if self.get_resource_scope(scope.trace_id)?.as_ref() == Some(scope) {
                    Ok(())
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }

    pub fn get_resource_scope(
        &self,
        trace_id: TraceId,
    ) -> Result<Option<TraceResourceScope>, SqlError> {
        self.connection()
            .borrow()
            .query_row(
                "SELECT trace_id, nonce, relative_path, accounting_method, lifecycle_state,
                        created_at, final_event_id, updated_at
                 FROM trace_resource_scopes WHERE trace_id = ?1",
                [trace_id.get()],
                decode_scope,
            )
            .optional()
    }

    pub fn list_resource_scopes(&self) -> Result<Vec<TraceResourceScope>, SqlError> {
        let connection = self.connection().borrow();
        let mut statement = connection.prepare(
            "SELECT trace_id, nonce, relative_path, accounting_method, lifecycle_state,
                    created_at, final_event_id, updated_at
             FROM trace_resource_scopes ORDER BY trace_id",
        )?;
        statement
            .query_map([], decode_scope)?
            .collect::<Result<Vec<_>, _>>()
    }

    pub fn update_resource_scope_state(
        &mut self,
        trace_id: TraceId,
        lifecycle_state: ResourceScopeLifecycleState,
        final_event_id: Option<EventId>,
        updated_at: SystemTime,
    ) -> Result<(), SqlError> {
        if !matches!(
            lifecycle_state,
            ResourceScopeLifecycleState::Finalized | ResourceScopeLifecycleState::Orphaned
        ) && final_event_id.is_some()
        {
            return Err(SqlError::InvalidQuery);
        }
        let changed = self.connection().borrow_mut().execute(
            "UPDATE trace_resource_scopes
             SET lifecycle_state = ?2, final_event_id = ?3, updated_at = ?4
             WHERE trace_id = ?1",
            params![
                trace_id.get(),
                lifecycle_state.as_storage_str(),
                final_event_id.map(EventId::get),
                encode_time(updated_at),
            ],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(SqlError::QueryReturnedNoRows)
        }
    }
}

fn decode_scope(row: &Row<'_>) -> Result<TraceResourceScope, SqlError> {
    let accounting_method = decode_accounting_method(&row.get::<_, String>("accounting_method")?)?;
    let lifecycle_state =
        ResourceScopeLifecycleState::from_storage_str(&row.get::<_, String>("lifecycle_state")?)
            .ok_or(SqlError::InvalidQuery)?;
    Ok(TraceResourceScope {
        trace_id: TraceId::new(row.get("trace_id")?),
        nonce: row.get("nonce")?,
        relative_path: PathBuf::from(row.get::<_, String>("relative_path")?),
        accounting_method,
        lifecycle_state,
        created_at: decode_time(row.get("created_at")?),
        final_event_id: row
            .get::<_, Option<u64>>("final_event_id")?
            .map(EventId::new),
        updated_at: decode_time(row.get("updated_at")?),
    })
}

fn encode_accounting_method(method: ResourceAccountingMethod) -> &'static str {
    method.as_str()
}

fn decode_accounting_method(raw: &str) -> Result<ResourceAccountingMethod, SqlError> {
    match raw {
        "cgroup_v2" => Ok(ResourceAccountingMethod::CgroupV2),
        "procfs_rss_sum" => Ok(ResourceAccountingMethod::ProcfsRssSum),
        _ => Err(SqlError::InvalidQuery),
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

    use super::*;

    #[test]
    fn registry_is_idempotent_and_rejects_a_second_scope_for_the_trace() {
        let mut storage = SqliteStorage::open_in_memory().unwrap();
        let scope = TraceResourceScope::active(
            TraceId::new(7),
            "nonce1",
            "traces/trace-7-nonce1",
            ResourceAccountingMethod::CgroupV2,
            UNIX_EPOCH + Duration::from_secs(10),
        );
        storage.create_resource_scope(&scope).unwrap();
        storage.create_resource_scope(&scope).unwrap();
        assert_eq!(
            storage.get_resource_scope(scope.trace_id).unwrap(),
            Some(scope.clone())
        );

        let conflicting = TraceResourceScope::active(
            scope.trace_id,
            "nonce2",
            "traces/trace-7-nonce2",
            ResourceAccountingMethod::CgroupV2,
            scope.created_at,
        );
        assert!(storage.create_resource_scope(&conflicting).is_err());
    }

    #[test]
    fn registry_state_and_final_event_round_trip() {
        let mut storage = SqliteStorage::open_in_memory().unwrap();
        let scope = TraceResourceScope::active(
            TraceId::new(8),
            "nonce",
            "traces/trace-8-nonce",
            ResourceAccountingMethod::CgroupV2,
            UNIX_EPOCH + Duration::from_secs(20),
        );
        storage.create_resource_scope(&scope).unwrap();
        let updated_at = UNIX_EPOCH + Duration::from_secs(30);
        storage
            .update_resource_scope_state(
                scope.trace_id,
                ResourceScopeLifecycleState::Finalized,
                Some(EventId::new(99)),
                updated_at,
            )
            .unwrap();

        let stored = storage.get_resource_scope(scope.trace_id).unwrap().unwrap();
        assert_eq!(
            stored.lifecycle_state,
            ResourceScopeLifecycleState::Finalized
        );
        assert_eq!(stored.final_event_id, Some(EventId::new(99)));
        assert_eq!(stored.updated_at, updated_at);
        assert_eq!(storage.list_resource_scopes().unwrap(), vec![stored]);
    }
}
