use std::os::fd::AsRawFd;

use libbpf_rs::{ErrorKind, MapCore, MapFlags, MapHandle, Object};
use model_core::ids::TraceId;

use super::{
    DeleteOutcome, LaunchBindingTarget, LoaderError, PENDING_EXEC_BINDING_VALUE_SIZE, map_handle,
};

pub(super) struct Adapter {
    task_storage: MapHandle,
}

pub(super) struct Reservation;

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
        Ok(Self { task_storage })
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
        Ok(Reservation)
    }

    pub(super) fn publish(
        &self,
        target: &LaunchBindingTarget,
        _reservation: &Reservation,
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
            })
    }

    pub(super) fn delete(
        &self,
        target: &LaunchBindingTarget,
        _reservation: &Reservation,
    ) -> DeleteOutcome {
        match self
            .task_storage
            .delete(&target.pidfd.as_raw_fd().to_ne_bytes())
        {
            Ok(()) => DeleteOutcome::Deleted,
            Err(error) if error.kind() == ErrorKind::NotFound => DeleteOutcome::Missing,
            Err(error) => DeleteOutcome::Failed(LoaderError::new(
                "launch_binding",
                format!("delete pending task storage: {error}"),
            )),
        }
    }
}
