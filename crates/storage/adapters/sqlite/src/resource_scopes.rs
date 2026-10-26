//! Durable trace resource-scope registry.

use std::path::PathBuf;
use std::time::SystemTime;

use model_core::ids::TraceId;
use resource_profiling::model::ResourceAccountingMethod;
use resource_profiling::model::resource_scope::ResourceScopeLifecycleState;
use resource_profiling::model::resource_scope::TraceResourceScope;
use rusqlite::{Error as SqlError, OptionalExtension, Row, params};

use crate::SqliteStorage;
use crate::records::{decode_time, encode_time};

impl SqliteStorage {
    pub fn create_resource_scope(&mut self, scope: &TraceResourceScope) -> Result<(), SqlError> {
        let connection = self.connection().borrow_mut();
        match connection.execute(
            "INSERT INTO trace_resource_scopes (
                trace_id, nonce, relative_path, accounting_method, lifecycle_state,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                scope.trace_id.get(),
                scope.nonce,
                scope.relative_path.to_string_lossy(),
                encode_accounting_method(scope.accounting_method),
                scope.lifecycle_state.as_storage_str(),
                encode_time(scope.created_at),
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
                        created_at, updated_at
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
                    created_at, updated_at
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
        updated_at: SystemTime,
    ) -> Result<(), SqlError> {
        let changed = self.connection().borrow_mut().execute(
            "UPDATE trace_resource_scopes SET lifecycle_state=?2,updated_at=?3 WHERE trace_id=?1",
            params![
                trace_id.get(),
                lifecycle_state.as_storage_str(),
                encode_time(updated_at)
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
        updated_at: decode_time(row.get("updated_at")?),
    })
}

fn encode_accounting_method(method: ResourceAccountingMethod) -> &'static str {
    method.as_str()
}

fn decode_accounting_method(raw: &str) -> Result<ResourceAccountingMethod, SqlError> {
    match raw {
        "cgroup_v1" => Ok(ResourceAccountingMethod::CgroupV1),
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
