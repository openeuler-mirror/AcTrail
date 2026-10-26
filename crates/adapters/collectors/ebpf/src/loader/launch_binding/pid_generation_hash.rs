use std::os::fd::{AsFd, AsRawFd, BorrowedFd};

use libbpf_rs::{ErrorKind, MapCore, MapFlags, MapHandle, Object};
use model_core::ids::TraceId;
use process_identity::ProcessIdentityReader;

use super::{
    DeleteOutcome, LaunchBindingTarget, LoaderError, PENDING_EXEC_BINDING_VALUE_SIZE,
    configure_generation_ticks, map_handle,
};

const BINDING_KEY_SIZE: usize = 24;

pub(super) struct Adapter {
    bindings: MapHandle,
    pid_index: MapHandle,
    _generation_tick_ns: MapHandle,
}

pub(super) struct Reservation {
    binding_key: [u8; BINDING_KEY_SIZE],
    pid_key: [u8; 4],
}

impl Adapter {
    pub(super) fn from_object(object: &Object) -> Result<Self, LoaderError> {
        let bindings = map_handle(object, "pending_exec_bindings", "launch_binding")?;
        if bindings.key_size() as usize != BINDING_KEY_SIZE
            || bindings.value_size() as usize != PENDING_EXEC_BINDING_VALUE_SIZE
        {
            return Err(LoaderError::new(
                "launch_binding",
                format!(
                    "unexpected HASH binding ABI key_size={} value_size={}",
                    bindings.key_size(),
                    bindings.value_size()
                ),
            ));
        }
        let pid_index = map_handle(object, "pending_exec_pid_index", "launch_binding")?;
        if pid_index.key_size() as usize != std::mem::size_of::<u32>()
            || pid_index.value_size() as usize != BINDING_KEY_SIZE
        {
            return Err(LoaderError::new(
                "launch_binding",
                format!(
                    "unexpected PID index ABI key_size={} value_size={}",
                    pid_index.key_size(),
                    pid_index.value_size()
                ),
            ));
        }
        let generation_tick_ns =
            map_handle(object, "pending_exec_generation_tick_ns", "launch_binding")?;
        configure_generation_ticks(&generation_tick_ns, "launch_binding")?;
        Ok(Self {
            bindings,
            pid_index,
            _generation_tick_ns: generation_tick_ns,
        })
    }

    pub(super) fn reserve(
        &self,
        target: &LaunchBindingTarget,
        trace_id: TraceId,
        value: &[u8; PENDING_EXEC_BINDING_VALUE_SIZE],
    ) -> Result<Reservation, LoaderError> {
        self.validate_target(target)?;
        let reservation = Reservation {
            binding_key: Self::binding_key(target, trace_id),
            pid_key: target.observer_tgid.to_ne_bytes(),
        };
        self.bindings
            .update(&reservation.binding_key, value, MapFlags::NO_EXIST)
            .map_err(|error| {
                LoaderError::new("launch_binding", format!("reserve HASH binding: {error}"))
            })?;
        if let Err(index_error) = self.pid_index.update(
            &reservation.pid_key,
            &reservation.binding_key,
            MapFlags::NO_EXIST,
        ) {
            let rollback = self.bindings.delete(&reservation.binding_key);
            return Err(match rollback {
                Ok(()) => LoaderError::new(
                    "launch_binding",
                    format!("reserve PID index: {index_error}"),
                ),
                Err(rollback_error) => LoaderError::new(
                    "launch_binding_rollback",
                    format!(
                        "reserve PID index failed: {index_error}; binding rollback failed: {rollback_error}"
                    ),
                ),
            });
        }
        Ok(reservation)
    }

    pub(super) fn publish(
        &self,
        _target: &LaunchBindingTarget,
        reservation: &Reservation,
        value: &[u8; PENDING_EXEC_BINDING_VALUE_SIZE],
    ) -> Result<(), LoaderError> {
        self.bindings
            .update(&reservation.binding_key, value, MapFlags::EXIST)
            .map_err(|error| {
                LoaderError::new(
                    "launch_binding",
                    format!("publish counted HASH binding: {error}"),
                )
            })
    }

