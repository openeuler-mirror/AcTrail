use std::os::fd::AsRawFd;

use libbpf_rs::{ErrorKind, MapCore, MapFlags, MapHandle, Object};
use model_core::ids::TraceId;

use super::{
    DeleteOutcome, LaunchBindingTarget, LoaderError, PENDING_EXEC_BINDING_VALUE_SIZE,
    configure_generation_ticks, map_handle,
};

pub(super) struct Adapter {
    task_storage: MapHandle,
    observer_fallback: MapHandle,
    _generation_tick_ns: MapHandle,
}

pub(super) struct Reservation {
    observer_key: [u8; 4],
}

impl Adapter {
    pub(super) fn from_object(object: &Object) -> Result<Self, LoaderError> {
        let task_storage = map_handle(object, "pending_exec_bindings", "launch_binding")?;
        if task_storage.key_size() as usize != std::mem::size_of::<i32>()
            || task_storage.value_size() as usize != PENDING_EXEC_BINDING_VALUE_SIZE
        {
            return Err(LoaderError::new(
                "launch_binding",
                format!(
                    "unexpected task-storage ABI key_size={} value_size={}",
                    task_storage.key_size(),
                    task_storage.value_size()
                ),
            ));
        }
        let observer_fallback =
            map_handle(object, "pending_exec_observer_bindings", "launch_binding")?;
        if observer_fallback.key_size() as usize != std::mem::size_of::<u32>()
            || observer_fallback.value_size() as usize != PENDING_EXEC_BINDING_VALUE_SIZE
        {
            return Err(LoaderError::new(
                "launch_binding",
                format!(
                    "unexpected observer fallback ABI key_size={} value_size={}",
                    observer_fallback.key_size(),
                    observer_fallback.value_size()
                ),
            ));
        }
        let generation_tick_ns =
            map_handle(object, "pending_exec_generation_tick_ns", "launch_binding")?;
        configure_generation_ticks(&generation_tick_ns, "launch_binding")?;
        Ok(Self {
            task_storage,
            observer_fallback,
            _generation_tick_ns: generation_tick_ns,
        })
    }

    pub(super) fn reserve(
        &self,
        target: &LaunchBindingTarget,
        _trace_id: TraceId,
        value: &[u8; PENDING_EXEC_BINDING_VALUE_SIZE],
    ) -> Result<Reservation, LoaderError> {
        self.task_storage
            .update(
                &target.pidfd.as_raw_fd().to_ne_bytes(),
                value,
                MapFlags::NO_EXIST,
            )
            .map_err(|error| LoaderError::new("launch_binding", error.to_string()))?;
        let observer_key = target.observer_tgid.to_ne_bytes();
        if let Err(error) = self
            .observer_fallback
            .update(&observer_key, value, MapFlags::NO_EXIST)
        {
            let rollback = self
                .task_storage
                .delete(&target.pidfd.as_raw_fd().to_ne_bytes());
            return Err(LoaderError::new(
                "launch_binding_rollback",
                match rollback {
                    Ok(()) => format!("reserve observer fallback: {error}"),
                    Err(rollback_error) => format!(
                        "reserve observer fallback failed: {error}; task-storage rollback failed: {rollback_error}"
                    ),
                },
            ));
        }
        Ok(Reservation { observer_key })
    }

    pub(super) fn publish(
        &self,
        target: &LaunchBindingTarget,
        reservation: &Reservation,
        value: &[u8; PENDING_EXEC_BINDING_VALUE_SIZE],
    ) -> Result<(), LoaderError> {
        self.task_storage
            .update(
                &target.pidfd.as_raw_fd().to_ne_bytes(),
                value,
                MapFlags::EXIST,
            )
            .map_err(|error| {
                LoaderError::new(
                    "launch_binding",
                    format!("publish counted task-storage binding: {error}"),
                )
            })?;
        self.observer_fallback
            .update(&reservation.observer_key, value, MapFlags::EXIST)
            .map_err(|error| {
                LoaderError::new(
                    "launch_binding",
                    format!("publish counted observer fallback: {error}"),
                )
            })
    }

    pub(super) fn delete(
        &self,
        target: &LaunchBindingTarget,
        reservation: &Reservation,
    ) -> DeleteOutcome {
        let task_result = self
            .task_storage
            .delete(&target.pidfd.as_raw_fd().to_ne_bytes());
        let fallback_result = self.observer_fallback.delete(&reservation.observer_key);
        if let Err(error) = &task_result
            && error.kind() != ErrorKind::NotFound
        {
            return DeleteOutcome::Failed(LoaderError::new(
                "launch_binding",
                format!("delete pending task storage: {error}"),
            ));
        }
        if let Err(error) = &fallback_result
            && error.kind() != ErrorKind::NotFound
        {
            return if task_result.is_ok() {
                DeleteOutcome::DeletedWithCleanupFailure(LoaderError::new(
                    "launch_binding",
                    format!("delete observer fallback: {error}"),
                ))
            } else {
                DeleteOutcome::Failed(LoaderError::new(
                    "launch_binding",
                    format!("delete observer fallback: {error}"),
                ))
            };
        }
        if task_result.is_err() && fallback_result.is_err() {
            DeleteOutcome::Missing
        } else {
            DeleteOutcome::Deleted
        }
    }
}