    pub(super) fn delete(
        &self,
        _target: &LaunchBindingTarget,
        reservation: &Reservation,
    ) -> DeleteOutcome {
        match self.bindings.delete(&reservation.binding_key) {
            Ok(()) => match self.pid_index.delete(&reservation.pid_key) {
                Ok(()) => DeleteOutcome::Deleted,
                Err(error) => DeleteOutcome::DeletedWithCleanupFailure(LoaderError::new(
                    "launch_binding",
                    format!("binding deleted but PID index cleanup failed: {error}"),
                )),
            },
            Err(error) if error.kind() == ErrorKind::NotFound => DeleteOutcome::Missing,
            Err(error) => DeleteOutcome::Failed(LoaderError::new(
                "launch_binding",
                format!("delete pending HASH binding: {error}"),
            )),
        }
    }

    fn validate_target(&self, target: &LaunchBindingTarget) -> Result<(), LoaderError> {
        let pidfd = target.pidfd.as_fd();
        let observed_pid = Self::pidfd_observer_pid(pidfd)?;
        if observed_pid != target.observer_tgid {
            return Err(LoaderError::new(
                "launch_binding_target",
                format!(
                    "pidfd identifies observer PID {observed_pid}, not requested observer TGID {}",
                    target.observer_tgid
                ),
            ));
        }
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd.as_raw_fd(),
                0,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result != 0 {
            return Err(LoaderError::new(
                "launch_binding_target",
                format!(
                    "pidfd target is not signalable: {}",
                    std::io::Error::last_os_error()
                ),
            ));
        }
        let observation = crate::procfs::ProcfsIdentityReader
            .read_identity(target.observer_tgid)
            .map_err(|error| {
                LoaderError::new(
                    "launch_binding_target",
                    format!(
                        "read generation for pidfd target {}: {error:?}",
                        target.observer_tgid
                    ),
                )
            })?;
        let observed_generation = observation
            .host
            .as_ref()
            .map(|host| host.start_time_ticks)
            .ok_or_else(|| {
                LoaderError::new(
                    "launch_binding_target",
                    format!(
                        "pidfd target {} has no observer generation",
                        target.observer_tgid
                    ),
                )
            })?;
        if observed_generation != target.generation {
            return Err(LoaderError::new(
                "launch_binding_target",
                format!(
                    "pidfd target {} has procfs generation {observed_generation}, not requested generation {}",
                    target.observer_tgid, target.generation
                ),
            ));
        }
        Ok(())
    }

    fn pidfd_observer_pid(pidfd: BorrowedFd<'_>) -> Result<u32, LoaderError> {
        let path = format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd());
        let content = std::fs::read_to_string(&path).map_err(|error| {
            LoaderError::new("launch_binding_target", format!("read {path}: {error}"))
        })?;
        content
            .lines()
            .find_map(|line| line.strip_prefix("Pid:").map(str::trim))
            .ok_or_else(|| {
                LoaderError::new(
                    "launch_binding_target",
                    format!("{path} does not expose a Pid field"),
                )
            })?
            .parse::<u32>()
            .map_err(|error| {
                LoaderError::new(
                    "launch_binding_target",
                    format!("invalid Pid field in {path}: {error}"),
                )
            })
    }

    fn binding_key(target: &LaunchBindingTarget, trace_id: TraceId) -> [u8; BINDING_KEY_SIZE] {
        let mut key = [0_u8; BINDING_KEY_SIZE];
        key[0..4].copy_from_slice(&target.observer_tgid.to_ne_bytes());
        key[8..16].copy_from_slice(&target.generation.to_ne_bytes());
        key[16..24].copy_from_slice(&trace_id.get().to_ne_bytes());
        key
    }
}
